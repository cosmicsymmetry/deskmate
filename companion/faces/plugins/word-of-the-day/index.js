// This file is self-contained: the hosted QuickJS sandbox cannot import helpers.
const HEADERS = {
  Accept: "application/json",
  "User-Agent": "Deskmate/1.0 (https://github.com/cosmicsymmetry/deskmate)",
};
function date(context) {
  const local = context?.now?.local;
  if (
    !local ||
    !Number.isInteger(local.year) ||
    !Number.isInteger(local.month) ||
    !Number.isInteger(local.day)
  ) {
    throw new Error("A valid local date is required from the host clock.");
  }
  const year = local.year;
  const days = [
    31,
    year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0) ? 29 : 28,
    31,
    30,
    31,
    30,
    31,
    31,
    30,
    31,
    30,
    31,
  ];
  if (
    year < 1 ||
    year > 9999 ||
    local.month < 1 ||
    local.month > 12 ||
    local.day < 1 ||
    local.day > days[local.month - 1]
  ) {
    throw new Error("A valid local date is required from the host clock.");
  }
  const key = `${year}-${String(local.month).padStart(2, "0")}-${String(local.day).padStart(2, "0")}`;
  const label = `${local.day} ${["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][local.month - 1]}`;
  // The date is already localized by the host; use UTC only to count civil dates.
  const serial = Math.floor(new Date(`${key}T12:00:00Z`).getTime() / 86400000);
  return { key, label, serial, month: local.month, day: local.day, zone: context.now.timezone };
}
function taps(context) {
  const count = context?.event?.taps;
  return Number.isSafeInteger(count) && count > 0 ? count : 0;
}
function clean(value, max = 260) {
  if (typeof value !== "string") return null;
  const text = value.replace(/\s+/g, " ").trim();
  // The bundled font covers Latin, Greek and Cyrillic, not arbitrary scripts/emoji.
  if (!text || text.length > max || /[^\u0020-\u024f\u0370-\u052f\u2010-\u2027]/.test(text))
    return null;
  const words = text.split(" ");
  return words.length <= 60 && words.every((word) => word.length <= 28) ? text : null;
}
function xml(value) {
  return String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}
function measure(text) {
  return {
    measure: [...text.split(" ").map((word) => `0 ${word} 0`), "0 0", "00"].map((word) => ({
      text: word,
      size: 32,
      weight: 400,
    })),
  };
}
function metrics(context) {
  return (context.answers || []).find((answer) => answer.ok && Array.isArray(answer.measurements));
}
function paragraph(text, measured, top, bottom, largest = 32) {
  const words = text.split(" ");
  const values = measured?.measurements;
  if (
    !values ||
    values.length !== words.length + 2 ||
    values.some((value) => !Number.isFinite(value.width) || value.width < 0)
  )
    throw new Error("Text measurement was unavailable.");
  for (let size = largest; size >= 24; size -= 2) {
    // Standalone whitespace has no ink/advance in resvg; measure an internal gap.
    const space = ((values[words.length].width - values[words.length + 1].width) * size) / 32;
    const lines = [];
    let line = "";
    let width = 0;
    let overflow = false;
    for (let i = 0; i < words.length; i++) {
      const next = ((values[i].width - values[words.length].width) * size) / 32 - space;
      if (next > 396) overflow = true;
      if (line && width + space + next > 396) {
        lines.push(line);
        line = "";
        width = 0;
      }
      width += next + (line ? space : 0);
      line += `${line ? " " : ""}${words[i]}`;
    }
    if (line) lines.push(line);
    const height = lines.length * size * 1.23;
    if (!overflow && height <= bottom - top) {
      return lines
        .map(
          (line, index) =>
            `<text x="26" y="${(top + size + index * size * 1.23).toFixed(2)}" font-size="${size}">${xml(line)}</text>`,
        )
        .join("");
    }
  }
  throw new Error("The source text is too long to fit the panel legibly.");
}
function frame(body) {
  return `<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368" viewBox="0 0 448 368"><rect width="448" height="368" fill="#000000"/><g font-family="Inter" font-weight="400" fill="#f5f5f7">${body}</g></svg>`;
}
function footer(left, right) {
  return `<g font-size="17" fill="#a0a0a8"><text x="26" y="341">${xml(left)}</text><text x="422" y="341" text-anchor="end">${xml(right)}</text></g>`;
}
function response(context, name) {
  const answer = context.answers?.[0];
  if (!answer?.ok || answer.status < 200 || answer.status >= 300 || !answer.json)
    throw new Error(`${name} could not be refreshed. Try again shortly.`);
  return answer.json;
}

// Selection is ours, definitions are live Wiktionary text. This is not a claimed
// publisher's daily feed. Each list rotates on the owner's civil date.
const WORDS = {
  everyday:
    "amble,brisk,candid,delight,earnest,flourish,glean,haven,impulse,jubilant,kindle,lucid,mirth,nuance,openhearted,ponder,quaint,resilient,solace,tranquil,unison,vivid,wander,yearn,zest,buoyant,clarity,dapple,elegant,fathom,grace,harmony,intrepid,jovial,kinship,luminous,meander,nimble,opulent,poise,quietude,radiant,serene,thrive,upbeat,verdant,whimsy,zeal".split(
      ",",
    ),
  curious:
    "serendipity,ephemeral,petrichor,mellifluous,aureate,laconic,ineffable,liminal,halcyon,equanimous,apricity,susurrus,vellichor,redolent,quiescent,effervescent,sonorous,diaphanous,numinous,scintilla,palimpsest,syzygy,ineffaceable,ebullient,incandescent,iridescent,euphony,crepuscular,sempiternal,peregrination,diurnal,penumbra,reciprocity,equanimity,perennial,magnanimous,perspicacious,confluence,fortuitous,tenacious,assiduous,ruminate,sagacious,verisimilitude,cynosure,mellifluent,quiddity,plenitude".split(
      ",",
    ),
};
function selection(context) {
  const today = date(context);
  const vocabulary = context.settings?.vocabulary === "everyday" ? "everyday" : "curious";
  const words = WORDS[vocabulary];
  const old = context.state;
  const previous =
    old?.date === today.key &&
    old?.zone === today.zone &&
    old?.vocabulary === vocabulary &&
    Number.isSafeInteger(old.offset) &&
    old.offset >= 0
      ? old.offset % words.length
      : 0;
  const offset = (previous + (taps(context) % words.length)) % words.length;
  const word =
    words[
      ((((today.serial % words.length) + words.length) % words.length) + offset) % words.length
    ];
  return { today, vocabulary, offset, word };
}
function plain(html) {
  if (typeof html !== "string" || html.length > 6000) return null;
  // Only text is carried onto the panel; no upstream markup or resources survive.
  return clean(
    html
      .replace(/<[^>]*>/g, "")
      .replace(/&#(x[0-9a-f]+|[0-9]+);/gi, (_, value) => {
        const point =
          value[0].toLowerCase() === "x" ? parseInt(value.slice(1), 16) : parseInt(value, 10);
        return point > 0 && point <= 0x10ffff ? String.fromCodePoint(point) : "";
      })
      .replace(/&amp;/g, "&")
      .replace(/&quot;/g, '"')
      .replace(/&apos;|&#39;/g, "'")
      .replace(/&nbsp;/g, " ")
      .replace(/&lt;/g, "<")
      .replace(/&gt;/g, ">"),
    160,
  );
}
function definition(raw) {
  if (!Array.isArray(raw?.en)) return null;
  for (const entry of raw.en.slice(0, 12)) {
    const part = clean(entry?.partOfSpeech, 24);
    if (!part || !Array.isArray(entry.definitions)) continue;
    for (const item of entry.definitions.slice(0, 20)) {
      const text = plain(item?.definition);
      if (text) return { text, part: part.toLowerCase() };
    }
  }
  return null;
}
function content(context) {
  const choice = selection(context);
  const old = context.state;
  const cached =
    old?.date === choice.today.key &&
    old?.zone === choice.today.zone &&
    old?.word === choice.word &&
    old?.vocabulary === choice.vocabulary &&
    clean(old.text, 160) &&
    clean(old.part, 24);
  const item = cached
    ? { text: old.text, part: old.part }
    : definition(response(context, "Wiktionary"));
  if (!item)
    throw new Error("Wiktionary returned no short, readable English definition for this word.");
  return { ...choice, ...item };
}
export function plan(context) {
  if (!context?.now) return [];
  const choice = selection(context);
  const old = context.state;
  const cached =
    old?.date === choice.today.key &&
    old?.zone === choice.today.zone &&
    old?.word === choice.word &&
    old?.vocabulary === choice.vocabulary &&
    clean(old.text, 160) &&
    clean(old.part, 24);
  if (!cached && !context.answers?.length)
    return [
      {
        url: `https://en.wiktionary.org/api/rest_v1/page/definition/${choice.word}`,
        as: "json",
        headers: HEADERS,
      },
    ];
  if (metrics(context)) return [];
  return [measure(content(context).text)];
}
export function render(context) {
  const { today, vocabulary, offset, word, text, part } = content(context);
  const size = word.length > 13 ? 40 : word.length > 10 ? 44 : 52;
  return {
    svg: frame(
      `<text x="26" y="80" font-size="${size}" font-weight="600">${word}</text><text x="26" y="113" font-size="20" fill="#a0a0a8">${xml(part)}</text>${paragraph(text, metrics(context), 142, 309, 30)}${footer("Wiktionary · CC BY-SA", today.label)}`,
    ),
    state: { date: today.key, zone: today.zone, vocabulary, offset, word, text, part },
  };
}
