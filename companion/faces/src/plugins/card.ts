// What a plugin returns becomes an SVG here, and only here. The layout path goes
// through satori (flexbox), which is why a plugin never places text by hand and never
// meets `textWidth`'s letter-spacing trap. The SVG and PNG paths are pass-throughs
// with caps.
import satori from "satori";
import { CANVAS_HEIGHT, CANVAS_WIDTH } from "../kit/theme";

export class CardError extends Error {
  override name = "CardError";
}

const MAX_BOXES = 2_000;
const MAX_SVG_BYTES = 512 * 1024;
const MAX_PNG_BYTES = 1024 * 1024;
// A chain, as opposed to a wide tree, reaches yoga-layout's own recursion limit long
// before MAX_BOXES does: a 501-deep chain of otherwise-valid boxes (well under 2,000)
// was observed to throw a WASM "Out of bounds memory access" from yoga-layout, and
// that corrupts satori's shared module for every later render in this process, not
// just this one card. 400 leaves margin below the observed 501 boundary.
const MAX_DEPTH = 400;

let fonts: { name: string; data: ArrayBuffer; weight: 400 | 600; style: "normal" }[] | undefined;

async function loadFonts() {
  fonts ??= [
    {
      name: "Inter",
      data: await Bun.file(
        new URL("../../assets/fonts/Inter-Regular.ttf", import.meta.url),
      ).arrayBuffer(),
      weight: 400,
      style: "normal",
    },
    {
      name: "Inter",
      data: await Bun.file(
        new URL("../../assets/fonts/Inter-SemiBold.ttf", import.meta.url),
      ).arrayBuffer(),
      weight: 600,
      style: "normal",
    },
  ];
  return fonts;
}

function toElement(node: unknown, counter: { boxes: number }, depth = 0): unknown {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (typeof node !== "object" || node === null) {
    throw new CardError("a card node must be a box, a string or a number");
  }
  counter.boxes += 1;
  if (counter.boxes > MAX_BOXES) {
    throw new CardError(`this card has more than ${MAX_BOXES} boxes`);
  }
  if (depth > MAX_DEPTH) {
    throw new CardError(`this card is nested more than ${MAX_DEPTH} boxes deep`);
  }
  const { type, style, children, src } = node as {
    type?: unknown;
    style?: unknown;
    children?: unknown;
    src?: unknown;
  };
  const kind = type === "img" ? "img" : "div";
  return {
    type: kind,
    props: {
      style: (typeof style === "object" && style !== null ? style : {}) as Record<string, unknown>,
      ...(kind === "img" && typeof src === "string" ? { src } : {}),
      ...(children === undefined
        ? {}
        : {
            children: Array.isArray(children)
              ? children.map((child) => toElement(child, counter, depth + 1))
              : toElement(children, counter, depth + 1),
          }),
    },
  };
}

/** Every reference an SVG card may carry: embedded data only. */
function refuseExternalReferences(svg: string): void {
  // XML permits either quote style for an attribute value and is case-sensitive on
  // the attribute name -- but usvg's own href lookup is not guaranteed to be
  // stricter than a hostile author, so this matches "href" case-insensitively and
  // both quote styles rather than trusting either narrowing to hold downstream.
  for (const match of svg.matchAll(/(?:xlink:)?href\s*=\s*(?:"([^"]*)"|'([^']*)')/gi)) {
    const value = (match[1] ?? match[2] ?? "").trim();
    if (!value.startsWith("data:") && !value.startsWith("#")) {
      // usvg resolves a path href FROM OUR DISK by default (usvg-0.45.1
      // src/parser/image.rs:85-100), so this is a file-read surface, not a nicety.
      throw new CardError(
        `an SVG card may only reference embedded data, not ${JSON.stringify(value.slice(0, 40))}`,
      );
    }
  }
}

export async function cardToSvg(card: unknown): Promise<string> {
  if (typeof card !== "object" || card === null)
    throw new CardError("this plugin returned no card");
  const { layout, svg, png } = card as { layout?: unknown; svg?: unknown; png?: unknown };
  if (typeof svg === "string") {
    if (svg.length > MAX_SVG_BYTES)
      throw new CardError(`an SVG card may not exceed ${MAX_SVG_BYTES / 1024} KB`);
    refuseExternalReferences(svg);
    return svg;
  }
  if (typeof png === "string") {
    if (png.length * 0.75 > MAX_PNG_BYTES)
      throw new CardError(`a PNG card may not exceed ${MAX_PNG_BYTES / 1024} KB`);
    return (
      `<svg xmlns="http://www.w3.org/2000/svg" width="${CANVAS_WIDTH}" height="${CANVAS_HEIGHT}">` +
      `<image x="0" y="0" width="${CANVAS_WIDTH}" height="${CANVAS_HEIGHT}" preserveAspectRatio="xMidYMid meet" href="data:image/png;base64,${png}"/></svg>`
    );
  }
  if (layout === undefined) throw new CardError("a card must be a layout, an svg or a png");
  const element = toElement(layout, { boxes: 0 });
  return satori(element as never, {
    width: CANVAS_WIDTH,
    height: CANVAS_HEIGHT,
    fonts: await loadFonts(),
    // satori's default embeds every glyph as a <path>, which is pixel-accurate but
    // makes wrapped text indistinguishable from one long run -- the panel only cares
    // that it fits, so plain <text> (rasterized downstream by resvg, which carries
    // the same two Inter fonts) is enough and keeps the SVG far smaller.
    embedFont: false,
  });
}
