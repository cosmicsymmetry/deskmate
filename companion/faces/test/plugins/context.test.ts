import { beforeAll, describe, expect, test } from "bun:test";
import { FORMAT_SOURCE, buildNow } from "../../src/plugins/context";
import { runInSandbox, warmSandbox } from "../../src/plugins/sandbox";

beforeAll(async () => {
  await warmSandbox();
});

describe("buildNow", () => {
  test("converts to the card's time zone, not the host's", () => {
    const now = buildNow(new Date("2026-09-23T10:00:00Z"), "Asia/Dubai");
    expect(now.local.hour).toBe(14);
    expect(now.local.offsetMinutes).toBe(240);
    expect(now.utc).toBe("2026-09-23T10:00:00.000Z");
  });

  test("handles a zone with a half-hour offset and one on the other side of midnight", () => {
    expect(buildNow(new Date("2026-09-23T10:00:00Z"), "Asia/Kolkata").local.minute).toBe(30);
    const honolulu = buildNow(new Date("2026-09-23T06:00:00Z"), "Pacific/Honolulu").local;
    expect([honolulu.day, honolulu.hour]).toEqual([22, 20]);
  });

  test("falls back to UTC for a zone it does not know, rather than throwing", () => {
    expect(buildNow(new Date("2026-09-23T10:00:00Z"), "Mars/Olympus").timezone).toBe("UTC");
  });
});

describe("FORMAT_SOURCE", () => {
  test("formats numbers with separators, since the sandbox has no Intl", () => {
    const plugin = `${FORMAT_SOURCE}
export function render(){ return [format.number(324800), format.number(12.5, 2), format.compact(324800)]; }`;
    expect(runInSandbox<string[]>(plugin, "render", {})).toEqual(["324,800", "12.50", "325K"]);
  });

  test("formats a date and a relative age from the context it is given", () => {
    const plugin = `${FORMAT_SOURCE}
export function render(c){ return [format.date(c.now.local, "d MMM"), format.since(c.now.utc, "2026-09-23T08:30:00Z")]; }`;
    const out = runInSandbox<string[]>(plugin, "render", {
      now: buildNow(new Date("2026-09-23T10:00:00Z"), "Asia/Dubai"),
    });
    expect(out).toEqual(["23 Sep", "1h ago"]);
  });

  test("format.compact handles tier boundaries correctly: 999999 → 1M, not 1000K", () => {
    const plugin = `${FORMAT_SOURCE}
export function render(){ return [format.compact(999999), format.compact(999950), format.compact(999999999)]; }`;
    expect(runInSandbox<string[]>(plugin, "render", {})).toEqual(["1M", "1M", "1B"]);
  });

  test("format.compact handles negative values at boundaries", () => {
    const plugin = `${FORMAT_SOURCE}
export function render(){ return [format.compact(-999999), format.compact(-999999999)]; }`;
    expect(runInSandbox<string[]>(plugin, "render", {})).toEqual(["-1M", "-1B"]);
  });

  test("format.compact preserves existing behavior for common cases", () => {
    const plugin = `${FORMAT_SOURCE}
export function render(){ return [format.compact(999), format.compact(1000), format.compact(1500000)]; }`;
    expect(runInSandbox<string[]>(plugin, "render", {})).toEqual(["999", "1K", "1.5M"]);
  });
});
