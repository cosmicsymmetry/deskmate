import { expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

import {
  crc32c,
  Deframer,
  encodeNetworkConfig,
  encodeStatusRequest,
  rawDecoded,
  validateNetworkConfig,
} from "../src/lib/serial/codec";

const FIXTURES = join(import.meta.dir, "../../../../protocol/fixtures/v2");
const fixture = (name: string) => new Uint8Array(readFileSync(join(FIXTURES, name)));

const NETWORK_CONFIG_FIXTURE_VALUES = {
  ssid: "s".repeat(32),
  psk: "p".repeat(64),
  serverUrl: `wss://${"u".repeat(122)}`,
  deviceId: "d".repeat(32),
  token: "t".repeat(128),
  utcOffsetMinutes: 240,
  tier: 1 as const,
};

test("crc32c check value", () => {
  expect(crc32c(new TextEncoder().encode("123456789"))).toBe(0xe3069283);
});

test("status request is byte-identical to the Rust fixture", () => {
  const expected = fixture("status_request.bin");
  const decoded = rawDecoded(expected);
  const requestId = new DataView(decoded.buffer, decoded.byteOffset, decoded.byteLength).getUint32(
    4,
    true,
  );
  expect(encodeStatusRequest(requestId)).toEqual(expected);
});

test("network config is byte-identical to the Rust fixture", () => {
  expect(encodeNetworkConfig(7, NETWORK_CONFIG_FIXTURE_VALUES)).toEqual(
    fixture("network_config.bin"),
  );
});

test("decodes every status response fixture", () => {
  for (const name of [
    "status_response.bin",
    "status_response_networked.bin",
    "status_response_ota_failed.bin",
  ]) {
    const [decoded] = new Deframer().push(fixture(name));
    expect(decoded.type).toBe("status");
  }

  const [networked] = new Deframer().push(fixture("status_response_networked.bin"));
  if (networked.type !== "status") throw new Error("expected status");
  expect(networked.requestId).toBe(3);
  expect(networked.status).toEqual({
    protocolVersion: 2,
    firmwareVersion: "deskmate-m1",
    capabilities: 16352n,
    tier: 1,
    wifiState: 2,
    ip: "192.168.1.42",
    lastNetworkError: "dns resolution timed out",
  });
});

test("decodes ack and error fixtures", () => {
  const acks = readdirSync(FIXTURES).filter((name) => name.startsWith("ack_"));
  expect(acks.length).toBeGreaterThan(0);
  for (const name of acks) {
    const [decoded] = new Deframer().push(fixture(name));
    expect(decoded.type).toBe("ack");
  }

  const [error] = new Deframer().push(fixture("error.bin"));
  expect(error).toEqual({
    type: "error",
    requestId: 5,
    error: { code: 7, diagnostic: "stale revision" },
  });
});

test("hostile input never throws and does not poison the next frame", () => {
  const good = fixture("status_response.bin");
  for (const name of [
    "bad_crc.bin",
    "garbage.bin",
    "overlong.bin",
    "invalid_cbor.bin",
    "duplicate_keys.bin",
  ]) {
    const out = new Deframer().push(new Uint8Array([...fixture(name), ...good]));
    expect(out.at(-1)?.type).toBe("status");
    expect(out.slice(0, -1).every((decoded) => decoded.type === "invalid")).toBe(true);
  }
});

test("unsupported versions are rejected without poisoning the next frame", () => {
  const out = new Deframer().push(
    new Uint8Array([...fixture("unsupported_version.bin"), ...fixture("status_response.bin")]),
  );
  expect(out[0]).toEqual({ type: "invalid", reason: "version" });
  expect(out[1].type).toBe("status");
});

test("frames split across chunks reassemble", () => {
  const good = fixture("status_response.bin");
  const deframer = new Deframer();
  expect(deframer.push(good.slice(0, 5))).toEqual([]);
  expect(deframer.push(good.slice(5))[0].type).toBe("status");
});

test("network config limits match the wire", () => {
  const base = {
    ssid: "home",
    psk: "secret",
    serverUrl: "wss://x/v1/device/link",
    deviceId: "dev-0001",
    token: "t",
    utcOffsetMinutes: 0,
    tier: 1 as const,
  };
  expect(validateNetworkConfig(base)).toBeNull();
  expect(validateNetworkConfig({ ...base, ssid: "é".repeat(17) })).toMatch(/network name/i);
  expect(validateNetworkConfig({ ...base, psk: "x".repeat(65) })).toMatch(/password/i);
  expect(validateNetworkConfig({ ...base, serverUrl: "x".repeat(129) })).toMatch(/server url/i);
  expect(validateNetworkConfig({ ...base, deviceId: "x".repeat(33) })).toMatch(/device id/i);
  expect(validateNetworkConfig({ ...base, token: "x".repeat(129) })).toMatch(/token/i);
  expect(validateNetworkConfig({ ...base, utcOffsetMinutes: 841 })).toMatch(/utc offset/i);
});
