# Remove manifest-based plugins entirely

Branch: `refactor/remove-plugin-manifests`, cut from `refactor/retire-device-data-cards` at
`bca8df8`. Owner direction, 2026-09-11, verbatim:

> I want to remove that feature altogether. We'll only [have] custom on-device cards like
> clock or pomodoro and server-side cards which are basically pictures delivered to the
> device.

The product now has exactly **two** ways a card gets a face:

1. **On-device** — `clock` and `pomodoro`. A host-built scene whose bindings (`time:*`,
   `date`, `timer.*`) let the panel keep it correct between pushes.
2. **Server-side** — `picture`. An external producer sends a 448x368 PNG to a
   token-addressed webhook; the frame is stored as a durable asset and drawn.

Everything that existed to compile a TOML manifest into a scene, fetch data for it, or
rasterize one, goes.

## Established facts — do not re-derive these

1. **Nothing in the fleet uses a plugin.** `/var/lib/deskmate/configs/dev-0005.json` and
   the Mac's store both hold `clock` + `picture` at schema v8. Checked 2026-09-11.
2. **The replacement is already in production.** `tools/picture-producers/claude_limits_png.py`
   with `deskmate-claude-limits.{service,timer}` is the picture twin of the `claude-limits`
   plugin and is what currently feeds the owner's panel. **Do not delete
   `tools/picture-producers/`.**
3. **`egress.rs` has exactly two consumers, both plugin modules.** With plugins gone the
   server makes no outbound HTTP at all.
4. **`image_ingest.rs` uses `resvg::tiny_skia::{Pixmap, IntSize}` only** — the pixel
   buffer, not the SVG renderer.
5. **No firmware change, and this is a decision, not an oversight.** The device keeps
   `VolatileAssets` (bit 9), `DurableAssetEncoding` (bit 10), every scene node kind, and
   `PROTOCOL_CURRENT_CAPABILITIES` stays **2027**. A capability states what the device
   *accepts*; the host simply stops sending volatile assets. Touching firmware statics
   would oblige an on-board OTA download re-verification for a change that is otherwise
   pure host-side deletion, and this repo has twice lost days to memory-layout shifts with
   every test green.
6. Protocol stays **v1**. No message type is removed — `AssetBegin`/`AssetChunk`/
   `AssetCommit`/`AssetRelease` all still serve picture frames.

## Scope

### S1. Schema v9 and its migration

- `CURRENT_SCHEMA_VERSION` becomes `9`; surviving card kinds are `clock`, `pomodoro`,
  `picture`. Create `docs/config/v9.md` from v8.
- Add `ConfigOrigin::MigratedV8`. **Extend the existing v8 machinery rather than writing a
  second one**: `drop_retired_cards_from_json` gains `"plugin"`, the version arm widens to
  `4..=8`, and `finish_migration` is unchanged. Add a migration test that a v8 document
  containing a plugin card loads with that card and its playlist entries gone.
- Fixtures: copy `full.json` to `v8-roundtrip.json` before bumping, per the convention.
  **`plugin-card.json` keeps its plugin card** — it becomes the v8 fixture proving a saved
  plugin card still migrates. Do not edit it to remove the card; that would delete the only
  evidence the migration works.

### S2. Delete the plugin implementation

- **`companion/crates/plugin`** — the whole crate, and its workspace member entry.
- **`companion/plugins/`** — all four curated manifests and their assets.
- **`docs/plugins/manifest-v1.md`, `manifest-v2.md`** — delete; they document a format that
  no longer exists.
- **Server:** `plugin_registry.rs`, `plugin_provider.rs`, `plugin_refresher.rs`,
  `plugin_host.rs`, `test_plugins.rs`, `egress.rs`, `rasterizer.rs`; the `GET /v1/plugins`
  route, the card-preview route, and the operator scene route's `template: "plugin"` form.
  `DESKMATE_PLUGINS_DIR` and its default go with them.
- **`tools/fonts/`** — the pinned Inter faces existed for the rasterizer. Verify nothing
  else `include_bytes!`es them before deleting (`lvgl-sim` has its own
  `assets/Inter-subset.ttf`, which is a different file and stays).
- **app-core:** the `PluginHost` trait and every `Plugin*` DTO in `admin.rs`,
  `ProviderRequest`/`ProviderRefresher`/`SystemProviderRefresher` and the provider worker
  (nothing produces a request any more), plugin snapshots in `WorkerState`, and
  `RuntimeHandle::start_with_plugin_host`'s host parameter.
- **`companion/crates/providers`** — after this only `validate_http_url` survives, used by
  `config.rs` for `WidgetTapAction::OpenUrl`. Fold that one function into `app-core` and
  delete the crate. Say so in your report if something else turns out to need it.
- **`resvg` -> `tiny-skia`** in `server`'s dependencies, for `image_ingest.rs`'s `Pixmap`.
- **Frontend:** the `plugin` card kind, `PluginCatalog`/`PluginKindOption`/`ServerCardState`
  and their IPC commands, `ServerStateProjection`, catalog polling, the
  `IncompatibleServer` notice, `pluginCardFlag`'s plugin arm, the add menu's plugin group,
  and the read-only plugin identity row added yesterday.

### S3. What must keep working — check each one

- **Picture cards end to end**: ingest, staleness inference, durable asset transfer with
  RLE565, `AssetRelease` keep-set composition, the card face.
- **`AssetRelease` is a keep-set where omission means delete.** `desired_assets()` was the
  union of plugin-registry assets and image-source frames; it becomes image sources alone.
  The guard that skips the release pass when nothing is desired **must survive** — an empty
  desired set is never a no-op on this wire, and deleting that guard once wiped every asset
  on the device. `server/tests/hostile_device.rs` asserts the exact request sequence; keep
  it passing.
- **Render negotiation**: the `Rasterize` outcome and the binding classification that only
  raster needed are gone, but a scene must still not be pushed while the device lacks a
  digest it references. Keep whatever gates a **picture** card's frame; delete the rest.
  Decide the shape yourself and say what you chose.
- The six scene builders, `DisplayTemplate`, `scene_parity.rs`, `lvgl-sim`'s
  non-plugin cases and goldens, and the operator scene route's `digital_clock` diagnostic
  form all stay.

### S4. Evidence that changes, and must be reported as a number

`lvgl-sim`'s `plugin_scene_cases()` (16 rows) and `claude_limits_scene_cases()` go, with
their goldens. The hardware framebuffer matrix is currently **96 total / 10 excluded / 86
identical**, observed on the board 2026-09-06. Recompute the new expected split and state
it plainly as a **software prediction, not an observation** — the next hardware session
must run `framebuffer_diff` fresh. Update every place the old numbers are written down.

## Explicit non-goals

- **No firmware change.** Nothing under `firmware/`. Capabilities stay 2027.
- **No protocol change.** No message type, key, or capability bit is removed.
- **Do not delete `tools/picture-producers/`** — that is the replacement, and it is live.
- **Do not delete the durable asset path, RLE565, or the image-source machinery.**
- **Do not commit.**

## Docs

`docs/config/v9.md` (new, from v8); `CLAUDE.md` (one bullet, house style: what went, that
firmware/protocol/capabilities are untouched, that the fleet had no plugin card, and that
a server-side card can no longer tick between pushes); `PRODUCT.md`; the deploy README's
plugin sections and `DESKMATE_PLUGINS_DIR`; mark `docs/config/v8.md` superseded. Prune the
plugin paragraphs from `docs/scene/template-parity-ledger.md` rather than leaving it
describing deleted machinery.

## What is deliberately lost — state it, do not work around it

A server-side card can no longer change between pushes: a picture is frozen until the next
one arrives. Only a manifest could bind `time:*`/`timer.*` and have the device animate it
(`svg-live-clock` did, and is a session fixture that was never in this repository). Clock
and pomodoro remain the only faces that tick on the device. That is the owner's model.

## Gates

```sh
cd companion
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets --no-fail-fast
cargo test --workspace --doc
cd apps/deskmate && /Users/rodion/.bun/bin/bun test && /Users/rodion/.bun/bin/bun run check
```

Your sandbox cannot bind loopback sockets; report which `server` tests fail that way rather
than working around them. The orchestrator re-runs them.

## Report

Files changed and lines removed, test counts before/after, the new framebuffer-matrix
prediction, every decision this brief did not settle, and anything contradicting the
"Established facts" above.
