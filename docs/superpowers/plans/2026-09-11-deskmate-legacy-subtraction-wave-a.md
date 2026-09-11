# Legacy Subtraction — Wave A Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove three concepts the product no longer has — pre-v4 config
schemas, a second renderer, and dead M2 tooling — and put walls inside the two
files that grew without them.

**Architecture:** Every change is a deletion or a move. Nothing gains a feature,
no frozen contract moves, and no firmware image is built. The load-bearing
evidence is that the existing gates still pass and that the 29 face goldens
regenerate byte-identically from the surviving renderer.

**Tech Stack:** Rust 2024 (workspace at `companion/`), TypeScript/React
(`companion/apps/deskmate`), Tauri v2, C99 firmware host tests.

**Spec:** `docs/superpowers/specs/2026-09-11-deskmate-legacy-subtraction-design.md`

## Global Constraints

- Config schema stays **v9**. `CURRENT_SCHEMA_VERSION` is not touched.
- Protocol stays **v1**. `PROTOCOL_CURRENT_CAPABILITIES` stays **2027**.
- **No firmware source changes.** Any diff under `firmware/main/` fails this
  wave — a firmware image owes an on-board OTA download check this wave does not
  budget for. `firmware/host_tests/` still runs, unchanged, as a regression net.
- Card kinds stay exactly three: `Clock`, `Pomodoro`, `Picture`.
- Conventional commit prefixes (`feat:`, `fix:`, `test:`, `docs:`, `chore:`,
  `refactor:`). Do not rewrite shared history.
- Full gate set before handoff, every command run separately so a failure names
  the missing coverage:

```sh
make -C firmware/host_tests clean test
make -C firmware/host_tests sanitize
cd companion && cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo test --workspace --doc
cd apps/deskmate && bun test && bun run check
```

`cargo` needs `export PATH="$HOME/.cargo/bin:$PATH"`. Never pipe a cargo
invocation into `tail` and read the exit status — in zsh that reports `tail`'s
status and hides a failure as a pass. Redirect to a file and check `$?`.

## Non-goals, and why

- **`config.rs`'s `mod strict_tagged_enum` stays.** Its shadow enums look like
  duplication but are a recursion breaker: the real enum's hand-written
  `Deserialize` cannot deserialize into itself, so it needs a derived twin.
  The allowed-key table it is paired with (`ValidatingDeserialize::allowed_fields`)
  already exists. This is a serde workaround, not legacy, and rewriting a
  mechanism whose job is silently rejecting unknown fields is the worst
  risk/reward in the wave.
- **Picture-card previews stay unrendered.** The Mac holds no frame
  (`lib.rs:445` starts `start_serial(config, None)`); see the spec.
- **`docs/hardware/board-notes.md` is untouched.** Append-only physical evidence.
- **No `WorkerState` redesign.** Task 4 moves code; it does not restructure state.

## File structure after this wave

| Path | Responsibility | Change |
|---|---|---|
| `crates/app-core/src/store.rs` | load/save, and migration from **v4+ only** | shrinks ~660 lines |
| `crates/app-core/src/runtime/mod.rs` | handle, worker loop, `WorkerState`, scheduling | split out of `runtime.rs` |
| `crates/app-core/src/runtime/device.rs` | `RuntimeDevice`, serial impl, device-error handling | new |
| `crates/app-core/src/runtime/scene.rs` | scene building and pushing, asset reconciliation | new |
| `crates/app-core/src/runtime/timers.rs` | pomodoro, alerts, interrupt flush | new |
| `crates/app-core/src/runtime/sync.rs` | full/pending sync, field push, time sync | new |
| `crates/app-core/src/runtime/snapshot.rs` | publication, subscriptions, diagnostics | new |
| `apps/deskmate/src-tauri/src/ipc.rs` | `IpcError`, DTOs, input validators | new |
| `apps/deskmate/src-tauri/src/commands/*.rs` | Tauri commands, grouped by surface | split |
| `crates/lvgl-sim/src/lib.rs` | scene rendering only | `RenderRequest`/`SimTemplate` removed |
| `crates/lvgl-sim/reference-oracle/` | — | **deleted** |
| `crates/app-core/tests/scene_parity.rs` | — | **deleted** |
| `crates/deskmate-cli/src/m2.rs` | — | **deleted** |

---

### Task 1: Delete the pre-v4 migration chain

Independent of every other task. Can run first or in parallel.

**Files:**
- Modify: `companion/crates/app-core/src/store.rs`
- Modify: `companion/crates/app-core/tests/store.rs`
- Delete: `docs/config/v3.md`

**Interfaces:**
- Consumes: nothing.
- Produces: `ConfigOrigin` loses `MigratedV0`, `MigratedV1`, `MigratedV2`,
  `MigratedV3`. Later tasks must not reference them. `ConfigOrigin::Current`,
  `MigratedV4`..`MigratedV8` and `Defaults` are unchanged.

- [ ] **Step 1: Write the failing test**

In `companion/crates/app-core/tests/store.rs`, add:

```rust
#[test]
fn a_pre_v4_document_is_refused_as_an_unsupported_version() {
    // v0-v3 predate M4 (2026-08-05) and no document at those versions exists
    // in the fleet, on the Mac, or on the server. Refusing one is a typed,
    // legible outcome -- NOT a validation failure, which would be presented
    // as the owner's settings being broken.
    let directory = test_directory("pre-v4-refused");
    let path = directory.path().join("config.json");
    std::fs::write(&path, br#"{"schema_version":3,"preferences":{},"cards":[]}"#)
        .expect("write v3 document");

    let store = ConfigStore::new(&path);
    let outcome = store.load();

    let recovery = outcome.recovery().expect("a v3 document must be refused");
    assert!(
        matches!(
            recovery,
            StoreError::UnsupportedVersion { found: 3, supported: CURRENT_SCHEMA_VERSION }
        ),
        "a v3 document must be an UnsupportedVersion refusal, got {recovery:?}"
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cd companion && cargo test -p app-core --test store a_pre_v4_document_is_refused
```

Expected: FAIL. Today the v3 arm migrates the document instead of refusing it,
so `recovery()` is `None` and `.expect(...)` panics.

- [ ] **Step 3: Delete the pre-v4 arms from `decode_config`**

In `store.rs`, in `decode_config`, delete the four match arms `3 => {...}`,
`2 => {...}`, `1 => {...}` and `0 => {...}` in their entirety. The `found =>`
arm that returns `StoreError::UnsupportedVersion` already follows them and now
catches 0 through 3. Keep `CURRENT_SCHEMA_VERSION =>` and `version @ (4..=8) =>`
exactly as they are.

- [ ] **Step 4: Delete the four `ConfigOrigin` variants**

In `store.rs`, remove `MigratedV0`, `MigratedV1`, `MigratedV2` and `MigratedV3`
from `pub enum ConfigOrigin` (lines 124-127).

- [ ] **Step 5: Delete every type and function reachable only from those arms**

Delete these items from `store.rs`. All of them are private and used only by the
deleted arms:

Types (contiguous block, `LegacyConfigV0` at line 247 through `LegacyWidgetV2`,
ending immediately before `fn card_from_legacy_v2`): `LegacyConfigV0`,
`LegacyConfigV1`, `LegacyWidgetSize`, `LegacyCalendarSource`,
`LegacyWeatherUnits`, `LegacyJsonFieldMapping`, `LegacyCardAlert`,
`LegacyWidgetSettings`, `LegacyScreenSettings`, `LegacyConfigV2`,
`LegacyConfigV3`, `LegacyCarouselV3`, `LegacyCardPresenceV3`, `LegacyCardV3`,
`LegacyCarouselV2`, `LegacyScreenV2`, `LegacyLayoutV2`, `LegacyInterruptPolicy`,
`LegacyWidgetV2`, and `LegacyCardKind`.

Functions: `card_from_legacy_v2`, `migrate_v2`, `card_from_legacy_v3`,
`migrate_v3_alert`, `migrate_v3`, `synthesize_playlist`,
`card_from_legacy_widget`, `migrate_legacy`, `legacy_alert`,
`legacy_widget_id`.

**Do NOT delete** `VersionHeader` (line 240), `finish_migration`,
`drop_retired_cards_from_json`, or `parse_json`. The `4..=8` arm needs all four.
`synthesize_playlist` goes because its only three callers (`migrate_v2`,
`migrate_v3`, `migrate_legacy`) all go with it — `finish_migration` does not
call it.

- [ ] **Step 6: Let the compiler find the rest**

```sh
cd companion && cargo build -p app-core 2>&1 | tee /tmp/wave-a-t1.log; echo "EXIT=$?"
```

Delete anything the compiler names as unused that is reachable only from the
removed code. Do not silence a warning with `#[allow(dead_code)]` — if something
survives, it has another caller and must stay.

- [ ] **Step 7: Delete the pre-v4 migration tests**

From `companion/crates/app-core/tests/store.rs`, delete these tests entirely:

- `v3_migrates_to_one_playlist_preserving_rotation_order_and_dwell`
- `v3_all_alert_only_migrates_to_valid_config`
- `v0_v1_v2_migrate_directly_to_v9`
- `v2_documents_migrate_to_cards_in_screen_order`
- `migrated_v2_documents_always_satisfy_the_rotation_rule`
- `v2_dashboard_layout_document_fails_to_deserialize_as_a_recoverable_error`
- `v2_orphan_widgets_become_alert_only_or_off_instead_of_being_dropped`
- `v1_orphan_widgets_become_alert_only_or_off_instead_of_being_dropped`

Then repair `save_round_trips_and_migration_is_explicit` (line 97), which
asserts `ConfigOrigin::MigratedV0` and `MigratedV1`. Rewrite it against a v4
document and `ConfigOrigin::MigratedV4`, keeping its existing assertions about
`schema_version`, `preferences.timezone` and `preferences.autostart` so the
"migration is explicit" claim it exists to make survives.

- [ ] **Step 8: Delete `docs/config/v3.md`**

The rule this establishes, recorded in Task 7's CLAUDE.md edit: **a
`docs/config/vN.md` exists if and only if the code can still read vN.** `v4`
through `v8` stay because the `4..=8` arm stays.

```sh
git rm docs/config/v3.md
```

Then remove any link to it. Check with:

```sh
grep -rn "config/v3" --include='*.md' --include='*.rs' . | grep -v '^./.git'
```

- [ ] **Step 9: Run the tests**

```sh
cd companion && cargo test -p app-core > /tmp/t1-test.log 2>&1; echo "EXIT=$?"
```

Expected: PASS, including `a_pre_v4_document_is_refused_as_an_unsupported_version`.

- [ ] **Step 10: Mutation-probe the refusal**

Temporarily widen the surviving arm to `version @ (3..=8)` and re-run the new
test. It MUST fail. Revert the widening. A refusal nothing can break is not
pinned.

- [ ] **Step 11: Commit**

```sh
git add -A
git commit -m "refactor: delete the pre-v4 config migration chain

Nine migration arms were not nine units of work. The 4..=8 arm is one
generic path -- strip retired card kinds from the JSON while it is still
Value, then deserialize strictly. Versions 0 through 3 each needed a full
shadow type hierarchy: twenty types and ten functions between them.

Schemas v0-v3 predate M4 (2026-08-05) and no document at those versions
exists in the fleet, on the Mac, or on the server. A pre-v4 document is now
StoreError::UnsupportedVersion, which the store already presents as a typed
refusal that preserves genuine last-good state -- never as the owner's
settings being unreadable.

docs/config/v3.md goes with the arm that read it, establishing the rule that
a schema document exists iff the code can still read that schema.

Mutation-probed: widening the surviving arm to 3..=8 fails the new test."
```

---

### Task 2: Move the card preview onto the scene path

Depends on nothing, but **Task 3 depends on this**. Do it first of the two.

**Files:**
- Modify: `companion/crates/app-core/src/runtime.rs` (export a preview entry point)
- Modify: `companion/crates/app-core/src/lib.rs` (re-export it)
- Modify: `companion/apps/deskmate/src-tauri/src/preview.rs`
- Modify: `companion/apps/deskmate/src-tauri/src/commands.rs:838-965`

**Interfaces:**
- Consumes: `lvgl_sim::scene::{SceneRenderRequest, SceneTimer}`,
  `lvgl_sim::Simulator::render_scene_png`, both already public.
- Produces: `app_core::preview_card_scene(config: &AppConfig, card_id: &str,
  fields: &[Field]) -> Result<protocol::Scene, String>` — the scene the device
  would receive for this card, built with revision 1 and no image-source host.
  Task 3 relies on this being the only renderer the app calls.

- [ ] **Step 1: Write the failing test**

In `companion/crates/app-core/tests/runtime.rs`, add:

```rust
#[test]
fn preview_card_scene_builds_the_same_face_the_device_would_receive() {
    // The preview must not be a second renderer. This pins that it produces a
    // scene at all for each locally-renderable kind, and refuses the one kind
    // whose pixels live on the server.
    let config = AppConfig::default();
    let clock_id = config.cards[0].id().to_owned();

    let scene = app_core::preview_card_scene(&config, &clock_id, &[])
        .expect("a clock card previews locally");
    assert!(
        !scene.nodes.is_empty(),
        "a clock preview must draw something"
    );

    let missing = app_core::preview_card_scene(&config, "no-such-card", &[]);
    assert!(
        missing.is_err(),
        "an unknown card id must be a typed refusal, not an empty frame"
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cd companion && cargo test -p app-core --test runtime preview_card_scene
```

Expected: FAIL to compile — `preview_card_scene` does not exist.

- [ ] **Step 3: Add the entry point in `runtime.rs`**

`build_card_scene` (line 1916) already takes no runtime state. Add directly
below it:

```rust
/// The scene the device would receive for one card, built for display rather
/// than for pushing: revision is fixed at 1 and nothing is minted, marked dirty
/// or sent.
///
/// A `Picture` card has no locally renderable face -- its frames live on the
/// server, where producers push them, and this process has no `ImageSourceHost`
/// -- so it is refused rather than drawn blank.
pub fn preview_card_scene(
    config: &AppConfig,
    card_id: &str,
    fields: &[Field],
) -> Result<protocol::Scene, String> {
    let card = config
        .cards
        .iter()
        .find(|card| card.id() == card_id)
        .ok_or_else(|| format!("card {card_id:?} is not present in the active configuration"))?;
    if matches!(card, CardSettings::Picture { .. }) {
        return Err(format!(
            "card {card_id:?} is a picture card; its frame lives on the server"
        ));
    }
    build_card_scene(config, card_id, fields, None, 1).map(|push| push.scene)
}
```

- [ ] **Step 4: Re-export it**

In `companion/crates/app-core/src/lib.rs`, add `preview_card_scene` to the
existing `pub use runtime::{...}` list, in alphabetical position.

- [ ] **Step 5: Run the test and watch it pass**

```sh
cd companion && cargo test -p app-core --test runtime preview_card_scene
```

Expected: PASS.

- [ ] **Step 6: Switch the preview thread to scenes**

In `companion/apps/deskmate/src-tauri/src/preview.rs`, change the job type and
the render call. Two edits:

```rust
struct PreviewJob {
    request: lvgl_sim::scene::SceneRenderRequest,
    reply: mpsc::Sender<Result<Vec<u8>, String>>,
}
```

```rust
                let result = simulator
                    .render_scene_png(&job.request)
                    .map_err(|error| format!("render failed: {error:?}"));
```

and the public method:

```rust
    pub fn render(
        &self,
        request: lvgl_sim::scene::SceneRenderRequest,
    ) -> Result<Vec<u8>, String> {
```

Everything else in the file — the one-simulator-per-process rule, the dedicated
thread, latest-wins coalescing, the "superseded" reply — is unchanged and must
stay.

- [ ] **Step 7: Build the scene request in `render_card_preview`**

In `commands.rs`, replace the body between `let template = preview_template_for(...)`
and the `let request = lvgl_sim::RenderRequest {...}` block. The picture-card
early return at line 854 stays exactly as it is. The new body:

```rust
    let data = snapshot
        .card_data
        .iter()
        .find(|data| data.card_id == card_id);
    let (fields, sample) = match data {
        Some(data) if !data.fields.is_empty() => (data.fields.clone(), false),
        _ => (Vec::new(), true),
    };

    let scene = app_core::preview_card_scene(&snapshot.config, &card_id, &fields)
        .map_err(|message| IpcError::Internal { message })?;

    let now = chrono::Utc::now();
    let utc_offset_minutes = utc_offset_minutes(&snapshot.config.preferences.timezone, now)
        .map_err(|message| IpcError::Internal { message })?;

    let request = lvgl_sim::scene::SceneRenderRequest {
        scene,
        assets: Vec::new(),
        utc_offset_minutes,
        now_unix_seconds: now.timestamp(),
        timer: preview_timer(&fields),
        fields: Vec::new(),
        orientation: preview_orientation(snapshot.config.preferences.orientation),
    };
```

`fields` here are `app_core::CardField`, not `lvgl_sim::SimField` — the scene
carries its own values, so the old `sim_field` conversion is no longer needed
for text. What the scene cannot carry is the live timer, which is a binding
resolved at render time. Add:

```rust
/// The timer a pomodoro scene's `timer.*` bindings resolve against.
///
/// `None` for every card that pushes no timer fields, which is what the
/// simulator expects for a face with no timer binding.
fn preview_timer(fields: &[app_core::CardField]) -> Option<lvgl_sim::scene::SceneTimer> {
    let total = field_integer(fields, "duration_seconds")?;
    let remaining = field_integer(fields, "remaining_seconds").unwrap_or(total);
    Some(lvgl_sim::scene::SceneTimer {
        total_ms: u32::try_from(total.max(0)).unwrap_or(u32::MAX).saturating_mul(1_000),
        remaining_ms: u32::try_from(remaining.max(0)).unwrap_or(u32::MAX).saturating_mul(1_000),
        running: field_boolean(fields, "running"),
    })
}
```

with `field_integer`/`field_boolean` local helpers over `&[CardField]` matching
the shape already used in `runtime.rs:1896-1914`. Confirm `SceneTimer`'s exact
field names against `crates/lvgl-sim/src/scene.rs` before writing this — build
the struct from what is there, not from this snippet, if they differ.

- [ ] **Step 8: Delete the now-unreachable preview helpers**

From `commands.rs`, delete `preview_template_for` (line 926) and `sim_template`
(line 940). Delete `sim_field` (line 951) **only if** the compiler reports it
unused. Delete the `preview_template_for_resolves_every_built_in_template` test
(line 1564) — the function it names is gone.

Keep `preview_orientation` and its test
`the_preview_renders_upright_whichever_way_the_panel_is_mounted`: the
upright-at-both-mountings rule is owner direction and is orthogonal to which
renderer draws it.

- [ ] **Step 9: Run the app's tests**

```sh
cd companion && cargo test -p deskmate-app > /tmp/t2-test.log 2>&1; echo "EXIT=$?"
```

Expected: PASS.

- [ ] **Step 10: Look at it**

Build and run the dev harness, open a clock card and a pomodoro card, and
confirm each draws a preview:

```sh
cd companion/apps/deskmate && VITE_DESKMATE_MOCK=1 bun run dev
```

The harness runs in Chrome, which does not share WKWebView's focus behaviour —
it is adequate for "does a frame appear", not for click handling. Record what
you saw.

- [ ] **Step 11: Commit**

```sh
git add -A
git commit -m "refactor: draw card previews from the scene the device receives

The settings window drew its previews through the retired C templates while
the device drew scenes, so the two were different renderers kept in
agreement by a 1,581-line parity gate. The preview now builds the same scene
build_card_scene would push and renders that.

app_core::preview_card_scene is deliberately not a new renderer: it is
build_card_scene with revision pinned to 1, no image-source host, nothing
minted and nothing sent. A picture card is refused rather than drawn blank,
because its frames live on the server and this process has no host.

preview_orientation stays. Rendering upright at both mountings is owner
direction about what the preview means, not about which renderer draws it."
```

---

### Task 3: Retire the oracle, the parity gate, and the three orphan templates

**Depends on Task 2.** Nothing may call `lvgl_sim::RenderRequest` when this
starts.

**Files:**
- Modify: `companion/crates/lvgl-sim/tests/golden.rs`
- Modify: `companion/crates/lvgl-sim/tests/clock_chrome.rs`
- Modify: `companion/crates/lvgl-sim/tests/tabular.rs`
- Modify: `companion/crates/lvgl-sim/src/lib.rs`, `src/cases.rs`, `build.rs`
- Modify: `companion/crates/app-core/src/config.rs`, `src/scene_build.rs`
- Delete: `companion/crates/lvgl-sim/reference-oracle/` (whole tree)
- Delete: `companion/crates/app-core/tests/scene_parity.rs`
- Delete: `companion/crates/lvgl-sim/tests/face_scene_cases.rs`
- Delete: `docs/scene/template-parity-ledger.md`
- Re-bless: `companion/crates/lvgl-sim/tests/golden/progress-ring--running-mid-countdown--landscape.png`

**Interfaces:**
- Consumes: `app_core::preview_card_scene` from Task 2.
- Produces: `lvgl_sim` exposes scene rendering only. `RenderRequest`,
  `SimTemplate`, `Simulator::render`, `Simulator::render_png` and
  `cases::golden_cases` no longer exist. `DisplayTemplate` has three variants:
  `DigitalClock`, `AnalogClock`, `ProgressRing`.

- [ ] **Step 1: Repoint the golden harness at the scene path**

Replace the body of `companion/crates/lvgl-sim/tests/golden.rs`:

```rust
mod common;

use lvgl_sim::cases;

#[test]
fn golden_frames_match() {
    common::assert_goldens(
        "",
        std::env::var("BLESS").is_ok(),
        Some("golden mismatches"),
        "orphan golden with no matching case (run with BLESS=1 to delete)",
        cases::face_scene_cases(),
        |_, _| {},
        lvgl_sim::Simulator::render_scene_png,
    );
}
```

- [ ] **Step 2: Run it — this is the wave's load-bearing evidence**

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cd companion && cargo test -p lvgl-sim --test golden > /tmp/t3-repin.log 2>&1; echo "EXIT=$?"
cat /tmp/t3-repin.log
```

Expected: FAIL, with **exactly one** mismatch:

```
progress-ring--running-mid-countdown--landscape: pixels differ
```

This has been measured, not predicted. If any other case differs, **stop**: the
parity gate was not proving what it claimed and the deletion must not proceed
until that is understood.

- [ ] **Step 3: Re-bless the one expected difference**

```sh
cd companion && BLESS=1 cargo test -p lvgl-sim --test golden
git diff --stat crates/lvgl-sim/tests/golden/
```

Expected: exactly one file changed,
`progress-ring--running-mid-countdown--landscape.png`. The reason, which
`face_scene_cases.rs:27-31` already records: the C oracle advances from LVGL's
private fake-tick anchor after fields are pushed, while a scene consumes the
supplied snapshot. The scene value is the deterministic one.

- [ ] **Step 4: Move `clock_chrome` and `tabular` to the scene path**

Both measure a *relationship between two frames* that a golden cannot state —
that no clock face draws a title chip, and that tabular digits share an
advance — so both must survive. Convert each to build its two frames from
`cases::face_scene_cases()` entries (or from scenes built the same way the
cases module does) and render with `render_scene`. Keep every assertion and
every doc comment; change only how the pixels are produced.

Run them:

```sh
cd companion && cargo test -p lvgl-sim --test clock_chrome --test tabular
```

Expected: PASS, with the same assertions as before.

- [ ] **Step 5: Delete the parity gate and the case cross-check**

```sh
git rm companion/crates/app-core/tests/scene_parity.rs
git rm companion/crates/lvgl-sim/tests/face_scene_cases.rs
```

`face_scene_cases.rs` existed to prove the two case sets rendered identically.
With one case set, it is comparing a thing to itself.

- [ ] **Step 6: Delete the oracle and the template render path**

```sh
git rm -r companion/crates/lvgl-sim/reference-oracle
```

From `companion/crates/lvgl-sim/src/lib.rs` delete `SimTemplate`,
`RenderRequest`, `Simulator::render` and `Simulator::render_png`. From
`src/cases.rs` delete `golden_cases()` and the six `*_cases` functions that feed
it (`digital_clock_cases`, `analog_clock_cases`, `progress_ring_cases`,
`row_list_cases`, `big_number_label_cases`, `icon_badge_text_cases`) along with
the `text`/`boolean`/`integer`/`case` helpers if the compiler reports them
unused. Keep `face_scene_cases`, `scene_cases`, `asset_font_scene_cases`,
`timer_producer_scene_cases`, `date_truncation_scene_cases` and
`claude_limits_scene_cases`.

From `companion/crates/lvgl-sim/build.rs` delete the `reference-oracle`
compilation and the guard that panics when the templates reappear under
`firmware/main/ui/`. The guard existed to stop the oracle drifting back into the
firmware image; with no oracle there is nothing to drift.

- [ ] **Step 7: Delete the three orphan display templates**

`RowList`, `BigNumberLabel` and `IconBadgeText` are unreachable from any card
since v8 — `config.rs:1741` allows a clock only `DigitalClock | AnalogClock` and
`config.rs:1753` allows a pomodoro only `ProgressRing` — and survived only
because the oracle needed them.

In `companion/crates/app-core/src/config.rs`:
- remove the three variants from `pub enum DisplayTemplate` and from
  `strict_tagged_enum::DisplayTemplateInner`;
- remove their arms from `DisplayTemplateInner`'s `allowed_fields`, from the
  `Deserialize` mapping (lines ~963-969) and from the `wire_config` mapping
  (lines ~1402-1407);
- `IconBadgeText` carries `icon_asset_id`, so check for and remove any handling
  of it left stranded (`config.rs:705`, `config.rs:1378`).

In `companion/crates/app-core/src/scene_build.rs`, delete the three
corresponding scene builders and their input structs.

`protocol::TemplateKind` keeps all six variants — it is the **wire** contract,
which this wave does not move. Only the host-side `DisplayTemplate` shrinks.

- [ ] **Step 8: Delete the ledger**

```sh
git rm docs/scene/template-parity-ledger.md
```

It is the record of a gate that no longer exists. Task 7 records in CLAUDE.md
that it was deleted and why, so the history is not simply lost.

- [ ] **Step 9: Full gates**

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cd companion
cargo fmt --all --check; echo "FMT=$?"
cargo clippy --workspace --all-targets -- -D warnings > /tmp/t3-clippy.log 2>&1; echo "CLIPPY=$?"
cargo test --workspace --all-targets > /tmp/t3-test.log 2>&1; echo "TEST=$?"
cargo test --workspace --doc > /tmp/t3-doc.log 2>&1; echo "DOC=$?"
```

All four must be 0.

- [ ] **Step 10: Confirm the hardware matrix still builds**

`framebuffer_diff` drives `SceneRenderRequest` only and must be unaffected:

```sh
cd companion && cargo build -p device --example framebuffer_diff; echo "EXIT=$?"
```

Expected: builds. Do **not** run it — it needs the board.

- [ ] **Step 11: Commit**

```sh
git add -A
git commit -m "refactor: retire the template oracle and the gate that justified it

Two renderers drew the same six faces: the device drew scenes, and the Mac's
card preview drew the retired C templates. 1,581 lines of parity gate and
2,170 lines of oracle C existed to keep them agreeing. Task 2 moved the
preview onto scenes, so the gate now compares a thing to itself.

Repointing the goldens at face_scene_cases() reproduced 28 of 29 landscape
frames byte-identically. The one mismatch is
progress-ring--running-mid-countdown, whose divergence the deleted
face_scene_cases.rs already documented: the oracle advances from LVGL's
fake-tick anchor while a scene consumes the supplied snapshot. It is
re-blessed to the deterministic value.

Coverage is unchanged -- the goldens still pin every pixel of every face, and
clock_chrome and tabular still measure the two frame relationships a golden
cannot state. What is gone is the assertion that this matches code we
deleted.

RowList, BigNumberLabel and IconBadgeText go with it. No card has been able
to name them since v8; only the oracle kept them alive. protocol::TemplateKind
is untouched -- that is the wire, which this wave does not move."
```

---

### Task 4: Split `runtime.rs`

**Depends on Task 2** (which adds `preview_card_scene` to this file).

**Files:**
- Create: `companion/crates/app-core/src/runtime/mod.rs` (from `runtime.rs`)
- Create: `companion/crates/app-core/src/runtime/device.rs`
- Create: `companion/crates/app-core/src/runtime/scene.rs`
- Create: `companion/crates/app-core/src/runtime/timers.rs`
- Create: `companion/crates/app-core/src/runtime/sync.rs`
- Create: `companion/crates/app-core/src/runtime/snapshot.rs`
- Delete: `companion/crates/app-core/src/runtime.rs`

**Interfaces:**
- Consumes: everything already in `runtime.rs`.
- Produces: no public API change whatsoever. `app_core`'s `pub use runtime::{...}`
  list in `lib.rs` must be byte-identical before and after.

- [ ] **Step 1: Prove the public surface before you touch anything**

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cd companion && cargo doc -p app-core --no-deps 2>/dev/null
grep -n "pub use runtime" crates/app-core/src/lib.rs > /tmp/t4-before.txt
cat /tmp/t4-before.txt
```

Keep `/tmp/t4-before.txt`. Step 8 diffs against it.

- [ ] **Step 2: Create the module directory and move `device.rs`**

```sh
cd companion/crates/app-core/src
mkdir runtime
git mv runtime.rs runtime/mod.rs
```

Then move into `runtime/device.rs`: `DeviceConnection`, `RuntimeDevice`,
`ImageSourceFrame`, `ImageSourceHost`, `SerialRuntimeDevice` and its two impls,
plus the device-error cluster — `ownership_was_refused`, `is_wrong_tier`,
`sync_device_result`, `mark_ownership_refused`, `handle_device_error`,
`mark_disconnected`, `is_disconnect`, `device_runtime_error`,
`runtime_command_device_error`, `run_runtime_device_command`,
`reply_to_runtime_device_command`, `update_device_status`.

Add `mod device;` to `runtime/mod.rs` and `pub(crate) use device::*;` — then
narrow that glob to a named list once it compiles. Items the rest of the crate
needs keep `pub`; items only `runtime/` needs become `pub(super)`.

- [ ] **Step 3: Compile after each move, not at the end**

```sh
cd companion && cargo build -p app-core 2>&1 | tail -30
```

A move is only a move if it compiles. Do not batch two modules before building.

- [ ] **Step 4: Move `scene.rs`**

`field_text`, `field_integer`, `field_boolean`, `build_card_scene`,
`preview_card_scene`, `waiting_for_first_picture_scene`,
`build_template_card_scene`, `record_scene_refusal`,
`handle_automatic_scene_error`, `native_requirements`, `push_active_scene`,
`execute_native_push`, `built_picture_face`, `clear_scene_refusal`,
`ensure_durable_assets_for_scene`, `frame_face_scene`,
`handle_automatic_asset_error`. Build.

- [ ] **Step 5: Move `timers.rs`**

`update_pomodoros`, `control_pomodoro`, `record_pomodoro_update`, `card_alert`,
`card_wants_completion_interrupt`, `arm_alert_hold`, `arm_alert_hold_if_active`,
`sync_alert_hold_to_active_interrupt`, `flush_interrupts`, `pomodoro_state`.
Build.

- [ ] **Step 6: Move `sync.rs`**

`synchronize_full`, `record_asset_sync_refusals`, `clear_asset_sync_refusals`,
`synchronize_pending`, `push_dirty_widgets`, `send_screen`, `send_time_sync`.
Build.

- [ ] **Step 7: Move `snapshot.rs`**

`RuntimeDiagnosticCounters`, `SubscriptionSlot`, `SubscriptionState`,
`RuntimeSubscription`, `SnapshotPublisher`, `initial_snapshot`, `empty_device`.
Build.

What stays in `mod.rs`: `RuntimeOptions`, `RuntimeHandle`, `WorkerState`,
`RuntimeWorkerInputs`, `run_runtime`, `process_command`,
`command_drives_a_full_sync`, `apply_image_source_update`,
`activate_screen_command`, `attempt_connect`, `run_scheduled_work`,
`detect_active_picture_face_change`, `drain_device_events`, `rotation_card_ids`,
`current_dwell`, `advance_rotation`.

`WorkerState` stays in `mod.rs` and its fields become `pub(super)`. Do **not**
give each module its own state struct — that is a rewrite, and this task is a
move.

- [ ] **Step 8: Move the tests with their subjects**

`runtime.rs`'s `#[cfg(test)] mod tests` (from line 2732) splits the same way:
each test travels to the module holding what it tests. A test that spans two
modules stays in `mod.rs`.

- [ ] **Step 9: Prove the public surface did not move**

```sh
cd companion && grep -n "pub use runtime" crates/app-core/src/lib.rs > /tmp/t4-after.txt
diff /tmp/t4-before.txt /tmp/t4-after.txt && echo "PUBLIC SURFACE UNCHANGED"
```

Expected: no diff. If the list changed, the split leaked.

- [ ] **Step 10: Full gates, then commit**

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cd companion
cargo fmt --all --check; echo "FMT=$?"
cargo clippy --workspace --all-targets -- -D warnings > /tmp/t4-clippy.log 2>&1; echo "CLIPPY=$?"
cargo test --workspace --all-targets > /tmp/t4-test.log 2>&1; echo "TEST=$?"
```

```sh
git add -A
git commit -m "refactor: give runtime.rs the walls it never grew

One file owned device I/O, the command loop, scheduling, pomodoro, alerts,
interrupts, scene building, scene pushing, assets and snapshot publication.
The seams were already there as contiguous function groups; they are now
modules.

This is a move, not a redesign. The public surface is byte-identical --
lib.rs's pub use list is unchanged and diffed to prove it -- and WorkerState
stays one struct in mod.rs with pub(super) fields. Giving each concern its
own state is the better end state and is deliberately not done here: mixing
a move with a rewrite makes both unreviewable."
```

---

### Task 5: Split `commands.rs`

**Depends on Task 2** (which rewrites `render_card_preview`).

**Files:**
- Create: `companion/apps/deskmate/src-tauri/src/ipc.rs`
- Create: `companion/apps/deskmate/src-tauri/src/commands/mod.rs`
- Create: `companion/apps/deskmate/src-tauri/src/commands/config.rs`
- Create: `companion/apps/deskmate/src-tauri/src/commands/network.rs`
- Create: `companion/apps/deskmate/src-tauri/src/commands/preview.rs`
- Delete: `companion/apps/deskmate/src-tauri/src/commands.rs`
- Modify: `companion/apps/deskmate/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `app_core::preview_card_scene` (Task 2).
- Produces: no change to any `#[tauri::command]` name or signature. The
  `invoke_handler![...]` list in `lib.rs` must name exactly the same commands.
  The TypeScript side is not touched.

- [ ] **Step 1: Pin the command list before you start**

```sh
cd companion && grep -n "generate_handler\|invoke_handler" -A 30 apps/deskmate/src-tauri/src/lib.rs > /tmp/t5-before.txt
cat /tmp/t5-before.txt
```

- [ ] **Step 2: Extract `ipc.rs`**

Move `IpcError`, `DraftPayload`, `WidgetTarget`, `DraftValidation`,
`ConfigApplyResult`, `PreviewFrame`, `AutostartStatus`, `MintedImageSource`,
`ServerEndpointRequest`, `ProvisionDeviceRequest`, `ServerConfigRequest`, and
the input validators `validate_bounded`, `validate_secret`, `validate_target`.
Build.

- [ ] **Step 3: Extract `commands/network.rs`**

`set_server_endpoint` + `_with_context`, `provision_device` + `_with_context`,
`factory_reset_device`, `use_local_ownership` + `_with_context`,
`save_server_config`, `save_server_config_blocking`, `PreparedServerSave`,
`prepare_server_save`, `server_error_after_local_save`, `save_destination`,
`device_link_url`, `server_config_url`, `put_server_config`,
`validate_network_config_request`, `mint_image_source`,
`mint_server_image_source`, `MintImageSourceRequest`, `ServerMintedImageSource`,
`ServerQueryContext`, `ServerSaveContext`, `ProvisionContext`. Build.

- [ ] **Step 4: Extract `commands/config.rs`**

`get_app_snapshot`, `validate_config_draft`, `validate_draft_for_device`,
`save_apply_config`, `save_and_apply`, `ConfigSaveContext`, `persist_config`,
`persist_config_parts`, `parse_valid_draft`, `parse_draft`,
`merge_command_owned_preferences`, `local_save_refusal`,
`ensure_device_compatibility`, `missing_capability_issues`,
`describe_capabilities`. Build.

- [ ] **Step 5: Extract `commands/preview.rs`**

`render_card_preview`, `preview_orientation`, `preview_timer`,
`unrendered_server_frame`, and the field helpers Task 2 added. Build.

What stays in `commands/mod.rs`: `control_pomodoro`, `resume_pushing`,
`get_network_settings`, `get_autostart_status`, `set_autostart_enabled`,
`autostart_status`, and `pub use` of each submodule's commands so `lib.rs`'s
handler list keeps working unchanged.

- [ ] **Step 6: Move the tests with their subjects**

`commands.rs`'s test module (from line 1447) splits the same way. The ignored
test `server_request_runs_after_the_mutation_lock_is_released` travels with
`network.rs` and stays ignored.

- [ ] **Step 7: Prove the command list did not move**

```sh
cd companion && grep -n "generate_handler\|invoke_handler" -A 30 apps/deskmate/src-tauri/src/lib.rs > /tmp/t5-after.txt
diff /tmp/t5-before.txt /tmp/t5-after.txt && echo "COMMAND LIST UNCHANGED"
```

- [ ] **Step 8: Gates and commit**

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cd companion && cargo clippy --workspace --all-targets -- -D warnings > /tmp/t5-clippy.log 2>&1; echo "CLIPPY=$?"
cargo test --workspace --all-targets > /tmp/t5-test.log 2>&1; echo "TEST=$?"
cd apps/deskmate && bun test && bun run check
```

```sh
git add -A
git commit -m "refactor: split the Tauri command surface by what it acts on

One file held the IPC error type, eleven DTOs, every input validator and
every command: config, networking, provisioning, preview, autostart. They
are now four modules and an ipc.rs, grouped by the surface each acts on.

No command name or signature changes and the TypeScript side is untouched;
lib.rs's handler list is diffed before and after to prove it."
```

---

### Task 6: Delete the M2 CLI tooling

Independent of every other task.

**Files:**
- Delete: `companion/crates/deskmate-cli/src/m2.rs`
- Delete: `companion/crates/device/examples/m2_stress.rs`
- Delete: `companion/examples/m2-carousel.json`
- Modify: `companion/crates/deskmate-cli/src/main.rs:13,530,664,675`
- Modify: `README.md:63-69`

**Interfaces:**
- Consumes: nothing.
- Produces: `deskmate-cli` keeps `status`, `time-sync`, `push-data`,
  `provision`, `factory-reset`. Nothing else calls into it.

- [ ] **Step 1: Confirm the commands cannot work before deleting them**

The seven M2 commands (`inspect-config`, `apply-config`, `select-screen`,
`push-clock`, `pomodoro`, `trigger-interrupt`, `events`) push `ApplyConfig` and
`PushData` with templates. Since stage 3a the device draws only host-pushed
scenes, so none of them can put a face on the panel. Verify the claim rather
than trusting it:

```sh
cd companion && grep -c "PushScene" crates/deskmate-cli/src/m2.rs
```

Expected: `0`. If it is not 0, stop and re-read before deleting.

- [ ] **Step 2: Delete the files**

```sh
git rm companion/crates/deskmate-cli/src/m2.rs
git rm companion/crates/device/examples/m2_stress.rs
git rm companion/examples/m2-carousel.json
```

- [ ] **Step 3: Unwire the CLI**

In `companion/crates/deskmate-cli/src/main.rs` remove `mod m2;` (line 13), the
`if m2::run_if_requested()? { ... }` dispatch (line 530), and both
`m2::M2_USAGE` usage-string concatenations (lines 664 and 675) so `USAGE` prints
alone.

- [ ] **Step 4: Drop now-unused dependencies**

```sh
cd companion && cargo build -p deskmate-cli 2>&1 | tail -20
cargo machete 2>&1 | tail -20
```

Remove from `crates/deskmate-cli/Cargo.toml` anything only `m2.rs` used —
likely `providers`, `engine`, `chrono-tz`. Verify each removal by rebuilding,
not by reading.

- [ ] **Step 5: Update the README**

Replace the "Legacy CLI inspection" section (lines 63-69) with a short
paragraph naming only the surviving commands, and drop the link to the M2
walkthrough plan.

- [ ] **Step 6: Gates and commit**

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cd companion && cargo clippy --workspace --all-targets -- -D warnings > /tmp/t6-clippy.log 2>&1; echo "CLIPPY=$?"
cargo test --workspace --all-targets > /tmp/t6-test.log 2>&1; echo "TEST=$?"
```

```sh
git add -A
git commit -m "chore: delete the M2 demo tooling that can no longer draw

inspect-config, apply-config, select-screen, push-clock, pomodoro,
trigger-interrupt and events push ApplyConfig and PushData with templates.
Since stage 3a the device draws only host-pushed scenes, so none of them can
put a face on the panel -- m2.rs contains no PushScene at all. The README
already labelled them legacy.

deskmate-cli keeps status, time-sync, push-data, provision and
factory-reset."
```

---

### Task 7: Documentation

**Do last.** It records what Tasks 1-6 actually did, so it cannot be written
first.

**Files:**
- Modify: `CLAUDE.md`
- Create: `docs/history.md`
- Modify: every delivered plan under `docs/superpowers/plans/`
- Modify: `docs/superpowers/plans/2026-08-03-deskmate-roadmap.md`

- [ ] **Step 1: Measure the starting point**

```sh
wc -c CLAUDE.md
grep -c '^- \[ \]' docs/superpowers/plans/*.md | sort -t: -k2 -rn
```

Record both numbers in the commit message.

- [ ] **Step 2: Split CLAUDE.md**

CLAUDE.md is loaded into every session. Move to `docs/history.md`, which nobody
loads by default, every passage that is a **narrative of how something came to
be** rather than a rule or a live trap. Specifically: milestone-by-milestone
accounts (M0-M4, V1, V2 stage histories), bullets opening "STALE AS WRITTEN",
superseded design worlds, and per-commit forensics whose lesson is already
stated as a rule elsewhere.

Keep in CLAUDE.md:
- Sources of truth, working agreement, engineering constraints, verification.
- Current state in one section: schema v9, protocol v1, capabilities 2027, three
  card kinds, what is deployed where.
- **Live traps only** — the ones that still bite: the OTA memory-layout hazard,
  `AssetRelease.digests` being a keep-set, `AssetBegin` key 3 always emitted,
  the WKWebView focus rule, the serial-read blocking rule, a flashed build being
  reverted unless `DESKMATE_FIRMWARE_VERSION` moves, a tier round-trip costing a
  device identity.
- The owner's standing product rules: one loop, a card is named by its template,
  no reintroducing the subtracted surfaces, the preview is upright at both
  mountings.

Put a line at the top of `docs/history.md` saying it is narrative, not
normative, and link it once from CLAUDE.md.

- [ ] **Step 3: Record this wave's own outcomes in CLAUDE.md**

Add, under current state:
- The v8/v9 subtraction and what remains (three card kinds; the server makes no
  outbound HTTP; a server-side card cannot change between pushes).
- **The rule from Task 1:** a `docs/config/vN.md` exists iff the code can still
  read vN; the store reads v4 and up.
- **The rule from Task 3:** there is one renderer. The preview draws the scene
  the device would receive. `docs/scene/template-parity-ledger.md` and the
  oracle were deleted because a gate between two renderers has no job when
  there is one; do not reintroduce a second renderer to "check" the first.
- That Waves B and C remain, naming the spec.

- [ ] **Step 4: Stop the plans from lying**

Roughly 500 unticked `- [ ]` boxes across delivered plans describe shipped work.
Apply the rule CLAUDE.md already states: add a STATUS header at the top of each
delivered plan, in this exact form, and **do not back-fill ticks**:

```markdown
> **STATUS: DELIVERED.** This plan's checkboxes were never ticked during
> execution and are not a progress signal. The work shipped; see the commits
> and `docs/hardware/board-notes.md`. Do not plan against the open boxes below.
```

- [ ] **Step 5: Update the roadmap**

Set the roadmap's "Current" paragraph to: V1 and V2 exited and tagged, V3 server
host in progress on `feat/v3-server-host`, and the legacy subtraction's three
waves with Wave A complete.

- [ ] **Step 6: Verify no link rot**

```sh
grep -rn "template-parity-ledger\|config/v3\|manifest-v1\|manifest-v2" \
  --include='*.md' --include='*.rs' . | grep -v '^./.git' | grep -v docs/history.md
```

Expected: no hits outside `docs/history.md`, which may reference them as history.

- [ ] **Step 7: Commit**

```sh
git add -A
git commit -m "docs: separate the rules from the archaeology

CLAUDE.md is loaded into every session and had grown to 113 KB, much of it a
narrative of how things came to be -- including bullets that open 'STALE AS
WRITTEN'. The narrative moves to docs/history.md, which nothing loads by
default. What stays is rules, current state, and traps that still bite.

Delivered plans get a STATUS header instead of back-filled ticks, because
roughly 500 open boxes across them describe work that shipped and a tick
nobody verified is worse than an honest header.

Records this wave's two new rules: a docs/config/vN.md exists iff the code
can still read vN, and there is one renderer -- do not add a second one to
check the first."
```

---

## Wave exit

- [ ] Full gate set, every command separately, all exit 0.
- [ ] `git diff --stat main` shows **no change under `firmware/main/`**.
- [ ] `grep -rn "RenderRequest\b" companion/crates companion/apps` returns
      nothing outside `SceneRenderRequest`.
- [ ] `CURRENT_SCHEMA_VERSION` is 9 and `PROTOCOL_CURRENT_CAPABILITIES` is 2027.
- [ ] The line count is recorded: `find companion -name '*.rs' -not -path
      '*/target/*' | xargs wc -l | tail -1` against the 49,846 this wave starts
      from.

## Self-review notes

Spec coverage: A1 → Task 1. A2 → Tasks 2 and 3. A3 → Tasks 4 and 5 (the
`config.rs` shadow-enum item is dropped, with the reason recorded under
"Non-goals"; the spec is amended to match). A4 → Tasks 6 and 7.

Type consistency: `preview_card_scene` is defined in Task 2 Step 3 and consumed
in Task 2 Step 7 and Task 5 Step 5 with the same signature. `SceneRenderRequest`
and `SceneTimer` come from `lvgl_sim::scene` throughout, and Task 2 Step 7
instructs the implementer to build `SceneTimer` from the real definition rather
than from the snippet.
