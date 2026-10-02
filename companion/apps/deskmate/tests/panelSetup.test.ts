import { expect, mock, test } from "bun:test";

import {
  CAPABILITY_NETWORKING,
  crc32c,
  type DeviceError,
  MessageType,
  type NetworkConfig,
  rawDecoded,
  type StatusResponse,
} from "../src/lib/serial/codec";
import type { PanelPort } from "../src/lib/serial/port";
import {
  PanelSetup,
  type SetupDeps,
  type SetupStep,
  WIFI_JOIN_TIMEOUT_MESSAGE,
} from "../src/lib/serial/setup";

type StatusScript = Partial<StatusResponse> | DeviceError;

type WrittenFrame =
  | { type: "status-request"; requestId: number }
  | { type: "network-config"; requestId: number; config: NetworkConfig };

const DEFAULT_STATUS: StatusResponse = {
  protocolVersion: 2,
  firmwareVersion: "deskmate-test",
  capabilities: CAPABILITY_NETWORKING,
  tier: 0,
  wifiState: 0,
  ip: "",
};

function unsigned(value: number | bigint): number[] {
  const bigint = BigInt(value);
  if (bigint <= 23n) return [Number(bigint)];
  if (bigint <= 0xffn) return [24, Number(bigint)];
  if (bigint <= 0xffffn) return [25, Number(bigint >> 8n), Number(bigint & 0xffn)];
  if (bigint <= 0xffffffffn) {
    return [
      26,
      Number((bigint >> 24n) & 0xffn),
      Number((bigint >> 16n) & 0xffn),
      Number((bigint >> 8n) & 0xffn),
      Number(bigint & 0xffn),
    ];
  }
  const bytes = [27];
  for (let shift = 56n; shift >= 0n; shift -= 8n) {
    bytes.push(Number((bigint >> shift) & 0xffn));
  }
  return bytes;
}

function cborUnsigned(value: number | bigint): number[] {
  const [head, ...rest] = unsigned(value);
  return [head, ...rest];
}

function cborText(value: string): number[] {
  const bytes = new TextEncoder().encode(value);
  const [head, ...rest] = unsigned(bytes.byteLength);
  return [head | 0x60, ...rest, ...bytes];
}

function cborMap(entries: Array<[number, number[]]>): Uint8Array {
  const [head, ...rest] = unsigned(entries.length);
  return Uint8Array.from([
    head | 0xa0,
    ...rest,
    ...entries.flatMap(([key, value]) => [...cborUnsigned(key), ...value]),
  ]);
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
  return Uint8Array.from([...encoded, 0]);
}

function responseFrame(messageType: number, requestId: number, payload: Uint8Array): Uint8Array {
  const decoded = new Uint8Array(10 + payload.byteLength + 4);
  const view = new DataView(decoded.buffer);
  decoded[0] = 2;
  decoded[1] = messageType;
  view.setUint32(4, requestId, true);
  view.setUint16(8, payload.byteLength, true);
  decoded.set(payload, 10);
  view.setUint32(decoded.byteLength - 4, crc32c(decoded.subarray(0, -4)), true);
  return cobsEncode(decoded);
}

function statusFrame(requestId: number, override: Partial<StatusResponse>): Uint8Array {
  const status = { ...DEFAULT_STATUS, ...override };
  const entries: Array<[number, number[]]> = [
    [0, cborUnsigned(status.protocolVersion)],
    [1, cborText(status.firmwareVersion)],
    [2, cborUnsigned(1)],
    [3, cborUnsigned(1)],
    [4, cborUnsigned(448)],
    [5, cborUnsigned(368)],
    [6, cborUnsigned(0)],
    [7, cborUnsigned(90)],
    [8, cborUnsigned(0)],
    [9, cborUnsigned(0)],
    [10, cborUnsigned(0)],
    [11, cborUnsigned(0)],
    [12, cborUnsigned(0)],
    [13, cborUnsigned(0)],
    [14, cborUnsigned(0)],
    [15, cborUnsigned(0)],
    [22, cborUnsigned(2)],
    [23, cborUnsigned(status.capabilities)],
    [24, cborUnsigned(status.tier)],
    [25, cborUnsigned(status.wifiState)],
    [27, cborText(status.ip)],
  ];
  if (status.lastNetworkError !== undefined) entries.push([29, cborText(status.lastNetworkError)]);
  return responseFrame(MessageType.StatusResponse, requestId, cborMap(entries));
}

function ackFrame(requestId: number): Uint8Array {
  return responseFrame(MessageType.Ack, requestId, cborMap([[0, cborUnsigned(13)]]));
}

function errorFrame(requestId: number, error: DeviceError): Uint8Array {
  return responseFrame(
    MessageType.Error,
    requestId,
    cborMap([
      [0, cborUnsigned(error.code)],
      [1, cborText(error.diagnostic)],
    ]),
  );
}

class CborReader {
  private offset = 0;

  constructor(private readonly bytes: Uint8Array) {}

  private argument(expectedMajor: number): number {
    const first = this.bytes[this.offset++];
    const major = first >> 5;
    if (major !== expectedMajor) throw new Error("unexpected CBOR type");
    const additional = first & 0x1f;
    if (additional <= 23) return additional;
    const length = 1 << (additional - 24);
    let value = 0;
    for (let index = 0; index < length; index += 1) value = value * 256 + this.bytes[this.offset++];
    return value;
  }

  mapLength(): number {
    return this.argument(5);
  }

  key(): number {
    return this.argument(0);
  }

  text(): string {
    const length = this.argument(3);
    const value = new TextDecoder().decode(this.bytes.subarray(this.offset, this.offset + length));
    this.offset += length;
    return value;
  }

  signed(): number {
    const first = this.bytes[this.offset];
    const major = first >> 5;
    const value = this.argument(major);
    if (major === 0) return value;
    if (major === 1) return -1 - value;
    throw new Error("unexpected CBOR integer");
  }
}

function decodeWrittenFrame(wire: Uint8Array): WrittenFrame {
  const decoded = rawDecoded(wire);
  const view = new DataView(decoded.buffer, decoded.byteOffset, decoded.byteLength);
  const messageType = decoded[1];
  const requestId = view.getUint32(4, true);
  if (messageType === MessageType.StatusRequest) return { type: "status-request", requestId };
  if (messageType !== MessageType.NetworkConfig) throw new Error("unexpected request type");

  const payloadLength = view.getUint16(8, true);
  const reader = new CborReader(decoded.subarray(10, 10 + payloadLength));
  const config: Partial<NetworkConfig> = {};
  for (let index = 0, length = reader.mapLength(); index < length; index += 1) {
    const key = reader.key();
    if (key === 1) config.ssid = reader.text();
    else if (key === 2) config.psk = reader.text();
    else if (key === 3) config.serverUrl = reader.text();
    else if (key === 4) config.deviceId = reader.text();
    else if (key === 5) config.token = reader.text();
    else if (key === 6) config.utcOffsetMinutes = reader.signed();
    else if (key === 7) config.tier = reader.signed() as 0 | 1;
  }
  return { type: "network-config", requestId, config: config as NetworkConfig };
}

class FakePort implements PanelPort {
  readonly written: WrittenFrame[] = [];
  openCount = 0;
  private statusIndex = 0;
  private opened = false;
  private disconnectFired = false;
  private readonly chunks: Uint8Array[] = [];
  private readonly waiters: Array<() => void> = [];
  private disconnectListener = () => {};

  constructor(
    private readonly options: {
      statuses?: StatusScript[];
      ackNetworkConfig?: boolean | DeviceError;
      silent?: boolean;
      disconnectAfterAck?: boolean;
      noiseBeforeStatus?: boolean;
    },
  ) {}

  async open(): Promise<void> {
    this.opened = true;
    this.openCount += 1;
  }

  async write(bytes: Uint8Array): Promise<void> {
    if (!this.opened) throw new Error("port is closed");
    const frame = decodeWrittenFrame(bytes);
    this.written.push(frame);
    if (this.options.silent) return;

    if (frame.type === "status-request") {
      const statuses = this.options.statuses ?? [{}];
      const scripted = statuses[Math.min(this.statusIndex, statuses.length - 1)] ?? {};
      this.statusIndex += 1;
      if (this.options.noiseBeforeStatus) {
        this.enqueue(responseFrame(12, 0, cborMap([])));
        this.enqueue(statusFrame(frame.requestId + 100, {}));
      }
      if ("code" in scripted) this.enqueue(errorFrame(frame.requestId, scripted));
      else this.enqueue(statusFrame(frame.requestId, scripted));
      return;
    }

    const ack = this.options.ackNetworkConfig ?? true;
    if (ack === true) this.enqueue(ackFrame(frame.requestId));
    else if (ack !== false) this.enqueue(errorFrame(frame.requestId, ack));

    if (this.options.disconnectAfterAck && !this.disconnectFired) {
      this.disconnectFired = true;
      queueMicrotask(() => {
        this.opened = false;
        this.disconnectListener();
        this.wake();
      });
    }
  }

  async *readable(): AsyncIterable<Uint8Array> {
    if (this.options.silent) return;
    while (this.opened || this.chunks.length > 0) {
      const chunk = this.chunks.shift();
      if (chunk !== undefined) {
        yield chunk;
        continue;
      }
      await new Promise<void>((resolve) => this.waiters.push(resolve));
    }
  }

  async close(): Promise<void> {
    this.opened = false;
    this.wake();
  }

  onDisconnect(listener: () => void): void {
    this.disconnectListener = listener;
  }

  private enqueue(chunk: Uint8Array): void {
    this.chunks.push(chunk);
    this.wake();
  }

  private wake(): void {
    for (const waiter of this.waiters.splice(0)) waiter();
  }
}

function harness(port: FakePort, overrides: Partial<Pick<SetupDeps, "claim" | "isLinked">> = {}) {
  let clock = 0;
  const steps: SetupStep[] = [];
  const claim =
    overrides.claim ??
    mock(async () => ({
      device_id: "dev-0042",
      token: "panel-token",
      link_url: "wss://example.test/v1/device/link",
    }));
  const isLinked = overrides.isLinked ?? mock(async () => true);
  const setup = new PanelSetup(
    {
      port,
      claim,
      isLinked,
      utcOffsetMinutes: () => 240,
      sleep: async (milliseconds) => {
        clock += milliseconds;
      },
      now: () => clock,
    },
    (step) => steps.push(step),
  );
  return { setup, steps, claim, isLinked, now: () => clock };
}

test("a board that never answers ends in no-response after 10 s", async () => {
  const port = new FakePort({ silent: true });
  const { setup, steps, now } = harness(port);

  await setup.connect();

  expect(steps.at(-1)).toEqual({ kind: "no-response" });
  expect(now()).toBe(10_000);
  expect(port.written).toHaveLength(20);
  expect(port.written.every((frame) => frame.type === "status-request")).toBe(true);
});

test("a v1 board or one without Networking is refused before any config write", async () => {
  const v1Port = new FakePort({
    statuses: [{ code: 3, diagnostic: "unsupported protocol version" }],
  });
  const v1 = harness(v1Port);
  await v1.setup.connect();
  expect(v1.steps.at(-1)).toEqual({
    kind: "incompatible",
  });
  expect(v1Port.written.map((frame) => frame.type)).toEqual(["status-request"]);

  const missingCapabilityPort = new FakePort({ statuses: [{ capabilities: 0n }] });
  const missingCapability = harness(missingCapabilityPort);
  await missingCapability.setup.connect();
  expect(missingCapability.steps.at(-1)).toEqual({
    kind: "incompatible",
  });
  expect(missingCapabilityPort.written.map((frame) => frame.type)).toEqual(["status-request"]);
});

test("the Wi-Fi password is written only to the port, never to claim()", async () => {
  const password = "correct horse battery staple";
  const port = new FakePort({ statuses: [{}, { wifiState: 2 }] });
  const claim = mock(async () => ({
    device_id: "dev-0042",
    token: "panel-token",
    link_url: "wss://example.test/v1/device/link",
  }));
  const { setup, steps } = harness(port, { claim });

  await setup.connect();
  await setup.submitWifi("Studio Wi-Fi", password);

  expect(claim).toHaveBeenCalledTimes(1);
  expect(claim.mock.calls[0]).toEqual([]);
  expect(JSON.stringify(await claim.mock.results[0]?.value)).not.toContain(password);
  const config = port.written.find((frame) => frame.type === "network-config");
  expect(config?.type === "network-config" ? config.config : undefined).toEqual({
    ssid: "Studio Wi-Fi",
    psk: password,
    serverUrl: "wss://example.test/v1/device/link",
    deviceId: "dev-0042",
    token: "panel-token",
    utcOffsetMinutes: 240,
    tier: 1,
  });
  expect(steps.at(-1)).toEqual({ kind: "linked", deviceId: "dev-0042" });
});

test("a failed join shows the board's error, and a retry reuses the same identity", async () => {
  const port = new FakePort({
    statuses: [
      {},
      { wifiState: 1 },
      { wifiState: 3, lastNetworkError: "auth failed" },
      { wifiState: 1 },
      { wifiState: 2 },
    ],
  });
  const { setup, steps, claim } = harness(port);

  await setup.connect();
  await setup.submitWifi("Home", "wrong password");
  expect(steps.at(-1)).toEqual({ kind: "wifi-failed", boardError: "auth failed" });

  await setup.submitWifi("Home", "right password");
  expect(steps.at(-1)).toEqual({ kind: "linked", deviceId: "dev-0042" });
  expect(claim).toHaveBeenCalledTimes(1);
  const configs = port.written.filter((frame) => frame.type === "network-config");
  expect(configs).toHaveLength(2);
  if (configs[0]?.type !== "network-config" || configs[1]?.type !== "network-config") {
    throw new Error("expected network config frames");
  }
  expect(configs[1].config.deviceId).toBe(configs[0].config.deviceId);
  expect(configs[1].config.token).toBe(configs[0].config.token);
  expect(configs[1].config.psk).toBe("right password");
});

test("a board stuck joining Wi-Fi for 45 s ends in wifi-failed, not an endless wait", async () => {
  const port = new FakePort({ statuses: [{}, { wifiState: 1 }] });
  const { setup, steps } = harness(port);

  await setup.connect();
  await setup.submitWifi("Home", "secret");

  expect(steps.at(-1)).toEqual({ kind: "wifi-failed", boardError: WIFI_JOIN_TIMEOUT_MESSAGE });
});

test("a board that restarts after NetworkConfig is reopened and the flow continues", async () => {
  const port = new FakePort({
    statuses: [{}, { wifiState: 2 }],
    disconnectAfterAck: true,
  });
  const { setup, steps } = harness(port);

  await setup.connect();
  await setup.submitWifi("Home", "secret");

  expect(port.openCount).toBe(2);
  expect(steps.at(-1)).toEqual({ kind: "linked", deviceId: "dev-0042" });
});

test("Wi-Fi up but no server link within 60 s ends in unreachable-server and keeps the claim", async () => {
  const port = new FakePort({ statuses: [{}, { wifiState: 2 }] });
  const isLinked = mock(async () => false);
  const { setup, steps, claim, now } = harness(port, { isLinked });

  await setup.connect();
  await setup.submitWifi("Home", "secret");

  expect(steps.at(-1)).toEqual({ kind: "unreachable-server", deviceId: "dev-0042" });
  expect(now()).toBe(60_000);
  expect(claim).toHaveBeenCalledTimes(1);
  expect(isLinked).toHaveBeenCalledTimes(60);
});

test("happy path ends in linked", async () => {
  const port = new FakePort({ statuses: [{}, { wifiState: 1 }, { wifiState: 2 }] });
  const { setup, steps } = harness(port);

  await setup.connect();
  await setup.submitWifi("Home", "secret");

  expect(steps).toContainEqual({ kind: "wifi-form" });
  expect(steps).toContainEqual({ kind: "writing" });
  expect(steps).toContainEqual({ kind: "wifi-joining" });
  expect(steps).toContainEqual({ kind: "linking" });
  expect(steps.at(-1)).toEqual({ kind: "linked", deviceId: "dev-0042" });
});

test("requests start at one and ignore DeviceEvent and stale response frames", async () => {
  const port = new FakePort({
    statuses: [{}, { wifiState: 2 }],
    noiseBeforeStatus: true,
  });
  const { setup, steps } = harness(port);

  await setup.connect();
  await setup.submitWifi("Home", "secret");

  expect(port.written.map((frame) => frame.requestId)).toEqual([1, 2, 3]);
  expect(steps.at(-1)).toEqual({ kind: "linked", deviceId: "dev-0042" });
});
