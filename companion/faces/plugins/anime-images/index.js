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
      (answer.base64.startsWith("iVBORw0KGgo") || answer.base64.startsWith("/9j/")),
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
