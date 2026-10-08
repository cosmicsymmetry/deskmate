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
