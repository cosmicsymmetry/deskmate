function configuration(message) {
  const error = new Error(message);
  error.configuration = true;
  throw error;
}

function sources(context) {
  const input = context?.settings?.strips;
  if (input === undefined || input === "") return [];
  if (typeof input !== "string" || input.length > 4096)
    configuration("Enter up to eight official GoComics CDN strip URLs, separated by spaces.");
  const urls = input.trim().split(/\s+/).filter(Boolean);
  if (urls.length > 8) configuration("Use at most eight strip URLs.");
  for (const url of urls) {
    if (
      !/^https:\/\/(?:assets\.amuniversal\.com\/|featureassets\.gocomics\.com\/assets\/)[a-fA-F0-9]{20,64}(?:\.(?:png|jpg|jpeg|gif))?$/.test(
        url,
      )
    )
      configuration(
        "Use https://featureassets.gocomics.com/assets/ or https://assets.amuniversal.com/ with the official image ID. GoComics page URLs cannot be rendered.",
      );
  }
  return urls;
}

function selection(context, urls) {
  const previous = context?.state;
  const current = previous?.version === 1 ? urls.indexOf(previous.url) : -1;
  const taps = context?.event?.taps;
  const step = Number.isSafeInteger(taps) && taps > 0 ? taps % urls.length : 1;
  return current < 0 ? 0 : (current + step) % urls.length;
}

export function plan(context) {
  const urls = sources(context);
  // Discovery probes plan({}); render provides the actionable missing-setting error.
  if (!urls.length || context?.answers?.length) return [];
  return [{ url: urls[selection(context, urls)], as: "bytes" }];
}

export function render(context) {
  const urls = sources(context);
  if (!urls.length)
    configuration(
      "Add an authorized Calvin and Hobbes image URL from featureassets.gocomics.com/assets/ or assets.amuniversal.com. Automatic GoComics discovery is unavailable; see the plugin README.",
    );
  const answer = context?.answers?.[0];
  if (
    !answer?.ok ||
    answer.status < 200 ||
    answer.status >= 300 ||
    typeof answer.base64 !== "string"
  )
    throw new Error(
      "The supplied strip could not load. Check that its public image URL still works; the previous strip is kept.",
    );
  const bytes = answer.base64;
  const mime = bytes.startsWith("iVBORw0KGgo")
    ? "image/png"
    : bytes.startsWith("/9j/")
      ? "image/jpeg"
      : bytes.startsWith("R0lGOD")
        ? "image/gif"
        : "";
  if (!mime) throw new Error("The supplied URL did not return a PNG, JPEG or GIF image.");
  const index = selection(context, urls);
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
          src: `data:${mime};base64,${bytes}`,
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
            { type: "div", children: "Calvin and Hobbes · Bill Watterson" },
            ...(urls.length > 1
              ? [{ type: "div", style: { fontSize: 15 }, children: `${index + 1}/${urls.length}` }]
              : []),
          ],
        },
      ],
    },
    state: { version: 1, url: urls[index] },
  };
}
