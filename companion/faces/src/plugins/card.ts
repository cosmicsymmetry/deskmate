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
// An img inside a {layout} card is otherwise uncapped: MAX_SVG_BYTES/MAX_PNG_BYTES
// only guard the two top-level shapes, so a plugin can defeat both entirely by
// embedding an oversized image as img.src instead (observed: a 5 MB data: URI
// produced a 6.99 MB SVG). Each image gets the same 1 MB ceiling the {png} shape
// itself uses, per the spec's card-limits table.
const MAX_IMAGES = 4;
// Kept below MAX_IMAGES x MAX_PNG_BYTES (4 MB) on purpose, mirroring the spec's own
// "N MB total, tighter than count x per-item" pattern (the fetch response-body cap
// is 4 MB total against 8 requests x 1 MB each) -- at exactly 4 MB this check could
// never fire independently of the per-image and count caps, which would make it
// dead code rather than an actual third guard.
const MAX_TOTAL_IMAGE_BYTES = 3 * 1024 * 1024;
// A few hundred characters is enough to identify what satori rejected; a hostile
// style value has been observed echoed back whole (megabytes) in a raw error's
// message, which is a log-flooding footgun once that message reaches anywhere logs
// are kept.
const MAX_ERROR_MESSAGE = 300;

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

/** Decoded byte size of a `data:` URI's payload -- without ever allocating the bytes. */
function dataUriByteLength(uri: string): number {
  const comma = uri.indexOf(",");
  if (comma === -1) {
    throw new CardError("an img src's data: URI has no payload");
  }
  const header = uri.slice(5, comma); // after "data:"
  const payload = uri.slice(comma + 1);
  if (/;base64/i.test(header)) {
    return Buffer.byteLength(payload, "base64");
  }
  try {
    return Buffer.byteLength(decodeURIComponent(payload), "utf8");
  } catch {
    throw new CardError("an img src's data: URI could not be decoded");
  }
}

function toElement(
  node: unknown,
  counter: { boxes: number; images: number; imageBytes: number },
  depth = 0,
): unknown {
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
  let imgSrc: string | undefined;
  if (kind === "img") {
    if (typeof src !== "string" || !src.startsWith("data:")) {
      // Not a nicety: satori resolves a non-"data:" img.src with its OWN raw
      // `fetch()` (node_modules/satori/dist/index.js, the `xt()` image loader),
      // bypassing kit/http.ts's private/loopback/metadata guard entirely -- an
      // unsanctioned SSRF surface, the same class of hole the SVG shape's href
      // check exists to close.
      throw new CardError(
        `an img src must be an embedded data: URI, not ${JSON.stringify(
          typeof src === "string" ? src.slice(0, 40) : typeof src,
        )}`,
      );
    }
    counter.images += 1;
    if (counter.images > MAX_IMAGES) {
      throw new CardError(`this card has more than ${MAX_IMAGES} images`);
    }
    const bytes = dataUriByteLength(src);
    if (bytes > MAX_PNG_BYTES) {
      throw new CardError(`an image in this card may not exceed ${MAX_PNG_BYTES / 1024} KB`);
    }
    counter.imageBytes += bytes;
    if (counter.imageBytes > MAX_TOTAL_IMAGE_BYTES) {
      throw new CardError(
        `the images in this card total more than ${MAX_TOTAL_IMAGE_BYTES / 1024} KB`,
      );
    }
    imgSrc = src;
  }
  return {
    type: kind,
    props: {
      style: (typeof style === "object" && style !== null ? style : {}) as Record<string, unknown>,
      ...(imgSrc === undefined ? {} : { src: imgSrc }),
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
  //
  // This regex is NOT the whole boundary. usvg parses strict XML before it ever
  // resolves an href, and refuses both an unquoted attribute value and an
  // entity-encoded attribute name (e.g. `&#104;ref="..."`) as malformed XML --
  // neither form reaches image resolution even though this regex does not match
  // either of them. That parse-time rejection is load-bearing here, not incidental:
  // do not read this function as the sole guard when touching the regex.
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

/** Refuses a declared root size other than the panel's, if the card bothered to declare one. */
function refuseWrongDimensions(svg: string): void {
  const tag = svg.match(/<svg\b[^>]*>/i)?.[0];
  if (tag === undefined) return;
  const width = tag.match(/\bwidth\s*=\s*"([^"]*)"|\bwidth\s*=\s*'([^']*)'/i);
  const height = tag.match(/\bheight\s*=\s*"([^"]*)"|\bheight\s*=\s*'([^']*)'/i);
  const declaredWidth = width?.[1] ?? width?.[2];
  const declaredHeight = height?.[1] ?? height?.[2];
  if (declaredWidth !== undefined && Number.parseFloat(declaredWidth) !== CANVAS_WIDTH) {
    throw new CardError(
      `an SVG card must be ${CANVAS_WIDTH}x${CANVAS_HEIGHT}, not width="${declaredWidth}"`,
    );
  }
  if (declaredHeight !== undefined && Number.parseFloat(declaredHeight) !== CANVAS_HEIGHT) {
    throw new CardError(
      `an SVG card must be ${CANVAS_WIDTH}x${CANVAS_HEIGHT}, not height="${declaredHeight}"`,
    );
  }
}

/**
 * Wraps whatever satori threw as a CardError, message truncated. The runtime around
 * this module tells "the plugin misbehaved" from "our bug" by the error's TYPE, so a
 * raw satori error (a `.trim is not a function` from its CSS parser on a function
 * style value, a `RangeError` decoding a corrupt image) is currently misclassified as
 * ours. It is also unbounded in size: a 5,000,000-character style value has been
 * observed echoed back whole in the message, which is a log-flooding footgun the
 * moment that message reaches anywhere logs are kept.
 */
function toCardError(error: unknown): CardError {
  if (error instanceof CardError) return error;
  const raw = error instanceof Error ? error.message : String(error);
  const message =
    raw.length > MAX_ERROR_MESSAGE
      ? `${raw.slice(0, MAX_ERROR_MESSAGE)}... (truncated from ${raw.length} characters)`
      : raw;
  return new CardError(`the layout could not be rendered: ${message}`);
}

export async function cardToSvg(card: unknown): Promise<string> {
  if (typeof card !== "object" || card === null)
    throw new CardError("this plugin returned no card");
  const { layout, svg, png } = card as { layout?: unknown; svg?: unknown; png?: unknown };
  // svg wins over layout when a plugin sets both -- checked first, deliberately, so
  // this is precedence, not an oversight a future reader might "fix" by reordering.
  if (typeof svg === "string") {
    // .length counts UTF-16 code units, not bytes -- a card built from multi-byte
    // text (any non-Latin script, or emoji) can be well under the cap in .length
    // while over it in the bytes an attacker actually gets to spend. Count bytes.
    const svgBytes = Buffer.byteLength(svg, "utf8");
    if (svgBytes > MAX_SVG_BYTES)
      throw new CardError(`an SVG card may not exceed ${MAX_SVG_BYTES / 1024} KB`);
    refuseWrongDimensions(svg);
    refuseExternalReferences(svg);
    return svg;
  }
  if (typeof png === "string") {
    // Exact decoded size, not the usual base64-length * 0.75 estimate: Buffer's own
    // byteLength(..., "base64") accounts for padding precisely, and the input here
    // is untrusted, so "safe estimate" is not a property worth trusting either way.
    const pngBytes = Buffer.byteLength(png, "base64");
    if (pngBytes > MAX_PNG_BYTES)
      throw new CardError(`a PNG card may not exceed ${MAX_PNG_BYTES / 1024} KB`);
    return (
      `<svg xmlns="http://www.w3.org/2000/svg" width="${CANVAS_WIDTH}" height="${CANVAS_HEIGHT}">` +
      `<image x="0" y="0" width="${CANVAS_WIDTH}" height="${CANVAS_HEIGHT}" preserveAspectRatio="xMidYMid meet" href="data:image/png;base64,${png}"/></svg>`
    );
  }
  if (layout === undefined) throw new CardError("a card must be a layout, an svg or a png");
  const element = toElement(layout, { boxes: 0, images: 0, imageBytes: 0 });
  // Loaded outside the try below on purpose: a missing/unreadable font file is OUR
  // bug, not the plugin's, and toCardError's job is to convert the latter, not hide
  // the former behind the same error type.
  const fontList = await loadFonts();
  try {
    return await satori(element as never, {
      width: CANVAS_WIDTH,
      height: CANVAS_HEIGHT,
      fonts: fontList,
      // satori's default embeds every glyph as a <path>, which is pixel-accurate but
      // makes wrapped text indistinguishable from one long run -- the panel only cares
      // that it fits, so plain <text> (rasterized downstream by resvg, which carries
      // the same two Inter fonts) is enough and keeps the SVG far smaller.
      embedFont: false,
    });
  } catch (error) {
    throw toCardError(error);
  }
}
