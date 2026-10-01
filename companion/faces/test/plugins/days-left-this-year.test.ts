import { beforeAll, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { renderRequest } from "../../src/main";
import { discoverPlugins } from "../../src/plugins/discovery";
import { parseManifest } from "../../src/plugins/manifest";
import { runPlugin } from "../../src/plugins/run";
import { runInSandbox, warmSandbox } from "../../src/plugins/sandbox";

const plugins = join(import.meta.dir, "../../plugins");
const folder = join(plugins, "days-left-this-year");
const source = readFileSync(join(folder, "index.js"), "utf8");
const manifest = parseManifest(
  JSON.parse(readFileSync(join(folder, "plugin.json"), "utf8")),
  "days-left-this-year",
);

beforeAll(warmSandbox);

async function draw(instant: string, timezone = "UTC") {
  let calls = 0;
  const result = await runPlugin({
    manifest,
    source,
    settings: {},
    now: new Date(instant),
    timezone,
    secrets: {},
    request: async () => {
      calls++;
      throw new Error("This face must never use the network");
    },
  });
  expect(calls).toBe(0);
  expect(result.state).toBeUndefined();
  expect(result.log).toEqual([]);
  return result.svg;
}

function count(svg: string, remaining: number, year: number, percent: string) {
  expect(svg).toContain(`>${remaining}</text>`);
  expect(svg).toContain(`>${remaining === 1 ? "day" : "days"} left in ${year}</text>`);
  expect(svg).toContain(`>${percent}% complete</text>`);
  expect(svg).toContain(">including today</text>");
}

describe("Days Left This Year", () => {
  test("empty-context discovery declares no requests or permissions", () => {
    expect(runInSandbox<unknown[]>(source, "plan", {})).toEqual([]);
    expect(manifest.hosts).toEqual([]);
    expect(manifest.secrets).toEqual([]);
    expect(manifest.fields).toEqual([]);
    expect(manifest.refreshSeconds).toBe(900);
  });

  test.each([
    ["2026-01-01T00:00:00Z", 365, 2026, "0.0"],
    ["2026-10-01T12:00:00Z", 92, 2026, "74.8"],
    ["2026-12-31T23:59:59Z", 1, 2026, "99.7"],
    ["2027-01-01T00:00:00Z", 365, 2027, "0.0"],
    ["2028-01-01T00:00:00Z", 366, 2028, "0.0"],
    ["2028-02-28T12:00:00Z", 308, 2028, "15.8"],
    ["2028-02-29T12:00:00Z", 307, 2028, "16.1"],
    ["2028-03-01T00:00:00Z", 306, 2028, "16.4"],
    ["2100-03-01T00:00:00Z", 306, 2100, "16.2"],
    ["2000-02-29T00:00:00Z", 307, 2000, "16.1"],
  ])("calendar boundary %s", async (instant, remaining, year, percent) => {
    count(await draw(instant), remaining, year, percent);
  });

  test("the progress bar uses completed dates and stays empty at New Year", async () => {
    const january = await draw("2028-01-01T00:00:00Z");
    expect(january).not.toContain('height="8" rx="4" fill="#f5f5f7"');
    const october = await draw("2026-10-01T00:00:00Z");
    expect(october).toContain('width="299.178" height="8" rx="4" fill="#f5f5f7"');
    const last = await draw("2026-12-31T23:59:59Z");
    expect(last).toContain('width="398.904" height="8" rx="4" fill="#f5f5f7"');
  });

  test("uses the supplied local date when UTC is still in the previous year", async () => {
    count(await draw("2026-12-31T10:00:00Z", "Pacific/Kiritimati"), 365, 2027, "0.0");
    count(await draw("2027-01-01T07:59:59Z", "America/Los_Angeles"), 1, 2026, "99.7");
    count(await draw("2027-01-01T08:00:00Z", "America/Los_Angeles"), 365, 2027, "0.0");
  });

  test("spring and autumn DST changes do not change a date's count", async () => {
    expect(await draw("2026-03-08T05:00:00Z", "America/New_York")).toBe(
      await draw("2026-03-09T03:59:59Z", "America/New_York"),
    );
    expect(await draw("2026-11-01T04:00:00Z", "America/New_York")).toBe(
      await draw("2026-11-02T04:59:59Z", "America/New_York"),
    );
  });

  test("missing or invalid host dates fail instead of showing an invented count", () => {
    for (const local of [undefined, {}, { year: 2026, month: 2, day: 29 }]) {
      expect(() => runInSandbox(source, "render", { now: { local } })).toThrow(
        "needs a valid date from the host clock",
      );
    }
  });

  test("discovery and the real render entrypoint produce a 448x368 PNG", async () => {
    const { faces, skipped } = await discoverPlugins(plugins);
    expect(skipped).toEqual([]);
    const face = faces.find((entry) => entry.kind === manifest.id);
    expect(face).toBeDefined();
    if (!face) throw new Error("Days Left This Year was not discovered");
    const result = await renderRequest(
      { kind: manifest.id, settings: {}, timezone: "UTC" },
      face,
      new Date("2026-10-01T00:00:00Z"),
    );
    const png = Buffer.from(result.png, "base64");
    expect(png.subarray(0, 8)).toEqual(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]));
    expect(png.readUInt32BE(16)).toBe(448);
    expect(png.readUInt32BE(20)).toBe(368);
    expect(png.length).toBeLessThan(1_048_576);
    expect(result.state).toBeUndefined();
  });
});
