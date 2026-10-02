import { beforeAll, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { renderRequest } from "../../src/main";
import { discoverPlugins } from "../../src/plugins/discovery";
import { parseManifest } from "../../src/plugins/manifest";
import { runPlugin } from "../../src/plugins/run";
import { runInSandbox, warmSandbox } from "../../src/plugins/sandbox";
import fullMoons from "./fixtures/moon-fulls.json";

const plugins = join(import.meta.dir, "../../plugins");
const folder = join(plugins, "moon-phase");
const source = readFileSync(join(folder, "index.js"), "utf8");
const manifest = parseManifest(
  JSON.parse(readFileSync(join(folder, "plugin.json"), "utf8")),
  "moon-phase",
);
// Expose the production calculation inside QuickJS for comparison with external
// ephemerides. This adds no alternate calculation or production-only test switch.
const calculationSource = `${source}\nrender = c => moonAt(Date.parse(c.utc));`;
type Moon = { phase: number; fraction: number; name: string; nextFull: number };
function calculate(utc: string) {
  return runInSandbox<Moon>(calculationSource, "render", { utc });
}
beforeAll(warmSandbox);

async function draw(utc: string, settings = {}, timezone = "UTC") {
  const result = await runPlugin({
    manifest,
    source,
    settings,
    now: new Date(utc),
    timezone,
    secrets: {},
    request: async () => {
      throw new Error("Moon Phase must never request the network");
    },
  });
  expect(result.state).toBeUndefined();
  expect(result.log).toEqual([]);
  return result.svg;
}

describe("Moon Phase", () => {
  test("discovery has no network, credentials or tap, and exposes hemisphere", async () => {
    expect(runInSandbox<unknown[]>(source, "plan", {})).toEqual([]);
    expect(manifest.hosts).toEqual([]);
    expect(manifest.secrets).toEqual([]);
    expect(manifest.refreshSeconds).toBe(900);
    const { faces, skipped } = await discoverPlugins(plugins);
    expect(skipped).toEqual([]);
    expect(faces.find((f) => f.kind === "moon-phase")?.fields).toEqual([
      {
        type: "enum",
        key: "hemisphere",
        label: "Hemisphere",
        default: "north",
        options: [
          { value: "north", label: "Northern" },
          { value: "south", label: "Southern" },
        ],
      },
    ]);
  });

  // Independent NASA/GSFC ephemerides: all 39 full moons in 2026, 2028 and 2099.
  // Read from one day before each event so a small model error cannot skip it.
  test.each(fullMoons)("full moon is within 15 minutes of NASA: %s", (reference) => {
    const expected = Date.parse(reference);
    const result = calculate(new Date(expected - 86400000).toISOString());
    expect(Math.abs(result.nextFull - expected)).toBeLessThan(15 * 60000);
    expect(new Date(result.nextFull).toISOString().slice(0, 10)).toBe(reference.slice(0, 10));
  });

  // USNO October 2026 events, including the 360->0 wrap at new moon.
  test.each([
    ["2026-10-03T13:25:00Z", "Last quarter", 270, 0.5],
    ["2026-10-10T15:50:00Z", "New moon", 0, 0],
    ["2026-10-18T16:12:00Z", "First quarter", 90, 0.5],
    ["2026-10-26T04:12:00Z", "Full moon", 180, 1],
  ] as const)("primary phase and illumination at %s", (utc, name, angle, fraction) => {
    const result = calculate(utc);
    const error = Math.abs(((result.phase - angle + 540) % 360) - 180);
    expect(error).toBeLessThan(0.15);
    expect(Math.abs(result.fraction - fraction)).toBeLessThan(0.005);
    expect(result.name).toBe(name);
  });

  test.each([
    ["2026-10-02", "Waning gibbous", -1],
    ["2026-10-06", "Waning crescent", -1],
    ["2026-10-14", "Waxing crescent", 1],
    ["2026-10-22", "Waxing gibbous", 1],
  ] as const)(
    "%s has the right name and lit side in both hemispheres",
    async (date, name, flip) => {
      const utc = `${date}T12:00:00Z`;
      const north = await draw(utc);
      const south = await draw(utc, { hemisphere: "south" });
      expect(north).toContain(`>${name}</text>`);
      expect(north).toContain(`scale(${flip} 1)`);
      expect(south).toBe(north.replace(`scale(${flip} 1)`, `scale(${-flip} 1)`));
      expect(await draw(utc, { hemisphere: "unknown" })).toBe(north);
    },
  );

  test("full moon search rolls forward after the event, including the new-moon wrap", () => {
    const before = calculate("2026-10-26T03:00:00Z");
    const after = calculate("2026-10-26T05:00:00Z");
    expect(new Date(before.nextFull).toISOString().slice(0, 10)).toBe("2026-10-26");
    expect(new Date(after.nextFull).toISOString().slice(0, 10)).toBe("2026-11-24");
    const at = calculate(new Date(Math.ceil(before.nextFull) + 1000).toISOString());
    expect(new Date(at.nextFull).toISOString().slice(0, 10)).toBe("2026-11-24");
  });

  test("UTC dates survive timezone differences, leap day and year rollover", async () => {
    const utc = "2026-12-31T12:00:00Z";
    const svg = await draw(utc);
    expect(svg).toContain("Next full moon · 22 Jan 2027 UTC");
    expect(await draw(utc, {}, "Pacific/Kiritimati")).toBe(svg);
    expect(await draw(utc, {}, "America/Los_Angeles")).toBe(svg);
    expect(await draw("2028-02-29T12:00:00Z")).toContain("Next full moon · 11 Mar UTC");
  });

  test("bad or unsupported clocks fail visibly, with no invented moon", () => {
    for (const utc of [
      undefined,
      null,
      "",
      "invalid",
      "1899-12-31T12:00:00Z",
      "2100-01-01T00:00:00Z",
    ]) {
      expect(() => runInSandbox(source, "render", { now: { utc } })).toThrow("valid host clock");
    }
    expect(() => runInSandbox(source, "render", {})).toThrow("valid host clock");
  });

  test.each(["1900-01-01T00:00:00Z", "2000-02-29T12:00:00Z", "2099-12-31T23:59:59Z"])(
    "valid boundary %s stays bounded and finds a future event",
    (utc) => {
      const result = calculate(utc);
      expect(result.phase).toBeGreaterThanOrEqual(0);
      expect(result.phase).toBeLessThan(360);
      expect(result.fraction).toBeGreaterThanOrEqual(0);
      expect(result.fraction).toBeLessThanOrEqual(1);
      expect(result.nextFull).toBeGreaterThan(Date.parse(utc));
      expect(result.nextFull - Date.parse(utc)).toBeLessThan(32 * 86400000);
    },
  );

  test.each(["north", "south"])(
    "%s renders a PNG through the real entrypoint",
    async (hemisphere) => {
      const { faces } = await discoverPlugins(plugins);
      const face = faces.find((f) => f.kind === "moon-phase");
      if (!face) throw new Error("Moon Phase was not discovered");
      const result = await renderRequest(
        { kind: face.kind, settings: { hemisphere }, timezone: "UTC" },
        face,
        new Date("2026-10-14T12:00:00Z"),
      );
      const png = Buffer.from(result.png, "base64");
      expect(png.subarray(0, 8)).toEqual(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]));
      expect(png.readUInt32BE(16)).toBe(448);
      expect(png.readUInt32BE(20)).toBe(368);
      expect(png.length).toBeLessThan(1048576);
    },
  );
});
