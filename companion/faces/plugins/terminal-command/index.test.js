import { beforeAll, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { XMLParser } from "fast-xml-parser";
import { pngFromSvg, textInk, textWidth } from "../../src/kit/raster";
import { renderRequest } from "../../src/main";
import { buildNow } from "../../src/plugins/context";
import { discoverPlugins } from "../../src/plugins/discovery";
import { parseManifest } from "../../src/plugins/manifest";
import { runPlugin } from "../../src/plugins/run";
import { runInSandbox, warmSandbox } from "../../src/plugins/sandbox";
import { COMMANDS } from "./index.js";

const source = readFileSync(join(import.meta.dir, "index.js"), "utf8");
const manifest = parseManifest(
  JSON.parse(readFileSync(join(import.meta.dir, "plugin.json"), "utf8")),
  "terminal-command",
);
const instant = new Date("2026-10-04T12:00:00Z");
const parser = new XMLParser({ ignoreAttributes: false, attributeNamePrefix: "" });
beforeAll(warmSandbox);
const draw = (options = {}) =>
  runPlugin({
    manifest,
    source,
    settings: {},
    now: instant,
    timezone: "UTC",
    secrets: {},
    request: async () => {
      throw new Error("No network allowed");
    },
    ...options,
  });
function textRuns(svg) {
  return [...svg.matchAll(/<text\b[^>]*>.*?<\/text>/g)].map(([text]) => parser.parse(text).text);
}
function command(svg) {
  return textRuns(svg)
    .filter((run) => run["font-weight"] === "600")
    .map((run) => run["#text"])
    .join("\n")
    .replace(/ \\\n/g, " ");
}

test("discovery exposes platform choices without network or secrets", async () => {
  expect(runInSandbox(source, "plan", {})).toEqual([]);
  expect(manifest.hosts).toEqual([]);
  expect(manifest.secrets).toEqual([]);
  expect(manifest.fields[0].options.map((option) => option.value)).toEqual([
    "both",
    "linux",
    "macos",
  ]);
  const { faces, skipped } = await discoverPlugins(
    join(import.meta.dir, ".."),
    { denylistPath: null },
    [manifest.id],
  );
  expect(skipped).toEqual([]);
  expect(faces.map((face) => face.kind)).toEqual([manifest.id]);
});
for (const platform of ["both", "linux", "macos"]) {
  test(`${platform}: a tap cycle covers every eligible command, with readable real PNGs`, async () => {
    const expected = COMMANDS.filter(
      (item) => platform === "both" || item.platform === "both" || item.platform === platform,
    );
    const seen = new Set();
    let state;
    let first;
    for (let i = 0; i <= expected.length; i++) {
      const result = await draw({
        settings: { platform },
        state,
        ...(i ? { event: { taps: 1, point: null } } : {}),
      });
      const current = command(result.svg);
      if (i === expected.length) {
        expect(current).toBe(first);
        break;
      }
      if (!i) first = current;
      expect(seen.has(current)).toBe(false);
      seen.add(current);
      const entry = expected.find((item) => item.command === current);
      expect(entry).toBeDefined();
      const runs = textRuns(result.svg);
      expect(
        runs
          .filter((run) => run["font-size"] === "26")
          .map((run) => run["#text"])
          .join(" "),
      ).toBe(entry.explanation);
      expect(result.svg).toContain(
        { both: "Linux + macOS", linux: "Linux", macos: "macOS" }[entry.platform],
      );
      for (const run of runs) {
        const size = Number(run["font-size"] || 18);
        const weight = Number(run["font-weight"] || 400);
        const ink = textInk(run["#text"], size, weight);
        const x =
          Number(run.x) -
          (run["text-anchor"] === "end" ? textWidth(run["#text"], size, weight) : 0);
        expect(x + ink.left).toBeGreaterThanOrEqual(30);
        expect(x + ink.right).toBeLessThanOrEqual(418);
        expect(Number(run.y) + ink.top).toBeGreaterThanOrEqual(30);
        expect(Number(run.y) + ink.bottom).toBeLessThanOrEqual(342);
        if (weight === 600) expect(size).toBeGreaterThanOrEqual(32);
      }
      const png = pngFromSvg(result.svg);
      expect(png.readUInt32BE(16)).toBe(448);
      expect(png.readUInt32BE(20)).toBe(368);
      expect(result.log).toEqual([]);
      expect(JSON.stringify(result.state).length).toBeLessThan(200);
      const plan = runInSandbox(source, "plan", {
        now: buildNow(instant, "UTC"),
        settings: { platform },
        state,
      });
      expect(plan).toHaveLength(1);
      expect(plan[0].measure.length).toBeLessThanOrEqual(64);
      expect(plan[0].url).toBeUndefined();
      state = result.state;
    }
    expect([...seen].sort()).toEqual(expected.map((item) => item.command).sort());
  });
}

test("same-day refresh and identical inputs preserve the selection", async () => {
  const initial = await draw();
  expect(await draw()).toEqual(initial);
  const tapped = await draw({ state: initial.state, event: { taps: 1, point: null } });
  expect(tapped.svg).not.toBe(initial.svg);
  expect(await draw({ state: tapped.state, now: new Date("2026-10-04T23:59:59Z") })).toEqual(
    tapped,
  );
});

test("coalesced taps equal separate taps, including wraparound and large counts", async () => {
  let sequential = await draw();
  for (let i = 0; i < 3; i++)
    sequential = await draw({ state: sequential.state, event: { taps: 1, point: null } });
  expect(await draw({ event: { taps: 33, point: null } })).toEqual(sequential);
  expect(await draw({ event: { taps: Number.MAX_SAFE_INTEGER, point: null } })).toEqual(
    await draw({ event: { taps: Number.MAX_SAFE_INTEGER % COMMANDS.length, point: null } }),
  );
});

test("local midnight, timezone and platform edits reset the tap offset", async () => {
  const tapped = await draw({ event: { taps: 7, point: null } });
  for (const options of [
    { now: new Date("2026-10-05T00:00:00Z") },
    { timezone: "Asia/Tokyo" },
    { settings: { platform: "linux" } },
    { settings: { platform: "macos" } },
  ]) {
    expect(await draw({ ...options, state: tapped.state })).toEqual(await draw(options));
  }
  const before = await draw({ now: new Date("2027-12-31T14:59:59Z"), timezone: "Asia/Tokyo" });
  const after = await draw({
    now: new Date("2027-12-31T15:00:00Z"),
    timezone: "Asia/Tokyo",
    state: before.state,
  });
  expect(before.state.date).toBe("2027-12-31");
  expect(after.state.date).toBe("2028-01-01");
  expect(after.svg).not.toBe(before.svg);
  expect((await draw({ now: new Date("2028-02-29T12:00:00Z") })).state.date).toBe("2028-02-29");
});

test("a daily deck is shuffled and visits every command before repeating", async () => {
  const start = Math.floor(instant.getTime() / 86400000 / 30) * 30;
  const seen = new Set();
  for (let i = 0; i < 30; i++)
    seen.add(command((await draw({ now: new Date((start + i) * 86400000) })).svg));
  expect(seen.size).toBe(COMMANDS.length);
  expect([...seen]).not.toEqual(COMMANDS.map((item) => item.command));
});

test("malformed state, settings and tap counts cannot inject commands", async () => {
  const initial = await draw();
  for (const state of [
    null,
    [],
    "bad",
    {},
    { ...initial.state, offset: -1 },
    { ...initial.state, offset: 1.5 },
    { ...initial.state, offset: 30 },
    { ...initial.state, command: "rm -rf /" },
  ]) {
    expect(await draw({ state })).toEqual(initial);
  }
  for (const platform of [null, "windows", 1, {}])
    expect(await draw({ settings: { platform } })).toEqual(initial);
  for (const taps of [-1, 1.5, "1", null, Number.MAX_SAFE_INTEGER + 1])
    expect(await draw({ event: { taps, point: null } })).toEqual(initial);
});

test("invalid clocks and missing measurements fail without a placeholder", () => {
  const now = buildNow(instant, "UTC");
  for (const local of [
    null,
    {},
    { year: 2026, month: 2, day: 30 },
    { year: 2026, month: 13, day: 1 },
  ]) {
    expect(() => runInSandbox(source, "render", { now: { ...now, local } })).toThrow(
      /valid local date/,
    );
  }
  expect(() => runInSandbox(source, "render", { now })).toThrow(/measure/);
});

test("all preview cases cross the actual entrypoint with the content their labels promise", async () => {
  const { faces } = await discoverPlugins(join(import.meta.dir, ".."), { denylistPath: null }, [
    manifest.id,
  ]);
  const { cases } = JSON.parse(readFileSync(join(import.meta.dir, "check.json"), "utf8"));
  for (const [index, example] of cases.entries()) {
    const expected = await draw({
      ...example,
      settings: example.settings || {},
      now: new Date(example.now),
    });
    if (index < COMMANDS.length) expect(command(expected.svg)).toBe(example.name);
    const actual = await renderRequest(
      { kind: manifest.id, ...example },
      faces[0],
      new Date(example.now),
    );
    expect(actual.png).toBe(pngFromSvg(expected.svg).toString("base64"));
    expect(actual.state).toEqual(expected.state);
  }
});
