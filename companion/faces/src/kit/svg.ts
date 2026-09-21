// The small SVG-authoring vocabulary the faces share.
//
// A face is a pure function from a view model to an SVG document string. Nothing
// here fetches or rasterizes; the only thing it leans on is text measurement,
// because a fixed panel has no scrollbar and text must be fitted before it is drawn.

import { FONT_FAMILY, escapeXml, textInk, textWidth } from "./raster";

export { escapeXml, textInk, textWidth };

/**
 * Fixed-point formatting with ties to even, matching the Rust `{:.N}` the original
 * faces were written against -- so a ported face emits a byte-identical document.
 * `toFixed` alone rounds an exact tie away from zero, and grid arithmetic does
 * produce exact ties (n/8 is exactly representable).
 */
export function fixed(value: number, digits: number): string {
  if (Object.is(value, -0)) {
    return `-${(0).toFixed(digits)}`;
  }
  const exact = value.toFixed(digits + 60);
  const cut = exact.length - 60;
  const tail = exact.slice(cut);
  if (tail[0] !== "5" || /[1-9]/.test(tail.slice(1))) {
    return value.toFixed(digits);
  }
  // An exact tie: keep the truncated value when its last digit is even.
  const truncated = exact.slice(0, cut);
  const last = truncated.replace(/[^0-9]/g, "").at(-1) ?? "0";
  if (Number(last) % 2 === 0) {
    return truncated.endsWith(".") ? truncated.slice(0, -1) : truncated;
  }
  return value.toFixed(digits);
}

const f2 = (value: number): string => fixed(value, 2);
const f3 = (value: number): string => fixed(value, 3);

/** Collapses runs of whitespace so a hard-wrapped title measures as one paragraph. */
export function normalizeWhitespace(text: string): string {
  return text.split(/\s+/u).filter(Boolean).join(" ");
}

export type Anchor = "start" | "middle" | "end";

/** One run of text, positioned by its baseline. */
export interface Text {
  x: number;
  baseline: number;
  content: string;
  size: number;
  fill: string;
  weight?: number;
  anchor?: Anchor;
  /** Positive values track the type out; small uppercase needs it on an emissive panel. */
  tracking?: number;
  opacity?: number;
}

/** The document under construction. */
export class Canvas {
  private body = "";

  constructor(
    private readonly width: number,
    private readonly height: number,
  ) {}

  rect(x: number, y: number, w: number, h: number, fill: string): void {
    this.body += `<rect x="${f2(x)}" y="${f2(y)}" width="${f2(w)}" height="${f2(h)}" fill="${fill}"/>`;
  }

  roundedRect(x: number, y: number, w: number, h: number, radius: number, fill: string): void {
    this.body += `<rect x="${f2(x)}" y="${f2(y)}" width="${f2(w)}" height="${f2(h)}" rx="${f2(radius)}" fill="${fill}"/>`;
  }

  /** A rect at partial opacity, for a tint of an existing role. */
  rectOpacity(
    x: number,
    y: number,
    w: number,
    h: number,
    radius: number,
    fill: string,
    opacity: number,
  ): void {
    this.body += `<rect x="${f2(x)}" y="${f2(y)}" width="${f2(w)}" height="${f2(h)}" rx="${f2(radius)}" fill="${fill}" fill-opacity="${f3(opacity)}"/>`;
  }

  circle(cx: number, cy: number, r: number, fill: string): void {
    this.body += `<circle cx="${f2(cx)}" cy="${f2(cy)}" r="${f2(r)}" fill="${fill}"/>`;
  }

  line(x1: number, y1: number, x2: number, y2: number, stroke: string, width: number): void {
    this.body += `<line x1="${f2(x1)}" y1="${f2(y1)}" x2="${f2(x2)}" y2="${f2(y2)}" stroke="${stroke}" stroke-width="${f2(width)}" stroke-linecap="round"/>`;
  }

  path(definition: string, fill: string): void {
    this.body += `<path d="${definition}" fill="${fill}"/>`;
  }

  strokedPath(definition: string, stroke: string, width: number): void {
    this.body += `<path d="${definition}" fill="none" stroke="${stroke}" stroke-width="${f2(width)}" stroke-linecap="round" stroke-linejoin="round"/>`;
  }

  filledPathOpacity(definition: string, fill: string, opacity: number): void {
    this.body += `<path d="${definition}" fill="${fill}" fill-opacity="${f3(opacity)}"/>`;
  }

  text(text: Text): void {
    const tracking = text.tracking ?? 0;
    const opacity = text.opacity ?? 1;
    const trackingAttribute =
      Math.abs(tracking) < Number.EPSILON ? "" : ` letter-spacing="${f2(tracking)}"`;
    const opacityAttribute =
      Math.abs(opacity - 1) < Number.EPSILON ? "" : ` fill-opacity="${f3(opacity)}"`;
    this.body +=
      `<text x="${f2(text.x)}" y="${f2(text.baseline)}" font-family="${FONT_FAMILY}" ` +
      `font-size="${f2(text.size)}" font-weight="${text.weight ?? 400}" fill="${text.fill}" ` +
      `text-anchor="${text.anchor ?? "start"}"${trackingAttribute}${opacityAttribute}>` +
      `${escapeXml(text.content)}</text>`;
  }

  finish(): string {
    // No `<defs>`: every face is flat fills and strokes, which is what keeps RLE565
    // compressing a frame to ~10 KB instead of the ~330 KB a gradient would force.
    const width = fixed(this.width, 0);
    const height = fixed(this.height, 0);
    return `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">${this.body}</svg>`;
  }
}

const ELLIPSIS = "…";

/**
 * Greedily breaks `text` into at most `maxLines` lines no wider than `maxWidth`,
 * ellipsizing the last line when the text runs out of room.
 *
 * Measured, not estimated: "Illinois" and "Wollongong" have the same character
 * count and very different widths.
 */
export function wrap(
  text: string,
  size: number,
  weight: number,
  maxWidth: number,
  maxLines: number,
): string[] {
  const normalized = normalizeWhitespace(text);
  if (normalized === "" || maxLines === 0) {
    return [];
  }

  const lines: string[] = [];
  let current = "";
  const words = normalized.split(" ");

  for (let index = 0; index < words.length; index += 1) {
    const word = words[index] ?? "";
    const candidate = current === "" ? word : `${current} ${word}`;

    if (textWidth(candidate, size, weight) <= maxWidth) {
      current = candidate;
      continue;
    }

    if (current !== "") {
      lines.push(current);
      current = "";
      if (lines.length === maxLines) {
        const remainder = words.slice(index).join(" ");
        return finishLines(lines, remainder, false, size, weight, maxWidth);
      }
    }

    // One word that does not fit on its own line. Break it by character rather
    // than letting it overhang -- and keep breaking, because one split is only
    // enough for a word under twice the box width.
    let remainder = word;
    while (remainder !== "" && lines.length < maxLines) {
      if (textWidth(remainder, size, weight) <= maxWidth) {
        break;
      }
      const [head, tail] = breakWord(remainder, size, weight, maxWidth);
      lines.push(head);
      remainder = tail;
    }
    if (lines.length === maxLines) {
      return finishLines(lines, remainder, index + 1 < words.length, size, weight, maxWidth);
    }
    current = remainder;
  }

  if (current !== "") {
    lines.push(current);
  }
  return lines;
}

/** Replaces the final line with an ellipsized version when text remains. */
function finishLines(
  lines: string[],
  remainder: string,
  moreWords: boolean,
  size: number,
  weight: number,
  maxWidth: number,
): string[] {
  if (remainder.trim() === "" && !moreWords) {
    return lines;
  }
  const last = lines.pop();
  if (last !== undefined) {
    lines.push(ellipsize(last, size, weight, maxWidth));
  }
  return lines;
}

/**
 * The drawn width of a run, tracking included.
 *
 * `textWidth` measures with no `letter-spacing`, and every eyebrow is tracked out.
 * The spacing is added to each character's advance, so a 27-character place name
 * tracked at 1.4 is ~38px wider than it measures. Use this wherever a run is
 * tracked, or a long label draws through its neighbour while measuring as fitted.
 */
export function trackedWidth(text: string, size: number, weight: number, tracking: number): number {
  if (text === "") {
    return 0;
  }
  // Counted over code points, which is what the shaper spaces.
  return textWidth(text, size, weight) + tracking * [...text].length;
}

function trimWithEllipsis(
  text: string,
  size: number,
  weight: number,
  tracking: number,
  maxWidth: number,
): string {
  const characters = [...text];
  while (characters.length > 0) {
    characters.pop();
    const candidate = `${characters.join("").trimEnd()}${ELLIPSIS}`;
    if (trackedWidth(candidate, size, weight, tracking) <= maxWidth) {
      return candidate;
    }
  }
  return ELLIPSIS;
}

/** Fits a run to `maxWidth`, tracking included, ellipsizing only if needed. */
export function fitTracked(
  text: string,
  size: number,
  weight: number,
  tracking: number,
  maxWidth: number,
): string {
  if (trackedWidth(text, size, weight, tracking) <= maxWidth) {
    return text;
  }
  return trimWithEllipsis(text, size, weight, tracking, maxWidth);
}

/** Returns `text` unchanged when it fits, otherwise truncates it with an ellipsis. */
export function fit(text: string, size: number, weight: number, maxWidth: number): string {
  if (textWidth(text, size, weight) <= maxWidth) {
    return text;
  }
  return ellipsize(text, size, weight, maxWidth);
}

function ellipsize(text: string, size: number, weight: number, maxWidth: number): string {
  if (textWidth(text, size, weight) <= maxWidth) {
    const ellipsized = `${text.trimEnd()}${ELLIPSIS}`;
    if (textWidth(ellipsized, size, weight) <= maxWidth) {
      return ellipsized;
    }
  }
  return trimWithEllipsis(text, size, weight, 0, maxWidth);
}

/** Splits an unbreakable word at the last character that fits. */
function breakWord(word: string, size: number, weight: number, maxWidth: number): [string, string] {
  const characters = [...word];
  let fits = 0;
  for (let index = 1; index <= characters.length; index += 1) {
    if (textWidth(characters.slice(0, index).join(""), size, weight) <= maxWidth) {
      fits = index;
    } else {
      break;
    }
  }
  // Always consume at least one character, or a box narrower than a single glyph
  // would loop forever.
  const split = Math.max(fits, 1);
  return [characters.slice(0, split).join(""), characters.slice(split).join("")];
}

/**
 * The largest of `candidates` (in the order given) at which `text` fits `maxWidth`
 * on one line, falling back to the last. This is what keeps a hero numeral as
 * large as it can be rather than sized for the widest possible input.
 */
export function fitSize(
  text: string,
  weight: number,
  maxWidth: number,
  candidates: readonly number[],
): number {
  for (const size of candidates) {
    if (textWidth(text, size, weight) <= maxWidth) {
      return size;
    }
  }
  return candidates.at(-1) ?? 12;
}
