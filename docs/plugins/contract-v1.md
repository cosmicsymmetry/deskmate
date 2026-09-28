# Deskmate plugin contract v1

A plugin is how someone other than us puts a face on a Deskmate panel: a folder of
code that becomes one entry in the add menu and draws one 448x368 card. This document
is for that author. It states what the running code does, not what a design intended
it to do -- where the two differ, a sentence says so and points at the source.

The design record is
`docs/superpowers/specs/2026-09-23-deskmate-plugin-contract-design.md`. This document
does not repeat its reasoning; it is the reference you keep open while writing a
plugin. A plugin is not a card kind -- it is a producer, exactly like an external
service POSTing a PNG (`docs/images/producer-guide.md`) or one of the four built-in
server-rendered faces (`docs/images/server-rendered-cards.md`). All three land the
same 448x368 PNG on the same picture-card asset path.

## What a plugin is

**A plugin makes no calls.** It exports two functions:

```ts
export function plan(context: PlanContext): Request[];
export function render(context: RenderContext): Card;
```

`plan` declares what it needs -- network requests, or text measurements -- and
`render` is handed the answers and returns a card. Neither function calls `fetch`,
`require`, or anything else: there is nothing to call, because the runtime that
executes them provides no imports at all (`companion/faces/src/plugins/sandbox.ts`).
The host (`companion/faces/src/plugins/run.ts`) runs `plan`, performs every request it
returned against your manifest's allowlist, runs `plan` again if you asked for more
(up to three rounds), then runs `render` and converts what it returns into an SVG,
which is rasterized to the PNG the server pushes.

Two consequences follow directly from this shape, and everything below is a detail of
one or the other:

- **The host can validate every request before performing it**, because your requests
  are data, not code that runs partway through your render. A request to a host your
  manifest did not declare is refused before any connection is attempted.
- **A plugin is a pure function of its inputs**, so it is testable with no network at
  all: record a set of answers, call `render` with them, compare the card. The
  `github-stats` example below is tested exactly that way in
  `companion/faces/test/plugins/discovery.test.ts`.

Everything that crosses the boundary between your code and the host -- `plan`'s
return value, `render`'s return value, a thrown error -- is serialized with
`JSON.stringify` and read back with `JSON.parse`
(`companion/faces/src/plugins/sandbox.ts:120-197`). Nothing that cannot survive that
round trip reaches the host: a function, a `Symbol` or a `BigInt` anywhere in your
return value is refused by name as a configuration error, and a `Uint8Array` --
including one you built yourself to hold PNG bytes -- serializes to an object of
numeric string keys, not to anything the host recognizes as an image. Wherever this
document says "bytes", it means a base64 string.

## The worked example: `github-stats`

`companion/faces/plugins/github-stats/` is real plugin code, not a fixture -- it ships
with the package and appears in the add menu. It calls one endpoint,
`https://api.github.com/users/{user}`, and draws a repo count with two tiles for
followers and following, falling back to `"--"` per field when the request failed or
the credential was not configured.

`plugin.json`:

```json
{
  "api": 1,
  "id": "github-stats",
  "version": "1.0.0",
  "label": "GitHub stats",
  "description": "Public repos, followers and following for a GitHub username.",
  "author": "Deskmate",
  "hosts": ["api.github.com"],
  "secrets": [
    {
      "key": "github_token",
      "label": "GitHub token",
      "kind": "api_key",
      "host": "api.github.com",
      "send_as": "bearer"
    }
  ],
  "fields": [
    {
      "type": "text",
      "key": "user",
      "label": "Username",
      "placeholder": "octocat",
      "default": "octocat"
    }
  ],
  "refreshSeconds": 900
}
```

`index.js`:

```js
function username(context) {
  const settings = (context && context.settings) || {};
  const typed = typeof settings.user === "string" ? settings.user.trim() : "";
  return typed === "" ? "octocat" : typed;
}

export function plan(context) {
  const user = username(context);
  return [
    {
      url: `https://api.github.com/users/${encodeURIComponent(user)}`,
      as: "json",
      headers: {
        Accept: "application/vnd.github+json",
        Authorization: "Bearer {{secret:github_token}}",
      },
    },
  ];
}

function tile(label, value) {
  return {
    type: "div",
    style: {
      display: "flex", flexDirection: "column", alignItems: "center",
      justifyContent: "center", width: 176, height: 96,
      background: "#15171d", borderRadius: 16,
    },
    children: [
      { type: "div", style: { fontSize: 36, fontWeight: 600, color: "#f4f4f5" }, children: String(value) },
      { type: "div", style: { fontSize: 15, color: "#8b8f98", marginTop: 4 }, children: label },
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
      style: { display: "flex", flexDirection: "column", justifyContent: "space-between",
        width: 448, height: 368, padding: 32, background: "#05070c" },
      children: [
        { type: "div", style: { display: "flex", flexDirection: "column" }, children: [
          { type: "div", style: { fontSize: 18, color: "#8b8f98" }, children: `@${user}` },
          { type: "div", style: { fontSize: 88, fontWeight: 600, color: "#f4f4f5" }, children: repos },
          { type: "div", style: { fontSize: 18, color: "#8b8f98" }, children: "public repos" },
        ] },
        { type: "div", style: { display: "flex", flexDirection: "row" }, children: [
          tile("followers", followers),
          { type: "div", style: { width: 16, height: 1 } },
          tile("following", following),
        ] },
      ],
    },
  };
}
```

What this draws: a dark panel with `@username` in small grey text, the public repo
count as an 88px headline beneath it, and a row of two rounded tiles below that for
followers and following, each a 36px number over a 15px label. `format.number` and
`format.compact` are the injected formatting global described below -- not something
this file imports.

Two things worth noticing that are not obvious from the shape alone:

- `plan` and `render` are called defensively (`(context && context.settings) || {}`)
  because discovery calls `plan` once against an **empty** context (`{}`) just to
  confirm the file parses and exports both functions
  (`companion/faces/src/plugins/discovery.ts:61-67`). A plugin that assumes
  `context.settings` always exists throws at discovery and is skipped from the
  catalog, not merely at render time.
- The `Authorization` header is written even when no `github_token` secret is
  configured. An unconfigured secret leaves `{{secret:github_token}}` as literal text
  in the header, which GitHub answers as an ordinary request (public data,
  rate-limited, no error) -- `render`'s `--` fallback is what makes that harmless
  rather than a crash. A plugin cannot ask "is my secret configured?" from inside
  `plan` or `render`; design for the answer being absent.

## The manifest, field by field

`plugin.json` is the only thing that grants a plugin reach, and it is what a reviewer
reads without opening `index.js`. Parsed and validated in
`companion/faces/src/plugins/manifest.ts`.

| Field | Type | Rule |
|---|---|---|
| `api` | `1` | Any other value is refused. This document describes api 1. |
| `id` | string | Must equal the plugin's folder name exactly. Stable: `data-cards.json` stores it where it stores a built-in face's `kind`. |
| `version` | string | Non-empty. Not otherwise validated -- there is no version comparison in this contract yet. |
| `label` | string | What the add menu calls it. |
| `description` | string | What the add menu shows beneath the label. |
| `author` | string | Shown in the listing. |
| `hosts` | string[] | Bare hostnames only (`api.github.com`), lowercase-compared. An IP literal (v4 or bracketed v6) is refused here, and refused again independently when a request is validated. No wildcards, no suffix matching: a request to a host not in this exact list is never performed. |
| `secrets` | array | See below. |
| `fields` | array | `FieldSpec[]`, the same shape the browser already renders for a built-in face's settings form (`{type: "text"\|"url", key, label, placeholder, default?}` or `{type: "enum", key, label, default, options}`) -- no new form code exists for a plugin. |
| `refreshSeconds` | integer, optional | Seconds. Read by the server **only when creating a new card's spec**, clamped to 60...86400, falling back to 900 if you omit it. Changing this in a plugin update never moves an existing card's cadence -- only the owner editing it does. |
| `tap` | string, optional | The sentence the settings window shows the owner about what a tap does. A plugin that omits it ignores taps, which is every plugin's default. |

### Secrets, and the one-host rule

```json
{ "key": "github_token", "label": "GitHub token", "kind": "api_key",
  "host": "api.github.com", "send_as": "bearer" }
```

| Field | Rule |
|---|---|
| `key` | Non-empty string. What you write as `{{secret:<key>}}` and what the owner's `plugin-secrets.json` entry is keyed by. |
| `label` | What the settings window shows the owner. |
| `kind` | Must be `"api_key"` -- the only kind this contract defines. |
| `host` | Must be one of the manifest's own `hosts`. A secret naming a host the manifest did not declare is refused at load. |
| `send_as` | `"bearer"` (an `Authorization: Bearer <value>` header), `"header"` (paired with `header`, an arbitrary header name), or `"query"` (paired with `param`, a query-parameter name). |
| `header` | Only legal with `send_as: "header"`. Naming it with `send_as: "query"` is refused. |
| `param` | Only legal with `send_as: "query"`. Naming it with any other `send_as` is refused. |

**A plugin that declares a secret may declare only the hosts that secret's own
manifest entries belong to.** If any secret is present, every host in `hosts` must be
the `host` of at least one secret; an extra host is refused by name
(`companion/faces/src/plugins/manifest.ts:144-152`, message: *"a plugin using a secret
may declare only the hosts its secrets belong to; remove ..."*). This is not a style
preference -- it is the whole reason a reviewer can check what a plugin does with a
credential from the manifest alone: if `github-stats` could also declare
`evil.example` as a host, a reviewer would have to read `index.js` to know whether the
GitHub token ever leaves `api.github.com`.

A plugin with no `secrets` entry has no such restriction and may declare any number of
hosts.

## What `plan` may return

`plan` returns an array mixing two request shapes
(`companion/faces/src/plugins/requests.ts:33-67`):

```ts
type Request =
  | { url: string; method?: "GET" | "POST"; headers?: Record<string, string>;
      body?: string; as: "json" | "text" | "bytes" }
  | { measure: { text: string; size: number; weight: number }[] };
```

**A network request.** `url` must resolve to a host in your manifest's `hosts`, or the
whole render fails as a configuration error naming the host
(`checkDeclaredHost`, same file, lines 327-339) -- this is a deterministic refusal,
so it is not retried; only an edit to `hosts` or the URL fixes it. `as` decides how the
body comes back in the answer: `"json"` (parsed; falls back to `"text"` if the body did
not actually parse as JSON), `"text"`, or `"bytes"` (base64, for passing an image
through unchanged). `method` defaults to `GET`; only `GET` and `POST` are accepted.

The request is performed through the same guarded fetch every built-in face uses
(`companion/faces/src/kit/http.ts`, described in `docs/images/server-rendered-cards.md`
under "Who makes the outbound requests"): http(s) only, private/loopback/link-local/
CGNAT/metadata addresses refused, the resolved address pinned against DNS rebinding, a
1 MB body cap, a 15 s deadline. **Unlike that guard's own `fetchText` used by the
built-in faces, a plugin's request does not follow a redirect** -- see "Behaviours
that will surprise you" below.

**A measure request.** `{ measure: [...] }` performs no network I/O at all -- it asks
the host to measure text through the exact engine and font database that will draw
your card, and answers with an advance width and an ink box per item, the same way
`textWidth`/`textInk` answer for a built-in face
(`companion/faces/src/kit/raster.ts`). This exists for a plugin that returns `{svg}`
and therefore places its own text by hand; a plugin using the `{layout}` shape never
needs it, because the layout engine wraps text for you. `size` must be greater than 0
and at most 256; `weight` must be exactly 400 or 600, the only two weights this
package bundles (`Inter-Regular.ttf`, `Inter-SemiBold.ttf`) -- any other weight is
refused rather than silently falling back to whatever the rasterizer's own font
matching would pick, because a measurement and a drawn glyph must agree exactly.

### Rounds

The host runs `plan`, validates and performs what it returned, and runs `plan` again
with the accumulated answers -- **up to three times** -- before running `render` once.
`answers` on `PlanContext` starts empty and grows with each round, in the order the
requests were declared, across every round so far. Returning `[]` from `plan` on any
round ends the loop immediately: a plugin that got everything it needed on round one
should return `[]` on round two, not repeat its round-one requests.

The request count (8), byte budget (4 MB) and measurement count (64) are **spent
across all three rounds, not reset per round** -- a plugin that uses 5 requests on
round one has 3 left for round two and round three combined
(`companion/faces/src/plugins/run.ts:79-90, 187-231`). Requesting more than what
remains of the budget on any round is a configuration error naming the limit, not a
silent truncation.

A network failure -- a timeout, a non-2xx status, a name that does not resolve --
never fails the whole render; it becomes `{ ok: false, status?, error }` in that
request's own answer, and `render` decides what that means for the card. Only a
violation of the contract itself (an undeclared host, a malformed request shape, the
budget exceeded) fails the render outright, because those cannot come good on a retry.

## What `render` is handed, and what it returns

```ts
interface RenderContext {
  settings: Settings;      // what the owner typed into the fields you declared
  answers: Answer[];       // every answer from every round, in declaration order
  now: NowContext;         // see below
  format: Format;          // an injected global, not an import -- see below
  state?: unknown;         // what this plugin returned as `state` last time
  event?: { taps: number; point: { x: number; y: number } | null };
}
```

`plan` receives the same shape minus `answers` (it has not run any requests yet on its
first call), so `plan` can decide what to fetch from `state` and `event` alone -- a
paging plugin answering a tap can fetch nothing at all.

### `now`

```ts
interface NowContext {
  utc: string;          // ISO 8601, instant.toISOString()
  timezone: string;      // IANA zone, or "UTC" on fallback
  local: {
    year: number; month: number; day: number; hour: number; minute: number;
    weekday: string;      // "Mon".."Sun"
    offsetMinutes: number;
    iso: string;          // "YYYY-MM-DDTHH:mm", no timezone suffix
  };
}
```

**The sandbox has no `Intl`.** `now.local` and `format.date` exist because a plugin
computing its own timezone arithmetic invents something broken -- verified: the
runtime's own `toLocaleString` silently drops thousands separators rather than
throwing (design spec's Evidence table). `now.local` is computed host-side, outside
the sandbox, with real `Intl` (`companion/faces/src/plugins/context.ts:16-66`), and
handed in as plain data.

**As of this contract, `now.timezone` is not the owner's configured timezone.** The
render request the server sends the faces subprocess carries `kind`, `settings`,
`state` and `event`, but no `timezone`
(`companion/crates/server/src/data_cards/faces_package.rs:347-364`), so a plugin's
`now` falls back to the faces host process's own system zone, and then to `"UTC"` if
even that fails (`companion/faces/src/plugins/discovery.ts:44-54`). This is a real gap
against the design's stated intent, not a plugin bug -- do not build a plugin whose
correctness depends on `now.timezone` being the owner's zone until this changes.

### `format`

An ordinary global, injected as source text ahead of your plugin's own code on every
sandbox call (`FORMAT_SOURCE`, `companion/faces/src/plugins/context.ts:68-114`) --
not something you import, and not available inside a string your plugin builds and
`eval`s (there is no `eval` either).

| Function | Behaviour |
|---|---|
| `format.number(value, decimals?)` | Thousands-grouped. `decimals` fixes the fractional digits; omitted, rounds to a whole number. `format.number(1234567)` -> `"1,234,567"`. |
| `format.compact(value)` | `"1.2K"`, `"3.4M"`, `"5.6B"`, rounding to a whole number above 100 of a unit. Falls back to the next smaller unit if the compacted value would itself round to 1000+ of the larger one. |
| `format.date(local, pattern)` | Token substitution against `now.local` (or any object with the same shape): `MMM` (short month name), `yyyy`, `HH`, `mm`, `dd` (zero-padded day), `d` (unpadded day). Each token is replaced **once**, via plain string `.replace`, not a global regex -- a pattern that repeats a token (`"dd/dd"`) only substitutes the first occurrence. There is no numeric-month token; use `local.month` directly if you need one. |
| `format.since(nowIso, thenIso)` | `"just now"` / `"12m ago"` / `"3h ago"` / `"2d ago"`, from two ISO instants. |

### `state` and `event`

Both are Track C1's shapes, unchanged for a plugin. `state` is whatever you returned
as `state` last render, or `undefined` on the first one; return `state` again to keep
it, `null` to clear it, or omit the field entirely to leave the stored value alone. A
failed render keeps both the last frame and the last state, so a rate-limited tap does
not lose a reader's place. `event.taps` coalesces -- three quick taps can arrive as one
render with `taps: 3` -- and `event.point` is always `null` until a later track wires
one through; a plugin that hit-tests gets the point in the same 448x368 space it
draws in, once that lands. Declaring `tap` in your manifest is what tells the settings
window a tap does something; a plugin without it is never told about one.

### What `render` returns

```ts
type Card =
  | { layout: LayoutNode }   // most plugins; laid out by the host via satori (flexbox)
  | { svg: string }          // full control, for a plugin drawing its own document
  | { png: string };         // base64, for an image fetched and passed straight through
```

`svg` wins if a plugin sets both `layout` and `svg`. For a fetched image passed
through unchanged, `png` is the same base64 string an `as: "bytes"` answer already
gave you as `answer.base64` -- `{ png: answer.base64 }` is the whole pass-through.

A `LayoutNode` is `{ type: "div" | "img", style?, children?, src? }`
(`companion/faces/src/plugins/card.ts:99-170`) -- CSS-like flexbox styles, converted
to `satori`'s element shape by the host. An `img`'s `src` must be an embedded
`data:` URI; an `http(s)://` reference is refused, because `satori`'s own SSRF guard
resolves a hostname once to validate it and again to fetch it, which is exactly the
DNS-rebinding gap `kit/http.ts` exists to close, and this package does not rely on a
third-party library's weaker guard for that.

`render` may also return `{ log: string[] }` beside the card: lines that reach our
log for the plugin's author, never the panel, capped to 10 lines of 200 characters
each and stripped of ANSI escapes and control characters before they are written.

## Limits

Every number below is a real constant in the running code, cited by file and line so
you can re-check it against a future version of this package. Exceeding a
deterministic one (an oversized source, too many boxes, an undeclared host) fails the
render as a configuration error and keeps the stored frame; the engine limits
(memory, deadline) fail it as transient.

| Limit | Value | Where |
|---|---|---|
| Plugin `index.js` source | 1 MB (1,048,576 bytes) | `plugins/sandbox.ts:47` (`SANDBOX_LIMITS.sourceBytes`), enforced at discovery in `plugins/discovery.ts:98-103` |
| Memory per sandboxed call (`plan` or `render`, each) | 16 MB | `plugins/sandbox.ts:45` |
| Time per sandboxed call (`plan` or `render`, each) | 2,000 ms | `plugins/sandbox.ts:46` -- wall-clock CPU time inside QuickJS, not network time; a request's own deadline is separate (below) |
| Rounds of `plan` | 3 | `plugins/run.ts:79` |
| Network requests per render (all rounds combined) | 8 | `plugins/run.ts:80` |
| Response bytes per render (all rounds combined) | 4 MB | `plugins/run.ts:84` (`4 x` the ingest cap) |
| One HTTP response body | 1 MB | `kit/http.ts:46` / `kit/limits.ts` (`INGEST_CAP_BYTES`) |
| One HTTP request's own deadline | 15,000 ms | `kit/http.ts:47` |
| Measurements per render (all rounds combined) | 64 | `plugins/run.ts:85` |
| One `measure` item's `size` | > 0 and <= 256 | `plugins/requests.ts:95` |
| One `measure` item's `weight` | 400 or 600 only | `plugins/requests.ts:85` |
| State returned beside a card | 16 KB encoded (JSON, UTF-8) | `plugins/run.ts:88`; over the cap, the state is dropped (not the render) and a log line says so |
| `log` lines returned beside a card | 10 lines, 200 characters each | `plugins/run.ts:93-94` |
| Boxes in a `{layout}` card | 2,000 | `plugins/card.ts:29` |
| Nesting depth of a `{layout}` card | 400 | `plugins/card.ts:38` -- a 501-deep chain of otherwise-valid boxes was observed to corrupt the layout engine's shared module for every later render in the process, not just the offending one; this cap exists with margin below that observed boundary |
| An `{svg}` card's document size | 512 KB | `plugins/card.ts:30` |
| A `{png}` card, or one `img` inside a `{layout}` card | 1 MB decoded | `plugins/card.ts:32` (the same ingest cap) |
| Images per `{layout}` card | 4 | `plugins/card.ts:44` |
| Total image bytes per `{layout}` card | 3 MB | `plugins/card.ts:50` -- kept below `4 x 1 MB` on purpose, so this check can actually fire independently of the per-image and per-count caps |
| Error message length from a rendering failure | 300 characters, then truncated with a count | `plugins/card.ts:55` |

## Errors: which one reaches the owner

Every failure a plugin can produce collapses to exactly one of two kinds
(`companion/faces/src/plugins/run.ts:100-118, 156-169`):

| Kind | Meaning | Owner sees |
|---|---|---|
| **Configuration** | Deterministic: the same refusal happens again next minute. The owner must change something before a retry can help. | An alert in the settings window, under the face's fields (`face_status` on `GET /v1/images`), naming what to fix. Not retried early -- nothing changes until the owner edits a setting. |
| **Transient** | The world did not cooperate this time -- a timeout, a 5xx, a rate limit, an ordinary `throw new Error(...)`. Worth retrying. | A status line, not an alarm (*"Couldn't refresh: ... The display keeps the last frame and this retries on its own."*), per `docs/images/server-rendered-cards.md`'s "What the owner is told". Retried after a minute, doubling up to the refresh interval. |

Either kind keeps the stored frame and stops it from being marked freshly refreshed.

**How you signal "the owner must fix something": throw an `Error` with a truthy
`configuration` property.**

```js
const err = new Error("no city called Xyz");
err.configuration = true;
throw err;
```

This is the only signal the contract defines -- there is no injected
`ConfigurationError` class to import (there is nothing to import at all). A plain
`throw new Error("...")`, or `throw "some string"`, or any thrown value with no
`configuration` property, is always transient
(`plugins/sandbox.ts:76-88`, `normalizeThrown`). The host itself throws
configuration errors on your behalf for everything it can already tell is
deterministic without running your code again: an undeclared host, a plan that is not
an array, a request over budget, a card over a shape cap, a plugin missing `plan` or
`render` entirely, or a source over the byte cap.

One gap the design record names explicitly and this contract does not close: the host
cannot tell "the layout engine choked on the CSS you handed it" apart from an internal
bug in the layout engine itself, so a genuinely broken card layout is retried forever,
silently, rather than surfaced to the owner as something you can fix. If your card
never renders and nothing appears in the settings window, check your `style` values by
hand -- this is the failure mode with no message.

## Behaviours that will surprise you

Each of these is deliberate, not an oversight, and each has bitten a real build of a
face in this package before being written down.

- **A redirect is not followed, and you cannot see where it points.** A plugin's
  network request goes through `createRequest`
  (`companion/faces/src/kit/http.ts:347-372`), which sets `redirect: "manual"` and
  passes the response straight back: a host that answers with a 301 gives you
  `{ ok: true, status: 301, ... }` with whatever body it sent, and `HttpReply` carries
  no headers at all -- there is no `Location` to read, even if you wanted to follow it
  yourself in a later round. Point `plan` at the final URL.
- **A secret substitutes only in the position its `send_as` declares.** Writing
  `{{secret:github_token}}` in a header when `send_as` is `"query"` (or in a header
  with a different name than the manifest's own `header` field, if it named one)
  leaves the placeholder as literal text -- it does not substitute in the "wrong"
  place as a courtesy, because an unmatched placeholder being visible and debuggable
  is safer than a credential landing somewhere a reviewer did not expect from the
  manifest alone.
- **Response bodies are scrubbed of secret values, and a binary response containing
  one is refused outright**, not scrubbed. A `json`/`text` answer has every stored
  secret's literal value replaced with `[redacted]`, recursively through object keys
  and values, before your `render` ever sees it; a `bytes` answer cannot be redacted
  the same way, so one whose raw bytes contain a secret's UTF-8 or base64 form comes
  back as `{ ok: false, error: "the response echoed a stored credential and was
  refused" }` instead (`companion/faces/src/plugins/requests.ts:240-314`). This
  catches an API that echoes headers back in a 200; it does not catch a percent-encoded
  or chunked rendering of the same value -- a documented gap, not a guarantee that no
  form of a secret can ever reach you.
- **Text is measured through the same engine that draws it, and neither includes
  letter-spacing.** A `measure` request answers with the same advance width `resvg`
  would produce, so a `{layout}` card (which wraps text for you) never needs to worry
  about this -- but a plugin drawing its own `{svg}` and tracking a run out with CSS
  `letter-spacing` will find the measured width is narrower than what it actually
  draws, the identical trap `CLAUDE.md` documents for this package's own built-in
  faces (`textWidth` does not account for `letter-spacing`). Budget the extra width
  yourself if you track a run.
- **Only two font weights exist.** `Inter-Regular.ttf` (400) and `Inter-SemiBold.ttf`
  (600) are the only fonts this package bundles
  (`companion/faces/assets/fonts/`); any other weight in a `style` or a `measure`
  request either falls back to whatever the rasterizer's default matching produces
  (in a `{layout}`/`{svg}` card) or is refused outright (in a `measure` request).
  Design around 400 and 600.
- **Cyrillic and Greek render; Arabic, CJK and colour emoji do not.** The bundled
  Inter files cover Latin, Cyrillic, Greek and a handful of symbols; a character
  outside that coverage draws as a missing-glyph box or nothing, not a fallback font.
- **A plugin cannot tell whether its own secret is configured.** `plan` and `render`
  never see whether `{{secret:...}}` resolved to a real value or stayed literal text
  (`github-stats` above is written around exactly this). Design your fallback --
  usually rendering as if the request simply failed -- rather than trying to branch on
  it.
- **A plugin cannot return raw bytes**, only a base64 string, because everything
  crossing the sandbox boundary is JSON. See "What a plugin is" above.

## Testing a plugin locally

There is no separate CLI or harness for a plugin under test -- it runs through the
same `main.ts` seam the server drives, and the same sandbox `bun test` already
exercises for `github-stats` and for the hostile-input fixtures under
`companion/faces/test/plugins/`.

1. **Place the folder.** `companion/faces/plugins/<id>/` (matching `id` in
   `plugin.json` to the folder name), or point `DESKMATE_PLUGINS_DIR` at a scratch
   directory while iterating, so you are not editing the shipped `github-stats`
   folder by mistake.
2. **Confirm discovery.** `cd companion/faces && bun run src/main.ts describe` --
   your plugin's `kind`/`label`/`fields` should appear in the printed catalog JSON. A
   folder that fails to load is skipped silently on stdout and named on stderr
   (`plugin <folder> skipped: <reason>`); nothing else in the catalog is affected,
   which is the isolation property that lets one broken plugin ship without emptying
   the add menu for everyone else's.
3. **Render it.** `bun run src/main.ts describe` warms the sandbox; a render needs the
   same warm-up, which `main` does automatically:

   ```sh
   echo '{"kind":"github-stats","settings":{"user":"octocat"}}' \
     | bun run src/main.ts render | tee /tmp/out.json
   jq -r .png < /tmp/out.json | base64 -d > /tmp/out.png
   ```

   `render`'s output on stdout is the JSON envelope `{"png": "<base64>", "state": ...}`,
   not a raw PNG -- decode the `png` field to look at the image.
   **`bun run dump` does not cover plugins**: it drives `src/cases.ts`, which calls the
   four built-in faces' own render functions directly and knows nothing about
   `discoverPlugins`. There is no golden-PNG or golden-SVG harness for a plugin card
   today; look at the decoded PNG by hand.
4. **Write a `bun test`.** `companion/faces/test/plugins/discovery.test.ts` and
   `run.test.ts` are the pattern to copy: write a manifest and source to a temp
   directory (or call `runPlugin`/`cardToSvg` directly with recorded `answers`), and
   assert on the resulting SVG or thrown error -- no network needed, which is the
   point of the pure-function shape. Run the package's own gates before trusting a
   change: `bun test`, `bun run check`, `bun run lint`, `bun run format:check`.
5. **Configure a secret**, if your plugin declares one: create (or edit)
   `plugin-secrets.json` under whatever `DESKMATE_CONFIG_DIR` points at, shaped
   (`companion/faces/src/plugins/secrets.ts:1-16`):

   ```json
   { "github-stats": { "github_token": "ghp_..." } }
   ```

   The top-level key is your plugin's `id`; a plugin only ever sees its own entry. No
   file, no `DESKMATE_CONFIG_DIR`, or no entry for your plugin's id are all "no
   secrets configured" and never an error -- your plugin must render sensibly with
   every secret absent, because that is the state of most renders before an owner
   fills the field in.

## Not yet true, though the design record describes it

Two pieces of the design spec are **not implemented on this branch** and nothing
below should be read as available today: a **remote plugin** (`remote.json`, code on
the author's own server, reached by POST) and the **self-host runner**
(`companion/faces/src/runner.ts`). Both remain prose in
`docs/superpowers/specs/2026-09-23-deskmate-plugin-contract-design.md`'s own "Not
verified" section. This document covers the hosted plugin only -- the one kind that
exists in `companion/faces/src/plugins/` today.
