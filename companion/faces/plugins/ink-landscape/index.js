// Original procedural artwork. No external code, assets, or random clock reads.
// A seed defines five height fields; contours descend from their ridgelines.
export function plan() {
  return [];
}

function hash(value) {
  let h = 2166136261;
  for (let i = 0; i < value.length; i++) {
    h = Math.imul(h ^ value.charCodeAt(i), 16777619);
  }
  return h >>> 0;
}

function random(seed) {
  let n = seed >>> 0;
  return function next() {
    n = (Math.imul(n, 1664525) + 1013904223) >>> 0;
    return n / 4294967296;
  };
}

function noise(x, seed) {
  const left = Math.floor(x);
  const t = x - left;
  const blend = t * t * (3 - 2 * t);
  const a = random((seed + Math.imul(left, 374761393)) >>> 0)();
  const b = random((seed + Math.imul(left + 1, 374761393)) >>> 0)();
  return a + (b - a) * blend;
}

function number(value) {
  return Math.round(value * 100) / 100;
}

function line(points) {
  return points.map((p, i) => `${i ? "L" : "M"}${number(p[0])} ${number(p[1])}`).join(" ");
}

function identity(context) {
  const local = context.now && context.now.local;
  if (
    !local ||
    !Number.isInteger(local.year) ||
    !Number.isInteger(local.month) ||
    !Number.isInteger(local.day) ||
    !Number.isInteger(local.hour)
  ) {
    throw new Error("Ink Landscape could not read the local date. Please try again.");
  }
  const hourly = context.settings && context.settings.change === "hourly";
  const bucket = `${local.year}-${local.month}-${local.day}${hourly ? `-${local.hour}` : ""}`;
  const previous = context.state;
  const offset =
    previous &&
    previous.version === 1 &&
    previous.bucket === bucket &&
    Number.isInteger(previous.offset) &&
    previous.offset >= 0 &&
    previous.offset < 65536
      ? previous.offset
      : 0;
  const incoming = context.event && context.event.taps;
  const taps = Number.isInteger(incoming) && incoming > 0 ? Math.min(32, incoming) : 0;
  return { version: 1, bucket, offset: (offset + taps) % 65536 };
}

export function render(context) {
  const state = identity(context || {});
  const seed = hash(`${state.bucket}/${state.offset}`);
  const rng = random(seed);
  const night = context.settings && context.settings.palette === "night";
  const paper = night ? "#080e15" : "#f1eee4";
  const wash = night
    ? ["#182833", "#223844", "#2c4852", "#45616a", "#729098"]
    : ["#d9d9cf", "#bfc6be", "#94a39a", "#586f67", "#293f3a"];
  const ink = night ? "#bdc8bd" : "#17302b";
  const parts = [
    '<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368" viewBox="0 0 448 368">',
    `<rect width="448" height="368" fill="${paper}"/>`,
    '<defs><linearGradient id="mist" x1="0" y1="0" x2="0" y2="1">',
    `<stop offset="0" stop-color="${paper}" stop-opacity="0"/>`,
    `<stop offset="1" stop-color="${paper}"/></linearGradient></defs>`,
  ];
  // Consume the same seed sequence in both palettes so a palette change keeps
  // exactly the same terrain and water.
  const moonX = 78 + rng() * 292;
  const moonY = 35 + rng() * 23;
  // The small moon is part of the composition, not an astronomical reading.
  if (night) {
    parts.push(`<circle cx="${number(moonX)}" cy="${number(moonY)}" r="14" fill="#e3e5d5"/>`);
  }
  const lean = rng() < 0.5 ? 1 : -1;
  for (let layer = 0; layer < 5; layer++) {
    const layerSeed = (seed + layer * 104729) >>> 0;
    const ridge = [];
    const baseline = 145 + layer * 31;
    const height = 48 + rng() * 45 + layer * 7;
    const center = (lean > 0 ? 75 + layer * 84 : 385 - layer * 82) + (rng() - 0.5) * 100;
    const width = 52 + rng() * 74;
    const secondHeight = 12 + rng() * 39;
    for (let x = -8; x <= 456; x += 4) {
      const broad = Math.exp(-((x - center) ** 2) / (2 * width ** 2));
      const second = Math.exp(-((x - (448 - center)) ** 2) / (2 * 76 ** 2));
      const detail =
        noise(x / 42, layerSeed) * 16 +
        noise(x / 13, layerSeed + 7) * 9 +
        noise(x / 5, layerSeed + 31) * 2.5;
      ridge.push([x, baseline - height * broad - secondHeight * second - detail]);
    }
    const bottom = baseline + 47;
    parts.push(`<path d="${line(ridge)} L456 ${bottom} L-8 ${bottom} Z" fill="${wash[layer]}"/>`);
    // Broken, descending contour bands give every ridge its own rock structure.
    // Opacity fades toward the mist; no texture asset or SVG filter is needed.
    for (let band = 0; band < 12; band++) {
      const points = ridge.map(([x, y]) => [
        x + Math.sin(x / 55 + band * 0.52 + layer) * band * 0.35,
        y + 3 + band * (3.8 + layer * 0.35) + noise(x / 27, layerSeed + band * 71) * band * 0.7,
      ]);
      parts.push(
        `<path d="${line(points)}" fill="none" stroke="${ink}" stroke-width="${number(0.35 + layer * 0.09)}" stroke-opacity="${number((1 - band / 14) * (night ? 0.19 : 0.24))}"/>`,
      );
    }
    parts.push(`<rect x="0" y="${baseline - 16}" width="448" height="64" fill="url(#mist)"/>`);
  }
  // A quiet foreground lake. Ripples are longer and farther apart toward us.
  for (let i = 0; i < 22; i++) {
    const y = 276 + rng() * 69;
    const x = rng() * 448;
    const length = 5 + rng() * 29 + (y - 276) * 0.18;
    parts.push(
      `<path d="M${number(x)} ${number(y)} q${number(length / 2)} -0.65 ${number(length)} 0" fill="none" stroke="${ink}" stroke-width="0.65" stroke-opacity="${number(0.08 + rng() * 0.15)}"/>`,
    );
  }
  parts.push("</svg>");
  return { svg: parts.join(""), state };
}
