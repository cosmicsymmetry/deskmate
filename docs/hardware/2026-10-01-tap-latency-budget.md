# C1 tap latency budget — ROD-14

Status: server-only instrumentation implemented on `track-c1-frames-in-psram`.
No deployment, PR #7 merge, firmware change or new glass measurement is part of this
work. This follows the owner's 2026-10-01 direction to retain 250 ms and chase the gap.
The authoritative physical evidence is the 2026-09-30 entry in `board-notes.md`.

## What the evidence supports

Eight staged taps on dev-0005, at 270 degrees, took **300–570 ms release to stable
pixels, median approximately 420 ms**. Contact to stable was 870–1130 ms; finger dwell
is excluded from the system budget. Video is 30 fps, with release uncertainty ±2 frames
and face-change uncertainty ±1 frame. First new pixels to stable took approximately
30–70 ms (one or two frames). These are historical measurements, not measurements of
this instrumentation. The median gap to 250 ms is about **170 ms**, or 40% of 420 ms.

**No individual segment can honestly be named the measured dominant segment.** The
measured aggregate before the first new pixels is larger than the visible 30–70 ms
settling interval, but spans device scheduling, both network directions and server work.
There are no deployed timing samples from the new logs yet. Server-only changes may
help; there is insufficient evidence that they can close the entire gap.

| Segment | Number / classification | How to isolate it |
|---|---|---|
| Release → gesture classification | Unknown on the board; classification on LVGL release is inferred from `ui/carousel.c` | Needs device timing or synchronized external observation; server cannot see it |
| Classification → device WebSocket send | Unknown measured value; **0–50 ms idle read wait** is inferred from the protocol task's 50 ms read timeout before `process_device_events`. Not an upper bound under load | Device instrumentation would be required for an exact split |
| Device send → server receive | Unknown; historical 76–88 ms asset request round trips are not a one-way tap measurement | `device tap received` establishes the server endpoint only |
| Server receive → runtime callback → blocking routing start | Unknown live value; runtime maximum idle wait is **25 ms** by source, plus scheduling or in-flight work | Subtract receive, dispatched and routing-start `unix_us`; `queue_us` directly times blocking-pool handoff |
| Routing → selector → staged lookup | Unknown live value; selector is a subprocess even on a hit | `tap selector completed.elapsed_us`, `staged lookup completed.elapsed_us`, routing timestamps |
| Staged lookup → state persistence → runtime notification → socket send | Unknown live value; includes face-state persistence, fanout, runtime work and asset checks | Lookup, decision, `image update queued for runtimes`, and `PushScene phase=send_start` timestamps, matched by source/digest/card |
| Socket send → device decode / scene build → response | Unknown live value; journal can measure the **combined** request/response interval, not its one-way or device components | `send_start`, `sent`, and matching `visual request acknowledged`; `scene request completed.elapsed_us` also includes host queueing and ACK validation |
| Scene loaded → panel blit → stable pixels | Unknown internal timings. **30–70 ms first-new-pixels → stable** measured from video | Video remains necessary. ACK follows synchronous scene construction/loading, not confirmation of physical scanout |

A local diagnostic ran `bun run src/main.ts tap` for weather 20 times, with an empty
settings object and one tap. Every response selected `days`. End-to-end process times
were **44.32–474.79 ms, median 44.88 ms**; first invocation 474.79 ms, remaining 19
44.32–47.49 ms. This is measured on the development Mac during build activity, with no
server wrapper, no network and no device. It is not a live budget allocation. The Rust
wrapper additionally polls child exit every 10 ms and permits up to 250 ms for pipe EOF
on the documented inherited-pipe race; neither is a mandatory delay on every request.

## Reading the instrumented journal

All new events use the explicit target **`server::tap_latency`**, at INFO. The default
`RUST_LOG=info` includes it; for a restrictive filter add `server::tap_latency=info` and
retain `server::runtime_device=info` for existing aggregate asset timings. Every new event
has `unix_us` sampled at the call site, independently of journal ingestion time. Durations
ending in `_us` use monotonic `Instant`. UTC subtraction assumes the server clock did
not step during the sample; do not confuse precision with accuracy.

After the owner deploys this branch, capture the relevant interval with:

```sh
journalctl -u deskmate-server --since 'YYYY-MM-DD HH:MM:SS UTC' \
  --until 'YYYY-MM-DD HH:MM:SS UTC' -o short-iso-precise --no-pager
```

For one isolated tap, follow:

1. `device tap received`: device, connection generation, device event sequence, card,
   `taps=1`. This is after valid frame/message decoding and before event routing, so
   duplicate or queue-dropped events can appear here without a later callback. The
   existing wire carries one tap per event, not a coalesced count.
2. `tap dispatched by runtime` → `tap routing started`: a span retains account, card
   and source across `spawn_blocking`. The first callback excludes non-picture routing.
3. `tap selector completed` and `staged lookup completed`: selector success, duration,
   selected view and lookup hit. `tap decision` says `staged_hit`, `render_fallback` or
   `dropped`; a staged hit carries the digest after state persistence.
4. Fallbacks emit `tap render started/completed`, including the actual coalesced `taps`
   count, duration and success. Background renders do not masquerade as taps.
5. `image update queued for runtimes` bridges source/digest to the target device list,
   for both staged selection and rendered frame notifications. Empty targets are visible.
6. `PushScene` has `phase=send_start`, then `sent` or `send_failed`, with device,
   generation, request id, card, revision and image digests. Count **sent** entries only
   to check the one-scene claim. `sent` means the local socket sink completed; it is
   not delivery or pixels. Match the ACK by device/generation/request id. ACK type 19
   and revision identify the accepted scene; `scene request completed.ok` includes
   validation of the ACK type/revision. Rejections and timeouts are separate events.
7. `AssetBegin` and `AssetCommit` have the same socket phases, request ids and digest.
   Begin includes size/volatile and its ACK includes `already_present`. Existing
   `server::runtime_device` summaries retain chunks, bytes, slowest chunk, commit and
   transfer elapsed milliseconds. Transfer `total_ms` starts after Begin ACK, so add
   Begin time separately. Healthy chunks are not individually logged.

There is deliberately no invented wire correlation id. Use the link-established interval
as well as device/generation/request id (generation is process/runtime local), and join
source/digest/card for notifications. Concurrent taps, shared sources across devices,
coalescing and background refresh can make causal attribution ambiguous; use isolated
taps for the bench budget and retain the full journal window. A staged store hit does
not prove residency on the panel: any resulting asset transfer will now be visible.

The journal alone yields the server interval and the scene request/response envelope.
It **cannot** independently split device classify/send, upstream transit, downstream
transit, decode/blit and stable pixels. No existing message supplies the necessary device
subsecond timestamps. Even adding server logs cannot give the old video an absolute
subsecond clock. A future video must include an independently calibrated time reference
to split upstream and downstream glass intervals; otherwise report only the residual:

`release→stable − (server receive→PushScene send_start)`.

That residual combines both network directions and device work; **do not divide an RTT
in half and call it measured**. A complete journal-only device split would require new
device telemetry and probably a wire amendment. This work stops at that boundary.

## Candidate reductions and boundary costs

| Candidate | Possible saving / confidence | Cost and decision |
|---|---|---|
| Keep a bounded face selector worker warm | Removes repeated Bun/import startup; local warm process cost ~45 ms is a clue, not a live saving estimate | Server/package only; requires lifecycle, isolation, memory limits, concurrency and crash/reload semantics. Measure selector first; not implemented |
| Reduce subprocess exit polling latency | At most roughly one 10 ms polling interval in an otherwise healthy exit | Server only; changing pipe/exit handling must preserve the inherited-pipe protection. Too small alone for the 170 ms median gap |
| Wake runtime immediately on incoming events / avoid blocking-pool queue delay | Up to a 25 ms idle runtime wait, plus any measured queue contention | Host/server-side architecture, no schema/wire/firmware. Touches shared app-core scheduling; measure queue intervals before redesign |
| Reduce synchronous state-persistence or notification work | Only the measured lookup→notification/send interval is available to recover | Server only; needs durability and rapid-tap ordering tests. Do not speculate that filesystem work dominates |
| Stage page four with a capacity-aware allocation policy | Avoids fallback subprocess/render plus chunk round trips for later pages; does not improve already-staged taps | Server only if within existing 15-resident-frame budget; cannot safely just raise the flat cap for every source. `MAX_STAGED_FRAMES_PER_SOURCE=3` means page four is unstaged. ROD-13 fixes its wrong page, not its speed |
| Shorter device event polling / tune display scheduling | Potential part of the inferred 50 ms idle wait and measured display transition | Firmware boundary; new image, owner authorization and physical verification. Batch with any statics/layout changes and verify OTA; not implemented |
| On-device view switching (C2) | Can remove server and tunnel from the interactive path | Wire + firmware design, owner approval, batched image and OTA/bench verification; not implemented |
| Change device send timeout from 200 ms | No proven nominal latency saving; may prevent a reply loss during a stall | Firmware change. Host waits 2000 ms, but device `esp_websocket_client_send_bin` allows 200 ms. Neither is a fixed per-tap delay; deliberately unchanged |

No performance reduction is included in this batch. The safe first decision is to measure
selector, queue and send/ACK intervals on the deployed path before spending a firmware
session or changing face execution semantics. The owner retains PR #7 merge, deployment
and the next physical bench sitting. Instrumentation and this honest partial budget are
the completed scope of ROD-14, not a claim that 250 ms has been reached.

## Verification

All required Rust gates passed: `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace --all-targets`, and `cargo test --workspace --doc`.
The full test and doctest passes used `RUST_TEST_THREADS=1`. Two earlier parallel
all-targets attempts stopped at the unchanged app-core fixture
`push_gate_waiter_panics_when_the_gate_is_not_opened`: its 100 ms thread-start deadline
expired (`eventual=Ok(false)`). It passed in isolation and in the complete serial run;
no tests were skipped and no app-core code or test was changed.

The added log-contract test passes under `server::tap_latency=info`: it feeds an encoded
tap through the socket decoder and pins card, sequence, tap count and timestamp fields,
scene request correlation/digest, asset-start visibility and healthy-chunk silence.
Existing staged/fallback, wire, runtime and account-isolation regressions all pass.
Web: 216 tests, check, format:check and build passed. Faces: 431 tests, check, lint and
format:check passed (the pre-existing unused RenderContext import warning remains).
`git diff --check` passed. Firmware tests/OTA were not run: firmware was untouched.

CI inspection before committing found the latest existing C1 run, 36703555968,
successful; that is evidence for its older commit only, not these changes.
No new hardware or deployed-server verification is claimed.
