# Deskmate plugin contract v1 — design

Track B of `docs/roadmap.md`, first spec. Brainstormed with the owner on 2026-09-22 and
2026-09-23.

**Supersedes `docs/superpowers/specs/2026-09-22-deskmate-plugin-contract-design.md`**
(commit `ce8f68e`), written by a parallel session that is now closed. That document's
hosted plugin was a trusted TypeScript module inside the faces runtime, guarded by code
review alone. This one sandboxes it, on the owner's direction: more flexible for authors,
and safer. Its remote tier, catalog isolation, cadence-from-catalog, self-host runner and
testing plan are kept, and are marked below where they come from it.

## What this is

A plugin is how someone other than us puts a face on a Deskmate panel. This spec defines
the contract — what a plugin *is*, what runs it, what it may reach, how it fails, how it
is tested.

It defines no submission flow, no review queue, no directory UI and no per-author
identity. Those wait: the first three on there being an outside author at all, identity
and plan limits on Track A.

**Tap and state are in, and are Track C1's shapes, not new ones.** PR #4 gives every face
a `state` that round-trips through the server and an `event: {taps, point}` for a tap, and
changes the render seam to a `{png, state}` envelope. A plugin uses exactly those. This
spec is written against the post-C1 seam and its plan is sequenced behind that merge; §
"Tap and state" says what a plugin sees.

## What does not change

- **No wire change, no firmware change, no config-schema change.** The schema/wire lock
  stays free. A plugin's output is a 448x368 PNG that reaches the device through the
  picture card's existing asset path, exactly as a producer's POST does.
- **A plugin is not a card kind.** The product rule holds: a card gets its face
  on-device (clock, pomodoro) or as a raster frame from a producer. A plugin is a
  producer.
- **The server names no plugin.** The catalog comes from the package's `describe`;
  `data_cards.rs` learns ids at runtime and knows nothing about what any of them draws.
- **There is still exactly one scene renderer.** A plugin rasters; it does not compose a
  scene.
- **The raw PNG push is untouched** (`docs/images/producer-guide.md`) and remains the
  universal fallback for anything needing credentials we do not hold, another language,
  or a machine we do not run.

## Why this is not the v9 manifest plugins

Manifest-based plugins were removed on 2026-09-11 and are not coming back. Kept from the
superseded spec, because the resemblance is close enough to be worth stating:

| | v9 manifests (deleted) | This spec |
|---|---|---|
| Who fetches the data | the server | the plugin host, on the plugin's declaration |
| Who decides the layout | the server, from a template registry | the plugin, in full |
| What the manifest declares | data shape, bindings, templates, size classes | an id, a label, the sites it may reach, the secrets it needs, the fields to collect |
| What the server interprets | the content | nothing |

The deleted design made the server a general fetcher and a second renderer. This one adds
neither: the fetching is declared per plugin and enforced against an allowlist, and the
rendering produces a frame, not a scene.

## The shape of the contract: a pure function

**A plugin makes no calls.** It declares what it needs, is handed the answers, and
returns a card. Everything else in this spec follows from that sentence.

```ts
export function plan(context: PlanContext): Request[];
export function render(context: RenderContext): Card;
```

The host runs `plan`, performs the requests it returns, runs `plan` again with the
answers if the plugin asks for more (at most 3 rounds), then runs `render`.

This was not the first design. The alternative — a plugin that calls `fetch` and
`measureText` mid-render — was spiked first and rejected, because mid-render calls are
what force WASI, async host functions and a second process. Removing them removed all
three. What remains is also a better contract on its own merits:

- **The requests are knowable before anything runs**, so the host, the reviewer and
  eventually the user can see exactly what a plugin will call.
- **A pure function is trivially testable**: record the answers, replay them, compare the
  card. No network in a test suite.
- **The contract outlives the machinery.** Nothing in it names QuickJS, Bun or Satori. If
  any of those disappoints, it is replaced without touching a published plugin.

## The two run modes

Both produce the same thing — a PNG for one picture card — and both appear in the add
menu through the same `describe` catalog. The remote mode is kept from the superseded
spec.

| | Code lives | Runs on | Review covers |
|---|---|---|---|
| **Hosted plugin** | submitted to us | our plugin host, sandboxed, one fresh sandbox per render | the code itself, at a pinned version and checksum |
| **Remote plugin** | the author's server | the author's server; we POST to it | the listing: identity, claimed behaviour, fields, a spot check of output |

A remote plugin's directory entry is labelled *runs on the author's server*, and a user
adding it is told their settings are sent to that host. Review cannot promise more,
because the author can change the endpoint's behaviour the day after approval. What we
hold is a kill switch: delisting stops every render at once.

## A hosted plugin

A folder under the plugins directory:

```
plugins/github-stats/
  plugin.json    the manifest: identity, reach, fields
  index.js       plan() and render(), bundled from the author's TypeScript
  cases.ts       its rendering cases (optional, for the golden harness)
```

```json
{
  "api": 1,
  "id": "github-stats",
  "version": "1.2.0",
  "label": "GitHub stats",
  "description": "Your commits, stars and open pull requests.",
  "author": "Acme",
  "hosts": ["api.github.com"],
  "secrets": [
    { "key": "github_token", "label": "GitHub token", "kind": "api_key",
      "host": "api.github.com", "send_as": "bearer" }
  ],
  "fields": [
    { "type": "text", "key": "user", "label": "Username", "placeholder": "octocat" }
  ],
  "refreshSeconds": 900
}
```

**The manifest is the only thing that grants reach**, and it is what a reviewer reads.
`fields` is today's `FieldSpec` unchanged, so the settings window renders a plugin's form
with no new code. `id` is stable, must equal the folder name, and is what
`data-cards.json` stores where it stores `kind` today. `api` other than `1` is refused.

**Identity is `id@version` plus a checksum of the folder.** The same id and version
always means the same bytes; a reviewed plugin cannot be quietly replaced.

### What `plan` returns

```ts
type Request =
  | { url: string; method?: "GET" | "POST"; headers?: Record<string, string>;
      body?: string; as: "json" | "text" | "bytes" }
  | { measure: { text: string; size: number; weight: number }[] };
```

A `url` must match a manifest host, or the render fails as a configuration error naming
the site. Headers may contain `{{secret:<key>}}`; see below. `as: "bytes"` exists so a
plugin can fetch an image and pass it through.

`measure` is for plugins that draw their own SVG and therefore place text by hand. The
host answers with the same advance widths and ink boxes `kit/raster.ts` computes, capped
at 64 measurements per render. A plugin using the layout path never needs it.

### What `render` is handed

```ts
interface RenderContext {
  settings: Settings;           // what the owner typed
  answers: Answer[];            // in the order the requests were declared
  now: { utc: string; local: LocalTime; timezone: string };
  format: Format;               // numbers, dates, durations, relative ages
  state?: unknown;              // what this plugin returned last time (C1)
  event?: { taps: number; point: { x: number; y: number } | null };  // C1
}
```

`plan` is handed the same context minus `answers`, so a plugin can decide what to fetch
from where it left off — a paging plugin answering a tap fetches nothing at all.

`now.local` and `format` exist because **the sandbox has no `Intl` and its clock is
UTC** — verified, see Evidence. Without them every plugin invents its own broken
time-zone arithmetic and `(324800).toLocaleString()` silently returns `324800`. `format`
is ordinary code injected with the sandbox, not a call out of it.

### What `render` returns

| Output | Shape | For |
|---|---|---|
| **Layout** | a tree of boxes with CSS-like styles | most plugins; the documented default |
| **SVG** | `{ svg: string }` | full control: charts, illustrations, our own kit |
| **PNG** | `{ png: Uint8Array }` | an image fetched and passed through unchanged |

The layout is laid out by the host (flexbox, via `satori`) and rastered by `resvg`, the
engine that already draws every face. Text wraps and shrinks because the layout engine
does it, which is the point: `textWidth` does not account for `letter-spacing`
(`CLAUDE.md`), and asking outside authors to dodge that trap by hand is a bad trade.

The tree is **our** schema, converted to the layout engine's element shape by the host.
That conversion is where node count, string length, style keys and image count are
capped, and it is what keeps a third-party library out of the published contract.

## The sandbox

A hosted plugin runs as JavaScript inside QuickJS compiled to WebAssembly, in the plugin
host process. It is given no imports: no `fetch`, no `require`, no files, no environment,
no clock beyond what it is handed. A fresh sandbox per render, disposed afterwards, so
nothing carries between renders or between users.

| Limit | Value | Basis |
|---|---|---|
| Plugin source | 1 MB | a bundled plugin is tens of KB |
| Memory per call | 16 MB | verified: an allocation loop dies here |
| Time per call | 2 s | verified: an infinite loop dies here |
| Rounds of requests | 3 | Hacker News needs 2 |
| Requests per render | 8 | our own faces' worst case |
| Response body | 1 MB each, 4 MB total | the existing fetch cap |
| Measurements | 64 per render | enough to fit a headline and a list |
| Boxes in a card | 2,000 | verified in the converter |
| Output | SVG 512 KB · PNG 1 MB | the PNG cap is the existing ingest cap |
| State returned | 16 KB encoded | C1's `face-state.json` cap, unchanged |

Exceeding a limit ends the render and keeps the stored frame. A deterministic overrun
(too many boxes, an undeclared site) is a configuration error, because retrying cannot
help; a timeout is transient.

## What the host does

1. Runs `plan`; checks every URL against the manifest's `hosts`.
2. Performs the requests through `kit/http.ts`'s existing guard — http(s) only; private,
   loopback, link-local, CGNAT and metadata addresses refused on every redirect hop; the
   resolved address pinned with the name in `Host` and as the TLS server name; body cap;
   deadline.
3. Substitutes secrets, then runs `render`, converts the card, rasters it, and returns
   C1's `{png, state}` envelope, whose PNG reaches `ImageSourceStore::accept` exactly as
   a producer's POST does.

**Secrets are never given to the plugin.** A plugin writes `{{secret:github_token}}` in a
header or query value; the host substitutes the stored credential **only** when that
request is going to the host the secret was registered for, and never returns it in an
answer. A plugin can therefore use a credential and cannot learn it, cannot send it
elsewhere, and cannot draw it onto the panel.

The remaining exposure is the data itself: a plugin that reads your calendar could put
the events in another declared request. The rule that closes it, and which a reviewer can
check from the manifest alone without reading code: **a plugin that uses a secret may
declare only the hosts that secret belongs to.**

**Where the secrets live.** The plugin host process holds the secrets for the render it
is performing, because this repository's rule is that the Rust server makes no outbound
GET and all fetching happens in the faces subprocess (`CLAUDE.md`, `egress.rs`). That is
weaker than "the sandbox process holds no secrets" and it is the honest consequence of
keeping that rule. The admin token stays in the server, as today.

## Remote plugins

Kept from the superseded spec, with its findings intact. A folder holding `remote.json`
instead of `plugin.json` and `index.js`:

```json
{
  "api": 1,
  "id": "acme-transit",
  "label": "Transit board",
  "description": "Departures from one stop",
  "author": "Acme",
  "endpoint": "https://plugins.acme.example/render",
  "refreshSeconds": 300,
  "secretRef": "acme-transit",
  "fields": [{ "type": "text", "key": "stop", "label": "Stop code", "placeholder": "1042" }]
}
```

The host turns it into a plugin whose render POSTs
`{"api": 1, "id": "...", "settings": {...}, "now": "<RFC 3339>"}` to the endpoint and
expects `image/png`.

It cannot reuse `fetchText`, for two reasons found while writing the superseded spec:
`dial()` takes no method or body, and `createFetchText` throws `TransientError` on
**every** non-ok status, so a remote plugin's `422` would be swallowed as transient and
the owner would never read the sentence saying what to fix. `kit/http.ts` gains a sibling
on the same `pinnedAddress`/`dial` guard: same refusals, same pinning, same deadline,
same body cap, plus a method, a body, extra headers, and the status handling below.
`dial` grows a method/body/headers parameter; `fetchText`'s behaviour is unchanged.

**A redirect is refused** on this path rather than followed: replaying a POST body across
hops raises method-change and body-replay questions a render endpoint has no reason to
raise. An author points at the final URL.

| Response | Meaning |
|---|---|
| `200` with a 448x368 PNG within the body cap | a frame |
| `422` | **the only** configuration error; the body becomes the sentence the owner reads, sanitized and capped |
| anything else: `4xx`, `429`, `5xx`, a redirect, a timeout, a wrong content type, wrong dimensions, an oversized body | transient; the stored frame stays |

`secretRef` names a key in a secrets file under `DESKMATE_CONFIG_DIR`, sent as
`Authorization: Bearer <secret>` so the author can verify the caller is us. It never
appears in the manifest.

## Cadence

Kept from the superseded spec. Today `data_cards.rs` stamps every browser-created card
with `default_refresh_seconds()` = 900, which is why the token card's 300 s is documented
as a hand-edit. A plugin declaring its own cadence is part of a plugin contract, so:

- `describe` reports each plugin's cadence as `refresh_seconds` (the catalog wire's own
  snake_case, not the manifest's `refreshSeconds`) when it declares one.
- The server uses it **when creating a spec**, clamped to 60…21600 (60s…6h) -- `worker.rs`'s
  own `MIN_REFRESH`/`MAX_REFRESH`, the range the scheduler re-clamps to at run time in any
  case, not an independently chosen 60…86400. A wider stored value would make the owner's
  file claim a cadence the scheduler silently overrides. Falls back to 900 when the plugin
  declares none.
- An existing spec keeps the cadence it has. A plugin update never rewrites a cadence the
  owner set.

This is the one piece that touches Rust and costs a binary redeploy. Adding the key to
`describe` output is safe against the deployed binary: `CatalogFace` derives a plain
`Deserialize` with no `deny_unknown_fields`, so an older server ignores it.

**A plugin cannot raise its own rate after the fact.** TRMNL moved to refreshing just
before a screen is shown and a user's plugin set to once a day began running about three
times an hour, projecting a $350/month inference bill (`usetrmnl/plugins` issue #113).
The cadence is the host's to enforce.

## Discovery, and why one bad plugin must not empty the catalog

Kept from the superseded spec. Plugins are discovered by listing the plugins directory
per invocation, as the catalog is already re-read every 60 s.

A folder is **skipped, with one line on stderr**, when it fails to load, declares an `api`
other than 1, is missing a required field, has an `id` that does not match its folder
name, or duplicates an id already seen. `describe` returns every plugin that did load.

This matters more than it looks: `load_catalog` treats a failed `describe` as an empty
catalog, and an empty catalog is an **empty add menu**. Without per-folder isolation one
plugin with a syntax error would remove every other plugin from the window. For the same
reason a render loads only the plugin it was asked for.

## Errors

The exit-code taxonomy is unchanged, because the Rust side already treats it as the
contract: **0** a frame, **2** the owner must change a setting, anything else transient,
and either failure keeps the stored frame without renewing its freshness.

What becomes a documented promise: anything a plugin throws that is not a
`ConfigurationError` is transient. An undeclared host, a missing setting, a card over a
deterministic cap and an expired credential are configuration errors and name what to
fix; a timeout, a 5xx and a rate limit are transient and retried from 60 s, doubling to
the interval.

A face that cannot draw must say so in the window, because the panel cannot
(`CLAUDE.md`). That rule now applies to plugins, through `face_status`.

`render` may also return log lines beside its card. They go to our log for the author and
the reviewer, capped and sanitized, and never to the panel.

## Tap and state

Both are Track C1's, unchanged, and a plugin is simply another face to them.

- **A plugin may return `state` beside its card**, capped at **16 KB encoded** per source
  and stored in `face-state.json` under `DESKMATE_CONFIG_DIR`. Omitted leaves the stored
  state alone; `null` clears it. A failed render keeps both the frame and the state, so a
  429 on a tap does not lose the reader's place.
- **A plugin that answers taps declares `tap` in its manifest**: the sentence the window
  shows the owner about what a tap does. A plugin without it ignores taps, which is every
  plugin's default.
- **Taps coalesce**: three quick taps can arrive as one render with `taps: 3`. `point` is
  `null` until C2 carries one; a plugin that hit-tests gets it in the same 448x368 space
  it drew in.

This is why a plugin can page through a list, flip between two views, or show "the next
one" on a tap without any state of its own — and why the earlier claim in this session
that a plugin cannot remember anything between renders is wrong, and was wrong the moment
C1 landed its seam.

**A stateful plugin is never shared** between users, for the obvious reason.

## Sharing identical renders

Same plugin, same version, same settings, no secrets involved: one render, shared. A
hundred people with weather for London cost one fetch and one draw. A plugin using a
secret is never shared, because its answer is that user's.

## Our four faces stay where they are

Weather, Hacker News, RSS and token remain first-party code on the existing path. The
spike proved they run unchanged in a sandbox — all 32 golden cases byte-identical — so
this is a choice, not a limitation:

- They place text by hand through `textWidth`/`textInk`, which the layout path does not
  need and the pure-function path answers only through declared `measure` requests. They
  would gain rounds and lose nothing.
- Their goldens are the owner-approved weather design's protection. Moving them is a
  change with no user-visible benefit, and `CLAUDE.md` calls the design settled.

The plugin format is therefore additive. If a first-party face is ever rewritten as a
plugin, the gate is byte-identical goldens and it is its own change.

## Self-hosting

Kept from the superseded spec. `companion/faces/src/runner.ts`:

```sh
bun run src/runner.ts ./plugins/acme-transit \
  --settings settings.json \
  --push https://deskmate.rodi.one/v1/images/TOKEN \
  --every 900
```

It runs the same `plan`/`render` and POSTs the PNG to an image-source token — about fifty
lines over a path already proven on hardware, and what makes "self-host it or submit it"
true rather than aspirational. A self-hosted plugin is not sandboxed, because it is not
our machine.

## Testing

- **Sandbox escape and limits**, as executable tests, from the spike's hostile set: an
  infinite loop, an allocation loop, `require("fs")`, `fetch`, enumerating globals, a
  throw. Each must be stopped and named.
- **The allowlist**: an undeclared host refused before any connection; a declared host
  reached; a redirect to a private address refused at the hop.
- **Secret injection**: substituted for the registered host, absent for any other, never
  present in an answer or a log line.
- **Caps**: each of the table's limits has a test that trips it and asserts which side of
  the configuration/transient split it lands on.
- **Determinism**: a plugin with recorded answers renders the same card twice, and a
  golden harness pins it.
- **Discovery**: a folder that throws on import, a wrong `api`, a mismatched `id`, a
  duplicate id, a folder holding neither manifest. Each is skipped and the rest survive.
- **Remote adapter**, against an injected fetch: 200, 422 (the message reaches the
  owner), 429, 5xx, timeout, wrong content type, wrong dimensions, oversized body.
- **Runner**: render a plugin, POST to a stub, assert the bytes and the failure paths.
- **Rust**: the cadence clamp, the fallback when a plugin declares none, and that an
  existing spec's cadence survives a catalog reload.
- **The four faces' goldens must not move**, since this spec changes nothing they do.

`bun run dump out/` and looking at the PNGs — the `.desk.png` ones especially — remains
the real check for anything that changes what a card draws. A desktop-sized comic is
unreadable at 0.4x, and only looking tells you.

## Documentation

- `docs/plugins/contract-v1.md` — the contract, written for an author: the two functions,
  the manifest, the limits, the outputs, worked examples.
- `docs/images/server-rendered-cards.md` — points at it, keeping the operational material
  (`data-cards.json`, manual settings) that is the operator's rather than the author's.
- `CLAUDE.md` — its faces paragraph says adding a face is a file in `faces/src/faces/`
  plus `deploy.sh --faces-only`. Still true for our faces; the plugin path is added
  beside it in the same change.

## Cost

| Boundary | Crossed? |
|---|---|
| `CURRENT_SCHEMA_VERSION` | no |
| The wire | no |
| Firmware statics | no |
| The Rust binary | yes, once, for cadence from the catalog |

Everything else ships with `deploy.sh --faces-only`: the package is read per refresh and
the catalog re-read every minute.

## Evidence

Measured on the owner's Mac, 2026-09-22/23, in a throwaway spike. Every number here came
from running it, not from estimation.

| What | Result |
|---|---|
| Our 4 faces' 32 golden cases, rendered inside a sandbox | **32/32 byte-identical** |
| Live weather render, real Open-Meteo data | works, ~0.6 s, network-dominated |
| Live GitHub render through `plan`/`fetch`/`render`/layout | plan 5 ms · fetch 218 ms · render 4.4 ms · layout+draw 38 ms |
| Undeclared host | refused before connecting, as a configuration error |
| Infinite loop | stopped at the deadline |
| Allocation loop | stopped at the memory cap |
| `require("fs")`, `fetch` | not defined |
| Globals visible to a plugin | its own function only |
| `Intl` in the sandbox | **absent**; clock is UTC; `toLocaleString` drops separators |
| Bundled Inter coverage | Latin, Cyrillic, Greek and ☀ yes; Arabic, CJK, colour emoji no |
| An XKCD strip on the panel | renders; **illegible at physical size** — a comic plugin must crop to one panel |
| Line art vs a flat card over the link | ~94 KB / ~50 chunks vs ~26 KB / ~13 (estimated from run counts) |

Two findings that shaped the design rather than confirming it: text measurement must use
the same engine and the same bounding box as the goldens (`abs_layer_bounding_box`;
picking the other box moved 19 of 32 images), and an SVG's image reference is resolved
**from our disk** by default in `usvg-0.45.1` (`src/parser/image.rs:85-100`), so the SVG
output path must accept `data:` references only.

## Two things the build found that the refresher must carry

- **The taxonomy has no third state, and a plugin can sit in the gap.** A rendering
  failure caused by the plugin's own bad CSS or an undecodable image is deterministic,
  but the host cannot tell it from an internal bug, so it is retried rather than blamed
  on the owner — every 60 seconds, forever, with nothing in the window. Deliberate: the
  alternative tells the owner to fix a setting that cannot fix it. The layer that
  schedules refreshes owes a repeated-failure escalation: after N consecutive failures,
  say the plugin is broken.
- **The response-byte budget can overrun by about 2x.** A request that dies mid-body
  charges nothing, because the fetch cannot report what it read, and charging the full
  cap on every failure would let four DNS failures exhaust a render's 4 MB. Bounded by
  the 8-request cap: at worst 8 MB is read against a 4 MB budget.

## Not verified

- The layout engine's behaviour at the caps, and its CSS subset's edges.
- QuickJS memory and deadline enforcement under concurrent renders.
- The remote adapter, which is prose from the superseded spec and has never run.
- Anything on hardware. No plugin-drawn frame has reached `dev-0005`, and
  `CLAUDE.md` already owes a server-rendered face on the panel at both mountings.
- The interaction with C1's seam, which was open as PR #4 when this was written. The plan
  is sequenced behind that merge; if C1's shapes change, this spec follows them rather
  than the reverse.
