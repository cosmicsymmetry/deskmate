import { beforeAll, expect, test } from "bun:test";
import { join } from "node:path";
import { pngFromSvg } from "../../src/kit/raster";
import { discoverPlugins } from "../../src/plugins/discovery";
import { runPlugin, type RunPluginInput } from "../../src/plugins/run";
import { runInSandbox, warmSandbox } from "../../src/plugins/sandbox";
import { shippedPlugin } from "./test_support";

const root = join(import.meta.dir, "../../plugins");
const { source, manifest } = shippedPlugin("ink-landscape");
beforeAll(warmSandbox);

async function draw(overrides: Partial<RunPluginInput> = {}) {
  return runPlugin({
    manifest,
    source,
    now: new Date("2026-10-02T12:00:00Z"),
    timezone: "UTC",
    settings: {},
    secrets: {},
    request: async () => {
      throw new Error("Ink Landscape must make no requests");
    },
    ...overrides,
  });
}

test("discovery exposes settings and a tap without network permissions", async () => {
  expect(runInSandbox<unknown[]>(source, "plan", {})).toEqual([]);
  expect(manifest.hosts).toEqual([]);
  expect(manifest.secrets).toEqual([]);
  const { faces, skipped } = await discoverPlugins(root, {}, ["ink-landscape"]);
  expect(skipped).toEqual([]);
  expect(faces[0]?.tap).toContain("another landscape");
  expect(faces[0]?.fields.map((field) => field.key)).toEqual(["palette", "change"]);
});

test("daily artwork remains stable between refreshes and has bounded output", async () => {
  const morning = await draw();
  const evening = await draw({ now: new Date("2026-10-02T23:59:59Z") });
  expect(morning.svg).toBe(evening.svg);
  expect(await draw({ state: morning.state })).toEqual(morning);
  expect(morning.log).toEqual([]);
  expect(Buffer.byteLength(morning.svg)).toBeLessThan(180_000);
  expect(JSON.stringify(morning.state).length).toBeLessThan(100);
  const png = Buffer.from(pngFromSvg(morning.svg));
  expect([png.readUInt32BE(16), png.readUInt32BE(20)]).toEqual([448, 368]);
});

test("local midnight resets the selection, including year rollover", async () => {
  const instant = new Date("2026-12-31T16:00:00Z");
  const utc = await draw({ now: instant });
  const tokyo = await draw({ now: instant, timezone: "Asia/Tokyo", state: utc.state });
  expect(tokyo.svg).not.toBe(utc.svg);
  expect(tokyo.state).toEqual({ version: 1, bucket: "2027-1-1", offset: 0 });
  expect(tokyo.svg).toBe((await draw({ now: new Date("2027-01-01T12:00:00Z") })).svg);
});

test("hourly landscapes change at the owner's hour, daily ones do not", async () => {
  const a = await draw({ settings: { change: "hourly" } });
  expect(a.svg).toBe(
    (await draw({ settings: { change: "hourly" }, now: new Date("2026-10-02T12:59:59Z") })).svg,
  );
  const b = await draw({
    settings: { change: "hourly" },
    now: new Date("2026-10-02T13:00:00Z"),
    state: a.state,
  });
  expect(b.svg).not.toBe(a.svg);
  expect(
    (await draw({ settings: { change: "hourly" }, timezone: "Asia/Kathmandu" })).state,
  ).toEqual({ version: 1, bucket: "2026-10-2-17", offset: 0 });
});

test("coalesced taps advance as far as individual taps and persist on refresh", async () => {
  const first = await draw();
  const once = await draw({ state: first.state, event: { taps: 1, point: null } });
  const twice = await draw({ state: once.state, event: { taps: 1, point: null } });
  const combined = await draw({ state: first.state, event: { taps: 2, point: null } });
  expect(combined).toEqual(twice);
  expect(once.svg).not.toBe(first.svg);
  expect((await draw({ state: combined.state })).svg).toBe(combined.svg);
});

test("malformed state and unsupported settings safely use the first landscape", async () => {
  const first = await draw();
  for (const state of [
    null,
    "bad",
    { version: 1, bucket: "2026-10-2", offset: -1 },
    { version: 2, bucket: "2026-10-2", offset: 2 },
    { version: 1, bucket: "2026-10-2", offset: 0.5 },
  ]) {
    expect(await draw({ state })).toEqual(first);
  }
  expect(await draw({ settings: { change: "bad", palette: "bad" } })).toEqual(first);
});

test("palettes preserve terrain and state; wrap and tap bounds remain finite", async () => {
  const day = await draw();
  const night = await draw({ settings: { palette: "night" }, state: day.state });
  expect(night.state).toEqual(day.state);
  expect(night.svg).not.toBe(day.svg);
  const paths = (svg: string) => [...svg.matchAll(/<path d="([^"]+)"/g)].map((match) => match[1]);
  expect(paths(night.svg)).toEqual(paths(day.svg));
  const wrapped = await draw({
    state: { version: 1, bucket: "2026-10-2", offset: 65535 },
    event: { taps: 1, point: null },
  });
  expect(wrapped).toEqual(day);
  const capped = await draw({ event: { taps: 9999, point: null } });
  expect(capped.state).toEqual({ version: 1, bucket: "2026-10-2", offset: 32 });
});

test("each day in a sample month makes valid distinct bounded artwork", async () => {
  const seen = new Set<string>();
  for (let day = 1; day <= 31; day++) {
    const result = await draw({
      now: new Date(Date.UTC(2026, 9, day)),
      settings: { palette: day % 2 ? "paper" : "night" },
    });
    expect(result.svg).not.toMatch(/NaN|Infinity|undefined/);
    expect(Buffer.byteLength(result.svg)).toBeLessThan(180_000);
    expect(seen.has(result.svg)).toBe(false);
    seen.add(result.svg);
  }
});
