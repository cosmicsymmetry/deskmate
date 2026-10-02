// Compact, inspected single-panel comics. Only references, never bundled artwork.
const COMICS = [149, 303, 88, 55, 231, 259, 162];

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
  const mime = bytes.startsWith("iVBORw0KGgo")
    ? "image/png"
    : bytes.startsWith("/9j/")
      ? "image/jpeg"
      : "";
  if (!mime) throw new Error("xkcd returned an unsupported image format.");
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
