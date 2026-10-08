# Local taps and pipelined asset transfer

Status: approved by the owner 2026-10-08 ("let's improve the firmware and the protocol, I
approve the changes"). This crosses the wire and firmware boundaries; it is additive within
protocol v2, gated by two new capability bits, so there is no flash-first order.

## Why

Observed on `dev-0008` on 2026-10-08 on a home network with ~440 ms median round trip
(p90 1.7 s): pictures arrived at ~4 KB/s, transfers were abandoned after a 2 s chunk
timeout, and taps lagged. Both have one cause -- every byte and every tap waits a full
panel -> Cloudflare -> homelab round trip:

- `AssetChunk` is strictly one request in flight (`runtime_device.rs` `send_asset_chunk`
  -> `require_ack`), so throughput is 1920 B / RTT whatever the link bandwidth.
- A tap on a picture card is a device -> server -> selector -> `PushScene` round trip
  (`carousel.c` `emit_event` -> `data_cards::tapped` -> `select_staged_view`). C1 made the
  server half ~5 ms; the network half is the rest.

## Part 1: pipelined `AssetChunk` (capability bit 12, `PipelinedAssetChunks`)

**Device.**
- The net RX ring grows from 4 to 16 frames. It stays a heap allocation (PSRAM,
  internal fallback), so no `.bss` moves.
- The websocket data handler stops dropping bytes when the ring is full. It waits, bounded
  at 1 s, for the protocol task to drain, and that wait backpressures TCP. Overflow after
  the bound still drops and counts, as today.
- Chunks are processed in arrival order and each is acked exactly as today.
- The device advertises bit 12.

**Host.**
- To a device advertising bit 12, up to `ASSET_CHUNK_WINDOW = 8` `AssetChunk` requests of
  one transfer may be outstanding. Acks are matched by request id.
- An error or timeout on any chunk abandons the transfer exactly as today.
- `AssetBegin`, `AssetCommit` and every other message keep the one-outstanding rule, and
  events are still demultiplexed.
- Without bit 12 the window is 1, which is today's behaviour.

At 440 ms RTT a 60 KB frame drops from ~14 s to ~2 s.

## Part 2: local tap views (capability bit 13, `LocalTapViews`)

**Wire.**
- `PushScene` gains optional key 3, `tap_views`: an array of 1..4 scene maps, the views a
  tap steps through *after* key 2's scene. It also gains key 4, `tap_wrap` (bool, default
  true): whether a tap on the last view returns to the first.
- `DeviceEvent` (Tap) gains optional key 6, `view_index` (`u8`): the index the device now
  shows, where 0 is key 2's scene and n is `tap_views[n-1]`.
- Scenes in `tap_views` may name only assets already confirmed resident, the same rule as
  key 2.
- A host sends keys 3/4 only to a device with bit 13. A device without bit 13 never sees
  them, and a host still accepts a Tap without key 6.

**Device.**
- On a tap on a card whose last push carried `tap_views`, the device advances its local
  index and draws that scene immediately, from PSRAM, under the LVGL lock, before
  emitting the event. The event carries `view_index`.
- At the last index with `tap_wrap = false` the screen stays put and the event still goes
  out, so the host can answer with new content.
- A new `PushScene` for the card resets the index to the one it implies, 0.

**Host.**
- *Built-in faces with views* (Hacker News, weather, RSS, token): the ring is the
  sequence `onTap` produces from the current state, iterated until it repeats or 4 views,
  with each step's state kept. `tap_wrap = true`.
  - On a Tap with `view_index = k`, the host commits step k's state and selected view
    *without pushing a scene*, because the panel already shows it.
  - It repushes only when the data refreshes, and that push is then a no-op visually.
- *Plugins* (fetch-on-tap): the ring is `[current, prefetched next]` with
  `tap_wrap = false`. The server renders the next image in the background as a staged
  view.
  - On a Tap with `view_index = 1`, the host promotes next to current, renders a new next,
    and repushes `[current, new next]` with index 0. That is the same pixels on screen, so
    there is no flicker.
  - A second tap before the repush lands on the last index, emits an event, and is served
    by the render path as today.
- A Tap without key 6 (old firmware) takes today's path unchanged.

**Budget.**
- The existing resident budget holds: 15 frames, 8 current plus 7 staged views shared by
  the account.
- A plugin's prefetched next is one staged view. Ring views beyond the budget are simply
  not sent, so a shorter ring falls back to the server path at its end.

## Costs and verification

- Firmware: USB flash of a new image (`firmware/version.txt` bumped together with
  `DESKMATE_FIRMWARE_VERSION`). **Mandatory on-board OTA download check** after the flash,
  because the RX-ring change and the new decoder keys touch firmware.
- Gates: firmware host tests, `sanitize`, `idf.py build`, workspace Rust and faces suites,
  golden fixtures under `protocol/fixtures/v2/` for the new keys.
- On-board observations owed: transfer throughput at the measured RTT, and a tap that
  changes the face with the server unreachable (built-in face), each at one mounting
  minimum.
