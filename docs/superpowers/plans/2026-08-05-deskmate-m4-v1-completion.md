# Deskmate M4 - v1 Completion Implementation Plan

**Status:** Active at Task 3. Tasks 1-2 froze the compatibility boundary and delivered
the bounded v1 provider runtime.
M3's physical exit is complete; the user explicitly
accepted the completed morning soak and waived repeating it after the focused
orientation/clean-canvas regression passed.

**Goal:** Complete the approved v1 breadth on top of the proven single-owner companion
runtime: weather/JSON-feed/RSS data, the remaining templates and dashboard layouts,
safe host tap actions, landscape mounting and asset management, wider calendar recurrence, an
in-app firmware update path, production USB identity, and release-grade macOS/Windows
verification.

**Exit demo:** Install a release build, create a multi-tile dashboard using local and
network providers, customize its visuals, choose either landscape mounting, and invoke an explicitly
approved host action from touch. Disconnect/reconnect and power-cycle without losing
authoritative state. Perform a verified firmware update in the app and recover safely
from an interrupted update. Complete the release smoke checklist on macOS and Windows.

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

### Task 3: Implement the remaining templates

**Likely files:**

- Modify: `firmware/main/core/widget_model.*`
- Modify: `firmware/main/ui/`
- Modify: `companion/crates/app-core/src/config.rs`
- Modify: `companion/apps/deskmate/src/components/DevicePreview.tsx`
- Create: host rendering/model tests and visual acceptance fixtures

- [ ] Add big-number + label, icon + badge + text, and analog-clock templates with
  explicit field schemas and full/standard/tile size compatibility.
- [ ] Bound formatting, truncation, glyph fallback, numeric range, and icon lookup.
  Missing fields/assets must produce a stable fallback rather than stale pixels.
- [ ] Keep LVGL mutations on the UI task, retain even-pixel CO5300 invalidation, and
  preserve clean-canvas interrupt layering.
- [ ] Match template semantics and proportions in the deterministic app preview while
  continuing to label it as a preview, not pixel-identical firmware output.
- [ ] Add plain-C model tests plus 90°/270° physical visual/touch checks for every new
  template and size class.

**Acceptance:** Each template survives malformed/maximal data and renders cleanly in
every supported size and orientation without heap drift or UI queue overflow.

---

### Task 4: Add tile dashboards and preserve landscape mounting

**Likely files:**

- Modify: `firmware/main/core/widget_model.*`
- Modify: `firmware/main/ui/`
- Modify: `companion/crates/app-core/`
- Modify: `companion/apps/deskmate/src/`
- Modify: `docs/hardware/board-notes.md`

- [ ] Define a bounded dashboard grid and deterministic tile placement rules, including
  overlap rejection, minimum touch targets, clean-canvas spacing, and mixed-template
  compatibility.
- [ ] Compile and replay dashboard layouts atomically; a rejected grid must leave the
  previous screen set visible and authoritative.
- [ ] Preserve the companion-owned 90°/270° mounting choice for dashboards. Keep
  portrait modes and on-device rotation gestures unsupported.
- [ ] Preserve active screen, tap hit-testing, touch coordinates, and preview ordering
  across orientation changes.
- [ ] Stress repeated layout swaps and rotation changes in host tests, then verify both
  orientations and mixed tile taps on the physical panel.

**Acceptance:** Full-screen and tile screens can coexist and rotate without corrupt
regions, phantom taps, duplicate actions, or loss of replay state.

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

**Likely files:**

- Modify: `companion/apps/deskmate/src/`
- Modify: `companion/apps/deskmate/src/styles.css`
- Modify: frontend tests

- [ ] Add typed editors, validation help, provider state, preview, and first-run guidance
  for every M4 provider/template/layout/action/asset/update setting.
- [ ] Preserve stable IDs, unsaved drafts, deterministic preview, keyboard/pointer
  parity, semantic controls, visible focus, reduced motion, and narrow-window behavior.
- [ ] Surface protocol/capability mismatches, offline queued replay, storage limits,
  update recovery, and dependency/provider errors without blocking unrelated edits.
- [ ] Test loading/empty/error/stale/offline/mismatch/update states and migration from a
  representative M3 document.

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
- [ ] On the target board, run every provider/template/layout/action at 90° and 270°,
  including malformed/maximal values, touch bursts, rotation, host loss, and reconnect.
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
versions, configure every approved v1 provider/template/layout/action, use the board for
an extended session through disconnects and sleep, update firmware safely in the app,
and recover through documented paths. The release has bounded resource use, narrow
privileges, an explicit security/advisory record, and no unobserved hardware claims.
