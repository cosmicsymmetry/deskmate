# Plugin card parity — server-rendered cards feel like built-in cards

**Status:** approved direction (owner, 2026-09-06: "the user should not see a difference
between server rendered cards and regular cards"); approach 2 of the three offered — the
**server renders the previews**. Design under review.
**Branch:** `feat/plugin-parity` (worktree `.worktrees/plugin-parity`), off
`feat/v2-networked-device` at `30b9174`.
**Builds on:** the one-loop UI (`docs/superpowers/plans/2026-09-06-deskmate-one-loop.md`),
the plugin manifest (`docs/plugins/manifest-v1.md`, `manifest-v2.md`), stage 4's
rasterizer (`docs/superpowers/plans/2026-08-29-deskmate-rasterization.md`).

## 1. The problem, precisely

A schema-v6 `plugin` card is rendered by the server: the server fetches its data, compiles
its manifest to a scene (or rasterizes its SVG template) and pushes the face to the panel.
In the Mac app that same card is a second-class object on every surface:

| Surface | Built-in card | Plugin card today |
|---|---|---|
| Add menu | six kinds with descriptions | the "Plugins on the server" group is wired to an empty list — the app has no registry source |
| Name (tile, legend, editor) | the template name, "Weather" | the machine id, `aqi` |
| Tile value | a live fact | the literal word "Plugin" |
| Freshness flag | the provider's real state | permanently `stale`, because the Mac's own runtime fakes a refresh and records an internal error |
| Preview | exact pixels from the simulator | "Preview unavailable", indistinguishable from a fault |
| Editor | source fields + kind knobs | "Name" and "Refresh every"; no control names the plugin, so a `plugin_id` issue has no home |

Root cause, established from the code: the webview cannot reach the server (CSP), every
server call goes through Rust with the admin token, and the app makes exactly one such
call today (the config PUT). Nothing on the Mac ever reads server state. Meanwhile the
server holds everything needed — the registry with asset bytes, the cached raw provider
JSON per card, a rasterizer — but retains no compiled scene and no frame, has no LVGL
simulator, and its wire `Scene` type deliberately has no JSON form.

## 2. Goals and non-goals

**Goals.** In networked tier a plugin card is named by a human name, shows a live
headline on its tile, carries the same freshness and error semantics as a built-in card,
previews on the stage, is added from the same menu with the same gesture, and is edited
in an editor that names its plugin. Every plugin state is reachable in the dev harness.

**Non-goals.** Rendering a plugin card on a Mac that owns the device in **local tier**
— there is no server to render it, and this design says so rather than pretending. A
picker *window* for a long registry (the menu scrolls; the window comes when search is
needed). A schema change: **v6 stays frozen**, protocol stays v1.

**Accepted deviation (owner's choice of approach 2).** A display-list plugin's preview is
produced by the server's `resvg` rasterizer, not by the panel's LVGL renderer, so it can
differ from the panel in the ways stage 4's rasterization work already found and pinned
(`docs/superpowers/plans/2026-08-29-deskmate-rasterization.md`'s Task 6 evidence row
`digital-clock--date-overflow`: LVGL ellipsizes an overflowing line where the raster
shows it whole). SVG-template
plugins are exact by construction: the raster *is* what the panel shows. PRODUCT.md's
"the preview is not a mock" is amended to say so for plugin cards. Approach 1 (the Mac
renders the server's compiled scene in its own simulator, byte-exact) remains available
later as an additive extension of the same route.

## 3. Manifest v2 amendment (additive, optional, absence tolerated)

Three optional top-level keys in `manifest_version = 2` documents. v1 stays frozen and
unchanged; a v1 manifest simply has none of them.

| Key | Type | Bound | Meaning |
|---|---|---|---|
| `display_name` | string | 1..=64 bytes (`MAX_DISPLAY_NAME_LEN`, matches the card `title` bound) | What the card **is**, on every surface: "Air quality". Falls back to the plugin id. |
| `description` | string | 1..=160 bytes (`MAX_DESCRIPTION_LEN`) | The add-menu's second line: "EPA index for a location". Falls back to "Plugin · <version>". |
| `summary` | expression string | evaluated output truncated to 32 bytes (`MAX_SUMMARY_LEN`); the same `{{ ... }}` language and functions as a text node, **data expressions only** — a device binding (`time:`, `timer.`, `date`, `field.`) is a manifest error, because the summary is evaluated on the server at refresh time where none of those resolve | The tile's live value: `"{{ data.current.aqi }}"`. Evaluated on every refresh, published as the card's `hero` field. Absent → the tile shows "—" like a weather card with no data. |

`name` stays the identity key and is never repurposed. Every table remains
`deny_unknown_fields`; the amendment is recorded in `docs/plugins/manifest-v2.md` with
these bounds. The curated plugins (`aqi`, `agenda`, `claude-limits`, `svg-aqi`) move to
v2 and declare all three keys; that is a content change, not a shape change, and the
two v1 fixtures in `crates/plugin/tests/fixtures/` stay v1 to keep v1 covered.

## 4. Server

All routes are admin-bearer (`AdminAuthenticated`), all additive.

**4.1 `GET /v1/plugins` gains** `display_name` (nullable), `description` (nullable),
`manifest_version`, `template` (`"display-list" | "svg"`), and `refresh_minutes`. The
response DTO (`PluginCatalog`, `PluginCatalogEntry`) moves into app-core's shared admin
module (`app-core/src/admin.rs`, beside `AdminConfigErrorBody`) so the server serializes
and the Mac deserializes **one** type. Existing fields are unchanged.

**4.2 `GET /v1/devices/{id}/cards/{card_id}/preview`** returns the card's face as a PNG:

```json
{ "png_base64": null | "...", "state": "fresh" | "stale" | "error" | "waiting",
  "message": null | "...", "refreshed_at_unix_ms": null | 1725600000000 }
```

`png_base64` is present exactly when `state` is `fresh` or `stale`; `message` is
present exactly when it is `error` or `waiting`.

Semantics: a new `RuntimeCommand::RenderCardPreview { card_id }` on `RuntimeHandle` runs on
the worker thread, exactly like `inject_plugin_snapshot`: it calls `build_card_scene` for
that card with revision `0` (never minted, never sent — no device I/O, no
`next_scene_revision` movement, no raster floor), then `PluginHost::rasterize` with
`RasterRequest::DisplayList { scene, fields }` for a display-list plugin or
`RasterRequest::PluginSvg { .. }` for an SVG template, and encodes the RGB565 frame to PNG
with `frame_png` lifted out of the test module. The scene carries the same stale/error
footer the panel shows, because it is compiled from the same cached snapshot. Outcomes:

- no snapshot cached yet → `200`, `state: "waiting"`, `message: "Waiting for the first
  refresh"`, no frame (the normal pre-first-fetch state, kept distinguishable as
  CLAUDE.md requires);
- plugin id not in the registry → `state: "error"`, `message: "Plugin \"x\" is not
  loaded on the server"`, no frame;
- card is not a plugin card → `404`;
- device unknown → `404`; a device with no runtime yet → `503`.

The route never activates the card, never touches the device, and is rate-limited by
the Mac's own polling (§5), not by the server.

**4.3 The summary field.** `ServerProviderRefresher` already emits `title`; it now also
evaluates `summary` (when declared) against the fetched snapshot with the plugin's
expression engine and emits `hero`. Evaluation failure emits no `hero` and logs once
per plugin. `GET /v1/devices/{id}` therefore carries the headline in `card_data`
unchanged in shape.

**4.4 Nothing else changes** on the wire, in the device link, or in the registry.

## 5. The Mac app (Rust)

**5.1 A shared server client.** The existing `ureq` agent, URL builder and error mapping
(`server_failure`: 401 → "the server rejected the admin token", 404, 422, else runtime
unavailable) are factored so GETs reuse them. Every GET reads its body through the same
64 KiB bound the 422 path uses, except the preview body, bounded at 1 MiB (a 448×368 PNG
is well under it).

**5.2 Three commands**, all `async`, all typed into the IPC contract fixture:

- `get_server_plugins` → `PluginCatalog` (§4.1).
- `get_server_card_state` → the per-device status, projected to
  `ServerCardState { card_id, provider: ProviderState, hero: Option<String>, errors:
  Vec<CardError> }` for **plugin cards only**. It is the source of a plugin tile's
  value, flag and error copy.
- `render_card_preview` keeps its name; `PreviewFrame` becomes
  `{ png_base64: string | null, sample: bool, state: string | null }` — additive, and
  built-in cards keep `png_base64` set and `state` null. The plugin arm: in networked
  tier it fetches §4.2 and returns the frame, or `png_base64: null` with `state` set to
  the server's message for `waiting` and `error`; in local tier it returns
  `png_base64: null, state: "Plugin cards render on the server"`. A transport failure
  is the existing typed error. The frontend prints a `state` as a state word on the
  stage (§6.5), never as "Preview unavailable".

**5.3 The hostless runtime stops pretending.** `replace_config` schedules no provider
deadline for a plugin card when the runtime has no plugin host, and the snapshot carries
no provider entry for it; the Mac's projection (§5.4) supplies the real one. Today's
permanent `stale` flag with an internal message is a defect, not a state.

**5.4 Projection.** A `ServerStateProjection` beside `NetworkedConfigProjection` holds
the last `ServerCardState` list; `project_snapshot` overlays it onto `providers`,
`card_data` (the `hero` field) and `card_errors` for plugin cards when the tier is
networked. The frontend polls `get_server_card_state` every 30 s while the window is
visible and on the same focus trigger the snapshot uses; a failed poll keeps the last
projection and surfaces one notice ("Couldn't reach the server for plugin data") that
clears on the next success. The catalog is fetched on window open and on Retry.

## 6. The frontend

**6.1 Naming.** `cardLabel(card, catalog)` returns the plugin's `display_name`, else the
plugin id **with a printed word**: a `not on the server` flag on the tile when the id is
unknown to a loaded catalog, and a `needs the server` flag in local tier. `cardTitle`
stays the owner's quiet line. `filmstripSegments`, the ring legend, the editor heading,
every `template — title` control label and every notice thread the catalog through the
same helper. `cardKindName`'s plugin arm ("Plugin") is deleted: there is no longer any
surface that calls a plugin card "Plugin".

**6.2 Picker.** Rows: `display_name` in bold, `description` beneath; fallbacks per §3.
Picking a row runs `addCard(config, { kind: "plugin", pluginId })`: title `""`, refresh
from the catalog's `refresh_minutes`, tap `none`, alert `none`, card id from
`nextId("plugin", …)` (never derived from the 64-byte plugin id, which can exceed the
32-byte card-id bound), through the same capacity guard and loop enrolment as every
other add. One code path, not two.

**6.3 Tile.** Value = the `hero` field from the projection, else "—". Flags from the
projected provider state, same meaning as on weather. Nothing on the tile says
"plugin".

**6.4 Editor.** Heading = display name, quiet line = title, "Name" field. A **Plugin**
field: a `<select>` over the catalog showing `display_name · version` with the id as the
small line — the editable, never free-text, home of `plugin_id`, and where a
`cards[i].plugin_id` issue attaches with `aria-invalid` and `FieldIssues`. An id the
catalog lacks renders as the selected-but-invalid option, "Not installed on the
server". With no catalog loaded the field is read-only and says why. "Refresh every"
keeps its control; its hint names the manifest's own cadence. Dwell and the gesture
note are already kind-neutral. Error copy names the true cause and offers Refresh only
when the Mac can refresh.

**6.5 Preview stage.** A frame renders like any other. `sample: true` shows the existing
"No data yet" badge. A `state` with no frame prints as a state word on the black stage
("Waiting for the first refresh", "Plugin cards render on the server", or the server's
error), in the same type as today's "Preview unavailable", which is reserved for real
transport failures.

**6.6 Dev harness.** `mockBackend` answers the three commands from a fixture catalog of
the four curated plugins and a `?scenario=plugin` document with one display-list and one
SVG plugin card in the loop; `mockPreview` returns a fixture PNG for them. Every state
in §5.2's outcomes is reachable by scenario.

## 7. Data flow

```
manifest v2 (display_name, description, summary)
      │ load
      ▼
server registry ──GET /v1/plugins──────────────▶ Mac: catalog ─▶ names, picker, editor select
      │ refresh (title + hero fields, provider state)
      ▼
server runtime ──GET /v1/devices/{id}──────────▶ Mac: ServerStateProjection ─▶ tile value, flags, errors
      │ RenderCardPreview (worker, revision 0, no device I/O)
      ▼
resvg frame ─────GET .../cards/{card}/preview──▶ Mac: render_card_preview ─▶ stage PNG
```

Nothing new reaches the device.

## 8. Errors and degraded states

| State | Tile | Stage | Editor |
|---|---|---|---|
| Local tier | `needs the server` flag, value "—" | "Plugin cards render on the server" | Plugin field read-only, "Needs the server to render" |
| Networked, server unreachable | last projection kept; one notice in the work column | last frame kept; state word if none | catalog from last fetch; Retry |
| Plugin not on the server | `not on the server` flag | server's message | selected-but-invalid option |
| Waiting for first refresh | "—", no flag | "Waiting for the first refresh" | — |
| Provider stale / error | `stale` flag, trouble line, Refresh disabled with the reason "refreshes on the server" | frame with the footer the panel shows | same trouble line |

Every validation issue stays claimed by a surface: `cards[i].plugin_id` moves from the
tile-only fallback to the editor's Plugin field (and stays on the tile).

## 9. Testing

- **plugin crate:** v2 parse of the three keys with bounds; `summary` evaluation
  including truncation; v1 documents unchanged; the curated manifests round-trip.
- **server:** catalog DTO shape pinned; preview route through the real
  `ServerPluginHost` for a display-list and an SVG plugin with fixture data, asserting a
  decodable 448×368 PNG and each §4.2 outcome; `RenderCardPreview` never mints a
  revision and never sends a frame (`hostile_device.rs`-style exact-sequence assertion);
  `hero` emitted on refresh and absent when undeclared.
- **app-core:** hostless runtime schedules no plugin refresh and reports no provider
  entry; the projection overlay.
- **Mac Rust:** loopback-server tests for each GET (the existing `TcpListener` harness),
  bounded bodies, error mapping; contract fixture sync.
- **frontend:** naming through the catalog and both fallbacks; picker rows; `addCard`
  plugin arm and both caps; editor Plugin select with the invalid-option and read-only
  states; tile value from the projection; stage state words; every harness scenario.
- **Mutation probes** on the new tests before merge, as on the one-loop change.

## 10. Rollout

1. Server first (registry-compatible: v1 manifests keep loading; v2 keys optional), then
   the curated manifests moved to v2, then the deploy — the Mac's GETs are additive and
   fail closed to the fallbacks against an old server (a 404 on the preview route reads
   as "Plugin cards render on the server" until the server is redeployed).
2. Mac app second. No schema bump, so no lockstep is required beyond the preview route.
3. Docs in the same change: `docs/plugins/manifest-v2.md` amendment, PRODUCT.md's
   preview positioning and objects, DESIGN.md's naming bullet, CLAUDE.md state.

## 10a. Corrections made while planning (2026-09-07)

Two facts checked against the code changed a sentence each; both are folded into the
plan and into §5.2/§6.5/§8 above.

- **The waiting state prints a word, not the "No data yet" badge.** The badge means "a
  real frame, rendered from sample data"; a plugin card waiting for its first refresh has
  no frame at all, so `render_card_preview` returns `png_base64: null, sample: false,
  state: "Waiting for the first refresh"` and the stage prints that sentence. §5.2's
  earlier "maps `waiting` to `sample: true`" is superseded.
- **The new `hero` field is safe on the wire, and this was verified rather than assumed.**
  A plugin card's wire template is `DigitalClock`, whose firmware registry declares only
  `title`, `show_seconds`, `stale` and `error`. `firmware/main/core/template_fields.h`
  states that unknown fields are ignored, so the device silently drops `hero` and no
  device-side change is needed. The plan records this as a comment beside the test.

## 11. Open question for the owner

None blocking. One judgement call is made here rather than asked: the tile value with
no `summary` is "—", the same as a weather card with no data, rather than a freshness
sentence, because a tile owns one fact and "3 min ago" is a fact about the fetch, not
the card.
