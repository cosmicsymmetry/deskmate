// Turning an authored SVG into pixels, and measuring text the way it will be drawn.
//
// `@resvg/resvg-js` is the same `resvg` engine the Rust server used, behind a
// binding. The font database holds the two bundled Inter faces and nothing else,
// so a face rasterizes to identical bytes on a Mac, in CI and on docker-vm --
// which is what makes a golden test a statement about the face rather than about
// the fonts the machine happened to have.

import { fileURLToPath } from "node:url";
import { Resvg, type ResvgRenderOptions } from "@resvg/resvg-js";
import { ConfigurationError } from "../face";
import { CANVAS_HEIGHT, CANVAS_WIDTH } from "./theme";

export const FONT_FAMILY = "Inter";

const FONT_FILES = ["Inter-Regular.ttf", "Inter-SemiBold.ttf"].map((name) =>
  fileURLToPath(new URL(`../../assets/fonts/${name}`, import.meta.url)),
);

function options(extra: Partial<ResvgRenderOptions> = {}): ResvgRenderOptions {
  return {
    font: {
      fontFiles: FONT_FILES,
      loadSystemFonts: false,
      defaultFontFamily: FONT_FAMILY,
      sansSerifFamily: FONT_FAMILY,
    },
    // The SVG is authored in device pixels: one user unit is one pixel.
    dpi: 96,
    ...extra,
  };
}

/**
 * Rasterizes a face to PNG bytes over opaque black -- the panel's ground, and the
 * same composite-over-black rule the server's PNG ingest applies.
 *
 * `zoom` exists for review only: 0.4 is roughly the panel's physical size on a
 * desktop display, which is the honest way to judge whether type is legible.
 */
export function pngFromSvg(svg: string, zoom = 1): Uint8Array {
  const resvg = new Resvg(
    svg,
    options({
      background: "#000000",
      ...(zoom === 1 ? {} : { fitTo: { mode: "zoom", value: zoom } }),
    }),
  );
  // The server entrypoint and author checker share this boundary. Use the
  // engine's dimensions: SVG attributes alone miss viewBox-only sizes, units
  // and CSS overrides. Refuse huge canvases before allocating their pixels.
  requirePanelSize(resvg.width, resvg.height);
  const rendered = resvg.render();
  requirePanelSize(rendered.width, rendered.height, zoom);
  return rendered.asPng();
}

function requirePanelSize(width: number, height: number, zoom = 1): void {
  const expectedWidth = Math.round(CANVAS_WIDTH * zoom);
  const expectedHeight = Math.round(CANVAS_HEIGHT * zoom);
  if (width !== expectedWidth || height !== expectedHeight) {
    throw new ConfigurationError(
      `the picture is ${width}x${height}; it must be exactly ${expectedWidth}x${expectedHeight}`,
    );
  }
}

const widths = new Map<string, number>();

/**
 * Exact advance width of one run in the bundled Inter faces, tracking excluded.
 *
 * Measured, not estimated: the run is laid out through the engine that will draw
 * it and the bounding box is read back. A fixed panel gives an underestimate
 * nowhere to go -- a headline measured 5% narrow runs off the canvas with no
 * scrollbar to reveal it.
 */
export function textWidth(text: string, size: number, weight: number): number {
  if (text.trim() === "") {
    return 0;
  }
  const key = `${weight}/${size}/${text}`;
  const cached = widths.get(key);
  if (cached !== undefined) {
    return cached;
  }
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="20000" height="${Math.ceil(size * 4)}">` +
    `<text x="0" y="${Math.ceil(size * 2)}" font-family="${FONT_FAMILY}" font-size="${size}" ` +
    `font-weight="${weight}">${escapeXml(text)}</text></svg>`;
  let width = 0;
  try {
    width = new Resvg(svg, options()).getBBox()?.width ?? 0;
  } catch {
    width = 0;
  }
  widths.set(key, width);
  return width;
}

/** Where a run's INK is, relative to its origin: x from the pen start, y from the baseline. */
export interface Ink {
  left: number;
  right: number;
  /** Negative: above the baseline. */
  top: number;
  bottom: number;
}

const inks = new Map<string, Ink>();

/**
 * The bounding box of what is actually painted, as opposed to `textWidth`'s advance.
 *
 * Advances are right for running text and wrong for a composed numeral: every glyph has
 * its own side bearings, so two runs set an advance apart are a different distance apart
 * for every price, and a "$" is taller than its own S, because its stroke overshoots
 * the cap line. Optical alignment needs the ink.
 */
export function textInk(text: string, size: number, weight: number): Ink {
  const key = `${weight}/${size}/${text}`;
  const cached = inks.get(key);
  if (cached !== undefined) {
    return cached;
  }
  const originX = 1_000;
  const originY = Math.ceil(size * 2);
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="20000" height="${Math.ceil(size * 4)}">` +
    `<text x="${originX}" y="${originY}" font-family="${FONT_FAMILY}" font-size="${size}" ` +
    `font-weight="${weight}">${escapeXml(text)}</text></svg>`;
  let ink: Ink = { left: 0, right: 0, top: 0, bottom: 0 };
  try {
    const box = new Resvg(svg, options()).getBBox();
    if (box !== undefined) {
      ink = {
        left: box.x - originX,
        right: box.x + box.width - originX,
        top: box.y - originY,
        bottom: box.y + box.height - originY,
      };
    }
  } catch {
    // A run that cannot be measured is placed as if it had no ink: drawn, unadjusted.
  }
  inks.set(key, ink);
  return ink;
}

/**
 * Escapes text for an XML text node or attribute value.
 *
 * Every string a feed, a ticker or a geocoder supplies passes through here on its
 * way into the document. All five predefined entities are escaped so the same
 * function is correct in both positions, and the control characters XML 1.0
 * forbids are dropped rather than allowed to make the document unparseable.
 */
export function escapeXml(text: string): string {
  let escaped = "";
  for (const character of text) {
    const code = character.codePointAt(0) ?? 0;
    if (character === "&") escaped += "&amp;";
    else if (character === "<") escaped += "&lt;";
    else if (character === ">") escaped += "&gt;";
    else if (character === '"') escaped += "&quot;";
    else if (character === "'") escaped += "&apos;";
    else if (character === "\t" || character === "\n" || character === "\r") escaped += " ";
    else if (code < 0x20) continue;
    else escaped += character;
  }
  return escaped;
}
