import { describe, expect, test } from "bun:test";
import { CardError, cardToSvg } from "../../src/plugins/card";
import { pngFromSvg } from "../../src/kit/raster";

const box = (children: unknown) => ({
  layout: {
    type: "div",
    style: { display: "flex", width: 448, height: 368, background: "#000", fontFamily: "Inter" },
    children,
  },
});

describe("cardToSvg", () => {
  test("lays out a flexbox card at the panel's size", async () => {
    const svg = await cardToSvg(
      box({ type: "div", style: { fontSize: 72, color: "#f5f5f7" }, children: "128" }),
    );
    expect(svg).toContain('width="448"');
    const png = pngFromSvg(svg);
    const view = new DataView(png.buffer, png.byteOffset, png.byteLength);
    expect([view.getUint32(16), view.getUint32(20)]).toEqual([448, 368]);
  });

  test("wraps long text instead of overflowing, which is the whole point of the layout path", async () => {
    const svg = await cardToSvg(
      box({
        type: "div",
        style: { fontSize: 31 },
        children: "a headline long enough to need two lines on a 448 pixel panel",
      }),
    );
    expect(svg.match(/<text/g)?.length ?? 0).toBeGreaterThan(1);
  });

  test("refuses a tree over the box cap", async () => {
    let deep: unknown = "leaf";
    for (let i = 0; i < 2_001; i += 1) deep = { type: "div", style: {}, children: deep };
    await expect(cardToSvg(box(deep))).rejects.toThrow(/boxes/);
  });

  test("passes an SVG card through, and refuses one that references our disk", async () => {
    await expect(
      cardToSvg({ svg: '<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"></svg>' }),
    ).resolves.toContain("<svg");
    await expect(
      cardToSvg({
        svg: '<svg xmlns="http://www.w3.org/2000/svg"><image href="/etc/hosts.png"/></svg>',
      }),
    ).rejects.toThrow(CardError);
    await expect(
      cardToSvg({
        svg: '<svg xmlns="http://www.w3.org/2000/svg"><image href="https://example.com/a.png"/></svg>',
      }),
    ).rejects.toThrow(CardError);
  });

  test("accepts an embedded data: image in an SVG card", async () => {
    const tiny = "data:image/png;base64,iVBORw0KGgo=";
    await expect(
      cardToSvg({
        svg: `<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"><image href="${tiny}"/></svg>`,
      }),
    ).resolves.toContain("<image");
  });

  test("refuses an SVG over the size cap", async () => {
    await expect(
      cardToSvg({ svg: `<svg xmlns="http://www.w3.org/2000/svg">${"<g/>".repeat(200_000)}</svg>` }),
    ).rejects.toThrow(/512/);
  });

  test("wraps a PNG card as a full-canvas image", async () => {
    const png = Buffer.from(
      pngFromSvg('<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"></svg>'),
    ).toString("base64");
    expect(await cardToSvg({ png })).toContain("data:image/png;base64,");
  });
});
