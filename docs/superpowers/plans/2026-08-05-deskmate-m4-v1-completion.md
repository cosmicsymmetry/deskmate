# Deskmate M4 - v1 Completion Implementation Plan

**Status:** Active at Task 3. Tasks 1-2 froze the compatibility boundary and delivered
the bounded v1 provider runtime. Task 2B replaced the widget/screen authoring model
with the card model (schema v3): `cards[]`, `presence`, `alert`, host-driven timed
rotation, and bounded alert triggers, compiling to the unchanged wire contract. No
physical verification has been performed for Task 2B; it is outstanding.
M3's physical exit is complete; the user explicitly
accepted the completed morning soak and waived repeating it after the focused
orientation/clean-canvas regression passed.

**Goal:** Complete the approved v1 breadth on top of the proven single-owner companion
runtime: weather/JSON-feed/RSS data, the remaining templates, the card model
(`presence`, `alert`, host-driven timed `carousel.advance` — dashboard layouts were
cancelled in favor of this, see Task 4 below), safe host tap actions, landscape
mounting and asset management, wider calendar recurrence, an in-app firmware update
path, production USB identity, and release-grade macOS/Windows verification.

**Exit demo:** Install a release build, configure a set of cards (local and network
providers, in rotation and alert-only) with timed or manual advance, customize their
visuals, choose either landscape mounting, and invoke an explicitly approved host action
from touch. Disconnect/reconnect and power-cycle without losing authoritative state.
Perform a verified firmware update in the app and recover safely from an interrupted
update. Complete the release smoke checklist on macOS and Windows.

**Prerequisites carried from M3:**

- Preserve one Rust runtime as the only owner of serial I/O, revisions, provider work,
  timers, interrupts, actions, assets, update state, and replay.
- Keep the webview on narrow typed IPC; do not grant raw filesystem, HTTP, shell,
  process, serial, or updater primitives to frontend code.
- Preserve last-good config/provider data, atomic versioned persistence, bounded queues,
  deterministic compilation, coalesced snapshots, and reconnect-safe full replay.
- Close M3's physical app-to-board, sleep/wake, reset-replay, and 30-minute soak gates
  before accepting new protocol or firmware breadth.
- Carry the Windows MSVC build into native Windows CI: a macOS cross-check currently
  stops in `ring` because the Windows SDK/MSVC C headers are unavailable.
- Review the 17 allowed RustSec advisories from M3. GTK3, proc-macro-error, and `unic-*`
  are unmaintained; glib 0.18.5 has an unsoundness advisory. Upgrade or remove affected
  dependency paths where feasible and document any release exception.

**Out of scope:** Arbitrary plugins or scripts, arbitrary shell command execution,
cloud accounts/sync, mobile clients, remote device control, and unbounded user-supplied
HTML. Add no general extension system for v1.

---

### Task 1: Freeze the v1 contract and migration boundary

**Likely files:**

- Modify: `companion/crates/app-core/src/config.rs`
- Modify: `companion/crates/app-core/src/state.rs`
- Modify: `companion/crates/protocol/`
- Modify: `firmware/main/core/`
- Modify: `docs/protocol/v1.md` or create a versioned successor when incompatibility is proven
- Create: config/protocol migration and compatibility fixtures

- [x] Enumerate provider, template, layout, tap-action, asset, rotation, and updater
  settings as closed tagged types. Define stable IDs and limits for every string, list,
  byte payload, image, dashboard cell, redirect, and update artifact.
- [x] Add a lossless migration from every released M3 config. Preserve unknown future
  versions as recoverable errors; never overwrite the last-good document.
- [x] Decide whether additions fit the existing negotiated protocol. If not, introduce
  an explicit version/capability handshake and prove old-app/new-firmware and
  new-app/old-firmware failure behavior without partial mutation.
- [x] Keep deterministic config compilation and transaction ordering: validate and
  compile, persist atomically, replace runtime state, then queue device replay.
- [x] Generate/check the expanded Rust/TypeScript serialization contract and firmware
  canonical-CBOR fixtures.

**Acceptance:** Fixture tests prove migration, bounds, deterministic compilation, and
both sides of the supported compatibility matrix before any new provider touches the
device.

**Implementation evidence (2026-08-05):** Config schema v2 now uses closed tagged
provider/template/layout/action/asset/update types with documented limits in
`docs/config/v2.md`. The store losslessly migrates released v0/v1 documents, preserves
future-version bytes and last-good state, and fixtures cover the full v2 surface and
invalid boundaries. Deterministic compilation emits only the proven M3 subset;
unimplemented M4 composition returns typed `requires-capability` issues before runtime
replacement or wire traffic. Protocol v1 remains compatible through additive status
keys 22-23 and seven frozen capability bits: released-M3-reader/current-firmware,
current-app/legacy-firmware, direct apply, and reconnect replay are covered without
partial device mutation. The generated Rust/TypeScript fixture covers every closed
variant and serializes unknown `u64` capability bits losslessly as fixed-width hex.
All 100 Rust tests pass (one intentional fixture-printer test ignored), workspace
formatting and strict Clippy pass, all 15 frontend tests plus formatting/lint/typecheck
and production build pass, all nine C host suites pass, and ESP-IDF builds a `0xbd940`
image with 26% of the smallest app partition free. No physical test was required because
Task 1 adds negotiation/schema boundaries but does not enable a new device feature. The
image was nevertheless flashed to the ready board; status readback reported protocol
1/max 1/capabilities 3, and the release app was reopened and reacquired the serial port.

---

### Task 2: Add bounded providers and wider ICS recurrence

**Likely files:**

- Modify: `companion/crates/providers/`
- Modify: `companion/crates/app-core/src/runtime.rs`
- Modify: `companion/crates/app-core/src/scheduler.rs`
- Create: weather, JSON-feed, RSS, and recurrence fixtures/tests

- [x] Implement weather through a typed provider adapter with explicit location/units,
  request timeout, redirect policy, response-size limit, refresh floor, and stable
  error categories. Do not embed service credentials in config snapshots or logs.
- [x] Implement JSON-feed extraction with a bounded declarative mapping; do not execute
  user code or accept recursive/unbounded traversal.
- [x] Implement RSS/Atom parsing as plain bounded text/metadata. Strip markup and reject
  active content before it reaches snapshots or firmware.
- [x] Expand ICS recurrence to the v1 cases in the product spec, including timezone and
  exception handling, with a bounded occurrence horizon and deterministic ordering.
- [x] Reuse provider-job coalescing, cancellation generations, stale age, last-good
  payloads, and queue-pressure diagnostics for every provider.
- [x] Keep all network/file reads in Rust and cover timeout, oversized, malformed,
  redirect, stale/recovery, cancellation, and scheduler-wake behavior with fixtures.

**Acceptance:** Providers can fail independently for an extended fixture run without
blocking keepalives, duplicating jobs, losing last-good rows, or growing memory/queues.

**Implementation evidence (2026-08-05):** `providers` now shares one credential-safe
HTTP boundary (10-second timeout, three redirects, 1 MiB strict-UTF-8 body) and stable
error categories. The typed Open-Meteo adapter explicitly geocodes location and requests
metric/imperial current conditions with a ten-minute floor. JSON uses bounded direct
`$.field[0]` traversal only; RSS/Atom emits five plain-text rows after rejecting DTDs,
processing instructions, active markup/handlers/schemes, excessive depth/nodes/text,
and malformed XML. ICS adds yearly/month-day/month/ordinal-weekday/set-position rules,
`RDATE`, `WKST`, and timezone-aware moved/cancelled recurrence exceptions while keeping
the horizon/scan bound and unsupported-rule counter.

The runtime now schedules calendar, weather, JSON, and RSS through the same bounded
worker, startup/manual/interval deadlines, in-flight coalescing, config-generation
cancellation, last-good/stale projection, and pressure counters. A mixed-provider test
forces queue pressure and an independent RSS failure while device status keepalives and
the other providers continue; the existing replacement race proves late-result discard.
The provider contract is documented in `docs/providers/v1.md`. All 113 Rust tests pass
(one intentional TypeScript-fixture printer ignored), workspace formatting and strict
Clippy pass. Task 2 changes no firmware or wire contract, so no board reflash or repeated
physical soak is required.

---

### Task 2B: Replace the widget/screen authoring model with cards

**Likely files:**

- Modify: `companion/crates/app-core/src/config.rs`, `store.rs`, `state.rs`,
  `scheduler.rs`, `runtime.rs`
- Modify: `companion/crates/providers/src/ics.rs`
- Modify: `companion/apps/deskmate/src-tauri/src/commands.rs`
- Modify: `companion/apps/deskmate/src/lib/{types.ts,types.contract.ts,configDraft.ts}`
- Create: `companion/apps/deskmate/src/components/{CardList,CardEditor,Filmstrip}.tsx`;
  delete `WidgetGallery.tsx`, `ScreenArranger.tsx`, `WidgetEditor.tsx`
- Rewrite: `docs/config/v2.md` → `docs/config/v3.md` (this document set, closed by the
  present Task 12)

**Spec:** `docs/superpowers/specs/2026-08-06-deskmate-card-model-design.md`. Full
task-by-task detail lives in
`docs/superpowers/plans/2026-08-06-deskmate-card-model.md`; this entry is the M4-plan
summary and evidence record the design doc's own §10 calls for.

- [x] Add `CardPresence`, `CardAlert`, `AlertHold`, `CarouselAdvance` value types with
  hand-written validating `Deserialize` impls (serde's `deny_unknown_fields` is a
  documented no-op on internally tagged enums; every nested tagged type needed the same
  treatment, not just the outer one).
- [x] Replace `AppConfig.widgets`/`screens` with `cards: Vec<CardSettings>`; delete
  `WidgetSize`, `WidgetInterruptPolicy`, `ScreenSettings`, `ScreenLayout`,
  `TileSettings`. Bump `CURRENT_SCHEMA_VERSION` to 3. Compilation lowers `cards[]` to
  the **unchanged** `ApplyConfig` wire shape, pinning `SizeClass::Full`, deriving each
  screen ID from its card ID, skipping `off` cards, and emitting `alert-only` cards as
  screenless widgets. No firmware file is touched by this change.
- [x] Add validation rules: 1..8 cards with unique IDs; `alert-only` requires a
  non-`none` alert; alert variant must match card kind (`on-timer-finish` pomodoro only,
  `before-event` calendar only); at least one card must be `in-rotation`; the existing
  numeric bounds for `dwell_seconds`/`default_dwell_seconds`/`lead_minutes`/`hold`.
- [x] Migrate v0, v1, and v2 documents to v3 directly (v0/v1 do not chain through an
  in-memory v2 step). Card order follows the legacy `screens[]` order, not `widgets[]`.
  Each card keeps its widget ID; the screen ID is discarded. A widget with no
  referencing screen (protocol-legal but validation-illegal in every prior schema)
  becomes `alert-only` when its historical/explicit interrupt policy was enabled, `off`
  otherwise, sorted by ID. `future-v3.json` is renamed `future-v4.json` so the
  unsupported-future-version fixture keeps asserting the opposite of a supported schema.
- [x] Add host-driven timed rotation to `Scheduler`/the runtime worker: a rotation
  deadline rearms per in-rotation card's own `dwell_seconds` (or the carousel default),
  advancing to the next in-rotation card on expiry, restarting on manual navigation, and
  never sleeping past a pending rotation deadline.
- [x] Add bounded alert triggers: pomodoro `on-timer-finish` and calendar
  `before-event`, keyed to the specific interrupt token so a differently-scheduled
  interrupt cannot clobber another's hold deadline. `AlertHold::Seconds` arms an
  absolute host-side deadline; `AlertHold::UntilDismissed` disarms it.
- [x] Add `next_start_unix_ms` (UTC-derived, `display_timezone`-independent) to the ICS
  provider's field snapshot so the `before-event` trigger has a machine-readable event
  start to compare against, with fixture coverage including a recurring event. This is
  the only file outside `app-core` and the frontend this task touches; the wire
  registry for `RowList` fields is unchanged, so firmware's
  `widget_model_push_widget_fields` counts this field as unknown and
  `unknown_field_count` climbs (saturating) for the life of the device — harmless,
  since the 14 fields pushed stay well under `MAX_FIELD_COUNT` (16), and a deliberate,
  documented consequence of adding a host-only field without a matching firmware
  registry update.
- [x] Widen the `AppSnapshot` typed IPC payload with `card_data: Vec<CardDataSnapshot>`
  carrying last-good field values per card, so the settings preview renders the same
  data the device receives instead of inventing sample content. Regenerate
  `types.contract.ts` from the Rust fixture printer; the sync test compares it
  byte-for-byte.
- [x] Rebuild the frontend: `CardList` (in-rotation + alerts/muted sections, numbered
  only where a carousel position exists) replaces `WidgetGallery` + `ScreenArranger`;
  `CardEditor` covers all six kinds with presence/alert controls; `Filmstrip` renders
  proportional-width dwell segments below the device mock and can play the rotation at
  real dwell timing. Fixed the section-7.4 defects in the same work: the widget cap is
  8 (was 16, silently inconsistent with the backend), all six card kinds are addable
  from the UI (weather/JSON-feed/RSS previously had no entry point), validation issues
  are keyed by card ID rather than array index (reordering no longer misattributes an
  error to the wrong row), first-run guidance clears without requiring both a pomodoro
  and a calendar card, and internal card IDs are no longer rendered to the user.

**Acceptance:** The workspace compiles and every existing test suite passes with the
widget/screen model fully removed from config, store, runtime, IPC, and the frontend;
`ApplyConfig` fixtures — including the maximum-capacity fixture — re-encode
byte-for-byte identically to before this task, proving the wire did not move.

**Implementation evidence (2026-08-06):** All 22 commits on `feat/card-model`
(`402049b`..`11c03b3`) land the design doc in full. `cargo fmt --all --check` and
`cargo clippy --workspace --all-targets -- -D warnings` are clean.
`cargo test --workspace` reports **161 tests passed, 0 failed** across the workspace's
Rust crates (1 additional test — the TypeScript contract fixture printer — is
`#[ignore]`d by design; it is invoked directly by the regeneration step, not by the
normal suite). `bun test` in `companion/apps/deskmate` reports **62 tests passed, 0
failed, 674 `expect()` calls** across 3 files; `bun run check` (`tsc --noEmit`),
`bun run lint` (Biome), and `bun run format:check` (Biome) all pass with zero findings.
`make -C firmware/host_tests clean test` passes all 9 plain-C suites with **no firmware
source file modified by this task** — the C sources compiled are byte-identical to
before Task 2B, which is the direct evidence that `ApplyConfig` and the rest of the wire
contract did not move under the authoring-model change. This task changes no firmware
and requires no physical board verification; none was performed or is claimed.

**Known limitation carried forward, not resolved by this task:** `AlertHold::Seconds`
bounds only the host's outstanding-alert bookkeeping, not the physical panel. Protocol
v1 has no host→device dismissal message; firmware's `show_carousel_screen` is skipped
whenever an interrupt is active
(`firmware/main/link/protocol_task.c`), and `docs/protocol/v1.md`'s `TriggerInterrupt`
section states M2 has no automatic interrupt timeout — only an on-device tap dismisses
an active interrupt. The runtime's `arm_alert_hold` (`companion/crates/app-core/src/
runtime.rs`) documents this at the point the deadline is armed. Two resolutions are
open and **neither has been chosen**: (a) amend the design so `hold` is host-side
bookkeeping only, documenting that a bounded alert's overlay persists until tapped, or
(b) add an additive host→device dismissal message — a wire change plus firmware work
plus physical verification. See `docs/config/v3.md`'s "Known limitation" subsection.

---

### Task 3: Implement the remaining templates

**Likely files:**

- Modify: `firmware/main/core/widget_model.*`
- Modify: `firmware/main/ui/`
- Modify: `companion/crates/app-core/src/config.rs`
- Modify: `companion/apps/deskmate/src/components/DevicePreview.tsx`
- Create: host rendering/model tests and visual acceptance fixtures

- [ ] Add big-number + label, icon + badge + text, and analog-clock templates with
  explicit field schemas. Each template gets exactly one 448x368 layout; size classes
  do not exist in the card model, so there is no per-size compatibility matrix to build.
- [ ] Bound formatting, truncation, glyph fallback, numeric range, and icon lookup.
  Missing fields/assets must produce a stable fallback rather than stale pixels.
- [ ] Keep LVGL mutations on the UI task, retain even-pixel CO5300 invalidation, and
  preserve clean-canvas interrupt layering.
- [ ] Match template semantics and proportions in the deterministic app preview while
  continuing to label it as a preview, not pixel-identical firmware output.
- [ ] Add plain-C model tests plus 90°/270° physical visual/touch checks for every new
  template.

**Acceptance:** Each template survives malformed/maximal data and renders cleanly at
both orientations without heap drift or UI queue overflow.

---

### Task 4: Add timed rotation and bounded alerts

**Replaced.** The tile-dashboard task originally planned here is cancelled — see
`docs/superpowers/specs/2026-08-06-deskmate-card-model-design.md` §1-2: the 448x368
panel at desk distance cannot render a legible multi-tile dashboard, and removing
dashboards also removes the per-size-class template variants Task 3 would otherwise
have owed. This task is **delivered by Task 2B**, entirely host-side, over the existing
priority-interrupt arbitration. It requires no firmware layout work, no grid placement
rules, and no grid physical checks, because the wire protocol and firmware are
unchanged — see Task 2B's implementation evidence.

**Likely files (as delivered by Task 2B):**

- Modified: `companion/crates/app-core/src/scheduler.rs`, `runtime.rs`, `config.rs`
- Modified: `companion/apps/deskmate/src/` (card list, editor, filmstrip)

- [x] Host-driven timed carousel rotation: `carousel.advance` is `manual` or
  `timed { default_dwell_seconds }`; each in-rotation card holds for its own
  `dwell_seconds` or the default; the host sends `ActivateScreen` on dwell expiry; the
  firmware has no rotation timer of its own.
- [x] Bounded alert triggers: pomodoro `on-timer-finish` and calendar `before-event`
  (via the new `next_start_unix_ms` ICS field), reusing the proven interrupt arbiter
  rather than adding a second mechanism.
- [x] Preserve the companion-owned 90°/270° mounting choice; rotation and alerts are
  orientation-independent host logic with no layout dependency. Portrait modes and
  on-device rotation gestures remain unsupported.
- [x] Preserve active screen, tap hit-testing, and preview ordering — rotation reuses
  `ActivateScreen`, and alerts reuse `TriggerInterrupt`/`DismissInterrupt`, both
  unchanged wire messages.
- [x] Stress rotation-deadline and alert-hold scheduling in host tests (dwell expiry
  ordering, dwell inheritance from the default, manual navigation overriding a pending
  advance, hold expiry keyed to a specific interrupt token so a differently-scheduled
  interrupt cannot clobber another's deadline). **Physical verification of rotation and
  alerts at 90° and 270° on the board has not been performed; it is outstanding** — see
  Task 10's exit gate.

**Acceptance:** Cards rotate on their configured dwell and alerts take over and yield
correctly in host tests; the wire protocol and firmware are unchanged, so no grid
corruption/phantom-tap/duplicate-action class of defect is applicable to this task.

**Known limitation (not resolved by this task):** `AlertHold::Seconds` bounds only the
host's outstanding-alert bookkeeping, not the physical panel — protocol v1 has no
host→device dismissal message, so the device continues showing an alert overlay until
an on-device tap regardless of the host-side hold timer. See Task 2B's evidence and
`docs/config/v3.md` for the two open, unchosen resolutions.

---

### Task 5: Add explicit, safe host tap actions

**Likely files:**

- Modify: `companion/crates/app-core/`
- Modify: `companion/apps/deskmate/src-tauri/`
- Modify: `companion/apps/deskmate/src/`
- Modify: `firmware/main/core/`
- Create: action validation/authorization tests

- [ ] Model only the approved “open URL” and “open application” actions. Reject command
  strings, arguments, environment expansion, scripts, and unknown URL schemes.
- [ ] Resolve actions in Rust from a configured widget/action ID; never trust a device
  event to provide a path or URL directly.
- [ ] Require clear settings-time consent, show the resolved target, allow disabling an
  action, and rate-limit repeated touch events.
- [ ] Use narrow OS APIs with platform-specific allowlists and typed failure state. Keep
  broad shell/process permissions out of the webview capability.
- [ ] Test forged/stale/duplicate events, disabled actions, unsafe schemes/paths, burst
  pressure, reconnect, and platform failures.

**Acceptance:** A physical tap opens only the exact approved target once, while crafted
serial events or config cannot become arbitrary code execution.

---

### Task 6: Add bounded asset and font push

**Likely files:**

- Modify: `companion/crates/protocol/`
- Modify: `companion/crates/device/`
- Modify: `companion/crates/app-core/`
- Modify: `firmware/main/core/`
- Modify: `firmware/main/ui/`
- Create: asset conversion and interrupted-transfer tests

- [ ] Freeze supported image/font formats, dimensions, glyph counts, storage budget,
  identifiers, hashes, and per-transfer limits. Convert assets in the trusted Rust host,
  not on the LVGL task.
- [ ] Add chunked, checksummed, resumable transfer with explicit staging/commit. An
  interruption or bad hash must leave the previous asset set bootable and selected.
- [ ] Cache by content hash, garbage-collect only unreferenced committed assets, and
  include the manifest in full replay/status diagnostics.
- [ ] Render missing/corrupt assets with bounded built-in fallbacks and report a typed
  error to settings.
- [ ] Test duplicates, reordering, full storage, bad chunks, disconnect at each phase,
  reboot during staging, and repeated replay before physical endurance checks.

**Acceptance:** Custom assets persist and replay without exceeding fixed RAM/flash
budgets; interrupted or corrupt transfers never prevent the standalone clock from booting.

---

### Task 7: Implement production identity and in-app firmware update

**Likely files:**

- Modify: firmware partition/USB/build configuration
- Modify: `companion/crates/device/`
- Modify: `companion/crates/app-core/`
- Modify: `companion/apps/deskmate/src-tauri/`
- Modify: `companion/apps/deskmate/src/`
- Create: updater packaging, validation, and recovery tests

- [ ] Obtain and record an authorized production VID/PID strategy before changing the
  development identity. Keep an explicit migration/discovery path for development units.
- [ ] Choose the update transport and recovery design from measured board capabilities;
  define signed artifact metadata, model/version compatibility, size/hash checks,
  progress, cancellation boundaries, and rollback/recovery behavior before coding.
- [ ] Verify the package entirely in Rust before device mutation. The webview may select
  an approved artifact and show typed progress but must not gain raw serial/filesystem
  or process access.
- [ ] Keep normal runtime traffic paused in a single explicit updater state, then
  rediscover, verify the new firmware identity/version, and replay authoritative state.
- [ ] Fault-inject corrupt/wrong-model/downgrade artifacts and disconnect/reboot at every
  update phase. Prove the standalone screen or documented recovery path remains usable.

**Acceptance:** A release artifact updates from the app on the physical board, reports
verified success, and survives an intentionally interrupted update through the proven
rollback/recovery path.

---

### Task 8: Complete the v1 settings experience

**Grown by Task 2B.** The card-model redesign
(`docs/superpowers/specs/2026-08-06-deskmate-card-model-design.md` §7) replaced the
`WidgetGallery` + `ScreenArranger` split this task originally targeted with a single
card list plus a filmstrip preview, delivered ahead of schedule as part of Task 2B
because the frontend could not compile against `cards[]` otherwise. The remaining scope
below — asset/update settings, and the loading/empty/error/stale/offline/mismatch/update
state coverage for M4 features not yet implemented (weather/JSON-feed/RSS provider
settings landed in Task 2B; asset and update settings have not) — is still open.

**Likely files:**

- Modify: `companion/apps/deskmate/src/`
- Modify: `companion/apps/deskmate/src/styles.css`
- Modify: frontend tests

- [x] **(Delivered by Task 2B.)** Single card list (`CardList.tsx`): an "in rotation"
  section, numbered and drag/keyboard-reorderable in carousel order, and an "alerts and
  muted" section for `alert-only`/`off` cards, which have no carousel position and are
  therefore not numbered. Replaces the three-panel widget-gallery/screen-arranger/draft
  synchronization it originally required.
- [x] **(Delivered by Task 2B.)** `CardEditor.tsx` covers all six card kinds — clock,
  pomodoro, calendar, weather, JSON feed, RSS — each with presence and alert controls,
  where the gallery previously offered only three (clock, pomodoro, calendar) despite
  the backend having shipped all six providers in Task 2.
  The card editor states each card's tap behaviour explicitly per the design's gesture
  disclosure requirement, rather than leaving it undiscoverable.
- [x] **(Delivered by Task 2B.)** `Filmstrip.tsx`: proportional-width dwell segments
  below the device mock, showing computed loop length, doubling as the reorder control,
  and able to play the rotation at real dwell timing.
- [x] **(Delivered by Task 2B.)** The typed IPC `AppSnapshot` gains
  `card_data: Vec<CardDataSnapshot>` carrying last-good field values per card, so
  `DevicePreview` renders from the same values the device receives rather than inventing
  preview content; unconfigured/never-fetched/offline/errored cards show a clearly
  marked sample state.
- [x] **(Delivered by Task 2B.)** Section 7.4 defects fixed in the same work: the card
  cap is 8 (was a UI-only 16, inconsistent with the backend's 8-card limit), validation
  issues are keyed by card ID rather than array index (dragging a card in the
  reorderable list no longer misattributes an error to the wrong row), first-run
  guidance clears without requiring both a pomodoro and a calendar card, and internal
  card IDs are no longer rendered to the user.
- [ ] Add typed editors, validation help, provider state, preview, and first-run
  guidance for the remaining M4 asset/update settings. There is no `layout` setting to
  add an editor for — dashboards were cancelled by the card model.
- [ ] Preserve stable IDs, unsaved drafts, deterministic preview, keyboard/pointer
  parity, semantic controls, visible focus, reduced motion, and narrow-window behavior
  for the asset/update additions.
- [ ] Surface protocol/capability mismatches, offline queued replay, storage limits,
  update recovery, and dependency/provider errors without blocking unrelated edits.
- [ ] Test loading/empty/error/stale/offline/mismatch/update states for asset/update
  settings, and migration from a representative M3 document (card-model migration
  fixtures already cover v0/v1/v2 → v3; this item is about the settings UI's handling
  of a freshly migrated document, not the migration logic itself).

**Acceptance:** A nontechnical user can complete the full v1 demo and recover from each
documented failure state without editing JSON or invoking the CLI.

---

### Task 9: Harden and package macOS and Windows releases

**Likely files:**

- Modify: CI workflows and Tauri bundle configuration
- Modify: dependency manifests/lockfiles
- Create: release and support documentation

- [ ] Add native macOS and Windows CI for formatting, warnings-as-errors lint, all tests,
  production frontend build, and Tauri debug/release bundles. Do not use a macOS
  `x86_64-pc-windows-msvc` cross-check as the Windows release gate.
- [ ] Resolve or explicitly review every RustSec/npm advisory. Remove avoidable legacy
  GTK3/unmaintained paths; record owner, impact, and upgrade trigger for any exception.
- [ ] Review IPC commands, CSP/capabilities, URL/app launch policy, logs, update trust,
  persistence permissions, secrets, and generated artifact provenance.
- [ ] Configure versioning, icons/metadata, signing/notarization or the documented v1
  distribution equivalent, reproducible locked builds, and upgrade/uninstall behavior.
- [ ] Test clean install, upgrade from M3 config, first run, single instance, autostart,
  close-to-tray, quit, offline use, corrupt recovery, and uninstall on both platforms.

**Acceptance:** Reviewable macOS and Windows artifacts are produced by locked native CI,
with no unexplained security advisories or broad frontend privileges.

---

### Task 10: Run the v1 exit gate and hand off

- [ ] Run all Rust/frontend/firmware checks, dependency/security audits, capability
  review, native CI builds, and updater fault-injection suites from a clean checkout.
- [ ] On the target board, run every provider/template/action at 90° and 270°, including
  malformed/maximal values and touch bursts. There is no `layout` axis to cross these
  against — dashboards were cancelled by the card model, so every card renders at its
  one 448x368 layout regardless of orientation.
- [ ] At both 90° and 270°, confirm timed rotation: each card holds for its configured
  dwell (including inherited-default and per-card-override cases), the loop wraps, a
  swipe mid-dwell restarts the dwell rather than advancing early, and no duplicate
  `ActivateScreen` is observed.
- [ ] At both 90° and 270°, confirm bounded alerts: a pomodoro `until-dismissed` alert
  holds until tapped and restores the correct carousel screen; a calendar alert with a
  bounded `hold.seconds` fires exactly once per event and self-dismisses its host-side
  bookkeeping at the hold — per the documented known limitation, confirm on the physical
  panel whether the overlay itself also clears at the hold or only on tap, and record
  the observed behavior rather than assuming either.
- [ ] Confirm that setting every card to `alert-only` or `off` in the app is rejected
  with the rotation-rule validation error, and that the device keeps its previous
  configuration.
- [ ] Unplug and replug mid-rotation. Confirm replay restores the active card and any
  pending alert state without duplicates, at both orientations.
- [ ] Prove full replay after repeated physical power cycles and real host sleep/wake.
- [ ] Complete an in-app update plus an interrupted-update recovery on the physical board.
- [ ] Run an extended tray-resident mixed soak. Record host CPU/memory, device heap,
  flash/storage use, counters, queue pressure, provider timings, reconnects, resets,
  missed/duplicate events, render/touch responsiveness, and update state.
- [ ] Update README, roadmap, protocol, hardware notes, support/recovery instructions,
  release checklist, and known limitations with observed results only.
- [ ] Mark M4/v1 complete and create release tags only after review and explicit user
  authorization.

**M4 exit gate:** A nontechnical user can install Deskmate on supported macOS and Windows
versions, configure every approved v1 provider/template/action, use the board for
an extended session through disconnects and sleep, update firmware safely in the app,
and recover through documented paths. The release has bounded resource use, narrow
privileges, an explicit security/advisory record, and no unobserved hardware claims.
