// GitHub stats -- the worked example from
// docs/superpowers/specs/2026-09-23-deskmate-plugin-contract-design.md. One request
// to api.github.com's public user endpoint, a headline number and two tiles. Real
// plugin code, not a test fixture: changes require a new reviewed release.
//
// `plan`/`render` never call anything themselves -- they only declare what they
// need, and the host performs it (`docs/superpowers/specs/2026-09-23-deskmate-plugin-contract-design.md`).
// Discovery runs `plan()` once against `{}` to confirm this file parses and plan
// can run, so it reads fields defensively. Render runs with the full context.

function username(context) {
  const settings = (context && context.settings) || {};
  const typed = typeof settings.user === "string" ? settings.user.trim() : "";
  return typed === "" ? "octocat" : typed;
}

export function plan(context) {
  // The host re-runs plan() after every round with the answers collected so far
  // (docs/plugins/contract-v1.md, "Rounds"). One answer is everything this plugin
  // needs, so it returns [] once it has one -- omitting this check does not fail
  // anything (the sandbox simply stops at the 3-round cap instead), but it does
  // spend three GitHub requests per render instead of one, silently.
  const answers = (context && context.answers) || [];
  if (answers.length > 0) return [];
  const user = username(context);
  return [
    {
      url: `https://api.github.com/users/${encodeURIComponent(user)}`,
      as: "json",
      headers: {
        Accept: "application/vnd.github+json",
        // Substituted only when a github_token secret is configured for this
        // plugin (`src/plugins/secrets.ts`). The host adds the Bearer prefix.
        // With no credential the placeholder stays literal; render() falls back
        // to "--" when the reply lacks the expected numeric fields.
        Authorization: "{{secret:github_token}}",
      },
    },
  ];
}

function tile(label, value) {
  return {
    type: "div",
    style: {
      display: "flex",
      flexDirection: "column",
      alignItems: "center",
      justifyContent: "center",
      width: 176,
      height: 96,
      background: "#15171d",
      borderRadius: 16,
    },
    children: [
      {
        type: "div",
        style: { fontSize: 36, fontWeight: 600, color: "#f4f4f5" },
        children: String(value),
      },
      {
        type: "div",
        style: { fontSize: 15, color: "#8b8f98", marginTop: 4 },
        children: label,
      },
    ],
  };
}

export function render(context) {
  const user = username(context);
  const answers = (context && context.answers) || [];
  const answer = answers[0];
  const data = answer && answer.ok && answer.json && typeof answer.json === "object" ? answer.json : {};

  const repos = typeof data.public_repos === "number" ? format.number(data.public_repos) : "--";
  const followers = typeof data.followers === "number" ? format.compact(data.followers) : "--";
  const following = typeof data.following === "number" ? format.compact(data.following) : "--";

  return {
    layout: {
      type: "div",
      style: {
        display: "flex",
        flexDirection: "column",
        justifyContent: "space-between",
        width: 448,
        height: 368,
        padding: 32,
        background: "#05070c",
      },
      children: [
        {
          type: "div",
          style: { display: "flex", flexDirection: "column" },
          children: [
            { type: "div", style: { fontSize: 18, color: "#8b8f98" }, children: `@${user}` },
            { type: "div", style: { fontSize: 88, fontWeight: 600, color: "#f4f4f5" }, children: repos },
            { type: "div", style: { fontSize: 18, color: "#8b8f98" }, children: "public repos" },
          ],
        },
        {
          type: "div",
          style: { display: "flex", flexDirection: "row" },
          children: [
            tile("followers", followers),
            { type: "div", style: { width: 16, height: 1 } },
            tile("following", following),
          ],
        },
      ],
    },
  };
}
