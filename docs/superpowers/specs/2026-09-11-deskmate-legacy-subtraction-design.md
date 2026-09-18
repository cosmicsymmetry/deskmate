# Deskmate legacy subtraction — design

> **HISTORICAL BASELINE NOTE (2026-09-18).** References to the Mac app describe the
> pre-deletion system that this subtraction work evaluated. The Tauri companion no
> longer exists.

Date: 2026-09-11. Status: Wave A specified; Waves B and C committed in scope,
specified at their turn.

## Why

Seven schema versions and four rendering approaches left every surface carrying
the shape of a product that no longer exists. The cost is not bytes, it is
arithmetic: adding the plugin card kind cost 342 lines in `config.rs` alone
before any UI, and a card kind is enumerated in fifteen files. The next feature
pays that toll again unless the shapes underneath it are the ones the product
actually has.

This is subtraction, not refactoring for its own sake. Every item below removes
a concept, not just an indirection.

## What already landed

Two breaking bumps merged into `main` as `faa5a97` before this spec was written:

- **v8** retired the device-rendered data cards (calendar, weather, json-feed,
  rss) and the `providers` fetch layer behind them. −7,332 lines.
- **v9** removed the TOML plugin manifests, the expression language, the `resvg`
  rasterizer, and the SSRF egress guard that existed only to serve them.
  −29,244 lines.

Net −34,106 across 199 files. The Rust workspace went 74,268 → 49,846 lines.
Three card kinds remain: **clock, pomodoro, picture**. Firmware, protocol v1 and
capabilities 2027 were untouched by both, because a capability states what the
device *accepts*, not what the host *sends*.

The live server already runs the v9 build (deployed 2026-09-11 08:31 UTC,
9.6 MB against the 20 MB pre-v9 backup, zero `resvg` strings). Nothing is owed
operationally.

## The three waves

| Wave | Contract | Hardware | Content |
|---|---|---|---|
| **A** | none (v9 stays frozen) | none | migration chain, the second renderer, module boundaries, documentation |
| **B** | schema v10 | none | playlists |
| **C** | protocol v2 | one session | the pre-scene wire |

Ordering is load-bearing. Wave A retires the last consumer of the three orphan
`DisplayTemplate` variants, so by the time Wave B rewrites the config the enum is
already down to what a card can name. Wave C is last because it is the only item
that spends the repo's scarcest resource — an on-board OTA verification — and
because it should carry whatever else firmware owes by then, on one image.

---

# Wave A

No frozen contract moves and no firmware image; every item is provable on the
host. A1 does narrow accepted *input* — a pre-v4 document stops being readable —
which is a deliberate behaviour change, not a contract revision.

## A1 — The migration chain

`store.rs` dispatches nine `ConfigOrigin::MigratedV0..V8` arms. They are not
equal work. The `4..=8` arm is one generic path — strip retired card kinds from
the JSON while it is still `Value`, then deserialize the current shape strictly —
about thirty lines total. Versions 0 through 3 each need a full shadow type
hierarchy: `LegacyConfigV0/V1/V2/V3`, `LegacyWidgetSettings`, `LegacyWidgetV2`,
`LegacyCardV3`, `LegacyCardPresenceV3`, `LegacyScreen`, `LegacySizeClass`,
`LegacyInterruptPolicy`, `LegacyCardKind`, and nine migration functions.

Schemas v0–v3 predate M4 (2026-08-05). No document at those versions exists in
the fleet, on the Mac, or on the server.

**Decision.** Delete the 0/1/2/3 arms and every type and function reachable only
from them. Keep the `4..=8` arm. A document below v4 becomes
`StoreError::UnsupportedVersion`, which the store already presents correctly —
it is a typed refusal that preserves genuine last-good state and is never
labelled "your last working settings".

**The rule that replaces the judgement call:** a `docs/config/vN.md` exists if
and only if the code can still read vN. `v3.md` goes with the v3 arm. `v4`–`v8`
stay while the `4..=8` arm does. This makes the doc set self-maintaining instead
of a growing pile.

Keep one test asserting a v3 document is refused with a legible message naming
the supported version. Delete the v0–v3 migration tests.

## A2 — The second renderer

Today the system has two renderers for the same six faces:

- The **device** draws host-pushed scenes. It has since stage 3a.
- The **Mac app's card preview** draws the retired C templates, through
  `lvgl_sim::RenderRequest` → `SimTemplate` → `crates/lvgl-sim/reference-oracle/`
  (`commands.rs:875`).

They agree only because `crates/app-core/tests/scene_parity.rs` (1,581 lines)
forces them to, against 2,170 lines of oracle C. Three of the six templates —
`RowList`, `BigNumberLabel`, `IconBadgeText` — are unreachable from any card
since v8 (`config.rs:1741,1753`) and survive *only* because the oracle needs
them.

**Decision.** Move the preview onto the scene path and retire the oracle with
the gate that justified it.

The machinery is already in place: `build_card_scene` (`runtime.rs:1916`) takes
`&AppConfig`, a card id, fields, an optional `ImageSourceHost` and a revision —
no runtime state — and `lvgl_sim::scene::SceneRenderRequest` already accepts
assets, a timer, fields and an orientation. The preview becomes: build the same
scene the device would receive, render it in the simulator.

Removed: `reference-oracle/`, `scene_parity.rs`, `face_scene_cases.rs`,
`cases::golden_cases()`, `RenderRequest`, `SimTemplate`, the `build.rs` guard
that keeps the oracle out of the firmware image, the three orphan
`DisplayTemplate` variants and their scene builders, and
`docs/scene/template-parity-ledger.md`.

Repointed, not removed: `tests/golden.rs` renders the 32 face PNGs from
`face_scene_cases()` instead of `golden_cases()`. `clock_chrome.rs` and
`tabular.rs` measure *relationships between frames* that a golden cannot state
(no title chip; tabular digit advance) and move to the scene path intact.

**The repin is its own proof.** Parity holds today at zero differing pixels with
no tolerance, so regenerating the goldens from the scene path must reproduce
them byte-identically apart from cases the parity gate already excludes. Any
other changed pixel means the gate was lying, and the deletion stops until that
is understood. See "Proven, not predicted" below for the measured result.

Coverage is unchanged: the goldens still pin every pixel of every face. What is
lost is the assertion "this matches code we deleted". The hardware matrix is
unaffected — `framebuffer_diff` already drives `SceneRenderRequest` only.

**One thing gets better as a side effect**, and it is not the goal: the preview
shows what the panel shows *by construction* rather than by a gate.

**Picture cards are NOT part of that**, and an earlier draft of this spec was
wrong to claim they were. `build_card_scene` can draw a picture face, but only
from an `ImageSourceHost`, and the Mac app has none — `lib.rs:445` starts the
runtime as `start_serial(config, None)`, and the only implementation is
`ServerImageSourceHost`. Frames live on the server because that is where
producers push them. So a picture card's preview stays
`PICTURE_PREVIEW_IS_PUSH_ONLY` and draws nothing. Rendering one means fetching
the frame from the server in networked tier, which is a feature, not a
side effect, and is out of scope here.

**Proven, not predicted.** Repointing `tests/golden.rs` at `face_scene_cases()`
with `render_scene_png` was run before this spec was finalised: 28 of the 29
landscape goldens regenerate byte-identically, and the single mismatch is
`progress-ring--running-mid-countdown--landscape`, whose divergence
`face_scene_cases.rs:27-31` already documents — the oracle advances from LVGL's
fake-tick anchor while a scene consumes the supplied snapshot. That golden is
re-blessed with the reason recorded; the scene value is the more deterministic
of the two. Its hardware exclusion is unaffected: the device still ticks a
running timer through `scene_view_tick_bindings`, so the race the exclusion
names is still real.

Non-goal: changing what the preview *decides*. `preview_orientation` stays —
the preview renders upright at both mountings, on the owner's explicit
direction, and that rule is orthogonal to which renderer draws it.

## A3 — Module boundaries

`runtime.rs` is 2,731 lines of implementation plus 1,037 of tests, and it owns
device I/O, the command loop, scheduling, pomodoro, alerts, interrupts, scene
building, scene pushing, assets, and snapshot publication. The seams are already
visible as contiguous function groups; the file simply never grew walls.

**Decision.** Split along the existing seams into `runtime/`:

| Module | Contents | ≈ lines |
|---|---|---|
| `mod.rs` | `RuntimeOptions`, `RuntimeHandle`, `WorkerState`, `run_runtime`, `process_command`, rotation helpers, `run_scheduled_work`, `attempt_connect` | 900 |
| `device.rs` | `RuntimeDevice`, `SerialRuntimeDevice`, `DeviceConnection`, and the device-error cluster (`handle_device_error` … `update_device_status`) | 450 |
| `scene.rs` | `build_card_scene` and everything from `field_text` to `handle_automatic_asset_error` | 490 |
| `timers.rs` | pomodoro, alerts, interrupt flush | 300 |
| `sync.rs` | `synchronize_full`/`_pending`, `push_dirty_widgets`, `send_screen`, `send_time_sync` | 250 |
| `snapshot.rs` | `SnapshotPublisher`, subscriptions, diagnostics, `initial_snapshot` | 250 |

`WorkerState` stays the shared spine in `mod.rs` with `pub(super)` fields.
Giving each concern its own state struct is the better end state and is
explicitly **not** this wave's work: it would mix a move with a rewrite, and the
move is what makes the rewrite reviewable later.

Deletion before splitting, not after — v9 already took `runtime.rs` from 6,771
to 3,768, and A2 takes more.

**`config.rs`'s shadow enums stay — an earlier draft of this spec said
otherwise and was wrong.** `mod strict_tagged_enum` looks like a duplicate of
the allowed-key table it sits beside, and the proposal was to collapse the two.
Reading it, the table already exists (`ValidatingDeserialize::allowed_fields`,
`config.rs:124`) and the shadow enums are not a second copy of it: they are a
**recursion breaker**, because the real enum's hand-written `Deserialize`
cannot deserialize into itself and needs a derived twin to land in. That is a
serde workaround for `deny_unknown_fields` being a no-op on internally tagged
enums, not legacy. It is verbosity, and by this spec's own test — every item
removes a concept, not an indirection — it does not qualify. Rewriting a
mechanism whose job is silently rejecting unknown fields, for about 165 lines,
is the worst risk-to-reward trade in the wave.

**`commands.rs`** (1,600 implementation lines) splits by the same method, and
A2 removes its preview-template dispatch outright.

## A4 — Documentation

- **`CLAUDE.md` is 113 KB and is loaded into every session.** Much of it is
  archaeology, including entries that open "STALE AS WRITTEN — corrected". Slim
  it to durable rules, current state, and traps that are still live; move the
  narrative history to `docs/history.md`, which nobody loads by default. The
  prior cleanup deferred this to the owner; it is now the most expensive
  document in the repo per unit of value.
- **Plans that lie.** Roughly 500 unticked `- [ ]` boxes across delivered plans
  describe shipped work. Apply the rule CLAUDE.md already states: a STATUS
  header at the top of each delivered plan saying so. Do not back-fill ticks.
- **Dead tooling.** `deskmate-cli`'s M2 commands (`apply-config`, `select-screen`,
  `push-clock`, `pomodoro`, `trigger-interrupt`, `events`, `inspect-config`) push
  `ApplyConfig`/`PushData` with templates, which draw nothing on scene-native
  firmware. Delete `m2.rs` and the README section that labels them legacy. Same
  for `crates/device/examples/m2_stress.rs`.
- `docs/config/v3.md` goes with A1, per A1's rule.

`docs/hardware/board-notes.md` (298 KB) is **not** in scope. It is append-only
physical evidence, it is never loaded by default, and its size is a property of
how much has been verified — which is the good kind of large.

---

# Wave B — playlists (schema v10)

Committed in scope, specified at its turn.

The document models many playlists (`playlists[]`, `active_playlist_id`: 200 and
70 references) for a product that exposes exactly one loop and has no way to
make another. That indirection is what forces the "Missing card" tile, the
"not in loop" state, the leftover banner's playlist arms, and the `claimedIssues`
narrowing.

The shape: the card list *is* the loop, ordered, with per-card dwell and one
global advance. v10 is subtractive and reuses v8's `Value`-level strip rather
than growing a second migration.

Decision points to settle then, not now: whether a card can exist outside the
loop at all (today's "not in loop" state is a migration artifact, and if v10
says no, the state and its UI go with it), and whether `advance` stays global or
becomes per-card.

# Wave C — the pre-scene wire (protocol v2)

Committed in scope, specified at its turn. The only item that spends a hardware
session.

The device still holds a widget model with `TemplateKind`, `SizeClass`,
screens, and a per-template field registry (`template_fields.c`), while every
pixel arrives via `PushScene`. `SizeClass::Standard` and `::Tile` appear only in
test fixtures; `SizeClass::Full` is pinned at every production call site. After
v9 **nothing in production emits a `field.*` binding** — the entire field-schema
apparatus survives to carry three magic pomodoro keys (`duration_seconds`,
`remaining_seconds`, `running`) that `protocol_task.c:352` reads by name.

The shape: `ApplyConfig` becomes a card list; the timer becomes explicit state
rather than three named fields; `template_fields.c`, most of `widget_model.c`,
`TemplateKind`, `SizeClass`, `ScreenConfig` and the three error codes about them
go.

Decision points to settle then: whether this is protocol v2 or a further
additive v1 (v2 permits deletion, which is the point, and the fleet is one
device the owner flashes); and what `field.*` means afterwards, given picture
cards may want it back.

**The standing hazard applies in full.** This repo has twice lost days to
memory-layout shifts with every test green — 105 bytes of static DRAM once
broke OTA downloads. A firmware change owes an on-board OTA download check and a
panel check, and Wave C should carry whatever else firmware owes by then so the
cost is one image and one session.

---

## Non-goals

- Rewriting `WorkerState` into per-concern state structs (A3 enables it; it is
  not this work).
- Touching `board-notes.md`.
- Any behaviour change the owner has not asked for. A2 changes what draws a
  preview, not what the preview decides.
- Reintroducing anything the subtraction pass of 2026-08-21 removed.

## Verification

Each wave runs the full gate set before handoff, and the gates are the existing
ones — this work adds no new kind of proof:

```sh
make -C firmware/host_tests clean test
make -C firmware/host_tests sanitize
cd companion && cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo test --workspace --doc
cd apps/deskmate && bun test && bun run check
```

Wave A additionally requires the golden repin to be byte-identical (A2), which
is the wave's own load-bearing evidence. Wave C additionally requires an
on-board OTA download check, a panel check at both orientations, and a
`framebuffer_diff` run, recorded in `docs/hardware/board-notes.md`.

A deletion is verified by what still passes, so the discipline is the one this
repo already uses: mutation-probe any guard that survives a deletion, and treat
a test that supplies its own inputs as unproven until something else derives
them.
