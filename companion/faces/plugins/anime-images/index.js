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

// Selected from the provider’s SFW collection; original media stays on Nekos.best.
const COLLECTION = [
  {
    artist_name: "となせが",
    artist_href: "https://www.pixiv.net/en/users/53797367",
    source_url: "https://www.pixiv.net/en/artworks/98076589",
    url: "https://nekos.best/api/v2/neko/e6a94ae4-bc96-42df-bebc-96869c27334f.png",
    dimensions: {
      width: 600,
      height: 848,
    },
  },
  {
    artist_name: "Hong",
    artist_href: "https://www.pixiv.net/en/users/306422",
    source_url: "https://www.pixiv.net/en/artworks/76673075",
    url: "https://nekos.best/api/v2/neko/04396bfc-af6c-4aae-b388-69a594e627a9.png",
    dimensions: {
      width: 707,
      height: 1000,
    },
  },
  {
    artist_name: "はみこ@お仕事募集中",
    artist_href: "https://www.pixiv.net/en/users/221597",
    source_url: "https://www.pixiv.net/en/artworks/111620945",
    url: "https://nekos.best/api/v2/neko/3cea3a61-14e3-48d9-967f-beda328f93b2.png",
    dimensions: {
      width: 764,
      height: 1200,
    },
  },
  {
    artist_name: "球根",
    artist_href: "https://www.pixiv.net/en/users/12272873",
    source_url: "https://www.pixiv.net/en/artworks/87136840",
    url: "https://nekos.best/api/v2/neko/3eafe050-0386-48f9-a2a8-1b3b3dbefd32.png",
    dimensions: {
      width: 850,
      height: 1200,
    },
  },
  {
    artist_name: "馴鹿",
    artist_href: "https://twitter.com/tonakai_91",
    source_url: "https://twitter.com/tonakai_91/status/1698224446736421371",
    url: "https://nekos.best/api/v2/neko/91bc6e9e-f1c2-4c7f-921f-58dc9c5c40c3.png",
    dimensions: {
      width: 900,
      height: 1200,
    },
  },
  {
    artist_name: "うみ猫",
    artist_href: "https://www.pixiv.net/en/users/22874219",
    source_url: "https://www.pixiv.net/en/artworks/77820353",
    url: "https://nekos.best/api/v2/neko/b105ab7c-1617-45dc-a23a-4a139b34851e.png",
    dimensions: {
      width: 643,
      height: 1024,
    },
  },
];

const HEADERS = { "User-Agent": "Deskmate (https://github.com/cosmicsymmetry/deskmate)" };
function options(context) {
  const selection = context?.settings?.selection || "curated";
  const fit = context?.settings?.fit || "contain";
  if (!["curated", "fresh"].includes(selection) || !["contain", "cover"].includes(fit)) {
    const error = new Error("Choose a listed anime selection and image framing.");
    error.configuration = true;
    throw error;
  }
  return { selection, fit };
}
function chosen(context, offset = 0) {
  const previous =
    context?.state?.version === 1
      ? COLLECTION.findIndex((item) => item.url === context.state.url)
      : -1;
  const taps = context?.event?.taps;
  const step = Number.isSafeInteger(taps) && taps > 0 ? taps % COLLECTION.length : 1;
  if (previous >= 0) return COLLECTION[(previous + step + offset) % COLLECTION.length];
  const utc = String(context?.now?.utc || "anime");
  let hash = 0;
  for (let i = 0; i < utc.length; i++) hash = (hash * 31 + utc.charCodeAt(i)) >>> 0;
  return COLLECTION[(hash + offset) % COLLECTION.length];
}
function candidate(context) {
  const answer = context?.answers?.[0];
  if (
    !answer?.ok ||
    answer.status < 200 ||
    answer.status >= 300 ||
    !Array.isArray(answer.json?.results)
  )
    return null;
  // Fetch one promising image, with one known-small third-round fallback.
  return (
    answer.json.results
      .slice(0, 8)
      .filter(
        (item) =>
          item &&
          typeof item.url === "string" &&
          /^https:\/\/nekos\.best\/api\/v2\/neko\/[a-zA-Z0-9-]+\.png$/.test(item.url) &&
          item.url !== context?.state?.url &&
          Number.isFinite(item.dimensions?.width) &&
          item.dimensions.width > 0 &&
          Number.isFinite(item.dimensions?.height) &&
          item.dimensions.height > 0,
      )
      .sort(
        (a, b) =>
          a.dimensions.width * a.dimensions.height - b.dimensions.width * b.dimensions.height,
      )[0] || null
  );
}
function usable(answer) {
  return Boolean(
    answer?.ok &&
      answer.status >= 200 &&
      answer.status < 300 &&
      typeof answer.base64 === "string" &&
      ["image/png", "image/jpeg"].includes(validatedImageMime(answer.base64)),
  );
}
export function plan(context) {
  const { selection } = options(context);
  const answers = context?.answers || [];
  if (selection === "curated") {
    if (!answers.length) return [{ url: chosen(context).url, as: "bytes", headers: HEADERS }];
    if (answers.length === 1 && !usable(answers[0]))
      return [{ url: chosen(context, 1).url, as: "bytes", headers: HEADERS }];
    return [];
  }
  if (!answers.length)
    return [{ url: "https://nekos.best/api/v2/neko?amount=8", as: "json", headers: HEADERS }];
  if (answers.length === 1)
    return [{ url: (candidate(context) || chosen(context)).url, as: "bytes", headers: HEADERS }];
  if (answers.length === 2 && candidate(context) && !usable(answers[1]))
    return [{ url: chosen(context).url, as: "bytes", headers: HEADERS }];
  return [];
}
function shortCredit(item) {
  const name = typeof item.artist_name === "string" ? item.artist_name : "";
  // Inter has no CJK. Preserve original credits in state; display source account.
  if (name && /^[\u0020-\u024f\u0370-\u052f]+$/.test(name))
    return name.length > 24 ? `${name.slice(0, 23)}…` : name;
  const account =
    typeof item.artist_href === "string"
      ? item.artist_href.match(/\/(?:users\/)?([a-zA-Z0-9_]+)\/?$/)?.[1]
      : "";
  return account ? `Artist ${account}` : "Artist at source";
}
export function render(context) {
  const { selection, fit } = options(context);
  const answers = context?.answers || [];
  const fresh = selection === "fresh" ? candidate(context) : null;
  const fallback = selection === "curated" && answers.length > 1;
  const item = fresh && usable(answers[1]) ? fresh : chosen(context, fallback ? 1 : 0);
  const answer =
    selection === "curated"
      ? answers[fallback ? 1 : 0]
      : answers.length > 2
        ? answers[2]
        : answers[1];
  if (!usable(answer))
    throw new Error(
      "Anime image could not load within the image limit. The previous artwork is kept; refresh will retry.",
    );
  return {
    layout: {
      type: "div",
      style: {
        display: "flex",
        flexDirection: "column",
        width: 448,
        height: 368,
        background: "#101217",
      },
      children: [
        {
          type: "img",
          src: `data:${answer.base64.startsWith("/9j/") ? "image/jpeg" : "image/png"};base64,${answer.base64}`,
          style: { width: 448, height: 328, objectFit: fit },
        },
        {
          type: "div",
          style: {
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
            width: 448,
            height: 40,
            padding: "0 12px",
            color: "#e3e5eb",
            fontSize: 17,
          },
          children: [
            { type: "div", children: shortCredit(item) },
            { type: "div", style: { color: "#b5bac5", fontSize: 16 }, children: "nekos.best" },
          ],
        },
      ],
    },
    state: {
      version: 1,
      url: item.url,
      artist: String(item.artist_name || "").slice(0, 160),
      source: String(item.source_url || "").slice(0, 512),
      artistUrl: String(item.artist_href || "").slice(0, 512),
    },
    log: [
      `Artist: ${item.artist_name || "See source"}`,
      `Source: ${item.source_url || "nekos.best"}`,
    ],
  };
}
