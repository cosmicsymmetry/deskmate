# Deskmate plugin contract v1 — design

Track B of `docs/roadmap.md`, first spec. Brainstormed with the owner on 2026-09-22.

## What this is

A plugin is how someone other than us puts a face on a Deskmate panel. This spec defines
the contract — what a plugin *is*, how it is run, how it fails, how it is tested — and
migrates our own four server-rendered faces onto it, so the format is proven by four real
users rather than by prose.

It defines no submission flow, no review queue, no directory UI, no per-author identity
and no tap handling. Those wait: the first three on there being an outside author at all,
identity and plan limits on Track A, tap on Track C1.

## What does not change

- **No wire change, no firmware change, no config-schema change.** The schema/wire lock
  stays free. A plugin's output is a 448x368 PNG that reaches the device through the
  picture card's existing asset path, exactly as a producer's POST does.
- **A plugin is not a card kind.** The product rule holds: a card gets its face
  on-device (clock, pomodoro) or as a raster frame from a producer. A plugin is a
  producer.
- **The server still names no plugin.** The catalog comes from the package's `describe`;
  `data_cards.rs` learns kinds at runtime and knows nothing about what any of them draws.
- **There is still exactly one scene renderer.** A plugin rasters; it does not compose a
  scene.

## Why this is not the v9 manifest plugins

Manifest-based plugins were removed on 2026-09-11 and are not coming back. The remote
plugins in this spec resemble them enough to be worth separating explicitly:

| | v9 manifests (deleted) | This spec |
|---|---|---|
| Who fetches the data | the server | the plugin (hosted: the faces runtime; remote: the author's server) |
| Who decides the layout | the server, from a template registry | the plugin, in full |
| What the manifest declares | data shape, bindings, templates, size classes | an id, a label, the fields to collect, and where to POST |
| What the server interprets | the content | nothing |

The deleted design made the server a general fetcher and a second renderer. This one adds
neither. A remote manifest carries no data shape, no layout and no templates.

## The two run modes

Both produce the same thing — a PNG for one picture card — and both appear in the add
menu through the same `describe` catalog.

| | Code lives | Runs on | Review |
|---|---|---|---|
| **Hosted plugin** | submitted to us | our faces runtime, one subprocess per render | the code itself, at a pinned version |
| **Remote plugin** | the author's server | the author's server; we POST to it | the listing: identity, claimed behaviour, fields, a spot check of output |

A remote plugin's directory entry is labelled *runs on the author's server*, and a user
adding it is told their settings are sent to that host. Review cannot promise more than
that, because the author can change the endpoint's behaviour the day after approval. What
we do hold is a kill switch: delisting stops every render at once.

The raw PNG push (`docs/images/producer-guide.md`) is unchanged and remains the universal
fallback for anything that needs credentials, another language, or a machine we do not
run.

## A hosted plugin

A folder under the plugins directory:

```
plugins/weather/
  plugin.ts      default export: Plugin
  cases.ts       its rendering cases (optional)
```

```ts
interface Plugin {
  api: 1;                    // contract version; the runtime refuses any other value
  id: string;                // stable; stored in data-cards.json as today's `kind`
  label: string;             // what the add menu calls it
  description: string;       // one line, for the directory
  author: string;            // "Deskmate" for ours
  refreshSeconds?: number;   // default cadence; 60..86400, falls back to 900
  fields: FieldSpec[];       // unchanged from today's FaceDefinition
  render(settings: Settings, now: Date): Promise<string | Uint8Array>;
}
```

`render` resolves to an SVG document (rastered by the runtime through `@resvg/resvg-js`,
as today) or to PNG bytes already at 448x368. `FieldSpec`, `Settings`, `ConfigurationError`
and `TransientError` keep their current meanings.

Plugin code imports the kit through one specifier, `@deskmate/plugin-kit`: `Canvas`,
`wrap`, `fit`, `fitTracked`, `fitSize`, the theme tokens, `relativeAge`, and the guarded `fetchText`. It is a workspace entry point, not an npm publication —
publishing is outward-facing and waits for an outside author.

**`id` is the stable identifier and must equal the folder name.** The four existing ids —
`weather`, `hackernews`, `rss`, `token` — are unchanged, so the deployed
`data-cards.json` loads without a migration.

## A remote plugin

A folder holding `remote.json` instead of `plugin.ts`:

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

The runtime turns it into an ordinary `Plugin` whose `render` POSTs
`{"api": 1, "id": "...", "settings": {...}, "now": "<RFC 3339>"}` to the endpoint and
expects `image/png`.

It cannot reuse `fetchText`, for two reasons found while writing this spec: `dial()` takes
no method or body, and `createFetchText` throws `TransientError` on **every** non-ok
status — a remote plugin's `422` would be swallowed as transient and the owner would never
read the sentence saying what to fix. So `kit/http.ts` gains a sibling built on the same
`pinnedAddress`/`dial` guard: same address refusals, same pinned resolved IP, same TLS
server name, same deadline, same body cap, plus a method, a request body, extra headers,
and the status handling below. `dial` grows a method/body/headers parameter; `fetchText`'s
behaviour is unchanged.

**A redirect is refused** on this path rather than followed. Replaying a POST body across
hops adds method-change and body-replay questions that a render endpoint has no reason to
raise; an author points at the final URL. A redirect is transient and says so.

| Response | Meaning |
|---|---|
| `200` with a 448x368 PNG within the body cap | a frame |
| `422` | **the only** configuration error; the body becomes the sentence the owner reads, sanitized and capped |
| anything else: `4xx`, `429`, `5xx`, a redirect, a timeout, a wrong content type, wrong dimensions, an oversized body | transient; the stored frame stays |

`secretRef` names a key in a secrets file under `DESKMATE_CONFIG_DIR`; the runtime sends
it as `Authorization: Bearer <secret>` so the author can verify the caller is us. The
secret never appears in the manifest.

## Cadence

Today `data_cards.rs` stamps every browser-created card with `default_refresh_seconds()`
= 900, which is why the token card's 300 s is documented as a hand-edit of
`data-cards.json`. A plugin declaring its own cadence is part of a plugin contract, so:

- `describe` reports each plugin's `refresh_seconds` when it declares one.
- The server uses it **when creating a spec**, clamped to 60..86400, falling back to 900.
- An existing spec keeps the cadence it has. A plugin update never rewrites a cadence the
  owner set.

This is the one piece that touches Rust and therefore costs a binary redeploy; everything
else in this spec ships with `deploy.sh --faces-only`.

Adding `refresh_seconds` to `describe` output is safe against the deployed binary:
`CatalogFace` derives a plain `Deserialize` with no `deny_unknown_fields`, so an older
server ignores the new key rather than refusing the catalog. `description`, `author` and
`source` are additive in the same way, and nothing displays them until the directory
exists.

## Discovery, and why one bad plugin must not empty the catalog

Plugins are discovered by listing the plugins directory per invocation, as the catalog is
already re-read every 60 s.

A folder is **skipped, with one line on stderr**, when it fails to load, declares an `api`
other than 1, is missing a required field, has an `id` that does not match its folder
name, or duplicates an id already seen. `describe` returns every plugin that did load.

This matters more than it looks. `load_catalog` treats a failed `describe` as an empty
catalog — survivable by design, since the device link and external producers keep working —
but an empty catalog is an **empty add menu**. Without per-folder isolation, one plugin
with a syntax error would remove every other plugin from the window.

For the same reason `render` dynamically imports only the plugin it was asked for: a
broken sibling cannot fail a healthy plugin's refresh.

## Errors

The exit-code taxonomy is unchanged, because the Rust side already treats it as the
contract: **0** a frame, **2** the owner must change a setting, anything else transient,
and either failure keeps the stored frame without renewing its freshness.

What becomes a documented promise rather than an implementation detail: anything a plugin
throws that is not a `ConfigurationError` is transient. A plugin author who wants the
owner to see a sentence in the window throws `ConfigurationError`; everything else is
retried in a minute.

A face that cannot draw must say so in the window, because the panel cannot. That rule is
unchanged and now applies to plugins.

## Self-hosting

`companion/faces/src/runner.ts`:

```sh
bun run src/runner.ts ./plugins/acme-transit \
  --settings settings.json \
  --push https://deskmate.rodi.one/v1/images/TOKEN \
  --every 900
```

It calls the same `render` and POSTs the PNG to an image-source token. About fifty lines
over a path that already works on hardware, and it is what makes "self-host it or submit
it" true rather than aspirational.

## Testing

- **The migration gate is byte-identical goldens.** `companion/faces/test/golden/` holds
  the Rust renderer's own SVG for the 19 existing cases. If weather, Hacker News, RSS and
  token render the same bytes from the new layout, the reshape changed nothing a person
  can see. Any diff is a defect until proven to be a design change, and a design change is
  `bun run dump --update`, reviewed as a diff.
- Cases move to `plugins/<id>/cases.ts`; `bun run dump` walks discovered plugins instead
  of a static list.
- Discovery tests: a folder that throws on import, a wrong `api`, a mismatched `id`, a
  duplicate id, a folder holding neither `plugin.ts` nor `remote.json`. Each is skipped
  and the rest of the catalog survives.
- Remote adapter tests against an injected fetch, in the style `FetchText` injection
  already uses: 200, 422 (message reaches the owner), 429, 5xx, timeout, wrong content
  type, wrong dimensions, oversized body.
- Runner test: render a plugin, POST to a stub, assert the bytes and the failure paths.
- Rust side: the cadence clamp, the fallback when a plugin declares none, and that an
  existing spec's cadence is preserved across a catalog reload.

`bun run dump out/` and looking at the PNGs — the `.desk.png` ones especially — remains
the real check for anything that changes what a face draws. This spec changes no drawing,
which is exactly what the goldens assert.

## Documentation

- `docs/plugins/contract-v1.md` — the contract, written for an author. The home of
  everything above that an author needs.
- `docs/images/server-rendered-cards.md` — becomes a pointer to it, keeping the
  operational material (`data-cards.json`, manual settings) that is the operator's rather
  than the author's.
- `CLAUDE.md` — its faces paragraph says "adding a face is a file in `faces/src/faces/`
  plus `deploy.sh --faces-only`". That stops being true and is updated in the same change.

## Cost

| Boundary | Crossed? |
|---|---|
| `CURRENT_SCHEMA_VERSION` | no |
| The wire | no |
| Firmware statics | no |
| The Rust binary | yes, once, for cadence from the catalog |

Everything else is `deploy.sh --faces-only`: the package is read per refresh and the
catalog re-read every minute.
