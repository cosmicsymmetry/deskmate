import { describe, expect, test } from "bun:test";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { allCases } from "../src/cases";
import { pngFromSvg } from "../src/kit/raster";
import { CANVAS_HEIGHT, CANVAS_WIDTH } from "../src/kit/theme";

const golden = (name: string): string => `${import.meta.dir}/golden/${name}.svg`;
const cases = allCases();

describe("every case", () => {
  test("has a unique name and a golden, and no golden is orphaned", () => {
    const names = cases.map((entry) => entry.name);
    expect(new Set(names).size).toBe(names.length);
    const onDisk = readdirSync(`${import.meta.dir}/golden`).map((file) => file.replace(".svg", ""));
    expect(onDisk.sort()).toEqual([...names].sort());
  });

  test.each(cases)("$name matches its golden byte for byte", ({ name, svg }) => {
    // A deliberate design change is `bun run dump --update`, reviewed as a diff.
    expect(existsSync(golden(name))).toBe(true);
    expect(svg).toBe(readFileSync(golden(name), "utf8"));
  });

  test.each(cases)("$name rasterizes to the panel's exact size", ({ svg }) => {
    // The ingest path refuses anything that is not 448x368 with a 422.
    const png = pngFromSvg(svg);
    const view = new DataView(png.buffer, png.byteOffset, png.byteLength);
    expect([view.getUint32(16), view.getUint32(20)]).toEqual([CANVAS_WIDTH, CANVAS_HEIGHT]);
  });

  test.each(cases)("$name positions no text outside the canvas", ({ svg }) => {
    // The failure a fixed panel makes invisible: text laid out past the edge simply
    // is not there, with no scrollbar, no clipping artifact and no error.
    for (const match of svg.matchAll(/<text x="([\d.-]+)" y="([\d.-]+)"/g)) {
      const x = Number(match[1]);
      const y = Number(match[2]);
      expect(x).toBeGreaterThanOrEqual(0);
      expect(x).toBeLessThanOrEqual(CANVAS_WIDTH);
      expect(y).toBeGreaterThanOrEqual(0);
      expect(y).toBeLessThanOrEqual(CANVAS_HEIGHT);
    }
  });

  test.each(cases)("$name is flat fills and strokes", ({ svg }) => {
    // RLE565 compresses a flat frame to ~10 KB and EXPANDS a gradient to ~330 KB,
    // so this is a delivery constraint as much as a visual one.
    expect(svg).not.toMatch(/<defs|Gradient|<filter|<image|<pattern/);
  });
});
