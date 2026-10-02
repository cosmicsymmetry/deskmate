import { describe, expect, spyOn, test } from "bun:test";
import { Resvg } from "@resvg/resvg-js";
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

  test("refuses an SVG over the size cap in bytes even when under it in UTF-16 length", async () => {
    // "字" is one UTF-16 code unit but three UTF-8 bytes: 200,000 of them is ~200 KB
    // by .length (under the 512 KB cap) but ~586 KB by actual byte size (over it).
    const svg = `<svg xmlns="http://www.w3.org/2000/svg">${"字".repeat(200_000)}</svg>`;
    expect(svg.length).toBeLessThan(512 * 1024);
    expect(Buffer.byteLength(svg, "utf8")).toBeGreaterThan(512 * 1024);
    await expect(cardToSvg({ svg })).rejects.toThrow(/512/);
  });

  test("wraps a PNG card as a full-canvas image", async () => {
    const png = Buffer.from(
      pngFromSvg('<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"></svg>'),
    ).toString("base64");
    const svg = await cardToSvg({ png });
    expect(svg).toContain("data:image/png;base64,");
    // Exactly one reference, and it is the embedded frame -- nothing else got in.
    expect([...svg.matchAll(/href\s*=/gi)]).toHaveLength(1);
  });

  test("refuses a PNG card that is not base64, so it cannot smuggle markup into the frame", async () => {
    // The top-level {png} shape is concatenated into an SVG document. Buffer's
    // byteLength(..., "base64") is a length estimate that validates nothing, so
    // without an alphabet check this payload closes our own <image> element and
    // opens one pointing at a path -- which usvg resolves BY READING OUR DISK, the
    // exact reference the {svg} branch's refuseExternalReferences exists to refuse.
    const payload = 'AAAA"/><image href="/etc/passwd" x="0"/><x y="';
    await expect(cardToSvg({ png: payload })).rejects.toThrow(CardError);
    await expect(cardToSvg({ png: payload })).rejects.toThrow(/base64/);
  });

  test("refuses a PNG card with base64 whitespace or an out-of-alphabet character", async () => {
    await expect(cardToSvg({ png: "iVBO Rw0K" })).rejects.toThrow(/base64/);
    await expect(cardToSvg({ png: "iVBORw0K<!-" })).rejects.toThrow(/base64/);
    // Valid alphabet, but not a whole number of base64 quanta.
    await expect(cardToSvg({ png: "iVBORw0" })).rejects.toThrow(/base64/);
  });

  test("a real base64 PNG card still renders", async () => {
    const png =
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    const svg = await cardToSvg({ png });
    expect(svg).toContain(`href="data:image/png;base64,${png}"`);
    expect(svg).toContain('width="448"');
  });

  const img = (decodedBytes: number) => {
    const payload = "A".repeat(Math.ceil(decodedBytes / 3) * 4);
    return {
      type: "img",
      style: { width: 10, height: 10 },
      src: `data:image/png;base64,${payload}`,
    };
  };

  test("refuses an image inside a layout card over the PNG shape's own size cap", async () => {
    // MAX_SVG_BYTES/MAX_PNG_BYTES only guard the two top-level shapes -- an oversized
    // img.src inside {layout} would otherwise defeat both.
    await expect(cardToSvg(box(img(2 * 1024 * 1024)))).rejects.toThrow(/1024/);
  });

  test("refuses more than 4 images in one layout card", async () => {
    const five = [img(1024), img(1024), img(1024), img(1024), img(1024)];
    await expect(cardToSvg(box(five))).rejects.toThrow(/images/);
  });

  test("refuses images in one layout card whose combined size exceeds the total cap", async () => {
    // Each is under the per-image cap (900 KB < 1024 KB) and there are only 4 of them
    // (not more than MAX_IMAGES), so only the total cap (3 MB) can be what trips here.
    const each = 900 * 1024;
    const four = [img(each), img(each), img(each), img(each)];
    await expect(cardToSvg(box(four))).rejects.toThrow(/total/);
  });

  test("refuses an img src inside a layout card that is not an embedded data: URI", async () => {
    // satori does refuse a private/loopback/link-local http(s) host itself -- but it
    // resolves the hostname twice (once to check, once to fetch), the DNS-rebinding
    // gap kit/http.ts closes by pinning the validated address (see card.ts for why
    // we don't rely on satori's own check). The message is asserted specifically (not
    // just CardError) so this test cannot
    // be satisfied by satori itself failing to fetch the URL and that failure getting
    // wrapped afterward -- it has to be refused before satori ever sees it.
    await expect(
      cardToSvg(box({ type: "img", style: {}, src: "http://example.com/a.png" })),
    ).rejects.toThrow(/embedded data: URI/);
    await expect(cardToSvg(box({ type: "img", style: {}, src: "/etc/hosts.png" }))).rejects.toThrow(
      /embedded data: URI/,
    );
  });

  test("wraps a hostile style value's raw satori failure as CardError", async () => {
    await expect(
      cardToSvg(box({ type: "div", style: { color: () => "red" }, children: "x" })),
    ).rejects.toThrow(CardError);
  });

  test("wraps an unparseable style value's raw satori failure as CardError, without echoing it back whole", async () => {
    const huge = "x".repeat(5_000_000);
    let caught: unknown;
    try {
      await cardToSvg(box({ type: "div", style: { background: huge }, children: "x" }));
    } catch (error) {
      caught = error;
    }
    expect(caught).toBeInstanceOf(CardError);
    expect((caught as Error).message.length).toBeLessThan(1_000);
  });

  test("wraps an invalid base64 img src's raw satori failure as CardError", async () => {
    await expect(
      cardToSvg(
        box({ type: "img", style: {}, src: "data:image/png;base64,!!!not-base64-at-all!!!" }),
      ),
    ).rejects.toThrow(CardError);
  });

  test("the shared raster boundary refuses an SVG card with the wrong actual size", async () => {
    for (const dimensions of [
      'width="100" height="100"',
      'width="448" height="200"',
      'viewBox="0 0 100 100"',
      'width="448pt" height="368pt"',
    ]) {
      const svg = await cardToSvg({
        svg: `<svg xmlns="http://www.w3.org/2000/svg" ${dimensions}/>`,
      });
      expect(() => pngFromSvg(svg)).toThrow(/448x368/);
    }
  });

  test("the raster boundary checks dimensions before allocating a huge pixel buffer", () => {
    const render = spyOn(Resvg.prototype, "render").mockImplementation(() => {
      throw new Error("pixel allocation must not be reached");
    });
    try {
      expect(() =>
        pngFromSvg('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100000 100000"/>'),
      ).toThrow(/must be exactly 448x368/);
      expect(render).not.toHaveBeenCalled();
    } finally {
      render.mockRestore();
    }
  });

  test("the raster boundary validates the returned image, not just the parsed SVG", () => {
    const render = spyOn(Resvg.prototype, "render").mockReturnValue({
      width: 100,
      height: 100,
      pixels: Buffer.alloc(0),
      asPng: () => Buffer.alloc(0),
    });
    try {
      expect(() =>
        pngFromSvg('<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"/>'),
      ).toThrow(/picture is 100x100/);
    } finally {
      render.mockRestore();
    }
  });

  test("prefers svg over layout when a card sets both", async () => {
    const svg = await cardToSvg({
      layout: { type: "div", style: { display: "flex" } },
      svg: '<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"><!--won--></svg>',
    });
    expect(svg).toContain("<!--won-->");
  });
});
