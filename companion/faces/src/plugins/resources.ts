// Shared by live plugin rendering and author previews. Validate BEFORE Satori can
// load a resource, and again on the SVG BEFORE resvg can resolve local files.
// Audit of the pinned engines: docs/plugins/render-resource-audit.md.
import { XMLParser } from "fast-xml-parser";
import { INGEST_CAP_BYTES } from "../kit/limits";
import { CardError } from "./card-error";

const MAX_IMAGES = 4;
const MAX_TOTAL_IMAGE_BYTES = 3 * INGEST_CAP_BYTES;
export const BASE64 = /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/;

export interface ResourceBudget {
  images: number;
  imageBytes: number;
}

const budget = (): ResourceBudget => ({ images: 0, imageBytes: 0 });
const localName = (name: string): string => name.slice(name.lastIndexOf(":") + 1).toLowerCase();

// fast-xml-parser does not decode every numeric reference in attributes. Keep
// entity processing off and decode XML's five names/numbers once ourselves so
// resource checks see exactly the characters that usvg's XML parser sees.
function xmlText(value: unknown): string {
  return String(value).replace(/&([^;]+);/g, (_match, entity: string) => {
    const named: Record<string, string> = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'" };
    const decoded = named[entity];
    if (typeof decoded === "string") return decoded;
    if (!/^#(?:[0-9]+|x[0-9a-fA-F]+)$/.test(entity)) {
      throw new CardError("a card SVG contains an unsupported XML entity");
    }
    const code = entity.startsWith("#x")
      ? Number.parseInt(entity.slice(2), 16)
      : Number(entity.slice(1));
    if (code === 0 || code > 0x10ffff || (code >= 0xd800 && code <= 0xdfff)) {
      throw new CardError("a card SVG contains an invalid XML character reference");
    }
    return String.fromCodePoint(code);
  });
}

function refuse(where: string): never {
  throw new CardError(
    `${where}: external resources are not allowed in a card; use an embedded data: URI. Fetch images through plan() first.`,
  );
}

/** Only image data can reach an engine; in particular, no text/plain SVG sniffing
 * or compressed SVG that could hide another document from this validation. */
export function embeddedImage(uri: unknown, count: ResourceBudget, where: string): void {
  if (typeof uri !== "string" || !uri.startsWith("data:")) refuse(where);
  // Bound allocation even for percent-encoded payloads (three characters/byte).
  if (uri.length > INGEST_CAP_BYTES * 3 + 256) {
    throw new CardError("an image in this card may not exceed 1024 KB");
  }
  const header =
    /^data:(image\/(?:png|apng|jpeg|jpg|gif|webp|svg\+xml))(?:;charset=utf-8)?(;base64)?,/i.exec(
      uri,
    );
  if (!header) {
    throw new CardError(
      "a card data: URI must contain a PNG, JPEG, GIF, WebP or uncompressed SVG image",
    );
  }
  const payload = uri.slice(header[0].length);
  let data: Buffer;
  if (header[2]) {
    if (!BASE64.test(payload)) throw new CardError("an image data: URI must contain valid base64");
    if (Buffer.byteLength(payload, "base64") > INGEST_CAP_BYTES) {
      throw new CardError("an image in this card may not exceed 1024 KB");
    }
    data = Buffer.from(payload, "base64");
  } else {
    try {
      data = Buffer.from(decodeURIComponent(payload), "utf8");
    } catch {
      throw new CardError("an image data: URI could not be decoded");
    }
  }
  if (data.byteLength > INGEST_CAP_BYTES) {
    throw new CardError("an image in this card may not exceed 1024 KB");
  }
  count.images += 1;
  count.imageBytes += data.byteLength;
  if (count.images > MAX_IMAGES) throw new CardError("this card has more than 4 images");
  if (count.imageBytes > MAX_TOTAL_IMAGE_BYTES) {
    throw new CardError("the images in this card total more than 3072 KB");
  }
  if (header[1]?.toLowerCase() === "image/svg+xml") {
    if (data[0] === 0x1f && data[1] === 0x8b) {
      throw new CardError("compressed SVG images are not allowed in a card");
    }
    let svg: string;
    try {
      svg = new TextDecoder("utf-8", { fatal: true }).decode(data);
    } catch {
      throw new CardError("an embedded SVG must be UTF-8 text");
    }
    svgResources(svg, count);
  }
}

/** CSS escapes/comments and at-rules are deliberately outside the card subset.
 * Rejecting them avoids different URL interpretations in Satori and usvg. Scan
 * EVERY style property, including shorthands, masks, fonts and future additions. */
export function cssResources(css: string, count: ResourceBudget, fragments = false): void {
  if (/\\|\/\*|\*\/|@/.test(css)) {
    throw new CardError(
      "card styles cannot contain CSS escapes, comments or @ rules; external resources are not allowed",
    );
  }
  if (/\b(?:image-set|-webkit-image-set|image|src)\s*\(/i.test(css))
    refuse("this CSS image function");
  const urls = /url\s*\(/gi;
  while (urls.exec(css) !== null) {
    let index = urls.lastIndex;
    while (/\s/.test(css[index] ?? "") && index < css.length) index += 1;
    const quote = css[index] === '"' || css[index] === "'" ? css[index++] : undefined;
    const end = css.indexOf(quote ?? ")", index);
    if (end === -1) throw new CardError("a card style contains an incomplete url()");
    const uri = css.slice(index, end).trim();
    let close = end;
    if (quote) {
      close += 1;
      while (/\s/.test(css[close] ?? "") && close < css.length) close += 1;
    }
    if (css[close] !== ")") throw new CardError("a card style contains an invalid url()");
    if (!(fragments && uri.startsWith("#"))) embeddedImage(uri, count, "a card style url()");
    urls.lastIndex = close + 1;
  }
}

export function layoutStyle(
  style: unknown,
  count: ResourceBudget,
): Record<string, string | number> {
  if (style === undefined) return {};
  if (typeof style !== "object" || style === null || Array.isArray(style)) {
    throw new CardError("card styles must be an object of text or number values");
  }
  const result: Record<string, string | number> = {};
  for (const [name, value] of Object.entries(style)) {
    if (typeof value !== "string" && typeof value !== "number") {
      throw new CardError("card style values must be text or numbers");
    }
    if (typeof value === "string") cssResources(value, count);
    result[name] = value;
  }
  return result;
}

function styleText(nodes: unknown, cdata = false): string {
  if (!Array.isArray(nodes)) return "";
  return (nodes as Record<string, unknown>[])
    .map((node) =>
      Object.entries(node)
        .filter(([name]) => name !== ":@")
        .map(([name, value]) =>
          name === "#text"
            ? cdata
              ? String(value)
              : xmlText(value)
            : styleText(value, cdata || name === "#cdata"),
        )
        .join(""),
    )
    .join("");
}

/** Parse XML, including numeric entities and arbitrary namespace prefixes, rather
 * than looking for href with a regex. Validation never rewrites accepted SVGs. */
export function svgResources(svg: string, count: ResourceBudget = budget()): void {
  if (/<!\s*(?:DOCTYPE|ENTITY)\b/i.test(svg) || /<\?(?!xml(?:\s|\?>))/i.test(svg)) {
    throw new CardError(
      "SVG cards cannot contain DTDs, entities or processing instructions that load resources",
    );
  }
  let nodes: unknown;
  try {
    nodes = new XMLParser({
      preserveOrder: true,
      ignoreAttributes: false,
      attributeNamePrefix: "",
      parseTagValue: false,
      parseAttributeValue: false,
      processEntities: false,
      trimValues: false,
      ignoreDeclaration: true,
      cdataPropName: "#cdata",
      maxNestedTags: 400,
    }).parse(svg, true);
  } catch {
    throw new CardError("a card must contain valid SVG XML");
  }
  const pending: unknown[] = [nodes];
  while (pending.length) {
    const current = pending.pop();
    if (!Array.isArray(current)) continue;
    for (const node of current as Record<string, unknown>[]) {
      const tag = Object.keys(node).find((name) => name !== ":@");
      if (!tag) continue;
      const kind = localName(tag);
      if (kind === "#text") continue;
      // Concatenate XML text/CDATA before scanning; a URL token can straddle
      // their boundary even though the XML parser returns separate records.
      if (kind === "style") cssResources(styleText(node[tag]), count, true);
      for (const [name, raw] of Object.entries((node[":@"] ?? {}) as Record<string, unknown>)) {
        const value = xmlText(raw);
        const attribute = localName(name);
        if (attribute === "base") refuse("an SVG base URL");
        if (attribute === "href" || attribute === "src") {
          // usvg treats even '#file.png' as a disk path on image/feImage when
          // it cannot resolve a node. Only their data: form is accepted.
          if (!(value.startsWith("#") && kind !== "image" && kind !== "feimage")) {
            embeddedImage(value, count, "an SVG resource reference");
          }
        } else if (name !== "xmlns" && !name.startsWith("xmlns:")) {
          cssResources(value, count, true);
        }
      }
      pending.push(node[tag]);
    }
  }
}
