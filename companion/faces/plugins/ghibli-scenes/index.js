// Self-contained hosted plugin: no imports or direct networking.
// catalog.json records the verified source metadata; CATALOG below is its bundle.
const CATALOG = [
  {
    id: "totoro15",
    collection: "totoro",
    title: "My Neighbor Totoro",
    year: 1988,
    credit: "© 1988 Hayao Miyazaki/Studio Ghibli",
    image: "https://www.ghibli.jp/gallery/totoro015.jpg",
    source: "https://www.ghibli.jp/works/totoro/",
  },
  {
    id: "totoro30",
    collection: "totoro",
    title: "My Neighbor Totoro",
    year: 1988,
    credit: "© 1988 Hayao Miyazaki/Studio Ghibli",
    image: "https://www.ghibli.jp/gallery/totoro030.jpg",
    source: "https://www.ghibli.jp/works/totoro/",
  },
  {
    id: "totoro45",
    collection: "totoro",
    title: "My Neighbor Totoro",
    year: 1988,
    credit: "© 1988 Hayao Miyazaki/Studio Ghibli",
    image: "https://www.ghibli.jp/gallery/totoro045.jpg",
    source: "https://www.ghibli.jp/works/totoro/",
  },
  {
    id: "chihiro1",
    collection: "chihiro",
    title: "Spirited Away",
    year: 2001,
    credit: "© 2001 Hayao Miyazaki/Studio Ghibli, NDDTM",
    image: "https://www.ghibli.jp/gallery/chihiro001.jpg",
    source: "https://www.ghibli.jp/works/chihiro/",
  },
  {
    id: "chihiro15",
    collection: "chihiro",
    title: "Spirited Away",
    year: 2001,
    credit: "© 2001 Hayao Miyazaki/Studio Ghibli, NDDTM",
    image: "https://www.ghibli.jp/gallery/chihiro015.jpg",
    source: "https://www.ghibli.jp/works/chihiro/",
  },
  {
    id: "chihiro45",
    collection: "chihiro",
    title: "Spirited Away",
    year: 2001,
    credit: "© 2001 Hayao Miyazaki/Studio Ghibli, NDDTM",
    image: "https://www.ghibli.jp/gallery/chihiro045.jpg",
    source: "https://www.ghibli.jp/works/chihiro/",
  },
  {
    id: "howl15",
    collection: "howl",
    title: "Howl’s Moving Castle",
    year: 2004,
    credit: "© 2004 Diana Wynne Jones/Hayao Miyazaki/Studio Ghibli, NDDMT",
    image: "https://www.ghibli.jp/gallery/howl015.jpg",
    source: "https://www.ghibli.jp/works/howl/",
  },
  {
    id: "howl30",
    collection: "howl",
    title: "Howl’s Moving Castle",
    year: 2004,
    credit: "© 2004 Diana Wynne Jones/Hayao Miyazaki/Studio Ghibli, NDDMT",
    image: "https://www.ghibli.jp/gallery/howl030.jpg",
    source: "https://www.ghibli.jp/works/howl/",
  },
  {
    id: "howl45",
    collection: "howl",
    title: "Howl’s Moving Castle",
    year: 2004,
    credit: "© 2004 Diana Wynne Jones/Hayao Miyazaki/Studio Ghibli, NDDMT",
    image: "https://www.ghibli.jp/gallery/howl045.jpg",
    source: "https://www.ghibli.jp/works/howl/",
  },
  {
    id: "majo15",
    collection: "majo",
    title: "Kiki’s Delivery Service",
    year: 1989,
    credit: "© 1989 Eiko Kadono/Hayao Miyazaki/Studio Ghibli, N",
    image: "https://www.ghibli.jp/gallery/majo015.jpg",
    source: "https://www.ghibli.jp/works/majo/",
  },
  {
    id: "majo30",
    collection: "majo",
    title: "Kiki’s Delivery Service",
    year: 1989,
    credit: "© 1989 Eiko Kadono/Hayao Miyazaki/Studio Ghibli, N",
    image: "https://www.ghibli.jp/gallery/majo030.jpg",
    source: "https://www.ghibli.jp/works/majo/",
  },
  {
    id: "majo45",
    collection: "majo",
    title: "Kiki’s Delivery Service",
    year: 1989,
    credit: "© 1989 Eiko Kadono/Hayao Miyazaki/Studio Ghibli, N",
    image: "https://www.ghibli.jp/gallery/majo045.jpg",
    source: "https://www.ghibli.jp/works/majo/",
  },
  {
    id: "ponyo1",
    collection: "ponyo",
    title: "Ponyo",
    year: 2008,
    credit: "© 2008 Hayao Miyazaki/Studio Ghibli, NDHDMT",
    image: "https://www.ghibli.jp/gallery/ponyo001.jpg",
    source: "https://www.ghibli.jp/works/ponyo/",
  },
  {
    id: "ponyo30",
    collection: "ponyo",
    title: "Ponyo",
    year: 2008,
    credit: "© 2008 Hayao Miyazaki/Studio Ghibli, NDHDMT",
    image: "https://www.ghibli.jp/gallery/ponyo030.jpg",
    source: "https://www.ghibli.jp/works/ponyo/",
  },
  {
    id: "ponyo45",
    collection: "ponyo",
    title: "Ponyo",
    year: 2008,
    credit: "© 2008 Hayao Miyazaki/Studio Ghibli, NDHDMT",
    image: "https://www.ghibli.jp/gallery/ponyo045.jpg",
    source: "https://www.ghibli.jp/works/ponyo/",
  },
  {
    id: "laputa15",
    collection: "laputa",
    title: "Castle in the Sky",
    year: 1986,
    credit: "© 1986 Hayao Miyazaki/Studio Ghibli",
    image: "https://www.ghibli.jp/gallery/laputa015.jpg",
    source: "https://www.ghibli.jp/works/laputa/",
  },
  {
    id: "laputa30",
    collection: "laputa",
    title: "Castle in the Sky",
    year: 1986,
    credit: "© 1986 Hayao Miyazaki/Studio Ghibli",
    image: "https://www.ghibli.jp/gallery/laputa030.jpg",
    source: "https://www.ghibli.jp/works/laputa/",
  },
  {
    id: "laputa45",
    collection: "laputa",
    title: "Castle in the Sky",
    year: 1986,
    credit: "© 1986 Hayao Miyazaki/Studio Ghibli",
    image: "https://www.ghibli.jp/gallery/laputa045.jpg",
    source: "https://www.ghibli.jp/works/laputa/",
  },
];

function hash(text) {
  let value = 2166136261;
  for (let i = 0; i < text.length; i++) value = Math.imul(value ^ text.charCodeAt(i), 16777619);
  return value >>> 0;
}

function selection(context) {
  const settings = (context && context.settings) || {};
  const collection = CATALOG.some((item) => item.collection === settings.collection)
    ? settings.collection
    : "all";
  const items =
    collection === "all" ? CATALOG : CATALOG.filter((item) => item.collection === collection);
  const mode = settings.change === "shuffle" ? "shuffle" : "daily";
  const local = context && context.now && context.now.local;
  const day = local && `${local.year}-${local.month}-${local.day}`;
  if (
    !day ||
    !Number.isInteger(local.year) ||
    !Number.isInteger(local.month) ||
    !Number.isInteger(local.day) ||
    local.month < 1 ||
    local.month > 12 ||
    local.day < 1 ||
    local.year < 1900 ||
    local.year > 9999 ||
    local.day > new Date(Date.UTC(local.year, local.month, 0)).getUTCDate()
  ) {
    throw new Error("The gallery needs a valid date from the host clock.");
  }
  const state = context.state || {};
  const valid =
    state.collection === collection &&
    state.mode === mode &&
    Number.isInteger(state.index) &&
    state.index >= 0 &&
    state.index < items.length;
  const event = context.event;
  const taps =
    event && Number.isInteger(event.taps) && event.taps > 0 ? Math.min(32, event.taps) : 0;
  const ordinal = Math.floor(Date.UTC(local.year, local.month - 1, local.day) / 86400000);
  const dailyIndex = ((ordinal % items.length) + items.length) % items.length;
  let index = valid && (mode === "shuffle" || state.day === day) ? state.index : dailyIndex;
  if (taps) index = (index + taps) % items.length;
  else if (mode === "shuffle" && valid) {
    // A deterministic shuffle for this input, without an immediate repeat.
    index = (index + 1 + (hash(`${context.now.utc}:${index}`) % (items.length - 1))) % items.length;
  }
  return { item: items[index], state: { collection, mode, day, index } };
}

function xml(value) {
  return String(value).replace(
    /[&<>"']/g,
    (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&apos;" })[char],
  );
}

function imageData(answer) {
  const b64 = answer && answer.ok && answer.base64;
  // Reserve headroom under the 512 KiB SVG cap after base64 expansion.
  if (
    typeof b64 !== "string" ||
    b64.length < 32 ||
    b64.length > 506668 ||
    !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(b64)
  ) {
    throw new Error("The gallery image is unavailable or too large. The previous frame is kept.");
  }
  // Decode only indexed bytes, not an unbounded binary array in QuickJS.
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  const byte = (index) => {
    const offset = Math.floor(index / 3) * 4;
    const bits =
      (alphabet.indexOf(b64[offset]) << 18) |
      (alphabet.indexOf(b64[offset + 1]) << 12) |
      ((alphabet.indexOf(b64[offset + 2]) & 63) << 6) |
      (alphabet.indexOf(b64[offset + 3]) & 63);
    return (bits >>> (16 - (index % 3) * 8)) & 255;
  };
  const size = (b64.length / 4) * 3 - (b64.endsWith("==") ? 2 : b64.endsWith("=") ? 1 : 0);
  if (
    size > 380000 ||
    byte(0) !== 255 ||
    byte(1) !== 216 ||
    byte(size - 2) !== 255 ||
    byte(size - 1) !== 217
  ) {
    throw new Error(
      "The gallery did not return a complete JPEG image. The previous frame is kept.",
    );
  }
  // Check a bounded JPEG marker stream and its dimensions before embedding.
  let offset = 2;
  for (let marker = 0; marker < 512 && offset + 9 < size; marker++) {
    if (byte(offset) !== 255) break;
    const kind = byte(offset + 1);
    if (kind === 255) {
      offset++;
      continue;
    }
    const length = (byte(offset + 2) << 8) | byte(offset + 3);
    if (length < 2 || offset + length + 2 > size) break;
    if (kind === 192 || kind === 193 || kind === 194) {
      const height = (byte(offset + 5) << 8) | byte(offset + 6);
      const width = (byte(offset + 7) << 8) | byte(offset + 8);
      if (width < 1 || height < 1 || width > 4096 || height > 4096 || width * height > 8000000)
        break;
      return `data:image/jpeg;base64,${b64}`;
    }
    if (kind === 218) break;
    offset += length + 2;
  }
  throw new Error(
    "The gallery JPEG has invalid or oversized dimensions. The previous frame is kept.",
  );
}

export function plan(context) {
  if (!context || !context.now) return [];
  if ((context.answers || []).length) return [];
  const { item } = selection(context);
  return [{ url: item.image, as: "bytes" }];
}

export function render(context) {
  const { item, state } = selection(context);
  const image = imageData((context.answers || [])[0]);
  const framing = context.settings && context.settings.framing === "fill" ? "slice" : "meet";
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368" viewBox="0 0 448 368">
      <title>${xml(item.title)} (${item.year})</title>
      <desc>Official Studio Ghibli gallery. ${xml(item.credit)}. ${xml(item.source)}. Studio usage terms apply; this image is not CC0.</desc>
      <defs><clipPath id="scene"><rect width="448" height="306"/></clipPath></defs>
      <rect width="448" height="368" fill="#080b0b"/>
      <image width="448" height="306" preserveAspectRatio="xMidYMid ${framing}" clip-path="url(#scene)" href="${image}"/>
      <g font-family="Inter, sans-serif">
        <text x="18" y="331" font-size="17" font-weight="600" fill="#f0f3ea">${xml(item.title)}</text>
        <text x="430" y="331" text-anchor="end" font-size="11" fill="#c3ccc3">ghibli.jp</text>
        <text x="18" y="353" font-size="10" fill="#c3ccc3">${xml(item.credit)}</text>
      </g></svg>`,
    state,
  };
}
