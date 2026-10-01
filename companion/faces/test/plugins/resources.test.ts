import { expect, spyOn, test } from "bun:test";
import { promises as dns } from "node:dns";
import { CardError, cardToSvg } from "../../src/plugins/card";
import { pngFromSvg } from "../../src/kit/raster";

const external = "https://assets.example.invalid/should-never-load";
const png =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
const data = `data:image/png;base64,${png}`;
const svg = (body: string) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368">${body}</svg>`;
const embeddedSvg = (body: string) =>
  `data:image/svg+xml;base64,${Buffer.from(svg(body)).toString("base64")}`;
const layout = (style: object) => ({
  layout: { type: "div", style: { display: "flex", width: 448, height: 368, ...style } },
});

// Every resource-loading entry found in the pinned Satori/usvg audit, including
// forms currently ignored by one engine. No DNS/network fallback is a test oracle:
// the explicit configuration refusal must happen BEFORE any fetch is attempted.
const refused: [string, unknown][] = [
  [
    "invalid image base64",
    { layout: { type: "img", src: "data:image/png;base64,!!!not-base64!!!" } },
  ],
  [
    "style URL split across XML text and CDATA",
    { svg: svg(`<style>rect { fill: u<![CDATA[rl(${external})]]>; }</style>`) },
  ],
  [
    "style URL split across two CDATA nodes",
    { svg: svg(`<style><![CDATA[rect { fill: u]]><![CDATA[rl(${external}); }]]></style>`) },
  ],
  [
    "data image with unsupported MIME",
    { layout: { type: "img", src: "data:image/raw;base64,AAAA" } },
  ],
  [
    "invalid embedded SVG UTF-8",
    { layout: { type: "img", src: "data:image/svg+xml;base64,//4=" } },
  ],
  ["unknown XML entity", { svg: svg('<rect fill="&foreign;"/>') }],
  ["invalid XML code point", { svg: svg('<rect fill="&#x110000;"/>') }],
  ["invalid SVG markup", { svg: svg('<image href="data:image/png;base64,AAAA>') }],
  ["incomplete style URL", layout({ backgroundImage: "url(data:image/png;base64,AAAA" })],
  ["malformed quoted style URL", layout({ backgroundImage: `url('${data}' garbage)` })],
  ...["backgroundImage", "background", "maskImage", "WebkitMaskImage", "fontFamily"].map(
    (key): [string, unknown] => [key, layout({ [key]: `url(${external})` })],
  ),
  ["nested child style", { layout: { children: { style: { maskImage: `url(${external})` } } } }],
  ["img src", { layout: { type: "img", src: external } }],
  ["protocol-relative", layout({ backgroundImage: "url(//example.com/image.png)" })],
  ["file URL", layout({ backgroundImage: "url(file:///tmp/private.png)" })],
  ["relative path", layout({ backgroundImage: "url(private.png)" })],
  ["CSS case and whitespace", layout({ backgroundImage: `URL ( '${external}' )` })],
  ["multiple backgrounds", layout({ backgroundImage: `url(${data}), url(${external})` })],
  ["CSS escaped identifier", layout({ backgroundImage: String.raw`u\72l(${external})` })],
  ["CSS comment", layout({ backgroundImage: `url/**/(${external})` })],
  ["image-set string URL", layout({ backgroundImage: `image-set('${external}' 1x)` })],
  ["SVG image", { svg: svg(`<image href="${external}"/>`) }],
  ["SVG feImage", { svg: svg(`<filter id="f"><feImage href="${external}"/></filter>`) }],
  ["SVG use", { svg: svg(`<use href="${external}#icon"/>`) }],
  ["SVG font-face-uri", { svg: svg(`<font-face-uri href="${external}"/>`) }],
  ["SVG foreign content", { svg: svg(`<foreignObject><img src="${external}"/></foreignObject>`) }],
  [
    "SVG arbitrary xlink prefix",
    { svg: svg(`<image xmlns:p="http://www.w3.org/1999/xlink" p:href="${external}"/>`) },
  ],
  ["SVG entity encoded path", { svg: svg('<image href="&#47;tmp/private.png"/>') }],
  ["SVG numeric entity URL function", { svg: svg(`<rect fill="u&#114;l(${external})"/>`) }],
  ["SVG presentation URL", { svg: svg(`<rect filter="url(${external}#filter)"/>`) }],
  ["SVG inline style", { svg: svg(`<rect style="fill: url('${external}')"/>`) }],
  ["SVG style sheet", { svg: svg(`<style>rect { fill: url(${external}); }</style>`) }],
  [
    "SVG CDATA style sheet",
    { svg: svg(`<style><![CDATA[rect { fill: url(${external}); }]]></style>`) },
  ],
  ["SVG import", { svg: svg(`<style>@import '${external}';</style>`) }],
  [
    "SVG font source",
    { svg: svg(`<style>@font-face { font-family: X; src: url(${external}); }</style>`) },
  ],
  [
    "SVG local font source",
    { svg: svg("<style>@font-face { font-family: X; src: local(private); }</style>") },
  ],
  ["XML base", { svg: svg(`<g xml:base="${external}"><use href="#icon"/></g>`) }],
  ["XML stylesheet instruction", { svg: `<?xml-stylesheet href="${external}"?>${svg("")}` }],
  ["XML external DTD", { svg: `<!DOCTYPE svg SYSTEM "${external}">${svg("")}` }],
  [
    "XML entity expansion",
    { svg: `<!DOCTYPE svg [<!ENTITY path "/tmp/private.png">]>${svg('<image href="&path;"/>')}` },
  ],
  ["image fragment falling back to disk", { svg: svg('<image href="#private.png"/>') }],
  [
    "feImage fragment falling back to disk",
    { svg: svg('<filter id="f"><feImage href="#private.png"/></filter>') },
  ],
  [
    "SVG inside img data",
    { layout: { type: "img", src: embeddedSvg(`<image href="${external}"/>`) } },
  ],
  [
    "SVG inside background data",
    layout({ backgroundImage: `url(${embeddedSvg(`<feImage href="${external}"/>`)})` }),
  ],
  [
    "SVG inside mask data",
    layout({ maskImage: `url(${embeddedSvg(`<style>@import '${external}';</style>`)})` }),
  ],
  [
    "SVG inside raw SVG data",
    { svg: svg(`<image href="${embeddedSvg('<image href="/tmp/private.png"/>')}"/>`) },
  ],
  [
    "percent encoded SVG data",
    {
      layout: {
        type: "img",
        src: `data:image/svg+xml,${encodeURIComponent(svg(`<image href="${external}"/>`))}`,
      },
    },
  ],
  [
    "text/plain SVG sniffing",
    {
      svg: svg(
        `<image href="data:text/plain;base64,${Buffer.from(svg(`<image href="${external}"/>`)).toString("base64")}"/>`,
      ),
    },
  ],
  [
    "compressed SVG data",
    {
      layout: {
        type: "img",
        src: `data:image/svg+xml;base64,${Buffer.from(Bun.gzipSync(svg(`<image href="${external}"/>`))).toString("base64")}`,
      },
    },
  ],
];

for (const [name, card] of refused) {
  test(`resource boundary refuses ${name} before resource loading`, async () => {
    const fetch = spyOn(globalThis, "fetch").mockRejectedValue(
      new Error("a renderer tried to fetch"),
    );
    const lookup = spyOn(dns, "lookup").mockRejectedValue(new Error("a renderer tried DNS"));
    try {
      const error = await cardToSvg(card).catch((error: unknown) => error);
      expect(error).toBeInstanceOf(CardError);
      expect((error as CardError).configuration).toBe(true);
      expect((error as Error).message).toMatch(/resource|data:|SVG|card style/);
      expect(fetch).not.toHaveBeenCalled();
      expect(lookup).not.toHaveBeenCalled();
    } finally {
      fetch.mockRestore();
      lookup.mockRestore();
    }
  });
}

for (const key of ["backgroundImage", "maskImage", "WebkitMaskImage"]) {
  test(`${key} accepts bounded data images and rejects oversized ones`, async () => {
    const accepted = await cardToSvg(layout({ [key]: `url(${data})` }));
    expect(pngFromSvg(accepted).length).toBeGreaterThan(0);
    const oversized = `data:image/png;base64,${"AAAA".repeat(350_000)}`;
    await expect(cardToSvg(layout({ [key]: `url(${oversized})` }))).rejects.toThrow(/1024 KB/);
  });
}

test("all layout image locations share the count and total-byte budgets", async () => {
  await expect(
    cardToSvg(layout({ backgroundImage: Array(5).fill(`url(${data})`).join(", ") })),
  ).rejects.toThrow(/4 images/);
  const large = `data:image/png;base64,${"AAAA".repeat(300_000)}`;
  await expect(
    cardToSvg({
      layout: {
        type: "div",
        style: { display: "flex", backgroundImage: `url(${large})`, maskImage: `url(${large})` },
        children: [
          { type: "img", src: large },
          { type: "img", src: large },
        ],
      },
    }),
  ).rejects.toThrow(/total/);
});

test("resource caps refuse excessive encoded data before decoding or allocation", async () => {
  const decode = spyOn(globalThis, "decodeURIComponent");
  try {
    await expect(
      cardToSvg({
        layout: { type: "img", src: `data:image/svg+xml,${"x".repeat(3 * 1024 * 1024 + 257)}` },
      }),
    ).rejects.toThrow(/1024 KB/);
    expect(decode).not.toHaveBeenCalled();
  } finally {
    decode.mockRestore();
  }
  const allocate = spyOn(Buffer, "from");
  try {
    await expect(
      cardToSvg({
        layout: { type: "img", src: `data:image/png;base64,${"AAAA".repeat(350_000)}` },
      }),
    ).rejects.toThrow(/1024 KB/);
    expect(allocate).not.toHaveBeenCalled();
  } finally {
    allocate.mockRestore();
  }
  await expect(
    cardToSvg({
      layout: { type: "img", src: `data:image/svg+xml,${"x".repeat(1024 * 1024 + 1)}` },
    }),
  ).rejects.toThrow(/1024 KB/);
});

test("malformed percent data and non-scalar styles fail as author errors", async () => {
  await expect(
    cardToSvg({ layout: { type: "img", src: "data:image/svg+xml,%ZZ" } }),
  ).rejects.toThrow(/could not be decoded/);
  for (const style of [[], null, { backgroundImage: [external] }]) {
    await expect(cardToSvg({ layout: { style } })).rejects.toThrow(/text or number/);
  }
});

test("compressed and non-UTF-8 SVG data have explicit author errors", async () => {
  await expect(
    cardToSvg({
      layout: {
        type: "img",
        src: `data:image/svg+xml;base64,${Buffer.from(Bun.gzipSync(svg(""))).toString("base64")}`,
      },
    }),
  ).rejects.toThrow(/compressed SVG/);
  await expect(
    cardToSvg({ layout: { type: "img", src: "data:image/svg+xml;base64,//4=" } }),
  ).rejects.toThrow(/UTF-8 text/);
});

test("embedded SVG is recursively checked without changing accepted bytes", async () => {
  const image = embeddedSvg('<rect width="448" height="368" fill="red"/>');
  const document = svg(`<image href="${image}"/>`);
  expect(await cardToSvg({ svg: document })).toBe(document);
  expect(pngFromSvg(document).length).toBeGreaterThan(0);
});

test("same-document paint/use references and bundled fonts still render", async () => {
  const document = svg(
    '<defs><linearGradient id="g"><stop stop-color="red"/></linearGradient><rect id="r" width="20" height="20" fill="url(#g)"/></defs><use href="#r"/><text y="60" font-family="Inter">OK</text>',
  );
  expect(await cardToSvg({ svg: document })).toBe(document);
  expect(pngFromSvg(document).length).toBeGreaterThan(0);
});

test("plugin card properties cannot supply Satori asset loaders, fonts or SVG nodes", async () => {
  const fetch = spyOn(globalThis, "fetch").mockRejectedValue(new Error("must not fetch"));
  try {
    const result = await cardToSvg({
      ...layout({ fontFamily: "Inter" }),
      fonts: [{ name: "Remote", data: external }],
      loadAdditionalAsset: external,
      graphemeImages: { x: external },
    });
    const svgNode = await cardToSvg({
      layout: { type: "image", href: external, src: external, children: "OK" },
    });
    expect(result).not.toContain(external);
    expect(svgNode).not.toContain(external);
    expect(fetch).not.toHaveBeenCalled();
  } finally {
    fetch.mockRestore();
  }
});
