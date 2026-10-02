// What a plugin returns becomes an SVG here, and only here. The layout path goes
// through satori (flexbox), which is why a plugin never places text by hand and never
// meets `textWidth`'s letter-spacing trap. The SVG and PNG paths are pass-throughs
// with caps.
import { INGEST_CAP_BYTES } from "../kit/limits";
import { CANVAS_HEIGHT, CANVAS_WIDTH } from "../kit/theme";
import satori from "satori";
import { CardError } from "./card-error";
import { BASE64, embeddedImage, layoutStyle, svgResources } from "./resources";
export { CardError } from "./card-error";

const MAX_BOXES = 2_000;
const MAX_SVG_BYTES = 512 * 1024;
// The ingest cap, shared with the fetch body cap and the render byte budget.
const MAX_PNG_BYTES = INGEST_CAP_BYTES;
// A chain, as opposed to a wide tree, reaches yoga-layout's own recursion limit long
// before MAX_BOXES does: a 501-deep chain of otherwise-valid boxes (well under 2,000)
// was observed to throw a WASM "Out of bounds memory access" from yoga-layout, and
// that corrupts satori's shared module for every later render in this process, not
// just this one card. 400 leaves margin below the observed 501 boundary.
const MAX_DEPTH = 400;
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
    embeddedImage(src, counter, "an img src");
    imgSrc = src as string;
  }
  return {
    type: kind,
    props: {
      style: layoutStyle(style, counter),
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
  return new CardError(`the layout could not be rendered: ${message}`, false);
}

/**
 * Base64 as the `{png}` shape is allowed to be written: the standard alphabet, no
 * whitespace, no line breaks, padded to a whole number of quanta.
 *
 * `Buffer.byteLength(x, "base64")` is a LENGTH ESTIMATE -- it decides nothing about
 * whether `x` is base64 at all, and the PNG branch concatenates `x` straight into an
 * SVG document. Without this check `AAAA"/><image href="/etc/passwd" x="0"/><x y="`
 * closes our own `<image>` element and opens one pointing at a path, which usvg
 * resolves BY READING THAT FILE FROM OUR DISK -- the exact reference
 * `svgResources` exists to refuse on the `{svg}` branch.
 */
function refuseNonBase64(png: string): void {
  if (!BASE64.test(png)) {
    throw new CardError(
      `a PNG card must be base64, not ${JSON.stringify(png.slice(0, 40))}${png.length > 40 ? "..." : ""}`,
    );
  }
}

/**
 * The branch that turns one of the three card shapes into an SVG document. Its result
 * is NOT the function's guarantee -- `cardToSvg` below is, because it runs
 * `svgResources` over whatever this returns. That is deliberate: the PNG
 * branch below builds its document by string concatenation, and before the
 * whole-branch review only the `svg` branch was guarded, so a plugin could put an
 * arbitrary `href` into the frame simply by picking the other shape. The guarantee
 * belongs to what this module RETURNS, not to which branch produced it, so a fourth
 * shape added later cannot reintroduce the hole by forgetting a call.
 */
async function buildCardSvg(card: unknown): Promise<string> {
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
    return svg;
  }
  if (typeof png === "string") {
    // Alphabet first, size second: byteLength(..., "base64") reports a plausible
    // number for a string that is not base64 at all, so a size check on its own
    // validates nothing about what gets concatenated into the document below.
    refuseNonBase64(png);
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

/**
 * The card a plugin returned, as an SVG document -- and the one place that decides
 * what an SVG document this package will rasterize may reference. Every path out of
 * `buildCardSvg` passes through `svgResources` here, so "no card can make
 * usvg read our disk" is a property of the return value rather than of any one branch.
 */
export async function cardToSvg(card: unknown): Promise<string> {
  const svg = await buildCardSvg(card);
  svgResources(svg);
  return svg;
}
