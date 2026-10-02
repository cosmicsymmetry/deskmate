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

function events(raw) {
  if (!Array.isArray(raw)) return [];
  return raw
    .slice(0, 100)
    .flatMap((item) => {
      const text = clean(item?.text, 235);
      const year = item?.year;
      return text && Number.isInteger(year) && year >= -9999 && year <= 9999
        ? [{ text, year }]
        : [];
    })
    .slice(0, 16);
}
function content(context) {
  const today = date(context);
  const old = context.state;
  const cached = old?.date === today.key && old?.zone === today.zone ? events(old.items) : [];
  const items = cached.length ? cached : events(response(context, "Wikipedia")?.selected);
  if (!items.length) throw new Error("Wikipedia returned no short, readable events for this date.");
  const previous =
    cached.length && Number.isSafeInteger(old.index) && old.index >= 0
      ? old.index % items.length
      : 0;
  const index = (previous + (taps(context) % items.length)) % items.length;
  return { today, items, index, item: items[index] };
}
export function plan(context) {
  if (!context?.now) return [];
  const today = date(context);
  const cached =
    context.state?.date === today.key &&
    context.state?.zone === today.zone &&
    events(context.state.items).length;
  if (!cached && !context.answers?.length)
    return [
      {
        url: `https://en.wikipedia.org/api/rest_v1/feed/onthisday/selected/${String(today.month).padStart(2, "0")}/${String(today.day).padStart(2, "0")}`,
        as: "json",
        headers: HEADERS,
      },
    ];
  if (metrics(context)) return [];
  return [measure(content(context).item.text)];
}
export function render(context) {
  const { today, items, index, item } = content(context);
  const year = item.year < 0 ? `${Math.abs(item.year)} BC` : String(item.year);
  return {
    svg: frame(
      `<text x="26" y="82" font-size="60" font-weight="600">${year}</text><text x="422" y="43" text-anchor="end" font-size="20" fill="#a0a0a8">${today.label}</text>${paragraph(item.text, metrics(context), 109, 309, 30)}${footer("Wikipedia · CC BY-SA", `${index + 1} / ${items.length}`)}`,
    ),
    state: { date: today.key, zone: today.zone, items, index },
  };
}
