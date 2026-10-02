// Self-contained hosted plugin: no imports or direct networking.
// catalog.json records the verified source metadata; CATALOG below is its bundle.
const CATALOG = [
  {
    id: 45434,
    title: "The Great Wave",
    artist: "Katsushika Hokusai",
    date: "ca. 1830–32",
    collection: "prints",
    image: "https://images.metmuseum.org/CRDImages/as/web-large/DP130155.jpg",
    source: "https://www.metmuseum.org/art/collection/search/45434",
  },
  {
    id: 55739,
    title: "Noboto at Shimosa",
    artist: "Katsushika Hokusai",
    date: "1832–33",
    collection: "prints",
    image: "https://images.metmuseum.org/CRDImages/as/web-large/DP140974.jpg",
    source: "https://www.metmuseum.org/art/collection/search/55739",
  },
  {
    id: 56683,
    title: "Snowy Gorge",
    artist: "Utagawa Hiroshige",
    date: "",
    collection: "prints",
    image: "https://images.metmuseum.org/CRDImages/as/web-large/DP146846.jpg",
    source: "https://www.metmuseum.org/art/collection/search/56683",
  },
  {
    id: 56906,
    title: "Fujikawa, Scene at the Border",
    artist: "Utagawa Hiroshige",
    date: "ca. 1833–34",
    collection: "prints",
    image: "https://images.metmuseum.org/CRDImages/as/web-large/DP123209.jpg",
    source: "https://www.metmuseum.org/art/collection/search/56906",
  },
  {
    id: 45294,
    title: "Twilight Moon at Ryogoku Bridge",
    artist: "Utagawa Hiroshige",
    date: "",
    collection: "prints",
    image: "https://images.metmuseum.org/CRDImages/as/web-large/DP123224.jpg",
    source: "https://www.metmuseum.org/art/collection/search/45294",
  },
  {
    id: 436535,
    title: "Wheat Field with Cypresses",
    artist: "Vincent van Gogh",
    date: "1889",
    collection: "landscapes",
    image: "https://images.metmuseum.org/CRDImages/ep/web-large/DP-42549-001.jpg",
    source: "https://www.metmuseum.org/art/collection/search/436535",
  },
  {
    id: 435973,
    title: "Italian Landscape",
    artist: "Camille Corot",
    date: "ca. 1825–28",
    collection: "landscapes",
    image: "https://images.metmuseum.org/CRDImages/ep/web-large/DP332568.jpg",
    source: "https://www.metmuseum.org/art/collection/search/435973",
  },
  {
    id: 435964,
    title: "Landscape with Figures",
    artist: "Camille Corot",
    date: "1872",
    collection: "landscapes",
    image: "https://images.metmuseum.org/CRDImages/ep/web-large/DP-16064-001.jpg",
    source: "https://www.metmuseum.org/art/collection/search/435964",
  },
  {
    id: 435983,
    title: "River with a Distant Tower",
    artist: "Camille Corot",
    date: "1865",
    collection: "landscapes",
    image: "https://images.metmuseum.org/CRDImages/ep/web-large/DP-14936-055.jpg",
    source: "https://www.metmuseum.org/art/collection/search/435983",
  },
  {
    id: 436965,
    title: "The Monet Family in Their Garden",
    artist: "Edouard Manet",
    date: "1874",
    collection: "paintings",
    image: "https://images.metmuseum.org/CRDImages/ep/web-large/DP-25465-001.jpg",
    source: "https://www.metmuseum.org/art/collection/search/436965",
  },
  {
    id: 436532,
    title: "Self-Portrait with a Straw Hat",
    artist: "Vincent van Gogh",
    date: "1887",
    collection: "paintings",
    image: "https://images.metmuseum.org/CRDImages/ep/web-large/DT1502_cropped2.jpg",
    source: "https://www.metmuseum.org/art/collection/search/436532",
  },
  {
    id: 435882,
    title: "Apples and a Pot of Primroses",
    artist: "Paul Cézanne",
    date: "ca. 1890",
    collection: "paintings",
    image: "https://images.metmuseum.org/CRDImages/ep/web-large/DT47.jpg",
    source: "https://www.metmuseum.org/art/collection/search/435882",
  },
  {
    id: 436524,
    title: "Sunflowers",
    artist: "Vincent van Gogh",
    date: "1887",
    collection: "paintings",
    image: "https://images.metmuseum.org/CRDImages/ep/web-large/DP-41223-001.jpg",
    source: "https://www.metmuseum.org/art/collection/search/436524",
  },
  {
    id: 436105,
    title: "The Death of Socrates",
    artist: "Jacques Louis David",
    date: "1787",
    collection: "paintings",
    image: "https://images.metmuseum.org/CRDImages/ep/web-large/DP-13139-001.jpg",
    source: "https://www.metmuseum.org/art/collection/search/436105",
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
  let index =
    valid && (mode === "shuffle" || state.day === day) ? state.index : hash(day) % items.length;
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

function metadata(answer, item) {
  const data = answer && answer.ok && answer.json;
  if (
    !data ||
    data.objectID !== item.id ||
    data.isPublicDomain !== true ||
    typeof data.primaryImageSmall !== "string" ||
    !/^https:\/\/images\.metmuseum\.org\/CRDImages\/[a-z]{2}\/web-large\/[A-Za-z0-9_.-]+\.jpg$/.test(
      data.primaryImageSmall,
    )
  ) {
    throw new Error(
      "The Met has no approved public-domain image for this work. The previous frame is kept.",
    );
  }
  return data;
}

export function plan(context) {
  // Discovery probes plan({}); it must not require the runtime clock.
  if (!context || !context.now) return [];
  const { item } = selection(context);
  const answers = context.answers || [];
  if (!answers.length)
    return [
      {
        url: `https://collectionapi.metmuseum.org/public/collection/v1/objects/${item.id}`,
        as: "json",
      },
    ];
  if (answers.length === 1)
    return [{ url: metadata(answers[0], item).primaryImageSmall, as: "bytes" }];
  return [];
}

export function render(context) {
  const { item, state } = selection(context);
  const answers = context.answers || [];
  metadata(answers[0], item);
  const image = imageData(answers[1]);
  const framing = context.settings && context.settings.framing === "fill" ? "slice" : "meet";
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368" viewBox="0 0 448 368">
      <title>${xml(item.title)} — ${xml(item.artist)}</title>
      <desc>Public-domain image from The Metropolitan Museum of Art. ${xml(item.source)}</desc>
      <defs><clipPath id="art"><rect width="448" height="304"/></clipPath></defs>
      <rect width="448" height="368" fill="#090b0a"/>
      <image width="448" height="304" preserveAspectRatio="xMidYMid ${framing}" clip-path="url(#art)" href="${image}"/>
      <g font-family="Inter, sans-serif">
        <text x="18" y="329" font-size="17" font-weight="600" fill="#f4f0e6">${xml(item.title)}</text>
        <text x="18" y="351" font-size="12" fill="#c8c6bd">${xml(item.artist)}${item.date ? ` · ${xml(item.date)}` : ""}</text>
        <text x="430" y="351" text-anchor="end" font-size="10" fill="#c8c6bd">The Met · CC0</text>
      </g></svg>`,
    state,
  };
}
