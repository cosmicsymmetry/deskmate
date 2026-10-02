export const PROTOCOL_VERSION = 2;

export const MessageType = {
  StatusRequest: 1,
  StatusResponse: 2,
  Ack: 4,
  Error: 8,
  NetworkConfig: 13,
} as const;

export const CAPABILITY_NETWORKING = 1n << 7n;

export type NetworkConfig = {
  ssid: string;
  psk: string;
  serverUrl: string;
  deviceId: string;
  token: string;
  utcOffsetMinutes: number;
  tier: 0 | 1;
};

export type StatusResponse = {
  protocolVersion: number;
  firmwareVersion: string;
  capabilities: bigint;
  tier: number;
  wifiState: 0 | 1 | 2 | 3;
  ip: string;
  lastNetworkError?: string;
};

export type Ack = { acknowledgedType: number };
export type DeviceError = { code: number; diagnostic: string };

export type Decoded =
  | { type: "status"; requestId: number; status: StatusResponse }
  | { type: "ack"; requestId: number; ack: Ack }
  | { type: "error"; requestId: number; error: DeviceError }
  | { type: "other"; requestId: number; messageType: number };

type Invalid = { type: "invalid"; reason: string };

const MAX_DECODED_FRAME = 2048;
const MAX_PAYLOAD_SIZE = MAX_DECODED_FRAME - 14;
const MAX_WIRE_FRAME = 2058;
const MAX_ENCODED_WITHOUT_DELIMITER = MAX_WIRE_FRAME - 1;
const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder("utf-8", { fatal: true });

class CodecError extends Error {
  constructor(readonly reason: string) {
    super(reason);
  }
}

const fail = (reason: string): never => {
  throw new CodecError(reason);
};

function requiredValue<T>(value: T | undefined, reason: string): T {
  if (value === undefined) throw new CodecError(reason);
  return value;
}

export function crc32c(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ ((crc & 1) === 0 ? 0 : 0x82f63b78);
    }
  }
  return ~crc >>> 0;
}

class CborEncoder {
  readonly bytes: number[] = [];

  private argument(major: number, value: bigint): void {
    if (value < 0n || value > 0xffffffffffffffffn) fail("cbor integer");
    const prefix = major << 5;
    if (value <= 23n) {
      this.bytes.push(prefix | Number(value));
    } else if (value <= 0xffn) {
      this.bytes.push(prefix | 24, Number(value));
    } else if (value <= 0xffffn) {
      this.bytes.push(prefix | 25, Number(value >> 8n), Number(value & 0xffn));
    } else if (value <= 0xffffffffn) {
      this.bytes.push(prefix | 26);
      for (let shift = 24n; shift >= 0n; shift -= 8n) {
        this.bytes.push(Number((value >> shift) & 0xffn));
      }
    } else {
      this.bytes.push(prefix | 27);
      for (let shift = 56n; shift >= 0n; shift -= 8n) {
        this.bytes.push(Number((value >> shift) & 0xffn));
      }
    }
  }

  map(length: number): void {
    this.argument(5, BigInt(length));
  }

  unsigned(value: number): void {
    this.argument(0, BigInt(value));
  }

  signed(value: number): void {
    if (!Number.isSafeInteger(value)) fail("cbor integer");
    if (value >= 0) this.argument(0, BigInt(value));
    else this.argument(1, BigInt(-1 - value));
  }

  text(value: string): void {
    const encoded = textEncoder.encode(value);
    this.argument(3, BigInt(encoded.byteLength));
    this.bytes.push(...encoded);
  }
}

class CborDecoder {
  private position = 0;

  constructor(private readonly bytes: Uint8Array) {}

  private byte(): number {
    const value = this.bytes[this.position];
    if (value === undefined) fail("cbor eof");
    this.position += 1;
    return value;
  }

  private take(length: number): Uint8Array {
    const end = this.position + length;
    if (!Number.isSafeInteger(end) || end > this.bytes.byteLength) fail("cbor eof");
    const value = this.bytes.subarray(this.position, end);
    this.position = end;
    return value;
  }

  private head(): { major: number; value: bigint } {
    const first = this.byte();
    const major = first >> 5;
    const additional = first & 0x1f;
    let value = 0n;
    if (additional <= 23) {
      value = BigInt(additional);
    } else if (additional === 24) {
      value = BigInt(this.byte());
      if (value < 24n) fail("non-canonical cbor");
    } else if (additional === 25) {
      const bytes = this.take(2);
      value = BigInt((bytes[0] << 8) | bytes[1]);
      if (value <= 0xffn) fail("non-canonical cbor");
    } else if (additional === 26) {
      const bytes = this.take(4);
      value = 0n;
      for (const byte of bytes) value = (value << 8n) | BigInt(byte);
      if (value <= 0xffffn) fail("non-canonical cbor");
    } else if (additional === 27) {
      const bytes = this.take(8);
      value = 0n;
      for (const byte of bytes) value = (value << 8n) | BigInt(byte);
      if (value <= 0xffffffffn) fail("non-canonical cbor");
    } else {
      fail("indefinite cbor");
    }
    return { major, value };
  }

  private length(value: bigint): number {
    if (value > BigInt(Number.MAX_SAFE_INTEGER)) fail("cbor length");
    return Number(value);
  }

  mapLength(): number {
    const { major, value } = this.head();
    if (major !== 5) fail("cbor map");
    return this.length(value);
  }

  arrayLength(): number {
    const { major, value } = this.head();
    if (major !== 4) fail("cbor array");
    return this.length(value);
  }

  unsignedBigint(): bigint {
    const { major, value } = this.head();
    if (major !== 0) fail("cbor unsigned");
    return value;
  }

  unsigned(maximum: number): number {
    const value = this.unsignedBigint();
    if (value > BigInt(maximum)) fail("cbor integer range");
    return Number(value);
  }

  signed(minimum: number, maximum: number): number {
    const { major, value } = this.head();
    let signed = 0n;
    if (major === 0) signed = value;
    else if (major === 1) signed = -1n - value;
    else fail("cbor signed");
    if (signed < BigInt(minimum) || signed > BigInt(maximum)) fail("cbor integer range");
    return Number(signed);
  }

  text(minimum = 0, maximum = Number.MAX_SAFE_INTEGER): string {
    const { major, value } = this.head();
    if (major !== 3) fail("cbor text");
    const length = this.length(value);
    if (length < minimum || length > maximum) fail("cbor text length");
    try {
      return textDecoder.decode(this.take(length));
    } catch {
      return fail("cbor utf-8");
    }
  }

  boolean(): boolean {
    const { major, value } = this.head();
    if (major !== 7 || (value !== 20n && value !== 21n)) fail("cbor boolean");
    return value === 21n;
  }

  mapEntries(readValue: (key: bigint) => void): void {
    const length = this.mapLength();
    let previous = -1n;
    for (let index = 0; index < length; index += 1) {
      const key = this.unsignedBigint();
      if (key <= previous) fail("duplicate or unsorted cbor key");
      previous = key;
      readValue(key);
    }
  }

  skip(depth = 0): void {
    if (depth >= 8) fail("cbor nesting");
    const { major, value } = this.head();
    if (major === 0 || major === 1) return;
    if (major === 2 || major === 3) {
      const bytes = this.take(this.length(value));
      if (major === 3) {
        try {
          textDecoder.decode(bytes);
        } catch {
          fail("cbor utf-8");
        }
      }
      return;
    }
    if (major === 4) {
      for (let index = 0; index < this.length(value); index += 1) this.skip(depth + 1);
      return;
    }
    if (major === 5) {
      let previous = -1n;
      for (let index = 0; index < this.length(value); index += 1) {
        const key = this.unsignedBigint();
        if (key <= previous) fail("duplicate or unsorted cbor key");
        previous = key;
        this.skip(depth + 1);
      }
      return;
    }
    if (major === 7 && (value === 20n || value === 21n || value === 22n)) return;
    fail("cbor type");
  }

  finish(): void {
    if (this.position !== this.bytes.byteLength) fail("trailing cbor data");
  }
}

function cobsEncode(decoded: Uint8Array): Uint8Array {
  const encoded: number[] = [0];
  let codeIndex = 0;
  let code = 1;
  for (const byte of decoded) {
    if (byte === 0) {
      encoded[codeIndex] = code;
      codeIndex = encoded.length;
      encoded.push(0);
      code = 1;
    } else {
      encoded.push(byte);
      code += 1;
      if (code === 0xff) {
        encoded[codeIndex] = code;
        codeIndex = encoded.length;
        encoded.push(0);
        code = 1;
      }
    }
  }
  encoded[codeIndex] = code;
  return Uint8Array.from(encoded);
}

function cobsDecode(encoded: Uint8Array): Uint8Array {
  if (encoded.byteLength === 0) fail("cobs");
  const decoded: number[] = [];
  let position = 0;
  while (position < encoded.byteLength) {
    const code = encoded[position];
    if (code === 0) fail("cobs");
    position += 1;
    const end = position + code - 1;
    if (end > encoded.byteLength) fail("cobs");
    for (; position < end; position += 1) decoded.push(encoded[position]);
    if (code !== 0xff && position < encoded.byteLength) decoded.push(0);
    if (decoded.length > MAX_DECODED_FRAME) fail("overlong");
  }
  return Uint8Array.from(decoded);
}

export function rawDecoded(wire: Uint8Array): Uint8Array {
  if (wire.byteLength > MAX_WIRE_FRAME) fail("overlong");
  if (wire.at(-1) !== 0) fail("delimiter");
  const encoded = wire.subarray(0, wire.byteLength - 1);
  if (encoded.includes(0)) fail("embedded delimiter");
  return cobsDecode(encoded);
}

function encodeFrame(messageType: number, requestId: number, payload: Uint8Array): Uint8Array {
  if (!Number.isInteger(requestId) || requestId <= 0 || requestId > 0xffffffff) {
    fail("request id");
  }
  if (payload.byteLength > MAX_PAYLOAD_SIZE) fail("overlong");

  const decoded = new Uint8Array(10 + payload.byteLength + 4);
  const view = new DataView(decoded.buffer);
  decoded[0] = PROTOCOL_VERSION;
  decoded[1] = messageType;
  view.setUint16(2, 0, true);
  view.setUint32(4, requestId, true);
  view.setUint16(8, payload.byteLength, true);
  decoded.set(payload, 10);
  view.setUint32(decoded.byteLength - 4, crc32c(decoded.subarray(0, decoded.byteLength - 4)), true);

  const encoded = cobsEncode(decoded);
  const wire = new Uint8Array(encoded.byteLength + 1);
  wire.set(encoded);
  return wire;
}

export function encodeStatusRequest(requestId: number): Uint8Array {
  return encodeFrame(MessageType.StatusRequest, requestId, Uint8Array.of(0xa0));
}

export function validateNetworkConfig(config: NetworkConfig): string | null {
  const byteLength = (value: string) => textEncoder.encode(value).byteLength;
  if (byteLength(config.ssid) > 32) return "The network name must be at most 32 bytes.";
  if (byteLength(config.psk) > 64) return "The Wi-Fi password must be at most 64 bytes.";
  if (byteLength(config.serverUrl) > 128) return "The server URL must be at most 128 bytes.";
  if (byteLength(config.deviceId) > 32) return "The device ID must be at most 32 bytes.";
  if (byteLength(config.token) > 128) return "The device token must be at most 128 bytes.";
  if (
    !Number.isInteger(config.utcOffsetMinutes) ||
    config.utcOffsetMinutes < -840 ||
    config.utcOffsetMinutes > 840
  ) {
    return "The UTC offset must be between -840 and 840 minutes.";
  }
  if (config.tier !== 0 && config.tier !== 1) return "The panel tier must be local or networked.";
  return null;
}

export function encodeNetworkConfig(requestId: number, config: NetworkConfig): Uint8Array {
  const validationError = validateNetworkConfig(config);
  if (validationError !== null) fail(validationError);

  const payload = new CborEncoder();
  payload.map(7);
  payload.unsigned(1);
  payload.text(config.ssid);
  payload.unsigned(2);
  payload.text(config.psk);
  payload.unsigned(3);
  payload.text(config.serverUrl);
  payload.unsigned(4);
  payload.text(config.deviceId);
  payload.unsigned(5);
  payload.text(config.token);
  payload.unsigned(6);
  payload.signed(config.utcOffsetMinutes);
  payload.unsigned(7);
  payload.unsigned(config.tier);
  return encodeFrame(MessageType.NetworkConfig, requestId, Uint8Array.from(payload.bytes));
}

function decodeStatus(payload: Uint8Array): StatusResponse {
  const decoder = new CborDecoder(payload);
  const required = new Set(Array.from({ length: 16 }, (_, key) => key));
  let protocolVersion: number | undefined;
  let firmwareVersion: string | undefined;
  let maximumProtocolVersion = PROTOCOL_VERSION;
  let capabilities = 0n;
  let tier = 0;
  let wifiState: 0 | 1 | 2 | 3 = 0;
  let ip = "";
  let lastNetworkError: string | undefined;

  decoder.mapEntries((rawKey) => {
    if (rawKey > BigInt(Number.MAX_SAFE_INTEGER)) {
      decoder.skip();
      return;
    }
    const key = Number(rawKey);
    required.delete(key);
    if (key === 0) protocolVersion = decoder.unsigned(0xff);
    else if (key === 1) firmwareVersion = decoder.text(1, 32);
    else if (key === 2 || key === 23) {
      const value = decoder.unsignedBigint();
      if (key === 23) capabilities = value;
    } else if (key === 3 || (key >= 9 && key <= 21)) decoder.unsigned(0xffffffff);
    else if (key === 4 || key === 5) decoder.unsigned(0xffff);
    else if (key === 6) decoder.unsigned(0xff);
    else if (key === 7) {
      const rotation = decoder.unsigned(0xffff);
      if (![0, 90, 180, 270].includes(rotation)) fail("status rotation");
    } else if (key === 8) {
      if (decoder.unsigned(1) > 1) fail("status link state");
    } else if (key === 22) maximumProtocolVersion = decoder.unsigned(0xff);
    else if (key === 24) {
      tier = decoder.unsigned(1);
    } else if (key === 25) {
      wifiState = decoder.unsigned(3) as 0 | 1 | 2 | 3;
    } else if (key === 26) decoder.signed(-128, 127);
    else if (key === 27) ip = decoder.text(0, 15);
    else if (key === 28) decoder.unsigned(4);
    else if (key === 29) lastNetworkError = decoder.text(0, 96);
    else if (key === 30) decoder.text(0, 96);
    else decoder.skip();
  });
  decoder.finish();

  if (required.size > 0) fail("status missing field");
  const actualProtocolVersion = requiredValue(protocolVersion, "status missing field");
  const actualFirmwareVersion = requiredValue(firmwareVersion, "status missing field");
  if (actualProtocolVersion !== PROTOCOL_VERSION) fail("status protocol version");
  if (maximumProtocolVersion < actualProtocolVersion) fail("status maximum protocol version");
  return {
    protocolVersion: actualProtocolVersion,
    firmwareVersion: actualFirmwareVersion,
    capabilities,
    tier,
    wifiState,
    ip,
    ...(lastNetworkError === undefined ? {} : { lastNetworkError }),
  };
}

const ACK_TYPES = new Set([3, 5, 9, 10, 11, 13, 14, 15, 16, 17, 18, 19]);
const ACK_REVISION_TYPES = new Set([5, 9, 19]);

function decodeAck(payload: Uint8Array): Ack {
  const decoder = new CborDecoder(payload);
  let acknowledgedType: number | undefined;
  let revision: number | undefined;
  let alreadyPresent: boolean | undefined;
  decoder.mapEntries((key) => {
    if (key === 0n) acknowledgedType = decoder.unsigned(0xff);
    else if (key === 1n) revision = decoder.unsigned(0xffffffff);
    else if (key === 2n) alreadyPresent = decoder.boolean();
    else decoder.skip();
  });
  decoder.finish();

  const ackType = requiredValue(acknowledgedType, "acknowledged type");
  if (!ACK_TYPES.has(ackType)) fail("acknowledged type");
  if (ACK_REVISION_TYPES.has(ackType) !== (revision !== undefined) || revision === 0) {
    fail("ack revision");
  }
  if ((ackType === 15) !== (alreadyPresent !== undefined)) fail("ack already present");
  return { acknowledgedType: ackType };
}

const ERROR_CODES = new Set([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 14, 15]);

function decodeError(payload: Uint8Array): DeviceError {
  const decoder = new CborDecoder(payload);
  let code: number | undefined;
  let diagnostic: string | undefined;
  decoder.mapEntries((key) => {
    if (key === 0n) code = decoder.unsigned(0xffff);
    else if (key === 1n) diagnostic = decoder.text(0, 96);
    else decoder.skip();
  });
  decoder.finish();
  const errorCode = requiredValue(code, "error code");
  if (!ERROR_CODES.has(errorCode)) fail("error code");
  return { code: errorCode, diagnostic: requiredValue(diagnostic, "error diagnostic") };
}

function validateOtherPayload(payload: Uint8Array): void {
  const decoder = new CborDecoder(payload);
  decoder.mapEntries(() => decoder.skip());
  decoder.finish();
}

function decodeWire(wire: Uint8Array): Decoded {
  const decoded = rawDecoded(wire);
  if (decoded.byteLength < 14) fail("frame too short");
  const view = new DataView(decoded.buffer, decoded.byteOffset, decoded.byteLength);
  const payloadLength = view.getUint16(8, true);
  if (decoded.byteLength !== 10 + payloadLength + 4) fail("length");
  const crcOffset = decoded.byteLength - 4;
  if (crc32c(decoded.subarray(0, crcOffset)) !== view.getUint32(crcOffset, true)) fail("checksum");
  if (view.getUint16(2, true) !== 0) fail("flags");
  if (decoded[0] !== PROTOCOL_VERSION) fail("version");

  const messageType = decoded[1];
  if (messageType < 1 || messageType > 19) fail("message type");
  const requestId = view.getUint32(4, true);
  if ((requestId === 0) !== (messageType === 12)) fail("request id");
  const payload = decoded.subarray(10, crcOffset);
  if (messageType === MessageType.StatusResponse) {
    return { type: "status", requestId, status: decodeStatus(payload) };
  }
  if (messageType === MessageType.Ack) {
    return { type: "ack", requestId, ack: decodeAck(payload) };
  }
  if (messageType === MessageType.Error) {
    return { type: "error", requestId, error: decodeError(payload) };
  }
  validateOtherPayload(payload);
  return { type: "other", requestId, messageType };
}

export class Deframer {
  private readonly encoded: number[] = [];
  private discarding = false;

  push(chunk: Uint8Array): Array<Decoded | Invalid> {
    const frames: Array<Decoded | Invalid> = [];
    for (const byte of chunk) {
      if (this.discarding) {
        if (byte === 0) {
          this.discarding = false;
          frames.push({ type: "invalid", reason: "overlong" });
        }
      } else if (byte === 0) {
        if (this.encoded.length > 0) {
          const wire = Uint8Array.from([...this.encoded, 0]);
          this.encoded.length = 0;
          try {
            frames.push(decodeWire(wire));
          } catch (error) {
            frames.push({
              type: "invalid",
              reason: error instanceof CodecError ? error.reason : "frame",
            });
          }
        }
      } else if (this.encoded.length === MAX_ENCODED_WITHOUT_DELIMITER) {
        this.encoded.length = 0;
        this.discarding = true;
      } else {
        this.encoded.push(byte);
      }
    }
    return frames;
  }
}
