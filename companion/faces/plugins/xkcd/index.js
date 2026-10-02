// Bundled into each standalone plugin because the sandbox has no imports.
// Structural validation, not a second image decoder: see README for its limits.
function validatedImageMime(base64) {
  if (typeof base64 !== "string" || !base64.length || base64.length > 1398104 || base64.length % 4)
    return "";
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  const lookup = new Uint8Array(128);
  lookup.fill(255);
  for (let i = 0; i < alphabet.length; i++) lookup[alphabet.charCodeAt(i)] = i;
  const padding = base64.endsWith("==") ? 2 : base64.endsWith("=") ? 1 : 0;
  const bytes = new Uint8Array((base64.length / 4) * 3 - padding);
  if (bytes.length > 1048576) return "";
  let out = 0;
  for (let i = 0; i < base64.length; i += 4) {
    let value = 0;
    for (let j = 0; j < 4; j++) {
      const code = base64.charCodeAt(i + j);
      const part = code === 61 && i + j >= base64.length - padding ? 0 : lookup[code];
      if (part === undefined || part === 255) return "";
      value = (value << 6) | part;
    }
    if (out < bytes.length) bytes[out++] = value >>> 16;
    if (out < bytes.length) bytes[out++] = value >>> 8;
    if (out < bytes.length) bytes[out++] = value;
  }
  const word = (i) =>
    (bytes[i] * 0x1000000 + (bytes[i + 1] << 16) + (bytes[i + 2] << 8) + bytes[i + 3]) >>> 0;
  const short = (i) => (bytes[i] << 8) | bytes[i + 1];
  const little = (i) => bytes[i] | (bytes[i + 1] << 8);
  const text = (i, n) => String.fromCharCode(...bytes.subarray(i, i + n));
  if (bytes.length >= 8 && text(0, 8) === "\x89PNG\r\n\x1a\n") {
    const crcTable = new Uint32Array(256);
    for (let i = 0; i < 256; i++) {
      let c = i;
      for (let bit = 0; bit < 8; bit++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      crcTable[i] = c >>> 0;
    }
    let cursor = 8,
      header = false,
      palette = false,
      idat = 0,
      idatEnded = false,
      color = -1;
    const zlib = [];
    while (cursor + 12 <= bytes.length) {
      const length = word(cursor),
        end = cursor + 12 + length;
      if (end > bytes.length) return "";
      const type = text(cursor + 4, 4);
      if (!/^[A-Za-z]{4}$/.test(type)) return "";
      let crc = 0xffffffff;
      for (let i = cursor + 4; i < end - 4; i++)
        crc = crcTable[(crc ^ bytes[i]) & 255] ^ (crc >>> 8);
      if ((crc ^ 0xffffffff) >>> 0 !== word(end - 4)) return "";
      if (!header && type !== "IHDR") return "";
      if (type === "IHDR") {
        if (header || length !== 13 || !word(cursor + 8) || !word(cursor + 12)) return "";
        color = bytes[cursor + 17];
        const depths = { 0: [1, 2, 4, 8, 16], 2: [8, 16], 3: [1, 2, 4, 8], 4: [8, 16], 6: [8, 16] };
        if (
          !depths[color]?.includes(bytes[cursor + 16]) ||
          bytes[cursor + 18] !== 0 ||
          bytes[cursor + 19] !== 0 ||
          bytes[cursor + 20] > 1
        )
          return "";
        header = true;
      } else if (type === "PLTE") {
        if (idat || palette || !length || length > 768 || length % 3) return "";
        palette = true;
      } else if (type === "IDAT") {
        if (idatEnded || (color === 3 && !palette)) return "";
        idat += length;
        for (let i = cursor + 8; zlib.length < 2 && i < end - 4; i++) zlib.push(bytes[i]);
      } else if (type === "IEND") {
        return length === 0 &&
          end === bytes.length &&
          idat >= 6 &&
          zlib.length === 2 &&
          (zlib[0] & 15) === 8 &&
          zlib[0] >>> 4 <= 7 &&
          !(zlib[1] & 32) &&
          (zlib[0] * 256 + zlib[1]) % 31 === 0
          ? "image/png"
          : "";
      } else {
        if (idat) idatEnded = true;
        // Unknown critical chunks cannot be interpreted safely.
        if (type[0] === type[0].toUpperCase()) return "";
      }
      cursor = end;
    }
    return "";
  }
  if (bytes.length >= 4 && bytes[0] === 255 && bytes[1] === 216) {
    let cursor = 2,
      frame = false,
      quantization = false,
      huffman = false,
      scans = 0,
      entropy = 0;
    while (cursor < bytes.length) {
      if (bytes[cursor++] !== 255) return "";
      while (bytes[cursor] === 255) cursor++;
      if (cursor >= bytes.length) return "";
      const marker = bytes[cursor++];
      if (marker === 217)
        return cursor === bytes.length && frame && quantization && huffman && scans && entropy
          ? "image/jpeg"
          : "";
      if (
        marker === 0 ||
        marker === 216 ||
        (marker >= 208 && marker <= 215) ||
        cursor + 2 > bytes.length
      )
        return "";
      const length = short(cursor),
        end = cursor + length;
      if (length < 2 || end > bytes.length) return "";
      if ([192, 193, 194].includes(marker)) {
        if (
          frame ||
          length < 11 ||
          bytes[cursor + 2] !== 8 ||
          !short(cursor + 3) ||
          !short(cursor + 5) ||
          length !== 8 + 3 * bytes[cursor + 7]
        )
          return "";
        frame = true;
      } else if (marker === 219) {
        let p = cursor + 2;
        while (p < end) {
          const precision = bytes[p] >>> 4;
          if (precision > 1 || (bytes[p] & 15) > 3) return "";
          p += 1 + 64 * (precision + 1);
        }
        if (p !== end) return "";
        quantization = true;
      } else if (marker === 196) {
        let p = cursor + 2;
        while (p < end) {
          if (p + 17 > end || bytes[p] >>> 4 > 1 || (bytes[p] & 15) > 3) return "";
          let symbols = 0;
          for (let n = 1; n <= 16; n++) symbols += bytes[p + n];
          if (!symbols || symbols > 256) return "";
          p += 17 + symbols;
        }
        if (p !== end) return "";
        huffman = true;
      } else if (marker === 218) {
        if (
          !frame ||
          !quantization ||
          !huffman ||
          length < 8 ||
          length !== 6 + 2 * bytes[cursor + 2]
        )
          return "";
        scans++;
        cursor = end;
        let count = 0;
        while (cursor < bytes.length) {
          if (bytes[cursor] !== 255) {
            cursor++;
            count++;
            continue;
          }
          let next = cursor + 1;
          while (bytes[next] === 255) next++;
          if (next >= bytes.length) return "";
          if (bytes[next] === 0 || (bytes[next] >= 208 && bytes[next] <= 215)) {
            cursor = next + 1;
            count++;
            continue;
          }
          break;
        }
        if (!count) return "";
        entropy += count;
        continue;
      }
      cursor = end;
    }
    return "";
  }
  if (bytes.length >= 14 && ["GIF87a", "GIF89a"].includes(text(0, 6))) {
    if (!little(6) || !little(8)) return "";
    const globalPalette = Boolean(bytes[10] & 128);
    let cursor = 13 + (globalPalette ? 3 * (1 << ((bytes[10] & 7) + 1)) : 0),
      frames = 0;
    while (cursor < bytes.length) {
      const marker = bytes[cursor++];
      if (marker === 59) return frames && cursor === bytes.length ? "image/gif" : "";
      if (marker === 33) {
        if (cursor >= bytes.length) return "";
        cursor++; // Extension label; each following sub-block is bounded below.
      } else if (marker === 44) {
        if (cursor + 9 > bytes.length || !little(cursor + 4) || !little(cursor + 6)) return "";
        const localPalette = Boolean(bytes[cursor + 8] & 128);
        if (!globalPalette && !localPalette) return "";
        cursor += 9 + (localPalette ? 3 * (1 << ((bytes[cursor + 8] & 7) + 1)) : 0);
        if (cursor >= bytes.length || bytes[cursor] < 2 || bytes[cursor] > 8) return "";
        cursor++;
        frames++;
      } else return "";
      let data = 0,
        terminated = false;
      while (cursor < bytes.length) {
        const length = bytes[cursor++];
        if (!length) {
          terminated = true;
          break;
        }
        if (cursor + length > bytes.length) return "";
        data += length;
        cursor += length;
      }
      if (!terminated || (marker === 44 && data < 2)) return "";
    }
  }
  return "";
}

// Compact, inspected single-panel comics. Only references, never bundled artwork.
const COMICS = [149, 303, 88, 55, 231, 259];

function options(context) {
  const mode = context?.settings?.mode || "compact";
  if (mode !== "compact" && mode !== "latest") {
    const error = new Error("Choose Compact selection or Latest for xkcd.");
    error.configuration = true;
    throw error;
  }
  return mode;
}

function selection(context) {
  const previous = context?.state;
  const valid =
    previous?.version === 1 &&
    previous.mode === "compact" &&
    Number.isInteger(previous.index) &&
    previous.index >= 0 &&
    previous.index < COMICS.length;
  const taps = context?.event?.taps;
  const step = Number.isSafeInteger(taps) && taps > 0 ? taps % COMICS.length : 1;
  if (valid) return (previous.index + step) % COMICS.length;
  const seed = String(context?.now?.utc || "xkcd");
  let hash = 0;
  for (let i = 0; i < seed.length; i++) hash = (hash * 31 + seed.charCodeAt(i)) >>> 0;
  return hash % COMICS.length;
}

function metadata(answer) {
  if (!answer?.ok || answer.status < 200 || answer.status >= 300)
    throw new Error("xkcd could not refresh. The previous comic is kept.");
  const data = answer.json;
  if (
    !data ||
    !Number.isSafeInteger(data.num) ||
    data.num < 1 ||
    typeof data.img !== "string" ||
    !/^https:\/\/imgs\.xkcd\.com\/comics\/[a-zA-Z0-9_./-]+\.(?:png|jpg|jpeg)$/.test(data.img)
  ) {
    throw new Error("xkcd returned an unsupported comic image. The previous comic is kept.");
  }
  return data;
}

function image(answer) {
  if (
    !answer?.ok ||
    answer.status < 200 ||
    answer.status >= 300 ||
    typeof answer.base64 !== "string"
  )
    throw new Error("xkcd image could not load. The previous comic is kept.");
  const bytes = answer.base64;
  const mime = validatedImageMime(bytes);
  if (mime !== "image/png" && mime !== "image/jpeg")
    throw new Error(
      "xkcd returned an incomplete or unsupported image. The previous comic is kept.",
    );
  return `data:${mime};base64,${bytes}`;
}

export function plan(context) {
  const mode = options(context);
  const answers = context?.answers || [];
  if (answers.length === 0)
    return [
      {
        url:
          mode === "latest"
            ? "https://xkcd.com/info.0.json"
            : `https://xkcd.com/${COMICS[selection(context)]}/info.0.json`,
        as: "json",
      },
    ];
  if (answers.length === 1) return [{ url: metadata(answers[0]).img, as: "bytes" }];
  return [];
}

export function render(context) {
  const mode = options(context);
  const data = metadata(context?.answers?.[0]);
  return {
    layout: {
      type: "div",
      style: {
        display: "flex",
        flexDirection: "column",
        width: 448,
        height: 368,
        background: "#ffffff",
        padding: "6px 8px 0",
      },
      children: [
        {
          type: "img",
          src: image(context?.answers?.[1]),
          style: { width: 432, height: 322, objectFit: "contain" },
        },
        {
          type: "div",
          style: {
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            width: 432,
            height: 40,
            color: "#333333",
            fontSize: 17,
          },
          children: [
            { type: "div", children: "xkcd.com · Randall Munroe" },
            { type: "div", children: `#${data.num}` },
          ],
        },
      ],
    },
    state: { version: 1, mode, index: selection(context), number: data.num },
  };
}
