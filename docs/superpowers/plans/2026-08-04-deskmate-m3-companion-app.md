# Deskmate M3 - Daily-Use Companion App Implementation Plan

> **STATUS — HISTORICAL IMPLEMENTATION RECORD.** This plan describes the deleted native
> companion and its paths at the time of delivery; it is not current architecture or build
> guidance.

**Status:** Complete on 2026-08-05. Software, macOS app, physical-board UI/replay,
sleep/wake, and tray-resident soak checks pass. The user accepted the completed morning
soak and explicitly waived repeating it after the later orientation/clean-canvas change,
whose focused physical regression also passed.

**Goal:** Replace the M2 CLI as the normal daily-use host with a Tauri v2 tray app. One
long-lived Rust runtime owns device discovery/reconnect, the complete replay set,
provider refreshes, pomodoro state, interrupts, and persistent configuration. A React +
TypeScript settings window lets a nontechnical user configure the three proven widgets,
preview them, and arrange carousel screens without editing files or using a terminal.

**Exit demo:** Install and launch Deskmate, configure a clock, pomodoro, and ICS calendar
in the settings window, reorder their screens, close the window, and continue using the
tray-resident device session. A power cycle automatically restores time, layout, and all
widget data. Relaunching the app focuses the existing instance. Autostart is configurable
and the app remains useful when the settings webview is closed.

**Architecture:** Keep Rust authoritative. The webview edits typed drafts through a
narrow IPC boundary; it does not own serial I/O, provider scheduling, revisions, or the
on-disk file. A new runtime/controller crate composes `device`, `engine`, `providers`,
and `protocol`, while the Tauri crate owns desktop lifecycle, tray, commands, and event
projection. Persist one versioned JSON document with atomic replacement and retain the
last-good file if validation or migration fails.

**M2 findings carried forward:**

- A reconnect owner can replay only state it issued. The app must establish time,
  config, every latest widget snapshot, active screen, and interrupts through one
  long-lived `DeviceSession`; never reproduce the original multi-process CLI hole.
- Device/config/data revisions are seeded from status and remain session-global. UI
  saves compile and validate a complete layout before any device mutation.
- Provider failures preserve last-good data and expose stale age/error state.
- The settings window is disposable; closing it must not stop keepalives or providers.
- Hardware renders at 448x368 in 90°/270° landscape. The preview uses the same logical
  proportions, template/size compatibility rules, and screen order. The later M3
  clean-canvas amendment removed the original status-strip design.
- M3 does not require firmware changes unless app integration reveals a protocol defect.

**Current Tauri v2 baseline:** Use a Vite SPA rather than SSR, Rust-side
`TrayIconBuilder`, the official single-instance and autostart plugins, and a minimal
window-scoped capability file. Tauri capabilities grant webview permissions but do not
sandbox Rust code, so IPC command validation remains mandatory. Reference the official
Tauri v2 project, tray, single-instance, autostart, and capabilities documentation when
implementing; lock resolved Rust and frontend dependencies in the repository.

**Out of scope:** Weather, JSON-feed, and RSS providers; dashboard grids and `tile`
layouts; new templates; open-URL/app tap actions; optional timed carousel advance;
asset/font push; firmware
update UI; production VID/PID; signing/notarization and broad release packaging. Those
remain M4 work. Do not add a general plugin system.

---

### Task 1: Freeze the persistent app/config/runtime contract

**Files:**
- Create: `companion/crates/app-core/Cargo.toml`
- Create: `companion/crates/app-core/src/lib.rs`
- Create: `companion/crates/app-core/src/config.rs`
- Create: `companion/crates/app-core/src/state.rs`
- Create: `companion/crates/app-core/tests/config.rs`
- Modify: `companion/Cargo.toml`
- Modify: `docs/protocol/v1.md` only if an actual wire clarification is required

- [x] Define a serde-backed, versioned `AppConfig` containing app preferences,
  autostart/pause state, clock/pomodoro/calendar widget settings, and ordered screens.
- [x] Keep IDs stable and explicit. Reject duplicate IDs, missing references, unsupported
  template/size combinations, invalid timer bounds, invalid timezone names, and empty or
  oversized ICS sources before touching disk or serial.
- [x] Define `AppSnapshot`/`DeviceSnapshot`/`ProviderSnapshot` DTOs for frontend reads and
  backend-to-window events. Represent disconnected, connecting, online, standalone,
  paused, stale, and error states explicitly rather than as strings.
- [x] Compile `AppConfig` deterministically into the frozen M2 `ApplyConfig` and complete
  per-widget field snapshots. Keep wire types out of the TypeScript frontend contract.
- [x] Add JSON fixtures for defaults, a fully configured sample, malformed documents,
  and at least one forward-compatible migration boundary.

**Acceptance:** Pure Rust tests prove config validation and deterministic compilation;
invalid drafts cannot partially mutate runtime state.

**Implementation evidence (2026-08-04):** `app-core` now owns strict serde DTOs for the
versioned persistent document and tagged frontend snapshots. Validation aggregates
stable path/code issues before compilation; M3 size compatibility is frozen to
clock/full, pomodoro/standard, and calendar/standard. Deterministic compilation emits
the frozen M2 layout plus complete initial fields for every widget. Seven focused tests
cover defaults/full/malformed/future fixtures, aggregated invalid drafts, deterministic
wire-valid output, zero revisions, and tagged runtime state.

---

### Task 2: Build an atomic versioned config store

**Files:**
- Create: `companion/crates/app-core/src/store.rs`
- Create: `companion/crates/app-core/tests/store.rs`

- [x] Resolve the platform app-data directory in the Tauri shell and pass an explicit
  file path into `app-core`; keep filesystem policy out of engine logic.
- [x] Load UTF-8 JSON with a strict size bound, schema version, validation, and migration.
  Missing files produce defaults; malformed files preserve the bad bytes for diagnosis
  and return a recoverable state instead of overwriting them silently.
- [x] Save by writing and syncing a sibling temporary file, then atomically replacing
  the target. Serialize writes so concurrent UI/provider changes cannot interleave.
- [x] Preserve last-good in-memory config when a load/save/migration fails and expose a
  typed error to the tray/window.
- [x] Test first run, round trip, migration, truncated/oversized JSON, failed replace,
  and concurrent save ordering in temporary directories.

**Acceptance:** Killing a save at any pre-replace point leaves either the previous valid
document or the complete new document, never a half-written active config.

**Implementation evidence (2026-08-04):** `ConfigStore` accepts only an explicit path,
serializes all operations behind one mutex, bounds active documents at 64 KiB, strictly
decodes UTF-8/JSON, migrates the explicit v0 boundary, and returns last-good config plus
a serializable recovery error without altering bad input. Saves use
`atomic-write-file` 0.3.0 for same-directory synced atomic replacement across Unix and
Windows, then sync the parent directory where supported. Five store tests cover first
run, round trip/migration, malformed/invalid-UTF-8/oversized preservation, failed
replacement, and four-way concurrent ordering. The Task 4 shell resolves Tauri's
platform app-data directory and supplies `config.json` as the store's explicit path.

---

### Task 3: Extract the single-owner background runtime

**Files:**
- Create: `companion/crates/app-core/src/runtime.rs`
- Create: `companion/crates/app-core/src/scheduler.rs`
- Create: `companion/crates/app-core/src/commands.rs`
- Create: `companion/crates/app-core/tests/runtime.rs`
- Modify: `companion/crates/device/src/session.rs` only for reusable hooks proven necessary
- Modify: `companion/crates/engine/`
- Modify: `companion/crates/providers/`
- Modify: `companion/crates/deskmate-cli/src/m2.rs`

- [x] Move the full-demo orchestration out of the CLI into `app-core`: one worker owns
  discovery, session reconnect, time sync, complete config, latest data per widget,
  active screen, timer state, and interrupts.
- [x] Add an explicit runtime command channel for apply-config, pause/resume pushing,
  pomodoro control, provider refresh, screen activation, and shutdown. Bound every queue
  and make pressure diagnostics visible.
- [x] Schedule clock setup, pomodoro one-second authoritative snapshots, and ICS refresh
  policy without busy loops. Provider work must not block serial keepalives/event reads.
- [x] Publish coalesced state snapshots to subscribers so a closed settings window costs
  no webview memory and a newly opened window immediately receives current state.
- [x] On power reset, replay the complete owned state in protocol order. On same-powered
  reconnect, respect retained revisions. On pause, stop provider/data mutations but keep
  status/discovery explicit and let firmware follow the documented liveness policy.
- [x] Retain `deskmate-cli demo` as a thin harness over the shared runtime where practical,
  preventing CLI and Tauri orchestration from diverging.
- [x] Test with mock transports/providers: cold boot, retained reconnect, power reset,
  event-before-reply, provider delay/failure, config replacement during refresh,
  pomodoro completion/dismissal, pause/resume, bounded subscriber pressure, and shutdown.

**Acceptance:** A headless `app-core` integration test demonstrates the entire M2 flow
and two reset/reconnect cycles without Tauri or physical serial hardware.

**Implementation evidence (2026-08-04):** `app-core` now owns a long-lived runtime over
the replay-capable `DeviceSession`, with a mockable serial boundary, bounded commands and
provider jobs, nonblocking ICS work, monotonic pomodoro scheduling, typed pressure
diagnostics, and one-slot coalescing subscribers. Full synchronization is issued as time,
layout, every latest widget snapshot, active screen, then interrupts; retained reconnects
reuse device revisions while reset reconnects replay the nested session's complete cache.
`deskmate-cli demo` now translates the checked M2 layout into `AppConfig` and only launches
the shared runtime. Eight runtime integration tests cover cold boot, retained reconnect,
two power resets, provider delay/failure and replacement races, pomodoro completion and
dismissal, pause/resume, bounded command/subscriber pressure, invalid commands, and clean
shutdown. The device-session suite continues to cover events arriving before replies and
the concrete revision-aware replay transport behavior.

---

### Task 4: Bootstrap the Tauri v2 desktop shell

**Files:**
- Create: `companion/apps/deskmate/package.json`
- Create: `companion/apps/deskmate/bun.lock`
- Create: `companion/apps/deskmate/index.html`
- Create: `companion/apps/deskmate/tsconfig.json`
- Create: `companion/apps/deskmate/vite.config.ts`
- Create: `companion/apps/deskmate/src-tauri/Cargo.toml`
- Create: `companion/apps/deskmate/src-tauri/build.rs`
- Create: `companion/apps/deskmate/src-tauri/src/main.rs`
- Create: `companion/apps/deskmate/src-tauri/src/lib.rs`
- Create: `companion/apps/deskmate/src-tauri/tauri.conf.json`
- Create: `companion/apps/deskmate/src-tauri/capabilities/main.json`
- Create: app/tray icons
- Modify: `companion/Cargo.toml`

- [x] Scaffold Tauri v2 + React + TypeScript + Vite inside the existing workspace and
  pin resolved dependencies. Keep the frontend a static SPA.
- [x] Start `app-core` during Tauri setup before opening settings. Own it in managed
  state and shut it down deliberately on real application exit.
- [x] Add a Rust-built tray menu: device status (disabled label), pause/resume pushing,
  open settings, autostart toggle, and quit. Tray icon/status updates follow snapshots.
- [x] Use the official single-instance plugin early in builder registration; a second
  launch shows and focuses the existing settings window.
- [x] Use the official autostart plugin with the macOS LaunchAgent path and corresponding
  Windows support. Do not silently enable it before the user chooses it.
- [x] Hide the settings window on close instead of exiting. Only the tray Quit action
  stops runtime/keepalives and exits.
- [x] Define the smallest window-scoped Tauri capability set. Prefer typed custom
  commands over broad filesystem, shell, HTTP, or process access from the webview.

**Acceptance:** Development launch shows one tray icon; repeated launches focus one
instance; window close leaves the runtime alive; tray Quit terminates it; autostart
toggle round-trips through the OS plugin.

**Implementation evidence (2026-08-05):** The pinned Tauri 2.11.5 shell and static
React/Vite SPA now live in the existing Cargo workspace with Cargo and Bun lockfiles.
Setup resolves Tauri's app-data directory, loads the atomic store, starts one managed
`RuntimeHandle`, builds the Rust tray, and only then reveals settings. Snapshot updates
drive the disabled device label, pause/resume text, tooltip, and online/offline template
icon. Single-instance is the first plugin; autostart uses the official plugin with
`MacosLauncher::LaunchAgent`; the one window-scoped capability grants only core event
listening and no filesystem, network, shell, process, or autostart API. A live macOS
debug-app smoke test observed one process after a forced second launch with the existing
window focused, close-to-hide with the process still resident, tray reopen, persisted
pause/resume, a real `Deskmate.plist` LaunchAgent enable/disable round trip, and tray Quit
leaving no process. Both toggles were restored to autostart off and pushing resumed.

---

### Task 5: Implement typed IPC and lifecycle projection

**Files:**
- Create: `companion/apps/deskmate/src-tauri/src/commands.rs`
- Create: `companion/apps/deskmate/src-tauri/src/events.rs`
- Create: `companion/apps/deskmate/src/lib/tauri.ts`
- Create: `companion/apps/deskmate/src/lib/types.ts`
- Create: `companion/apps/deskmate/src/lib/useAppState.ts`

- [x] Expose narrow commands for snapshot, validate draft, save/apply config,
  pause/resume, pomodoro control, manual refresh, autostart get/set, and settings-window
  lifecycle. Return serializable tagged errors with stable categories.
- [x] Emit one coalesced app-state event stream and recover from missed frontend events
  by fetching a fresh snapshot on focus/reopen.
- [x] Keep secrets and raw filesystem/network/serial primitives out of IPC. Bound every
  string/list at both serde and domain-validation layers.
- [x] Generate or compile-check matching TypeScript DTOs so Rust/TS drift fails CI.
- [x] Unit-test command authorization-independent logic and frontend subscription cleanup.

**Acceptance:** The frontend can be reloaded or closed/reopened without changing runtime
ownership, leaking listeners, or losing the latest state.

**Implementation evidence (2026-08-05):** The shell now registers a narrow custom-command
surface for snapshots, bounded/strict draft validation, transactional save/apply,
pause/resume, pomodoro control, provider refresh, autostart, and settings visibility.
Errors cross IPC as stable tagged categories, and drafts are capped at 64 KiB before the
existing per-field/count domain checks. One coalescing runtime subscription now projects
the same snapshot to the tray and `app-state` event; the React hook subscribes before its
initial fetch, refreshes on focus/visibility, and safely releases even a listener promise
that resolves after unmount. A Rust-generated serialization fixture is checked byte-for-byte
by a backend test and compiled with TypeScript `satisfies`, so contract drift fails either
side. Nine focused backend tests and two frontend lifecycle tests pass; workspace fmt,
Clippy, all 77 Rust tests, the frontend production build, and a Tauri debug no-bundle build
also pass.

---

### Task 6: Build the settings experience

**Files:**
- Create: `companion/apps/deskmate/src/main.tsx`
- Create: `companion/apps/deskmate/src/App.tsx`
- Create: `companion/apps/deskmate/src/styles.css`
- Create: `companion/apps/deskmate/src/components/DeviceHeader.tsx`
- Create: `companion/apps/deskmate/src/components/WidgetGallery.tsx`
- Create: `companion/apps/deskmate/src/components/WidgetEditor.tsx`
- Create: `companion/apps/deskmate/src/components/ScreenArranger.tsx`
- Create: `companion/apps/deskmate/src/components/DevicePreview.tsx`
- Create: `companion/apps/deskmate/src/components/ProviderStatus.tsx`
- Create: frontend tests

- [x] Create a focused desktop layout with persistent device status, widget gallery,
  selected-widget editor, ordered screen list, and a 448:368 device preview.
- [x] Support the M3 widget set only: digital clock/full, progress ring/standard with
  start-pause, and ICS row list/standard. Explain incompatible size choices instead of
  silently coercing them.
- [x] Make screen ordering work with pointer drag, keyboard controls, and explicit move
  buttons. Preserve stable IDs while reordering.
- [x] Validate drafts inline and show save/apply progress, last-good fallback, provider
  staleness, disconnected state, and protocol mismatch without blocking the editor.
- [x] Provide first-run guidance: connect USB, choose timezone, add an ICS URL/file,
  arrange screens, save. No terminal terminology in product copy.
- [x] Make the preview deterministic and visually parallel the firmware templates;
  clearly label it as a preview rather than pixel-identical LVGL rendering.
- [x] Offer exactly two persisted landscape mounting choices in Settings (USB below or
  USB above), apply them through the authoritative config, and keep rotation off the
  device's touch surface.
- [x] Remove the status strip from firmware and preview so every M3 widget uses the clean
  448x368 canvas. Retain the `standard` wire value only for v1 compatibility.
- [x] Test keyboard navigation, labels/focus, reduced motion, empty/error/loading states,
  reorder semantics, draft validation, and narrow-window behavior.

**Acceptance:** A user can create, edit, reorder, validate, and apply the full M2 demo
without touching JSON or a terminal, using either pointer or keyboard.

**Implementation evidence (2026-08-05):** The React settings app now provides a
persistent device header, first-run guidance, M3-only widget gallery/editor, provider
status, stable-ID screen arrangement, explicit and keyboard reorder controls, and a
deterministic 448:368 preview. Draft errors are shown inline while disconnected,
stale/error, persistence-recovery, and protocol-mismatch states remain independently
visible. Native buttons, lists, fieldsets, focus styles, live regions, narrow-window
layout, and reduced-motion behavior keep the editor usable without pointer input. The
pinned Biome formatter/linter, TypeScript check, 15 Bun tests (59 expectations), and
production Vite build all pass.

---

### Task 7: Integrate providers, timer state, and persistence end to end

**Files:**
- Modify: `companion/crates/app-core/`
- Modify: `companion/apps/deskmate/src-tauri/`
- Modify: `companion/apps/deskmate/src/`
- Create: integration fixtures/tests

- [x] Wire ICS URL/file settings through the existing bounded provider and persist only
  source metadata/config, never transient fetched payload as authoritative config.
- [x] Preserve and display last-good calendar rows with stale age/error after refresh
  failure; manual retry must not duplicate scheduler work.
- [x] Persist pomodoro duration/label and intentional timer state needed across settings
  window closure. Define app-restart behavior explicitly and test it.
- [x] Apply edits transactionally: validate/compile, persist, then replace runtime config;
  if device application fails, keep the saved valid config queued for reconnect and show
  the device error separately.
- [x] Confirm in software that pause/resume, window close/reopen, app restart, overdue
  sleep/wake scheduling, and USB reset handling do
  not create multiple runtimes or duplicate interrupts.

**Acceptance:** The tray app runs the clock, accelerated pomodoro, and fixture/file/URL
calendar for a complete session with visible stale/recovery behavior and durable edits.

**Implementation evidence (2026-08-05):** Calendar URL/file metadata is the only
provider state serialized to config; store tests prove fetched rows remain transient.
Unchanged providers retain their last-good rows and timestamps through unrelated edits,
manual refresh coalesces with work already in flight, and stale/error state is projected
separately. Unchanged live pomodoros and interrupts survive settings saves and window
closure. Duration and label persist across app restart, while running/paused/completed
state deliberately resets to idle: reconstructing elapsed monotonic time across process
downtime would invent state. Runtime and shell tests cover this policy, overdue deadline
coalescing after a simulated wake, persistence recovery, queued replay after replacement,
and USB-reset replay through the one runtime owner. Real host sleep and physical-board
reset observation remain part of Task 8's deferred hardware pass.

---

### Task 8: M3 desktop exit and M4 handoff

**Files:**
- Modify: `README.md`
- Modify: `CLAUDE.md`
- Modify: `docs/superpowers/plans/2026-08-03-deskmate-roadmap.md`
- Modify: `docs/hardware/board-notes.md`
- Create: M4 implementation plan

- [x] Run Rust format/Clippy/tests, frontend format/lint/typecheck/tests, dependency audit,
  Tauri capability review, production frontend build, and Tauri debug/release builds on
  macOS. Keep Windows code compiling in CI or document the first concrete blocker.
- [x] Verify first run from an empty app-data directory, corrupt-config recovery, save
  atomicity, single-instance focus, settings close/reopen, tray pause/resume/quit, and
  autostart opt-in/out.
- [x] On the physical board, configure all three widgets from the UI, reorder screens,
  run tap/pomodoro/interrupt flow, and verify 90°/270° rendering without using the CLI.
- [x] Power-cycle during active app ownership and confirm complete replay. Exercise host
  sleep/wake and settings webview reload without stopping the Rust runtime.
- [x] Run a 30-minute tray-resident mixed session with the settings window closed; record
  process memory/CPU, device heap/counters, provider/runtime queue pressure, reconnects,
  missed events, resets, and responsiveness.
- [x] Update status/docs and write the M4 plan from the software findings.
- [x] Mark M3 complete only after the deferred physical checks pass.
- [ ] Create an `m3` tag only after review fixes and only with explicit authorization.

**Software exit evidence (updated 2026-08-05):** Rust formatting, workspace Clippy with
warnings denied, and all 92 Rust tests pass (one hardware-fixture printer test remains
ignored). Firmware host tests pass all nine cases and ESP-IDF 5.5.5 produces the
`0xbd8d0` application. Frontend formatting, lint, typecheck, 15 tests, and production
build pass. The Tauri macOS `.app` release bundle succeeds. `bun audit`
reports no vulnerabilities. RustSec returns success with 17 allowed advisory warnings:
the inherited GTK3/proc-macro-error/unic crates are unmaintained, and glib 0.18.5 has an
unsoundness warning. These are recorded M4 dependency-hardening work, not silently
reported as clean. A Windows MSVC target check reaches `ring`'s native C build and then
fails because this macOS host has no Windows SDK/MSVC C headers (`assert.h` is missing);
Windows CI or a Windows development host is required for the actual compile gate. The
window capability still grants only core event listening; raw filesystem, network,
shell, process, serial, and autostart APIs are absent from the webview.

**M3 exit gate:** A nontechnical user can install/open the app, configure and arrange the
three proven widgets, close settings, and rely on the tray daemon for provider refresh,
touch actions, reconnect, and replay. No terminal is required for daily use, invalid
config/provider input preserves last-good behavior, and the settings webview never owns
the device session.

**Physical exit evidence (2026-08-05):** The resumed board session covered the complete
UI-configured clock/pomodoro/calendar flow, screen ordering and gestures, timer completion
and dismissal, active-owner power-cycle replay, host sleep/wake, settings lifecycle, and
a tray-resident morning soak. Findings from that session (active-screen replay, calendar
file selection, and interrupt-token seeding) were fixed and regression-tested. The later
orientation/clean-canvas firmware was flashed and the user confirmed both 90°/270° app
choices, no touch rotation, full-canvas rendering for all three widgets, working
pomodoro advancement, calendar content, and carousel swipes. Because those changes did
not alter the already-soaked runtime/queue behavior, the user explicitly accepted the
morning soak and waived a duplicate run. M3 is complete; no tag was created.
