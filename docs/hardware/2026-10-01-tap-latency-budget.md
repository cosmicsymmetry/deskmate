# C1 tap latency budget — ROD-14

Status, 2026-10-02: server reductions implemented on `track-c1/tap-latency`, pending
owner review, merge and deployment. Config v10, protocol v2 and firmware are unchanged.
The VM was accessed read-only. This report distinguishes **live journal**, **local
loopback bench**, and **historical glass** evidence. No new panel observation was made.

## Historical glass: the target remains 250 ms

Eight staged taps on dev-0005, at 270 degrees, took **300–570 ms release to stable
pixels, median approximately 420 ms** on 2026-09-30 ([board notes](board-notes.md)).
Contact to stable was 870–1130 ms; finger dwell is excluded from the system budget.
Video is 30 fps, with release uncertainty ±2 frames and face-change uncertainty ±1
frame. First new pixels to stable took approximately 30–70 ms (one or two frames).
The historical median gap to 250 ms is about **170 ms**.

These measurements predate this branch. Neither the new local 5.0 ms median nor the
local saving can be subtracted from that old 420 ms glass median. There is still no
successful live tap with which to allocate the complete deployed budget, and no
measurement proving that the 250 ms target has been reached.

## Live journal: one tap, dropped before selection

Read-only capture of all retained service entries since `2026-10-01 00:00:00 UTC`
returned 22,405 lines, spanning 00:10:40–21:12:34 UTC on October 1. There was exactly
one `device tap received`: **dev-0005, generation 1, sequence 2, picture-2**, at
20:07:40.493569 UTC. The requested 20:07:35–20:07:50 window contains the same tap.
The source did not declare a tap, so it was correctly dropped. Later background
scenes are not attributed to this event.

| Stage | `unix_us` | Delta from preceding observed stage |
|---|---:|---:|
| Received | 1790885260493569 | — |
| Dispatched by runtime | 1790885260516674 | 23.105 ms |
| Routing started | 1790885260517397 | 0.723 ms |
| Selector | N/A: source has no face declaring a tap | N/A |
| Staged lookup | N/A | N/A |
| Decision: dropped | 1790885260517427 | 0.030 ms from routing start |
| Image update queued | N/A | N/A |
| PushScene send_start | N/A | N/A |
| PushScene sent | N/A | N/A |
| Scene ACK | N/A | N/A |

Receive → drop was **23.858 ms**; routing finished at 1790885260517433. The logged
monotonic `queue_us=4776` starts before the dispatched trace and includes tracing /
callback work. It is not the 0.723 ms wall-clock difference between the dispatched
and routing-start trace sites. This sample supports investigating the idle runtime
wait; it supplies **no live selector, persistence, scene or ACK duration**.

## Review corrections after `b45332e`

Independent review reproduced reboot residency loss, a full-pool replacement wedge,
and an orphan runtime after handle drop. The initial tests did not cover these cases.
All six findings are addressed in `61ab3de`, `bc6b293` and `544dd69`; this section supersedes the original claim
that discarding every superseded render preserved tap counts.

1. **Connection residency:** discard every confirmed digest on link loss and before
   connecting again. Tests reboot an asset-tracking peer and require recovery both
   through the ordinary visible-scene path and an off-screen staged update. No scene
   may reference a missing digest.
2. **Replacement room:** before uploading under pool pressure, release obsolete
   frames using the complete desired set plus the last accepted scene's digests.
   Updating residency after release removes known dead digests; it never treats a
   requested keep-set as proof of an upload. Full sync, scene repair and individual
   updates use this rule. The exact reviewer sequence starts with fifteen residents,
   replaces a staged frame and the resting frame before reconciliation, then selects
   the replacement: no Busy response, no live-digest eviction. Fourteen is also tested.
   A release may now precede a nonresident update under pressure; any flash-compaction
   cost on the deployed device is unmeasured. Already-resident staged taps still skip
   upload and release entirely.
3. **Consumed taps:** a superseded fallback retries its consumed batch against the
   latest durable state, instead of discarding that count. Its successful result
   selects the resting slot, including an unchanged-pixels result. The competing
   staged tap finishes while the first render is blocked; two taps ultimately persist
   page two and show its matching frame. Superseded background renders have no taps
   to replay and are discarded. Continuous taps may force more than one render retry.
4. **Worker lifetime:** the event callback holds a weak sender. The test blocks device
   I/O, fills a one-entry command queue with a wake, drops the handle, then releases
   I/O and requires the device/worker to be dropped.
5. **Idle reaping:** a weak child monitor polls exit every 100 ms, reaps the child and
   signals the idle I/O loop to exit. The test keeps the Worker object alive, sends no
   subsequent request, and waits for both cleanup events. Pipe readers are still
   never joined, preserving inherited-writer protection.
6. **Staging ceiling:** a six-view fixture must stage page four but neither page five
   nor six. Deleting the `.take(...)` limit must fail this test; the old four-view
   fixture could only prove availability, not the upper bound.

## Reproducible local bench

`companion/crates/server/src/data_cards/latency_bench.rs` contains an ignored bench
using a local Axum server, its real authenticated device WebSocket route, the normal
runtime, durable account files, and a simulated protocol-v2 device. The real Bun
faces package selects and renders RSS from 16 captured headline entries; no external
feed is fetched. The peer records tap-send → matching PushScene-receive, acknowledges
requests, tracks resident digests and rejects scenes referencing absent assets.

Run from `companion/`, after installing its faces dependencies:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
RUST_LOG=server::tap_latency=info cargo test -p server --lib tap_latency_bench \
  -- --ignored --nocapture > /tmp/tap-latency-bench.log 2>&1
result=$?
rg 'BENCH (sample|summary)' /tmp/tap-latency-bench.log
exit "$result"
```

Each mode measures **35 isolated taps**, with initial state restored outside the timed
interval and varying idle phase. Staged mode selects page two. Forced-fallback mode
selects page four after explicitly evicting that view from the server cache, waiting
for the preceding render and staging pass to finish before resetting. Final page-four
mode uses the actual refresher to stage page four and connects the peer afterward to
prove it is resident. Setup and preload are excluded. The first tap is included: the
fixture loads the catalog lazily, whereas production loads it before listening.

All numbers below are **local Mac loopback, debug Rust 1.98.0 and Bun 1.3.8**, in
milliseconds (Apple M5, arm64, macOS 26.6.2).
They include filesystem syncs and host scheduling; no tunnel, board, LVGL, scanout or
finger timing is represented. p95 is the nearest-rank 34th observation of 35. These
are sequential diagnostic runs, not controlled claims about production savings.

| Implementation measured | Staged median / p95 | Forced fallback median / p95 | Resident page four median / p95 |
|---|---:|---:|---:|
| Baseline (`ef371b0`, bench only) | 79.484 / 105.094 | 268.923 / 295.932 | Unstaged; fallback path |
| Retained selector (`13d1f2c`) | 23.409 / 39.028 | 214.517 / 274.113 | Unstaged |
| Add event wake (before resident shortcut) | 8.726 / 15.060 | 198.570 / 267.409 | Unstaged |
| Add resident shortcut and ordering guards (`bbcb8d7`) | 6.410 / 8.927 | 196.622 / 236.864 | Unstaged |
| Add bounded fourth-page staging (`bbdad23`) | 4.953 / 6.181 | 188.984 / 207.389 | 5.919 / 76.651 |
| Pre-review, including selector retirement (`6259c03`) | 6.740 / 20.698 | 193.858 / 475.995 | 5.232 / 61.545 |
| After all six review fixes (`3cf03f8`) | **5.025 / 6.768** | **193.749 / 237.121** | **5.580 / 6.369** |

Baseline was explicitly rerun using the pinned Rust toolchain. The initial diagnostic
run from the repository root selected a different compiler and is excluded above.
The event-wake row was captured before adding the resident shortcut; both landed in
`a142aeb`. The pre-review run includes all outliers: first staged tap 193.553 ms; first
fallback 822.858 ms and second 475.995 ms. Several page-four taps took 47–66 ms while
the selector itself was about 0.3 ms. These outliers remain in the distribution. The 475.995 ms fallback trace contains
about 435 ms rendering, 32 ms commit-to-notification and 1.9 ms selection; the cause
of the render slowdown is unproven. Scheduling and inherited-pipe delay were not
isolated, so this is not a causal regression measurement. No isolated
latency saving is attributed to the graceful-retirement fix.

The rerun after review used the same bench, without concurrent builds, and completed
in 37.81 seconds. Its cold first staged tap was 140.968 ms, first fallback 290.703 ms;
these remain included. Slowest page-four tap was 6.641 ms. Relative to baseline, the
latest staged median is **74.459 ms lower (93.7%)**; fallback median is 75.174 ms lower.
Resident page four is 5.580 ms versus forced fallback 193.749 ms, a 188.169 ms local
difference. The lower p95 in this run does not isolate the cause of the earlier render
slowdown or prove a production regression was repaired by these correctness fixes.
Pre-rendering and residency move work ahead of the tap; capacity exhaustion still
uses fallback. No new glass measurement was made.

### Local segment medians and decisions

Segment medians below are calculated independently and do not necessarily sum to
end-to-end medians. The bench endpoint is peer receipt; the trace endpoint below is
server `send_start`.

| Staged segment | Baseline | Retained selector | Pre-review | After review |
|---|---:|---:|---:| ---: |
| Receive → dispatched | 14.278 ms | 15.464 ms | 0.101 ms | 0.053 ms |
| Dispatched → routing started | 0.041 ms | — | 0.047 ms | 0.029 ms |
| Selector | 59.975 ms | 0.345 ms | 0.517 ms | 0.323 ms |
| Staged lookup itself | 0.003 ms | — | 0.003 ms | 0.001 ms |
| Lookup → decision (includes synchronous persistence) | 4.455 ms | 4.624 ms | 5.027 ms | 4.365 ms |
| Decision → queued notification | 0.100 ms | — | 0.062 ms | 0.034 ms |
| Queued → PushScene send_start | 1.525 ms | 1.451 ms | 0.198 ms | 0.089 ms |
| Receive → PushScene send_start | 79.351 ms | 23.298 ms | 6.457 ms | 4.871 ms |

- **Retain only the selector.** Largest baseline cost: 59.975 ms selector → 0.345 ms
  in the first warm run; staged end-to-end median improved 56.075 ms. `tap-worker`
  is private newline JSON IPC between server and faces, not a device wire change.
  Rendering keeps a separate process and never acquires the selector mutex. Tests
  hold both background and fallback renders open and prove staged selection proceeds.
- **Wake the runtime on events.** A nonblocking hint uses its existing bounded command
  queue; the event queue retains its delivery policy. This removed the observed
  14–15 ms median idle wait locally (wake-run dispatch median 0.059 ms). Staged
  end-to-end median improved another 14.683 ms in that run. An integration test uses
  a three-second idle poll and requires the socket tap to finish within one second.
- **Send a resident selection directly.** Retain authoritative source/digest checks,
  then skip redundant AssetBegin/ACK for a digest confirmed on that connection.
  Notification → send fell to 0.182 ms in the first shortcut run (0.089 ms after review).
  The socket test requires zero AssetBegin and zero chunks for this selection.
  This also removes one device request/response exchange, whose live cost is unknown.
- **Keep state durable and ordered.** Lookup → decision costs roughly 4–5 ms. Moving
  persistence off the path would risk acknowledging a tap whose state was not saved;
  that tradeoff is not justified here. Keep atomic writes and fsync. A per-source
  transition lock serializes read/select/commit/notify, while a generation check drops
  older background renders completed after a newer staged selection; superseded tap
  renders rebase their consumed count against the new state. Eight simultaneous taps
  must advance through eight states, and reload must recover the eighth. Renders
  release the transition lock while drawing and cannot hold a tap behind a 45 s job.
- **Keep notification fanout synchronous.** The latest single-device decision → queue
  median is 0.034 ms. No measured multi-device fanout saving justifies changing it.
- **Keep the one-shot exit/EOF protection.** Retained selection no longer polls child
  exit. Fallback rendering still uses the established 10 ms exit poll and 250 ms EOF
  grace; its latest median render duration is 171.046 ms. Reducing at most one healthy
  poll interval is lower value than removing the render entirely for staged page four.
  An inherited-writer test proves the caller does not wait five seconds for EOF.
- **Stage page four within fifteen frames.** Admission under the image-store mutex
  reserves eight possible resting source frames and shares seven extra view slots
  account-wide; each refresher stages at most four views including rest. Concurrent
  refreshers cannot oversubscribe the account's desired set. Existing oversized caches
  are trimmed on load in memory without deleting files. This conservative reservation
  leaves some unused slots when fewer sources exist; a refused page uses fallback.

### Selector lifecycle and isolation

The worker inherits the same cleared environment as one-shot faces; it receives no
account directory or server credentials. Request state is passed explicitly, not
retained between calls. Limits are **64 KiB per request, 16 KiB per reply, five seconds
per exchange, 256 requests or 60 seconds per process**. The package also exits at
60 seconds while idle; Rust independently reaps that exit and stops its idle I/O
thread. The package retires after a reply when RSS reaches **128 MiB**. RSS is
a between-request retirement threshold, not an OS limit on transient allocation in
one request. Bounded input/output, the exchange deadline and process retirement bound
reuse; the pure built-in selector still shares the existing trust boundary of faces.

Normal retirement tells the Rust caller to replace the worker immediately. A crash,
broken pipe or unsupported worker verb falls back to a fresh one-shot `tap`; worker
startup is retried after a 60-second cooldown. Age-based replacement reads the package
again within the existing one-minute catalog cadence, so a faces-only deployment needs
no server restart. Tests replace the package in place, advance the retained worker's
age, and observe the new response; another proves memory retirement incurs no cooldown.

## Reading the instrumented journal

All new events use the explicit target **`server::tap_latency`**, at INFO. The default
`RUST_LOG=info` includes it; for a restrictive filter add `server::tap_latency=info` and
retain `server::runtime_device=info` for existing aggregate asset timings. Every new event
has `unix_us` sampled at the call site, independently of journal ingestion time. Durations
ending in `_us` use monotonic `Instant`. UTC subtraction assumes the server clock did
not step during the sample; do not confuse precision with accuracy.

Capture an isolated tap interval with:

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

## Device-side remainder and boundary costs

The live receive → send interval for a successful tap and the new release → stable
interval are still unknown. The next owner-run sitting should film isolated staged
and page-four taps with a calibrated clock while retaining the same journal window.
Report the residual only for those same taps:

`release→stable − (server receive→PushScene send_start)`.

That remainder contains both network directions and device work. Historical 76–88 ms
asset round trips cannot be halved into a measured one-way tap duration, and ACK means
scene acceptance, not stable pixels. The following are **proposals only**:

| Device/wire proposal | Expected saving and uncertainty | Boundary cost |
|---|---|---|
| Wake protocol task for an LVGL event, or reduce its 50 ms idle read wait to 5 ms | Up to 45 ms of the inferred idle wait; about 22.5 ms average only if arrivals are uniform. No on-board timing has measured it | Firmware change, owner-approved new image, USB flash and on-board OTA re-verification; inspect CPU/network behavior |
| Profile scene load, LVGL invalidation and panel blit scheduling before tuning | Historical first-new-pixels → stable is 30–70 ms. Only part might be recoverable; no exact saving claimed | Firmware timing/image work, USB flash, filmed panel check and OTA re-verification; batch with other firmware work |
| C2 on-device view selection with resident views | Removes the server/tunnel round trip from interaction. Amount unknown on the current live path | Wire and firmware design, owner approval, new image, USB flash and OTA verification; maintain server state/order semantics |
| Device monotonic timestamps for classification, send, load and blit | Diagnostic value; no direct nominal latency saving | Probably a wire amendment plus firmware image/USB/OTA cycle; server journal alone cannot supply these timestamps |
| Increase the 200 ms device send timeout | No proven nominal saving; may prevent loss during a stall | Firmware change; not a fixed 200 ms cost, so do not tune it to chase a fictional delay |

No firmware, schema or wire change, deployment, PR merge or physical verification is
part of this branch. The owner retains those decisions and the next panel sitting.

## Verification

All required local gates passed again after review on the pinned toolchain:

- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace --all-targets`: **854 passed**, zero failures; includes the
  server integration targets. Two intentional ignores: the existing contract-fixture
  printer and this opt-in benchmark (run separately for the measurements above).
- `cargo test --workspace --doc`: passed; no doctests defined.
- Web: **216 passed**, check, format:check and production build passed.
- Faces: **453 passed**, check, lint and format:check passed without lint findings.
- `git diff --check` passed. No firmware or protocol source changed; no local firmware
  flash, OTA or panel test was run.

**Initial implementation: 25 mutation probes were caught by test failures**, then restored. Compile errors
were not counted. Each row groups independently removed/bypassed guards:

| Guard mutations | Regression proving the failure |
|---|---|
| Rust request byte cap, response byte cap, missing newline | Oversized request bypasses retained worker; complete JSON one byte over response cap is refused; valid JSON without newline is refused |
| Rust exchange timeout, cleared environment, one-shot EOF grace | Blocked pipes return by deadline; retained process sees neither inherited HOME nor account directory; inherited writer cannot hold completion for five seconds |
| Rust retry cooldown, age limit, request count, graceful retirement | Crash uses fresh spawn and recovers; age/count replaces worker and loads changed package; memory retirement replaces it without disabling reuse |
| Source serialization, render generation check, tap generation increment | Eight simultaneous taps persist eight ordered states; staged taps finish during background/fallback rendering and their newer state/frame survive the old render |
| Runtime wake, resident shortcut | Socket tap beats the three-second idle timer and sends no AssetBegin/chunks |
| Shared staging admission, resting-slot reservation, load-time cache cap, fourth-page cap | Concurrent staging plus all eight resting sources stays at fifteen; legacy cache is bounded without deleting files; actual refresher stages page four |
| TypeScript count, RSS retirement, idle lifetime, framed request cap, unfinished request cap, response cap | Six `tap-worker.test.ts` boundary tests fail with the corresponding limit removed |

The first framing mutation exposed a weak timeout-only assertion; a separate test now
requires the actual framing refusal after EOF and catches that mutation. The restored
full suites passed afterward. The selector/render separation also has a direct test
that warms a selector, blocks a render using the same command, and receives another
selection before releasing the render.

**Review rework: ten additional mutations were caught**, with restored source:

| Removed behavior | Required regression failure |
|---|---|
| Clear residency on disconnect/connect | Rebooted peer cannot recover its visible scene or off-screen staged update |
| Reclaim before upload | Exact fifteen-frame replacement sequence returns Busy |
| Preserve live digests in release; record the accepted scene (two probes) | Pool test detects eviction of the live frame |
| Retry superseded consumed taps; select the fallback's frame (two probes) | Two-tap test retains page one, or persists page two while displaying page one's digest |
| Weak ownership of the wake sender | Full wake queue keeps the worker alive after handle drop |
| Reap idle child; stop idle I/O on exit (two probes) | Worker retained without further taps still owns the exited child or I/O thread |
| Four-frame `.take(...)` | Six-view fixture stages page five/six |

The full-suite run exposed an existing ownership test's 1.5-second detached check
racing the production two-second status poll (502 on config reapply). It passed in
isolation. The test now uses 20 ms status / 10 ms reconnect pacing; production timing
is unchanged. Existing pressed-pool and resume assertions were updated to enforce
the owner's pre-upload reclaim and live-frame preservation decisions.

**Main integration, 2026-10-02:** merged `origin/main` at `a886d6d` into the pushed
C1 branch with a merge commit. The only conflict was the roadmap: main's Track B row
is preserved byte-for-byte, and only C1's row is updated. PR #13's Squares/Dots faces,
tests, previews and Track B documentation are retained unchanged. All companion
gates passed again: Rust **854 passed** (two intentional ignores), web **216 passed**,
faces **469 passed**, plus fmt, Clippy, doctests, both typechecks, both linters, both
format checks and the web build. The Hacker News unstaged-page-four subprocess test
passed locally in 1.648 seconds; its Linux CI result still needs separate inspection.
No tap implementation or latency measurement changed during this merge.

GitHub CI for this branch is separate from these local results; inspect the PR checks
before merge. No deployment or new hardware verification is claimed.
