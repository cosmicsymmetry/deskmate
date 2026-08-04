# Deskmate M2 - Template Engine and First Widgets Implementation Plan

**Status:** Active by explicit user direction. Tasks 1-2 are complete; Task 3's
implementation is complete with physical heap/render proof open. Task 4 firmware and
host-test implementation is complete; its host-session end-to-end half awaits Task 5.
M1's software and physical exit gate is complete. Its `m1` tag awaits an intentional
repository checkpoint because M2 work began in the same shared worktree before the
last M1 physical carryover closed.

**Goal:** From a terminal, install a bounded carousel configuration and demonstrate a
digital clock, a host-driven pomodoro with a progress ring and tap start/pause, and an
ICS calendar list. Swipe navigation, the status strip, touch events, and a timer-finished
priority interrupt work on the physical board, while host loss still returns to the
standalone clock.

The target UI canvas is landscape 448x368 at 90 degrees, with a 270-degree flipped
orientation. All template size classes and gesture thresholds use logical dimensions,
not the panel's physical 368x448 order.

**Architecture:** Extend the frozen v1 envelope with new message types rather than
changing framing or wire version. Firmware owns a fixed-capacity configuration/model
core and LVGL template instances; data updates patch existing objects in LVGL context.
The device reports touch/navigation events as unsolicited request-ID-zero frames. The
Rust device crate grows a long-lived session that demultiplexes replies and events. New
`providers` and `engine` crates parse ICS, run the pomodoro state machine, schedule data
updates, and arbitrate interrupts; the CLI remains the M2 harness instead of introducing
the Tauri shell early.

**Out of scope:** Tauri/tray/settings UI, persistent user config, weather/JSON/RSS,
dashboard grids, auto-rotate, arbitrary asset/font transfer, firmware update, and
production VID/PID. M2 may use built-in LVGL symbols/fonts and a checked CLI config file.

## Constraints carried from M1

- Keep native USB Serial/JTAG (`303A:1001`) and the normal automatic flash path. Do not
  reintroduce TinyUSB or move diagnostics onto the protocol stream.
- Keep the v1 COBS/CRC-32C envelope, 2048-byte decoded-frame cap, canonical CBOR rules,
  one host request in flight, and fixed 4096-byte native RX/TX rings.
- Treat configuration, field data, ICS content, and event traffic as untrusted. Parsing
  and model mutation are bounded and atomic: a rejected update preserves the last good
  screen model.
- All template/model rules that do not require LVGL remain plain-C host-testable under
  `firmware/main/core/`. Only `firmware/main/ui/` creates or mutates LVGL objects.
- The protocol task never calls LVGL directly. Model changes are copied into a bounded
  UI command and executed in LVGL context. UI callbacks enqueue bounded device events;
  they never write USB synchronously.
- The standalone clock is always constructible without host config and wins after the
  10-second M1 liveness timeout. Reconnect must replay time, config, and data without a
  reboot.
- Maintain deliberate stack headroom on the 8 KiB protocol task. Audit nested Xtensa
  stack frames again after adding the largest config/event messages.
- Preserve the v2 CO5300 invalidation rounder from M1: dirty areas must start on even
  coordinates and end on odd coordinates before partial flushing. This also forces
  landscape strip heights to an even value before 90°/270° software rotation. The M1
  image remains single-buffered only while this fix is visually isolated; the earlier
  switch from two buffers to one did not change the corruption and is not a durable
  hardware rule.

## M2 protocol additions to freeze before UI work

Reserve v1 IDs 9 onward only after writing exact schemas and golden fixtures:

- `ApplyConfig` / `Ack`: monotonically revised, complete carousel config. It names
  widget instances, template kind, size class, ordered screens, tap action, and interrupt
  policy. Applying it is all-or-nothing.
- `ActivateScreen` / `Ack`: select an existing screen by stable ID for CLI testing;
  swipe navigation remains local to the device.
- `TriggerInterrupt` / `Ack`: show a configured widget with a bounded reason and event
  token; a second trigger follows the frozen replace/queue policy.
- `DeviceEvent`: unsolicited device-to-host frame with request ID zero. Events cover tap
  action, swipe/navigation, interrupt dismissal, and include a monotonic event sequence,
  widget/screen ID, action kind, and optional interrupt token.

`PushData` remains the scalar update primitive. M2 uses one global host-session revision
rather than a per-widget counter, preserving the M1 field and status contract and
`StatusResponse` key 9.

**Revision handshake on connect.** `link_state_accept_push()` accepts only strictly newer
revisions, and the device retains its counter across a host restart. A new host session
that begins at revision 1 against a still-powered device is therefore rejected with
`StaleRevision` on every push. The session must read `latest pushed revision` from the
connect-time `StatusResponse` and start above it. This applies to a restarted companion
process, not only to a replugged cable, and both cases are verified in Task 8.

Initial capacities to prove and then freeze from measured RAM/frame limits:

- at most 8 widget instances and 8 ordered screens;
- IDs retain the M1 32-byte UTF-8 cap, so a config widget ID and a `PushData.widget_id`
  are the same string;
- **no per-widget binding table**: each template declares a fixed set of field names and
  `PushData` keys match them directly (rationale below);
- at most 16 scalar fields per update, unchanged from M1;
- the calendar template displays at most 5 rows, each with a bounded title and time —
  10 fields, inside the 16-field cap;
- one active priority interrupt plus one explicitly defined pending slot (or reject
  additional triggers with `Busy`).

**Why the binding table is gone.** Counts alone do not guarantee a config fits one frame.
The envelope allows 2034 payload bytes; a binding table of 8 entries per widget at the
32-byte key cap costs about 289 bytes per widget, so 8 widgets encode to roughly 2.7 KB
before screens are added. A maximum-capacity config would be unrepresentable and the
"maximum valid config" fixture required by Task 1 could not be generated. The indirection
also buys nothing at M2, because the same host composes both the config and the
`PushData` keys it maps between. Templates therefore declare their own field names
(`label`, `duration_seconds`, `row0_title`, ...) and the host sends matching keys. A
widget entry then costs about 45 bytes. The checked worst-case 8-widget/8-screen fixture,
using 32-byte IDs throughout, measures 931 payload bytes (a typical config with short IDs
is nearer 600). If it ever exceeds 2034 bytes, lower the counts rather than adding
chunking before assets require it. A binding/rename layer can return in M3 if the settings
UI needs one.

Three contract properties must also be written down in Task 1 rather than left to emerge
from the implementation:

- **`DeviceEvent` delivery is at-most-once.** Events are unacknowledged. The monotonic
  event sequence lets the host detect a gap but never recover the lost event; the host
  reconciles through the next authoritative data push (Task 4).
- **Staleness is host-owned.** The engine decides that a widget's data is stale or
  errored and says so in the update; the device does not age data locally. This keeps the
  spec's "the app thinks" split intact.
- **Status-strip notification dots are driven by interrupt state only** in M2 — active
  plus pending interrupt slots. No separate notification message exists yet.

---

### Task 1: Freeze the M2 protocol/model contract

**Files:**
- Modify: `docs/protocol/v1.md`
- Modify: `companion/crates/protocol/src/message.rs`
- Modify: `companion/crates/protocol/src/lib.rs`
- Modify: `companion/crates/protocol/examples/generate-fixtures.rs`
- Modify: `companion/crates/protocol/tests/fixtures.rs`
- Modify: `protocol/fixtures/v1/`
- Modify: `firmware/main/core/protocol_message.c`
- Modify: `firmware/main/core/protocol_message.h`
- Modify: `firmware/host_tests/test_protocol.c`

- [x] Write exact numeric IDs, canonical CBOR maps, capacity limits, enum registries,
  revision/replay rules, the connect-time revision handshake, unsolicited-event rules and
  their at-most-once guarantee, host-owned staleness fields, and interrupt replacement
  behavior.
- [x] Publish the per-template field-name registry (name, type, required/optional,
  default) as part of the wire contract. A template's field names are the `PushData` keys;
  there is no binding indirection.
- [x] Extend the error registry for the new rejection classes (at minimum unknown widget
  or screen reference, unsupported template or size class, and config too large), keeping
  existing codes 1-9 stable.
- [x] Add typed Rust messages and deterministic fixtures for minimum/maximum valid
  config, every event kind, activation, interrupt, and all new error classes. Record the
  measured encoded size of the maximum-capacity config and assert it stays under the
  2034-byte payload limit in a test, not only in prose.
- [x] Add malformed cases for duplicate IDs/screens, missing widget references,
  unsupported template/size combinations, duplicate field names, excessive counts,
  invalid UTF-8, bad event request IDs, and oversized config. Freeze unknown field names
  as schema-valid and model-ignored, per Task 2, and cover that with a valid fixture.
- [x] Implement matching bounded C decode/encode and compare every fixture byte-for-byte.
- [x] Run default and sanitizer C suites plus Rust format, Clippy, and workspace tests.

**Acceptance:** Both languages share a documented, deterministic extension of wire v1;
bad configuration cannot partially mutate live state or poison the next frame.

**Evidence (2026-08-04):** IDs 9-12, error codes 10-14, status keys 16-20,
configuration/event enums, independent config/data revision handshakes, at-most-once
events, host-owned staleness, and the three template field registries are frozen in
`docs/protocol/v1.md`. The generator now produces 26 valid and 18 invalid fixtures. The
maximum-capacity config is 931/2034 payload bytes, and regeneration into a clean directory
has no diff. Rust workspace tests (23 tests), format, and Clippy with warnings denied pass;
plain-C default and ASan/UBSan suites pass and C re-encodes every valid Rust fixture
byte-for-byte. The integrated ESP-IDF 5.5.5 build succeeds at `0xb94e0` bytes with
`0x46b20` bytes (28%) free in the smallest app partition.

---

### Task 2: Fixed-capacity firmware widget model and field resolution

**Files:**
- Create: `firmware/main/core/widget_model.c`
- Create: `firmware/main/core/widget_model.h`
- Create: `firmware/main/core/template_fields.c`
- Create: `firmware/main/core/template_fields.h`
- Create: `firmware/host_tests/test_widget_model.c`
- Create: `firmware/host_tests/test_template_fields.c`
- Modify: `firmware/host_tests/Makefile`
- Modify: `firmware/main/core/link_state.c`
- Modify: `firmware/main/core/link_state.h`

- [x] Test-drive complete-config validation into staging storage, followed by one atomic
  swap only after all references, capacities, templates, size classes, and field names
  pass.
- [x] Store widget/screen definitions in fixed arrays with no allocation based on host
  lengths. Preserve the last good model and active screen on a rejected revision. The
  staging model is statically allocated alongside the live model — it must never sit on
  the protocol task stack, which already carries 4,336 bytes of nested encode frames
  inside its 8 KiB budget.
- [x] Resolve scalar `PushData` fields by matching keys against the active template's
  declared field-name table, and emit a compact dirty-field set so updates patch objects
  rather than rebuilding screens. Unknown keys are counted and ignored, not errors.
- [x] Define per-template required/default/error values and deterministic handling for
  missing, wrong-type, or stale fields.
- [x] Test carousel wraparound, screen selection after config replacement, global data
  revision ordering, host-timeout fallback, and reconnect replay from an empty boot.

**Acceptance:** Native tests prove model/config correctness, atomic replacement, bounded
storage, and incremental field resolution without ESP-IDF or LVGL.

**Evidence (2026-08-04):** `widget_model_t` owns two fixed config buffers, two matching
banks of per-widget field state, and one shared resolution scratch buffer; publication is
an index swap only after the complete candidate and default field bank validate. The
host ABI measures the whole model at 41,416 bytes and both host and Xtensa builds enforce
a 48 KiB compile-time ceiling. Config replay is idempotent only when the same revision is
semantically identical; rejected/stale configs preserve the live model, active screen,
and data. The field resolver publishes a 16-bit dirty mask, applies full snapshots
atomically, implements all documented defaults/ranges, rejects missing/wrong fields
without consuming the global data revision, and counts/ignores unknown names. Five
plain-C host binaries now cover config replacement, carousel wrap, active-screen
preservation, global cross-widget revisions, empty boot, timeout/reconnect retention,
template defaults, stale/error fields, and invalid relationships. Default and
ASan/UBSan suites pass; Rust regressions, format, Clippy, and the ESP-IDF 5.5.5 build
also pass (`deskmate.bin` `0xb94e0`, 28% app partition free).

---

### Task 3: LVGL template library and status strip

**Files:**
- Create: `firmware/main/ui/template_view.c`
- Create: `firmware/main/ui/template_view.h`
- Create: `firmware/main/ui/templates/digital_clock.c`
- Create: `firmware/main/ui/templates/progress_ring.c`
- Create: `firmware/main/ui/templates/row_list.c`
- Create: `firmware/main/ui/status_strip.c`
- Create: `firmware/main/ui/status_strip.h`
- Create: `firmware/main/ui/ui_command_queue.c`
- Create: `firmware/main/ui/ui_command_queue.h`
- Create: `firmware/main/ui/ui_runtime.c`
- Create: `firmware/main/ui/ui_runtime.h`
- Modify: `firmware/main/ui/clock_screen.c`
- Modify: `firmware/main/ui/clock_screen.h`
- Modify: `firmware/main/CMakeLists.txt`

- [x] Define a small template-view interface for create, patch fields, set stale/error
  state, activate/deactivate, and destroy; keep template-specific object graphs private.
- [x] Implement `full` and `standard` layouts for all three templates. **`tile` is
  deferred to M4 with the dashboard grid**: dashboard screens are out of M2 scope, so no
  M2 screen can host a tile, and tile layouts would ship verified only by the compiler.
  The size-class enum still reserves `tile` on the wire; templates declare they do not
  support it yet.
- [x] Build the status strip as an overlay showing local time, connection dot, and
  interrupt-state dots. `full` hides it; `standard` reserves its height.
- [x] Marshal model snapshots/dirty updates through a **fixed-capacity UI command queue**,
  with coalescing/backpressure behavior documented and measured. Do not use
  `lv_async_call()`: it allocates per call and can fail under exactly the burst conditions
  Task 8 stress-tests, which violates the no-allocation-on-host-input rule in `CLAUDE.md`.
- [ ] Prove repeated field patches do not recreate the screen/object tree and that
  config replacement deletes old objects without heap loss.

**Acceptance:** Built-in templates render all supported sizes, update in place, and keep
the status strip and standalone fallback independent of host-provided content.

**Implementation evidence (2026-08-04):** All three templates build full and standard
object graphs behind the template-view interface, share explicit stale/error state, and
patch existing labels/arcs in LVGL context. The standard layout reserves a 32-pixel
strip with local time, connection state, and active/pending interrupt dots; full hides
it. A four-slot, 10,272-byte fixed command queue coalesces same-widget snapshots and
scalar state, preserves scalar ordering across screen replacement, drops the newest
non-coalescible command under pressure, and exposes drop/coalesce/high-water counters.
The complete publish/queue/consume mailboxes have a 16 KiB compile-time ceiling and no
`lv_async_call()` remains. Connection timeout now crosses this queue and replaces a host
template with the independent standalone clock. Queue tests pass under default and
ASan/UBSan builds; the latest Task 4-integrated ESP-IDF 5.5.5 image links at `0xbdb80`
bytes with 26% free. The unchecked lifecycle item still needs physical
repeated-patch/config-swap heap evidence;
compiler success is not being treated as that proof.

---

### Task 4: Carousel, gestures, touch events, and priority interrupts

**Files:**
- Create: `firmware/main/core/navigation.c`
- Create: `firmware/main/core/navigation.h`
- Create: `firmware/main/core/device_event_queue.c`
- Create: `firmware/main/core/device_event_queue.h`
- Create: `firmware/main/core/interrupt_state.c`
- Create: `firmware/main/core/interrupt_state.h`
- Create: `firmware/host_tests/test_navigation.c`
- Create: `firmware/host_tests/test_interrupt_state.c`
- Create: `firmware/host_tests/test_device_event_queue.c`
- Create: `firmware/main/ui/carousel.c`
- Create: `firmware/main/ui/carousel.h`
- Modify: `firmware/main/link/protocol_task.c`
- Modify: `firmware/main/link/protocol_task.h`
- Modify: `firmware/main/main.c`

- [x] Test-drive horizontal swipe classification with distance/time thresholds, edge
  cases, rotation-correct coordinates, carousel wrap behavior, and tap-vs-swipe routing.
- [x] Emit bounded `DeviceEvent` frames through a firmware event queue. Define overflow
  behavior/counters and ensure USB backpressure cannot block an LVGL callback.
- [x] Add immediate local pressed/paused feedback while the host owns authoritative
  pomodoro state; reconcile the next host data update deterministically. Because
  `DeviceEvent` is at-most-once, a dropped tap must resolve on the next authoritative
  update rather than leaving the device latched in a local state the host never entered.
- [x] Drive progress-ring motion locally between pushes. The host sends running state plus
  a remaining-duration field, and the device animates the ring itself; it must not require
  a push per rendered step. This keeps animation ownership in firmware per the spec and
  keeps the ring smooth at a low push cadence.
- [x] Implement priority take-over, dismissal, timeout policy if any, and restoration of
  the exact previous carousel screen. Test duplicate/stale interrupt tokens.
- [x] Keep status/link polling responsive during gestures, animations, and event bursts.

**Acceptance:** Swipes navigate locally, taps reach the host once, and a timer-finished
interrupt dismisses back to the previous screen without blocking render or protocol tasks.
Swipe/tap classification, event-queue overflow, and interrupt state are verified on the
host suites here; the end-to-end "reaches the host" half of this acceptance can only close
once Task 5 lands the demultiplexing session, so leave it open across both tasks.

**Firmware/host-test evidence (2026-08-04):** Gesture classification freezes logical
tap/swipe distance and duration thresholds and covers both landscape rotations,
dominance/edge cases, carousel wrap, and tap-vs-swipe routing. LVGL callbacks enqueue
into an 8-slot, 808-byte at-most-once event queue; pressure drops newest after consuming
its sequence, making the gap observable. The protocol task processes at most two events
per 50 ms read poll, never writes USB in LVGL context, drives local navigation/model
activation, and publishes request-ID-zero events with a 10 ms write deadline. Interrupt
tests cover exact replay, stale/conflicting tokens, active plus FIFO pending, Busy retry
without token consumption, config clearing, promotion, and exact saved-screen
restoration; M2 chooses explicit tap dismissal with no automatic timeout. Progress rings
advance from the last authoritative snapshot on a 250 ms monotonic LVGL tick and
optimistic tap feedback is overwritten by the next full snapshot. Reconnect restores
retained content, while the offline UI command has priority over queued view work and
returns to standalone. Nine plain-C host binaries pass default and ASan/UBSan builds;
the integrated image links at `0xbdb80` bytes with 26% free. Task 5 still must prove
host-side event demultiplexing and the complete physical tap-to-host path.

---

### Task 5: Long-lived Rust device session

**Files:**
- Modify: `companion/crates/device/src/lib.rs`
- Create: `companion/crates/device/src/session.rs`
- Modify: `companion/crates/device/Cargo.toml`
- Modify: `companion/crates/deskmate-cli/src/main.rs`

- [ ] Introduce a connection/session loop that owns serial I/O and demultiplexes echoed
  request replies from request-ID-zero events while preserving M1 timeout/error classes.
- [ ] **Emit a keepalive heartbeat whenever no other request is in flight**, at an interval
  well inside the frozen 10,000 ms liveness deadline (3 s or less). Without it a screen
  that is not being pushed — a paused pomodoro, a calendar between refreshes — goes silent
  past the deadline and the device drops to the standalone clock in the middle of a live
  session. Test that an otherwise idle session keeps the device online indefinitely.
- [ ] **Seed the session's data revision from the connect-time `StatusResponse`** so a
  restarted host does not collide with the revision the device still retains. Test a
  reconnect against a device that was never power-cycled.
- [ ] Test fragmented/coalesced reply+event reads, an event arriving before a reply,
  duplicate/out-of-order events, disconnect during a request, bounded event-queue
  overflow, reconnect, and state replay.
- [ ] Expose typed `apply_config`, `activate_screen`, `trigger_interrupt`, event receive,
  and reconnect APIs; keep raw serial logic out of the CLI.
- [ ] Re-run the M1 CLI and hardware-acceptance suites to prevent protocol regressions.

**Acceptance:** One long-lived connection can issue commands and receive touch events
without response correlation failures, then reconnect and replay state after unplug.

---

### Task 6: ICS provider and host widget engine

**Files:**
- Create: `companion/crates/providers/Cargo.toml`
- Create: `companion/crates/providers/src/lib.rs`
- Create: `companion/crates/providers/src/ics.rs`
- Create: `companion/crates/providers/tests/fixtures/`
- Create: `companion/crates/engine/Cargo.toml`
- Create: `companion/crates/engine/src/lib.rs`
- Create: `companion/crates/engine/src/pomodoro.rs`
- Create: `companion/crates/engine/src/interrupts.rs`
- Modify: `companion/Cargo.toml`

- [ ] Define the provider output/refresh trait needed by the M2 CLI and parse ICS file
  and URL sources into the next bounded events using fixture-driven tests.
- [ ] Cover folded lines, time zones/UTC, all-day events, cancellation, malformed feeds,
  duplicate UIDs, and stable sorting. Preserve last-good output with an age/error marker
  on refresh failure.
- [ ] **Bound recurrence support explicitly rather than aiming at full RRULE.** M2
  supports `FREQ=DAILY|WEEKLY|MONTHLY` with `INTERVAL`, `COUNT`/`UNTIL`, `BYDAY` without
  ordinal prefixes, and `EXDATE`. Anything else — `BYSETPOS`, ordinal `BYDAY`, `BYMONTHDAY`
  and friends, `RECURRENCE-ID` overrides, `FREQ=YEARLY` — is skipped and counted, never
  guessed at. The counter is reported by the CLI so an unsupported rule is visible rather
  than a silently missing meeting. Every supported and skipped form gets a fixture.
  Widening this set is M4 work, not M2 scope creep.
- [ ] Implement a monotonic-clock pomodoro state machine with start, pause, resume,
  reset, completed state, periodic progress fields, and exactly-once completion interrupt.
- [ ] Implement host-side interrupt arbitration/token tracking compatible with the
  device's bounded interrupt state and reconnect replay.
- [ ] Keep wall-clock jumps/time-sync separate from pomodoro elapsed timing.

**Acceptance:** Deterministic tests turn ICS fixtures and pomodoro transitions into the
exact bounded field/config/interrupt messages expected by firmware.

---

### Task 7: M2 CLI demos

**Files:**
- Modify: `companion/crates/deskmate-cli/src/main.rs`
- Create: `companion/examples/m2-carousel.json`
- Modify: `README.md`

- [ ] Add scriptable commands to apply/inspect the sample layout, select a screen, push
  clock/calendar data, run an interactive pomodoro session, trigger an interrupt, and
  print device events as human text or JSON lines.
- [ ] Validate the entire config locally before opening serial; return stable nonzero
  exits for config/provider/device/rejection failures.
- [ ] Make the sample config demonstrate all three templates, both strip modes, stable
  template field names, ordered carousel screens, and tap actions.
- [ ] Ensure Ctrl-C closes cleanly and never leaves the device permanently online or in
  an undismissable interrupt state.

**Acceptance:** A clean terminal can reproduce every M2 feature without Tauri or hidden
test-only serial code.

---

### Task 8: M2 hardware exit and handoff

**Files:**
- Modify: `docs/hardware/board-notes.md`
- Modify: `docs/superpowers/plans/2026-08-03-deskmate-roadmap.md`
- Modify: `CLAUDE.md`
- Modify: `README.md`
- Create: M3 implementation plan

- [ ] Run all C default/sanitizer tests, Rust format/Clippy/tests, fixture regeneration
  diff, clean ESP-IDF build, image-size check, and stack-frame audit.
- [ ] Flash through the normal native path and run the sample CLI config from automatic
  discovery; repeat a second automatic flash from the running M2 image.
- [ ] Human-verify clock, progress ring, calendar rows, status strip, full/standard
  layouts, 90/270-degree landscape rotation, swipe/tap distinction, and responsive
  animations.
- [ ] Complete a real accelerated pomodoro: tap start/pause/resume, observe progress,
  receive one completion interrupt, dismiss it, and return to the prior screen.
- [ ] Exercise valid and malformed config/data/event traffic, at least 100 config swaps,
  1,000 field patches, event-queue pressure, and confirm recovery plus flat heap trend.
- [ ] Unplug/replug at least ten times while the M2 CLI session reconnects; confirm the
  device falls back after 10 seconds and replays time/config/data without reboot.
- [ ] Leave a session connected and idle — no config, data, or interrupt traffic — for at
  least 60 seconds and confirm the carousel stays online on the keepalive alone.
- [ ] Restart the CLI process without touching the cable and confirm the new session
  reads the retained revision and pushes successfully instead of taking `StaleRevision`.
- [ ] Run a 30-minute mixed render/protocol soak and record heap floor, event/UI queue
  pressure, parser counters, frame rate/responsiveness observations, and resets.
- [ ] Update status/docs, write the M3 plan from measured findings, and mark M2 complete.
- [ ] Create an `m2` tag only after review fixes and only with explicit authorization.

**M2 exit gate:** The physical device demonstrates the three first widgets from a clean
CLI session, local interaction and host state remain synchronized, malformed input and
disconnects preserve the standalone clock, and repeated updates/config changes do not
leak or stall.
