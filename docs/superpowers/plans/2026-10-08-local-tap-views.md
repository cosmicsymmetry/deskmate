# Brief: local tap views (Part 2 of the local-taps spec)

Spec: `docs/superpowers/specs/2026-10-08-deskmate-local-taps-and-pipelined-assets-design.md`,
section "Part 2". Part 1 (bit 12, pipelined chunks) is already in the tree; build on it.
Read `CLAUDE.md`, `docs/protocol/v2.md` (including Part 1's amendment), then this brief.

## Current pipeline (verified file:line map, 2026-10-08)
- **Device tap:** `firmware/main/ui/carousel.c:50-76` `emit_event` turns a tap into
  `PROTOCOL_EVENT_TAP`. The only local effect is timer feedback (`scene_view_apply_local_action`).
  Validation is at `link/protocol_task.c:1752` (`validate_tap_event`).
- **PushScene:** `link/protocol_task.c:652` `dispatch_push_scene` shows the scene via
  `show_scene` under the LVGL lock, keeps `context->scene`, and acks.
  `sizeof(scene_t)` is ~6 KB (see `core/scene_model.h:277`).
- **Host tap:** `app-core/src/runtime/mod.rs:1291` maps it to the picture source, then
  `server/src/device_link.rs:74` calls `data_cards::tapped` (`data_cards.rs:893`).
  - The fast path is `select_staged_view` (`:936-1021`): the selector `faces_package::tap`
    returns `{view, state?}`, `image_sources.select_view`, `face_state.put`, a generation
    bump, then `notify_image_source_changed`, which leads to a PushScene.
  - Plugins (`views:false, selector:false`) re-render through the refresher with
    `event.taps`.
- **Staging:** `server/src/data_cards/worker.rs:359-420` stages up to 3 non-empty views
  from the `views` verb. `image_sources.rs:43-45` sets the budget at 15 resident frames,
  i.e. 8 current + 7 staged.
- **Scene build:** `app-core/src/runtime/scene.rs:48-71`; sent at
  `server/src/runtime_device.rs:728-742`.

## 1. Wire (docs + both codecs + fixtures)
- Capability bit 13 `LocalTapViews`. `PROTOCOL_CURRENT_CAPABILITIES` gains it.
- `PushScene` key 3 `tap_views`: an array of 1..4 scene maps, decoded with exactly the
  rules of key 2. Key 4 `tap_wrap`: bool, default true, present only with key 3.
- `DeviceEvent` key 6 `view_index`: `u8`, Tap only, optional.
- Canonical CBOR, golden fixtures in `protocol/fixtures/v2/` for both, used by the Rust
  and C tests and by the web serial codec test if it reads them.
- Document it as a dated additive amendment in `docs/protocol/v2.md`.

## 2. Firmware
- Decode `tap_views` into a heap allocation, PSRAM first (`heap_caps_malloc(...,
  MALLOC_CAP_SPIRAM)`), freed and replaced on the next PushScene for any card. **No new
  or larger statics, and do not grow the protocol message union by 4 scene_t.** Decode
  key 3 straight into the PSRAM buffer, or decode one view at a time. Bound everything;
  the input is untrusted. Hardware-independent decode lives in `core/` and is host-tested,
  including under `sanitize`.
- On a tap whose card is the live scene's card and has tap views, advance the local index:
  - with `tap_wrap`, cycle over 0..n;
  - without it, stop at n and still emit the event.
  Then show that scene immediately, on the LVGL context and under the existing locking
  rules (never mutate LVGL from protocol callbacks: hand it to the UI context as the
  existing code does), and emit the Tap event with `view_index`.
- A tap on a card without tap views behaves exactly as today, with no key 6.
- If showing a tap view fails (missing asset), keep the current screen, still emit the
  event without advancing, and let the host answer.

## 3. Host
- Rust protocol: encode keys 3/4 and decode key 6. Send keys 3/4 only when the device
  advertises bit 13.
- **Built-in faces with a selector:**
  - At staging time (`worker.rs`), compute the ring by calling the selector from the
    current state repeatedly. Each step yields `{view, state}`. Stop when the view
    repeats the start or after 4 steps.
  - Stage those views (budget rules unchanged; views beyond budget are not sent) and keep
    each step's state with it.
  - The PushScene for the card carries `tap_views` in ring order with `tap_wrap = true`.
  - On a Tap with `view_index = k` (k > 0): commit step k's state and selected view,
    exactly what `select_staged_view` commits today, *without* pushing a scene. Then
    recompute the ring from the new state. If the ring changed (data refresh), the next
    push carries it; do not push just to rotate.
  - `view_index = 0` after a wrap means back at the start: commit that.
- **Plugins:**
  - The ring is `[current, next]`. `next` is a prefetched render with `event.taps = 1`
    against the current state, staged as a view under the budget, with `tap_wrap = false`.
  - On Tap `view_index = 1`: promote next (its frame and returned state) to current,
    prefetch a new next, and push `[current, new next]`.
  - A tap that reports the last index again, or a Tap without key 6, takes today's
    render path.
- A device without bit 13 and a Tap without key 6 must behave exactly as today. That is
  covered by existing tests, which must stay green unchanged.
- **Tests:**
  - ring computation (stop on repeat, cap at 4, budget truncation);
  - commit-without-push on `view_index`;
  - plugin promote + prefetch;
  - bit-13 gating of keys 3/4;
  - codec round trips;
  - golden fixtures.
  Faces: if the selector needs a change to iterate, extend `companion/faces` with tests.

## Rules
Same as `2026-10-08-pipelined-asset-chunks.md`: no commit, no `idf.py`, do not touch
`firmware/version.txt`, run every gate you can, list what you could not, and report
files, tests, verbatim gate results and every unsettled decision.


## Implementation report — 2026-10-08

Status: implemented, uncommitted. No `idf.py`, firmware version change, flash,
deployment, or physical-board verification was performed. Hardware verification
and the listener-dependent gates below remain outstanding, not passing.

### Files

- `CLAUDE.md`
- `companion/apps/deskmate/src/lib/types.contract.ts`
- `companion/apps/deskmate/src/lib/types.ts`
- `companion/apps/deskmate/tests/serialCodec.test.ts`
- `companion/crates/app-core/examples/scene_panel_check.rs`
- `companion/crates/app-core/src/lib.rs`
- `companion/crates/app-core/src/runtime/link.rs`
- `companion/crates/app-core/src/runtime/mod.rs`
- `companion/crates/app-core/src/runtime/scene.rs`
- `companion/crates/app-core/src/scene_build.rs`
- `companion/crates/app-core/src/state.rs`
- `companion/crates/app-core/tests/runtime.rs`
- `companion/crates/app-core/tests/runtime/alerts_and_commands.rs`
- `companion/crates/app-core/tests/runtime/loop_and_events.rs`
- `companion/crates/app-core/tests/runtime/scene_delivery.rs`
- `companion/crates/app-core/tests/runtime/tap_regressions.rs`
- `companion/crates/device/examples/framebuffer_diff.rs`
- `companion/crates/device/src/replay.rs`
- `companion/crates/device/src/session.rs`
- `companion/crates/protocol/examples/generate-fixtures.rs`
- `companion/crates/protocol/src/lib.rs`
- `companion/crates/protocol/src/message.rs`
- `companion/crates/protocol/tests/fixtures.rs`
- `companion/crates/server/src/admin.rs`
- `companion/crates/server/src/app_api/contract.rs`
- `companion/crates/server/src/data_cards.rs`
- `companion/crates/server/src/data_cards/latency_bench.rs`
- `companion/crates/server/src/data_cards/local_taps.rs`
- `companion/crates/server/src/data_cards/worker.rs`
- `companion/crates/server/src/device_link.rs`
- `companion/crates/server/src/image_sources.rs`
- `companion/crates/server/src/runtime_device.rs`
- `companion/crates/server/src/runtime_device/chunk_tests.rs`
- `companion/crates/server/tests/hostile_device.rs`
- `companion/crates/server/tests/ownership.rs`
- `companion/crates/server/tests/tap_isolation.rs`
- `companion/faces/src/main.ts`
- `companion/faces/test/main.test.ts`
- `docs/hardware/board-notes.md`
- `docs/protocol/v2.md`
- `docs/superpowers/plans/2026-10-08-local-tap-views.md`
- `docs/superpowers/specs/2026-10-08-deskmate-local-taps-and-pipelined-assets-design.md`
- `firmware/host_tests/Makefile`
- `firmware/host_tests/test_device_event_queue.c`
- `firmware/host_tests/test_local_tap_views.c`
- `firmware/host_tests/test_protocol.c`
- `firmware/main/CMakeLists.txt`
- `firmware/main/core/device_event_queue.c`
- `firmware/main/core/device_event_queue.h`
- `firmware/main/core/local_tap_views.c`
- `firmware/main/core/local_tap_views.h`
- `firmware/main/core/protocol_message.c`
- `firmware/main/core/protocol_message.h`
- `firmware/main/link/protocol_task.c`
- `firmware/main/ui/carousel.c`
- `firmware/main/ui/scene_view.c`
- `firmware/main/ui/scene_view.h`
- `protocol/fixtures/v2/device_event_tap_view.bin`
- `protocol/fixtures/v2/manifest.txt`
- `protocol/fixtures/v2/push_scene_tap_stop.bin`
- `protocol/fixtures/v2/push_scene_tap_views.bin`
- `protocol/fixtures/v2/status_response.bin`
- `protocol/fixtures/v2/status_response_networked.bin`
- `protocol/fixtures/v2/status_response_ota_failed.bin`

The small changes to existing examples/test constructors supply the new optional
fields; they do not change the old test behavior. The Rust and C unknown-key scene
tests now use key 5 because key 3 is no longer unknown. Status fixtures and their
manifest were regenerated; three new fixtures pin wrapping views, nonwrapping
views and a Tap view index. The web serial status fixture pins 16352.

### Tests added

- Rust protocol: `local_tap_scene_bounds_defaults_and_round_trips` and
  `local_view_index_is_optional_u8_and_tap_only`, plus all three golden fixtures
  through both Rust and C byte-exact decode/re-encode tests.
- Runtime: `local_taps_are_gated_resident_and_do_not_repush_on_selection_or_wrap`:
  capability gating, asset commit before the ring push, selection/wrap without
  another push, and repeated terminal-index fallback.
- Server: ring repeat/step cap, budget truncation, migration from a full legacy
  staging budget, indexed state commit without notification, plugin promotion
  exactly once followed by prefetch, bit-13 wire gating, and queued indexes
  across a scene Ack (real socket actor over an in-memory stream).
- Firmware: local index wrap/stop, no redundant show at the terminal index,
  refused-show index preservation, count bounds, key-4-without-key-3 rejection,
  malformed nested tap scenes, primary-scene preservation, and queued events
  retaining their taps while obsolete index hints are cleared. The event type
  remains 64 bytes; the host event queue remains 552 bytes.
- Faces: zero-tap selector query and repeated stateful weather transitions.

### Decisions and limits

- **Truncated rings do not wrap.** The brief says built-ins wrap but the design
  also requires fallback past the resident budget. Those cannot both hold for a
  partial ring. Complete cycles wrap; capped/budget-truncated sequences stop and
  repeated terminal indexes use the existing selector/render path. This
  clarification is recorded in the spec and protocol amendment. The four-step
  limit means four additional scenes plus the primary scene, matching key 3.
- C decoding validates each nested view through the existing scene-map decoder,
  using the primary destination as scratch and then decoding the primary last.
  It retains only borrowed encoded-map slices in the union. Dispatch decodes
  into one heap array (base plus views), PSRAM first with internal fallback;
  replacement/refusal preserves correct ownership. No source static/global is
  added or enlarged. Binary layout is unverified without the forbidden build.
- The selector accepts `taps = 0` only as a current-view query; render's existing
  event clamping is unchanged. Every additional selector call receives the
  preceding returned state, including absent-vs-null state semantics.
- Local staging starts on demand from a capable runtime. Legacy-only accounts
  retain the existing staging/selection path. When local staging takes over a
  source, it preserves its selected frame as primary and reuses obsolete staged
  slots; otherwise an account already using all seven extras could never get a
  local ring. Account capacity remains fifteen frames.
- Each runtime retains the exact pushed ring for index interpretation; committing
  a built-in tap recomputes only the next-push ring, without notifying a scene
  update. The runtime updates in-memory selection immediately; durable commits
  are processed in device order on a bounded 64-entry routing queue off its
  thread. Generation plus selection-epoch checks discard stale render/prefetch
  work. Legacy selectors can resolve logical view names through the local ring.
- Plugins keep one staged next frame/state, promote it once and prefetch from the
  promoted state. Repeat terminal events and absent/unusable indexes take the
  existing path. Prefetch failure leaves normal server rendering available.
- All ring digests must be confirmed resident before PushScene. Only the visible
  digest is protected as live during later replacement; the indexed Tap updates
  that digest too. This preserves the fifteen-resident/one-incoming slot budget.
- Queued index hints cannot outlive their scene. Firmware clears their presence
  on accepted replacement without dropping the events; the host correlates event
  receive time with its scene Ack using transport-local metadata, not another
  wire key. Cross-boundary and unacknowledged indexes fall back instead of
  selecting a different ring's state.
- No additional product decision is pending for the implementation. Existing
  ten-second host-loss fallback to the standalone clock is unchanged; this work
  does not establish indefinite offline display of a host card. The requested
  offline local-tap observation, OTA download and throughput measurement still
  require a physical board session.

### Gate results

- `make -C firmware/host_tests clean test`: exit 0; raw log `/tmp/local-taps-firmware-test.log`.
- `make -C firmware/host_tests sanitize`: exit 0; raw log `/tmp/local-taps-firmware-sanitize.log`.
- `cargo fmt --all --check`: exit 0; raw log `/tmp/local-taps-fmt.log`.
- `cargo clippy --workspace --all-targets -- -D warnings`: exit 0; raw log `/tmp/local-taps-clippy.log`.
- `cargo test --workspace --all-targets --no-fail-fast`: exit 101; raw log `/tmp/local-taps-workspace-tests.log`.
- `cargo test --workspace --doc`: exit 0; raw log `/tmp/local-taps-doc-tests.log`.
- `bun test`: exit 0; raw log `/tmp/local-taps-web-tests.log`.
- `bun run check`: exit 0; raw log `/tmp/local-taps-web-check.log`.
- `bun run format:check`: exit 0; raw log `/tmp/local-taps-web-format.log`.
- `bun run build`: exit 0; raw log `/tmp/local-taps-web-build.log`.
- `bun test`: exit 1; raw log `/tmp/local-taps-faces-tests.log`.
- `bun run check`: exit 0; raw log `/tmp/local-taps-faces-check.log`.
- `bun run lint`: exit 0; raw log `/tmp/local-taps-faces-lint.log`.
- `bun run format:check`: exit 0; raw log `/tmp/local-taps-faces-format.log`.

Firmware plain and sanitized suites both exit 0. Verbatim relevant output:

```text
test_local_tap_views: OK
test_protocol: OK
test_device_event_queue: OK (552-byte queue, capacity 8)
```

No sanitizer diagnostics. Rust formatting exits 0 with no output. Clippy:

```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 12.06s
```

The full Rust run uses `--no-fail-fast` to collect every denied target. Aggregate:
**814 passed, 115 failed, 2 ignored**. All 115 failure blocks contain this exact
OS error; no assertion failure with another cause was observed:

```text
Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }
```

Server library result, verbatim:

```text
test result: FAILED. 403 passed; 7 failed; 2 ignored; 0 measured; 0 filtered out; finished in 15.93s
```

The failed targets are:

```text
`-p server --lib`
    `-p server --test accounts`
    `-p server --test companion_api`
    `-p server --test device_link`
    `-p server --test firmware_download`
    `-p server --test hostile_device`
    `-p server --test image_routes`
    `-p server --test isolation`
    `-p server --test ownership`
    `-p server --test tap_isolation`
```

The new targeted server tests pass, verbatim:

```text
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 405 filtered out; finished in 1.15s
```

All ten new Rust tests also pass in the final workspace run. Workspace doctests
exit 0 (all five crates currently report zero doctests). App checks/format/build all
exit 0; app test output:

```text
 242 pass
 0 fail
 1316 expect() calls
Ran 242 tests across 17 files. [3.48s]
```

Faces typecheck, lint and format all exit 0. Full faces test output:

```text
 808 pass
 7 fail
 7077 expect() calls
Ran 815 tests across 28 files. [31.12s]
```

The seven faces failures are confined to `test/http.test.ts`: four loopback
server startup failures and three subsequent teardown errors. Bun reports:

```text
error: Failed to start server. Is port 0 in use?
 syscall: "listen",
   errno: 0,
    code: "EADDRINUSE"
```

The directly affected faces main/selector suite passes:

```text
 18 pass
 0 fail
 177 expect() calls
Ran 18 tests across 1 file. [6.13s]
```

The sandbox forbids the needed listeners and its approval policy is `never`;
these integration checks need an unrestricted rerun. `idf.py`, firmware binary
layout checks, flashing, OTA, on-board throughput and offline-tap observations
were not run, as required by this brief. No commit was created.
