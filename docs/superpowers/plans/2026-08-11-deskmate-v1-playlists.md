# V1 Playlist Authoring Model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **STATUS (recorded 2026-09-09): DELIVERED, and its UI has since been REPLACED. Zero of
> its 31 boxes were ticked; they are NOT a progress signal.** Schema v4's card library
> and named playlists shipped. The 2026-09-06 one-loop change then removed the library
> and named playlists *from the UI* on owner direction: the schema still carries
> `playlists[]` and `active_playlist_id` and older multi-playlist files round-trip
> unchanged, but the app exposes exactly one playlist and never creates another. Do not
> read this plan as a description of the current window.

**Goal:** Replace the per-card `presence` tri-state and global carousel with a card
library plus named playlists (one active), schema v4, without moving the wire
protocol.

**Architecture:** `AppConfig` gains `playlists: Vec<Playlist>` and
`active_playlist_id`; `CardPresence` and `CarouselSettings` are deleted.
Compilation lowers only the active playlist to screens (alert-capable cards
outside it stay screenless widgets — v3's existing path). Rotation and dwell
derive from the active playlist's entries. Migration v3→v4 synthesizes one
playlist; v0/v1/v2 migrate directly. The validation-fallback defect (defaults
mislabeled "your last working settings") is fixed in the same code path.

**Tech Stack:** Rust (serde with the repo's hand-written `strict_tagged_enum`
validating-Deserialize pattern), Tauri v2 typed IPC with the generated
`types.contract.ts` fixture, React frontend with Bun tests.

**Spec:** `docs/superpowers/specs/2026-08-11-deskmate-v1-reset-design.md` §4.

**Independence:** No dependency on the preview/typeface/redesign plan. The wire
contract does not move: `ApplyConfig` fixtures must re-encode byte-identically.

## Global Constraints

- Wire cap: at most 8 compiled widgets; library stays 1..=8 cards so the compiled
  set (active-playlist entries + alert-capable cards outside it) satisfies the cap
  by construction (spec §4.2).
- Playlists 1..=8, unique ids and names; names 1..48 chars; entries 1..=8, each
  referencing an existing card, no card twice in one playlist;
  `active_playlist_id` must resolve. Existing numeric bounds
  (`dwell_seconds`, `default_dwell_seconds`, `lead_minutes`, `hold`) unchanged.
- All new tagged types use hand-written validating `Deserialize`
  (`deny_unknown_fields` is a no-op on internally tagged enums).
- Validation issues keyed by stable id — `cards[<index>]` stays;
  playlist paths use `playlists[<index>]` / `playlists[<index>].entries[<j>]`
  with the card/playlist id in the message text (spec §4.5 keeps the Task 2B
  reordering fix).
- `alert` stays on the card; alerts fire regardless of the active playlist.
- Verification: from `companion/`: `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`; from `companion/apps/deskmate`: `bun test`,
  `bun run check`, `bun run lint`, `bun run format:check`, `bun run build`;
  `make -C firmware/host_tests clean test` (must pass with zero firmware file
  changes — that is the wire-didn't-move proof).
- Conventional commit prefixes; no tags.

---

### Task 1: Schema v4 types and validation

**Files:**
- Modify: `companion/crates/app-core/src/config.rs`
- Test: `companion/crates/app-core/tests/config.rs`

**Interfaces:**
- Produces (consumed by every later task):

```rust
pub const CURRENT_SCHEMA_VERSION: u32 = 4;

pub struct Playlist {
    pub id: String,
    pub name: String,
    pub advance: CarouselAdvance,      // existing enum, unchanged
    pub entries: Vec<PlaylistEntry>,
}
pub struct PlaylistEntry {
    pub card_id: String,
    pub dwell_seconds: Option<u16>,
}
pub struct AppConfig {
    pub schema_version: u32,
    pub preferences: AppPreferences,
    pub cards: Vec<CardSettings>,      // presence field removed from every variant
    pub playlists: Vec<Playlist>,
    pub active_playlist_id: String,
    // assets field (if present in current struct) carried unchanged
}
impl AppConfig {
    pub fn active_playlist(&self) -> Option<&Playlist>;
    /// Card ids the compiled widget set will contain: active-playlist entries
    /// (in entry order) followed by alert-capable cards outside it (by id).
    pub fn compiled_card_ids(&self) -> Vec<&str>;
}
pub const MAX_PLAYLISTS: usize = 8;
pub const MAX_PLAYLIST_ENTRIES: usize = 8;
pub const MAX_PLAYLIST_NAME_LEN: usize = 48;
```

Deleted: `CardPresence`, `CarouselSettings`, their `strict_tagged_enum` inners,
`CardPresence`-carrying field in all six `CardSettings` variants,
`AppConfig.carousel`.

- [ ] **Step 1: Write the failing tests**

In `companion/crates/app-core/tests/config.rs` add (representative set — write
all of them):

```rust
fn v4_config_with(playlists: Vec<Playlist>, active: &str) -> AppConfig {
    // helper: two clock cards "clock-a"/"clock-b", given playlists, active id
}

#[test]
fn default_config_is_v4_with_one_playlist() {
    let config = AppConfig::default();
    assert_eq!(config.schema_version, 4);
    assert_eq!(config.playlists.len(), 1);
    assert_eq!(config.active_playlist_id, config.playlists[0].id);
    assert_eq!(config.playlists[0].entries.len(), 1);
    assert!(config.validate().is_ok());
}

#[test]
fn active_playlist_id_must_resolve() { /* active: "nope" → ValidationCode + path "active_playlist_id" */ }

#[test]
fn playlist_entry_must_reference_existing_card() { /* entry card_id "ghost" → issue at playlists[0].entries[0].card_id */ }

#[test]
fn card_twice_in_one_playlist_rejected() { /* same card_id twice → DuplicateId at playlists[0].entries[1].card_id */ }

#[test]
fn same_card_in_two_playlists_is_allowed() { /* validates Ok */ }

#[test]
fn playlist_name_bounds_enforced() { /* empty name and 49-char name → issues at playlists[0].name */ }

#[test]
fn duplicate_playlist_names_rejected() { /* two playlists named "Work" → DuplicateId at playlists[1].name */ }

#[test]
fn empty_active_playlist_rejected_when_cards_exist() {
    /* active playlist with zero entries → OutOfRange at playlists[<active idx>].entries;
       replaces v3's "at least one card must be in the rotation" rule */
}

#[test]
fn per_playlist_timed_advance_bounds() { /* Timed { default_dwell_seconds: 1 } → OutOfRange at playlists[0].advance.default_dwell_seconds */ }

#[test]
fn unknown_field_in_playlist_rejected() {
    let json = r#"{"id":"p1","name":"Work","advance":{"kind":"manual"},"entries":[],"extra":1}"#;
    assert!(serde_json::from_str::<Playlist>(json).is_err());
}

#[test]
fn compiled_card_ids_are_entries_then_alert_outsiders() {
    /* playlist [b, a]; card c alert-capable outside; card d plain outside
       → ["b", "a", "c"] */
}
```

Also update every existing test that constructs `CardSettings` with `presence`
or reads `config.carousel` — they must construct playlists instead. (This is the
bulk of the diff; the compiler is the checklist.)

- [ ] **Step 2: Run to verify failure**

Run: `cd companion && cargo test -p app-core --test config`
Expected: FAIL to compile — `Playlist` not defined.

- [ ] **Step 3: Implement**

In `config.rs`:

1. Bump `CURRENT_SCHEMA_VERSION` to 4.
2. Add `Playlist`/`PlaylistEntry` with hand-written `Deserialize` following the
   existing `strict_tagged_enum` module's field-loop pattern (reject unknown
   fields explicitly; `advance` deserializes through the existing
   `CarouselAdvanceInner`). `PlaylistEntry` likewise
   (`card_id`, optional `dwell_seconds`).
3. Delete `CardPresence`, its inner, `presence` from all six `CardSettings`
   variants and their inners, `CarouselSettings`, and `AppConfig.carousel`; add
   `playlists` + `active_playlist_id`.
4. `Default for AppConfig`: the existing default clock card, plus
   `Playlist { id: "my-playlist", name: "My playlist", advance: CarouselAdvance::Manual, entries: [PlaylistEntry { card_id: <clock id>, dwell_seconds: None }] }`,
   active id `"my-playlist"`.
5. `validate()`: keep all card rules except the presence/rotation rules; drop
   `validate_card_behaviour`'s presence arm (alert-kind-matches-card-kind rule
   stays); add the playlist rules from Step 1's tests, iterating with
   `let path = format!("playlists[{index}]")` in the existing issue style.
6. `compiled_card_ids()` exactly per the Interfaces doc comment; `validate()`
   asserts `compiled_card_ids().len() <= MAX_CONFIG_CARDS` (defensive — with an
   8-card library it cannot trip, per Global Constraints).
7. `compile()`: replace `presence().is_in_rotation()` iteration with: for each
   active-playlist entry in order, emit screen + widget (screen id from card id
   as today); then each alert-capable card not in the active playlist as a
   screenless widget. Delete the `carousel` reference; dwell is not part of the
   wire (host-driven rotation), so `compile()` needs no dwell handling — dwell
   consumers are Task 4's runtime.

- [ ] **Step 4: Run and fix the workspace fallout**

Run: `cargo test -p app-core`
Expected: config tests pass; `store.rs`/`runtime.rs`/`scheduler.rs`/`commands.rs`
still fail to compile — that is Tasks 2–5's work. Where a later-task file blocks
this task's test run, make the *minimal* mechanical fix (e.g. replace a
`presence()` call with `todo!()`-free equivalent using `compiled_card_ids`) and
leave a `// TODO(plan task N)` marker only if the real change belongs to that
task; prefer landing Tasks 1–5 as one PR-shaped sequence of commits.

- [ ] **Step 5: Commit**

```bash
git add companion/crates/app-core
git commit -m "feat: schema v4 — playlists replace presence and the global carousel"
```

---

### Task 2: Migration v3→v4 (and v0/v1/v2 direct)

**Files:**
- Modify: `companion/crates/app-core/src/store.rs`
- Test: `companion/crates/app-core/tests/store.rs`
- Modify: fixtures directory used by store tests (rename `future-v4.json` →
  `future-v5.json`; add `v3-roundtrip.json`)

**Interfaces:**
- Consumes: Task 1's types.
- Produces: `ConfigOrigin::MigratedV3` variant; `load()` returning v4 configs
  from v0–v3 documents.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn v3_migrates_to_one_playlist_preserving_rotation_order_and_dwell() {
    // v3 JSON: cards [a(InRotation dwell 20), b(AlertOnly, alert set),
    // c(Off, alert set), d(InRotation dwell None)], carousel Timed{30}.
    // Expect: playlists == [ { id: "my-playlist", name: "My playlist",
    //   advance: Timed{30}, entries: [ {a, Some(20)}, {d, None} ] } ];
    // active == "my-playlist";
    // card b keeps its alert; card c's alert is None (spec §4.4 — the single
    // lossy point: an Off card's alert never fired and must not start firing);
    // cards keep ids and all other settings; origin == MigratedV3.
}

#[test]
fn v3_all_alert_only_migrates_to_valid_config() {
    // v3 with zero InRotation cards was validation-illegal but protocol-legal;
    // migration must still produce a *valid* v4 doc: entries empty is invalid,
    // so the synthesized playlist takes the first card by id as its sole entry
    // and that card keeps its alert. Assert validate().is_ok().
}

#[test]
fn v0_v1_v2_migrate_directly_to_v4() {
    // Reuse the existing legacy fixtures; assert schema_version 4, one
    // playlist, entries follow the legacy screens[] order (the existing
    // migrate_legacy/migrate_v2 card order), origins MigratedV0/V1/V2.
}

#[test]
fn future_v5_is_a_recoverable_error_preserving_bytes() {
    // The renamed future-v5.json fixture: load reports unsupported future
    // version, does not overwrite last-good (existing future-version test
    // updated, same assertions).
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app-core --test store`
Expected: FAIL — no v3 arm / fixtures missing.

- [ ] **Step 3: Implement**

In `store.rs`:

1. Add legacy mirror structs for v3 following the existing `LegacyConfigV2`
   pattern: `LegacyConfigV3 { schema_version, preferences, cards:
   Vec<LegacyCardV3>, carousel: LegacyCarouselV3 }` where `LegacyCardV3` mirrors
   the v3 `CardSettings` (six kinds with `presence` as a plain deserializable
   enum — the strict inners moved/deleted in Task 1, so the mirror owns its own
   permissive-but-bounded deserialization, as the v2 mirrors already do).
2. `fn migrate_v3(legacy: LegacyConfigV3) -> AppConfig`: strip `presence` from
   each card (clearing `alert` when presence was `Off`); build the synthesized
   playlist per the tests; fall back to first-card-by-id entry when no
   `InRotation` card exists.
3. The v0/v1/v2 paths already produce cards + an implied rotation order —
   update `migrate_legacy`/`migrate_v2` to emit the v4 shape through the same
   playlist-synthesis helper (rotation order = their existing card order,
   advance from their carousel/default).
4. Version dispatch in `load()`: arm `4` current; arm `3` → `migrate_v3`,
   origin `MigratedV3`; arms 2/1/0 unchanged targets; future versions keep the
   recoverable-error path.
5. Rename the fixture file; update its test's expected version number.

- [ ] **Step 4: Run**

Run: `cargo test -p app-core`
Expected: config + store tests pass.

- [ ] **Step 5: Commit**

```bash
git add companion/crates/app-core
git commit -m "feat: lossless-behaviour migration v0-v3 to schema v4 playlists"
```

---

### Task 3: Fix the validation-fallback defect

**Files:**
- Modify: `companion/crates/app-core/src/store.rs` and/or `runtime.rs` (wherever
  the fallback-to-defaults branch lives — locate via
  `grep -rn "last working" companion/crates companion/apps/deskmate/src`)
- Test: `companion/crates/app-core/tests/store.rs` or `tests/runtime.rs`
  (co-locate with the code that owns the branch)

**Interfaces:**
- Produces: on a persisted document that parses but fails validation, `load()`
  yields the previous **last-good** config plus a typed
  `ConfigOrigin`/error naming the failure — never `AppConfig::default()`
  presented as the user's settings (spec §4.5).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn invalid_persisted_config_keeps_last_good_and_reports_typed_error() {
    // Arrange: save a valid config A; then write a v4 document that parses
    // but fails validation (active_playlist_id: "ghost") to the store path.
    // Act: load().
    // Assert: the returned config == A (the last-good), NOT default();
    // the outcome carries a typed validation error whose issues name
    // "active_playlist_id"; nothing is queued for device replay from the
    // invalid document; and no string anywhere labels defaults as
    // "your last working settings" (grep the user-facing message constant).
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app-core`
Expected: FAIL — current behaviour substitutes defaults.

- [ ] **Step 3: Implement**

Route the parse-ok-validate-fail case through the same last-good preservation
path the parse-fail case already uses (the store keeps last-good bytes per the
M3 contract). Surface a distinct `LoadOutcome` variant carrying the
`ValidationIssue` list. Update the frontend-facing message in the snapshot
(Task 5 carries it into `AppSnapshot`): the copy must name the real situation —
"Your saved settings failed validation and were not applied" — and never claim
defaults are the user's settings. Only when **no last-good document exists at
all** may defaults load, labeled as defaults.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p app-core`

```bash
git add companion/crates/app-core
git commit -m "fix: keep last-good config on validation failure, never mislabeled defaults"
```

---

### Task 4: Runtime rotation from the active playlist

**Files:**
- Modify: `companion/crates/app-core/src/runtime.rs`
- Modify: `companion/crates/app-core/src/scheduler.rs` (only if its rotation API
  shape must change; prefer not)
- Test: `companion/crates/app-core/tests/runtime.rs`

**Interfaces:**
- Consumes: `AppConfig::active_playlist()`, `compiled_card_ids()` (Task 1).
- Produces: rotation behaviour — `rotation_card_ids(config)` returns the active
  playlist's entry card ids in entry order; dwell for a card =
  `entry.dwell_seconds.unwrap_or(default_dwell_seconds)` under
  `CarouselAdvance::Timed`, no rotation deadline under `Manual`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn rotation_follows_active_playlist_entry_order_and_dwell() {
    // Playlist [b (dwell 5), a (dwell None)] with Timed{10}: advancing past
    // b's 5s lands on a; past a's 10s (inherited default) wraps to b.
    // (Mirror the existing rotation-deadline test structure at
    // scheduler.rs:282 and the runtime rotation tests.)
}

#[test]
fn switching_active_playlist_is_a_config_apply_that_replays() {
    // Apply config with active=p1; then apply the same config with active=p2:
    // active screen becomes p2's first entry, a full replay is queued (the
    // ordinary apply transaction), rotation deadline rearms from p2's dwell.
}

#[test]
fn alert_outside_active_playlist_still_fires() {
    // Card c alert-capable, not in the active playlist: its interrupt trigger
    // still arms and fires; dismissal restores the active playlist's screen.
}

#[test]
fn manual_advance_playlist_has_no_rotation_deadline() { /* Manual: scheduler never reports rotation_due */ }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app-core --test runtime`
Expected: FAIL — `rotation_card_ids` still reads deleted `presence()` /
doesn't compile.

- [ ] **Step 3: Implement**

In `runtime.rs`:

1. `rotation_card_ids` (runtime.rs:1149) → active playlist entry ids in order.
2. The dwell lookup used by `rearm_rotation_for_active_screen`
   (runtime.rs:1016) and `advance_rotation` (runtime.rs:1179) → entry dwell
   with playlist-default fallback; `Manual` arms nothing.
3. The active-screen restoration after config replace (runtime.rs:947-963):
   "still valid" now means the previous active card id is an entry of the new
   active playlist; otherwise fall to the active playlist's first entry.
4. Alert arming paths: unchanged — they iterate cards with alerts, which no
   longer consults presence at all (the `alert-only` concept is now simply
   "has alert, not in active playlist", and arming never depended on the
   screen list).

- [ ] **Step 4: Run the full crate**

Run: `cargo test -p app-core`
Expected: all pass, including the pre-existing rotation stress tests updated to
build playlists.

- [ ] **Step 5: Commit**

```bash
git add companion/crates/app-core
git commit -m "feat: rotation and dwell derive from the active playlist"
```

---

### Task 5: Snapshot, IPC, and the generated TS contract

**Files:**
- Modify: `companion/crates/app-core/src/state.rs` (AppConfig inside snapshot
  already carries playlists; add the Task 3 typed load-error surface if it
  lives here)
- Modify: `companion/crates/protocol/examples/generate-fixtures.rs` (or wherever
  the TS fixture printer lives — the `#[ignore]`d printer test)
- Modify: `companion/apps/deskmate/src/lib/types.contract.ts` (regenerated)
- Modify: `companion/apps/deskmate/src/lib/types.ts`
- Modify: `companion/apps/deskmate/src-tauri/src/commands.rs` (validate/save
  paths compile against v4 — no new commands: switching playlists is an
  ordinary `save_apply_config`, spec §4.3)
- Test: the existing byte-for-byte contract sync test

- [ ] **Step 1: Regenerate the contract**

Run the fixture printer (the `#[ignore]`d test invoked directly, per Task 1's
M4 evidence: `cargo test -p <printer crate> -- --ignored <printer test name>`),
overwriting `types.contract.ts`. Update `types.ts` hand-written mirrors:
`Playlist`, `PlaylistEntry`, `AppConfig` (drop `carousel`, `CardPresence`;
add `playlists`, `active_playlist_id`; drop `presence` from `CardSettings`).

- [ ] **Step 2: Run the sync test to verify it passes byte-for-byte**

Run: `cd companion && cargo test --workspace` and
`cd companion/apps/deskmate && bun run check`
Expected: contract sync test passes; TypeScript fails only in components —
Task 6's work.

- [ ] **Step 3: Commit**

```bash
git add companion
git commit -m "feat: v4 playlists in AppSnapshot and the generated TS contract"
```

---

### Task 6: Frontend — library + playlists UX

**Files:**
- Modify: `companion/apps/deskmate/src/lib/configDraft.ts`
- Modify: `companion/apps/deskmate/src/components/CardList.tsx` (becomes
  Library + Playlists)
- Create: `companion/apps/deskmate/src/components/PlaylistPanel.tsx`
- Modify: `companion/apps/deskmate/src/components/CardEditor.tsx` (presence
  controls removed; alert controls stay)
- Modify: `companion/apps/deskmate/src/components/Filmstrip.tsx` (binds to the
  active playlist)
- Modify: `companion/apps/deskmate/src/App.tsx`, `src/styles.css`
- Test: `companion/apps/deskmate/tests/configDraft.test.ts`,
  `tests/components.test.tsx`, `tests/useAppState.test.ts`

**Interfaces:**
- Consumes: Task 5's types.
- Produces in `configDraft.ts` (replacing `rotationCards`, `nonRotationCards`,
  `moveCardWithinRotation`, `withAlert`'s presence coupling):

```ts
export function activePlaylist(config: AppConfig): Playlist | null;
export function playlistEntries(config: AppConfig, playlistId: string): CardSettings[];
export function libraryCards(config: AppConfig): CardSettings[]; // all cards
export function cardsOutsidePlaylist(config: AppConfig, playlistId: string): CardSettings[];
export function addPlaylist(config: AppConfig, name: string): AppConfig;   // id from slugified unique name
export function renamePlaylist(config: AppConfig, playlistId: string, name: string): AppConfig;
export function removePlaylist(config: AppConfig, playlistId: string): AppConfig; // refuses the last one
export function setActivePlaylist(config: AppConfig, playlistId: string): AppConfig;
export function addEntry(config: AppConfig, playlistId: string, cardId: string): AppConfig;
export function removeEntry(config: AppConfig, playlistId: string, index: number): AppConfig;
export function moveEntry(config: AppConfig, playlistId: string, from: number, to: number): AppConfig;
export function setEntryDwell(config: AppConfig, playlistId: string, index: number, dwell: number | null): AppConfig;
export function setPlaylistAdvance(config: AppConfig, playlistId: string, advance: CarouselAdvance): AppConfig;
export function loopSeconds(config: AppConfig, playlistId: string): number | null; // per-playlist now
export function removeCard(config: AppConfig, cardId: string): AppConfig; // also strips its entries from every playlist
```

- [ ] **Step 1: Write failing `configDraft` tests**

For each helper: pure-function tests in the existing `configDraft.test.ts`
style. The behavioural pins: `removeCard` strips the card's entries from all
playlists; `removePlaylist` on the active playlist activates the first
remaining and refuses when only one exists; `addEntry` refuses a duplicate
card in the same playlist and an entry when the playlist is at
`MAX_PLAYLIST_ENTRIES`; `moveEntry` is index-safe at the boundaries;
`loopSeconds` sums entry dwell with the playlist default under `timed` and is
null under `manual`.

- [ ] **Step 2: Run to verify failure, then implement the helpers**

Run: `bun test tests/configDraft.test.ts` — FAIL. Implement in
`configDraft.ts`, deleting the presence-era helpers listed above and updating
`filmstripSegments`/`nextFilmstripCardId`/`filmstripAdvance` to take the
active playlist's entries (their timing logic is otherwise unchanged). Run —
PASS.

- [ ] **Step 3: Rebuild the components**

Layout (spec §4.6, the brainstorm's approved sketch):

- **Left column — Library**: every card, one row each, with kind icon and
  name; a card shows a subtle badge when it has an alert ("alerts") and when
  it appears in no playlist ("unused"). Add-card flow unchanged (all six
  kinds). Selecting a card opens `CardEditor` (unchanged fields minus the
  presence radio group; the alert section stays).
- **Right column — Playlists**: playlist tabs/list with the active one marked
  (`● Work hours ◀ active`); switch-active is a draft edit to
  `active_playlist_id` saved through the normal apply flow; add/rename/delete
  playlist controls; the selected playlist's ordered entries with per-entry
  dwell inputs (blank = inherit default), drag/keyboard reorder reusing
  `CardList`'s existing reorder interaction and `cardMoveFromKey`; an
  "advance" control (manual / timed + default dwell) per playlist; an "Add
  from library" affordance listing `cardsOutsidePlaylist`.
- **Filmstrip**: binds to the active playlist's entries; unchanged play
  behaviour.
- First-run guidance: `firstRunSteps` updates to "add a card → add it to a
  playlist → save"; it must clear without any specific card kind (preserve the
  Task 2B fix).
- Validation display: `issuesForPath` gains the `playlists[i]` /
  `playlists[i].entries[j]` paths mapped onto the playlist panel rows;
  card-keyed issues stay on library rows (Global Constraints).
- The Task 3 load-error surface renders as a banner: "Your saved settings
  failed validation and were not applied", listing issue messages — never
  claiming defaults are the user's settings.

- [ ] **Step 4: Update component tests**

`components.test.tsx`: library renders all cards with unused/alert badges;
switching active playlist marks the draft dirty and saves through the mocked
IPC; entry reorder updates order; duplicate add is disabled; the
validation-failure banner shows the §4.5 copy. Delete presence-radio tests.
`useAppState.test.ts`: snapshot fixtures gain `playlists`.

- [ ] **Step 5: Full frontend suite**

Run: `bun test && bun run check && bun run lint && bun run format:check && bun run build`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add companion/apps/deskmate
git commit -m "feat: library + playlists settings UX replaces the presence tri-state"
```

---

### Task 7: Wire-freeze proof and close-out

**Files:**
- Test: existing `ApplyConfig` fixtures in `companion/crates/protocol/tests/`
  and app-core compile tests
- Modify: `docs/config/v3.md` → create `docs/config/v4.md`
- Modify: `CLAUDE.md` (current-state paragraph)

- [ ] **Step 1: Prove the wire did not move**

Run: `cargo test --workspace` — the `ApplyConfig` fixtures (including the
maximum-capacity fixture) must re-encode byte-for-byte identically; and
`make -C firmware/host_tests clean test` with `git status firmware/` showing
**zero modified firmware files** — the same evidence form Task 2B used.
Add one explicit test if not already implied: a v4 config whose active
playlist + alert outsiders equal a known v3 fixture's rotation compiles to the
identical `ApplyConfig` bytes.

- [ ] **Step 2: Documentation**

Write `docs/config/v4.md` from `v3.md`: the `playlists` section, `presence`
removal, migration rules (including the Off-card alert-clearing lossy point,
verbatim from spec §4.4), and the validation-failure behaviour from §4.5.
Update `CLAUDE.md`'s current-state paragraph: schema v4 playlists delivered,
the validation-mislabeling defect fixed, the unpowered-alert defect still open.

- [ ] **Step 3: Full verification and commit**

Every suite in Global Constraints from a clean state.

```bash
git add companion docs CLAUDE.md
git commit -m "docs: config v4 contract; wire-freeze evidence for playlists"
```
