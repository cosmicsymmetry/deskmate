import { crc32c } from "../lib/serial/codec";

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

export function responseFrame(
  messageType: number,
  requestId: number,
  payload: Uint8Array,
): Uint8Array {
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
