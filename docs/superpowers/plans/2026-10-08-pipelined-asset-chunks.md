# Brief: pipelined AssetChunk (Part 1 of the local-taps spec)

Spec: `docs/superpowers/specs/2026-10-08-deskmate-local-taps-and-pipelined-assets-design.md`,
section "Part 1". Read `CLAUDE.md` (repo rules, traps) and `docs/protocol/v2.md` first.

## 1. Protocol contract
- Add capability bit 12 `PipelinedAssetChunks` to `docs/protocol/v2.md`, as a dated
  additive amendment written like the 2026-10-02 brightness amendment. A host may keep up
  to 8 `AssetChunk` requests of ONE transfer outstanding to a device advertising bit 12.
  Every other message keeps one-outstanding.
- Rust: add the bit constant wherever `DisplayBrightness`/bit 11 is defined in
  `companion/crates/protocol`. Bump `PROTOCOL_CURRENT_CAPABILITIES` to include bit 12
  (4064 + 4096 = 8160), and update every place and test that pins 4064, including
  `CLAUDE.md`'s "Current state" line.
- Firmware: the same bit in the C capability definitions, advertised in the capability
  word.
- Update protocol golden fixtures under `protocol/fixtures/v2/` if a status/capability
  fixture pins the word. Keep the Rust and C fixture tests agreeing.

## 2. Firmware (`firmware/main/link/net_link.c`)
- `NET_LINK_RX_RING_CAPACITY` becomes 16 frames. It stays a heap allocation (PSRAM
  first, internal fallback, as now). **Do not add or grow any static/global**: firmware
  `.bss`/`.data` layout changes are the costliest boundary here (see CLAUDE.md traps). If
  a new semaphore is needed, create it at init into an existing static handle slot only if
  one exists; otherwise ask in your report and use the smallest possible change.
- In the `WEBSOCKET_EVENT_DATA` handler, when the ring cannot accept the bytes, wait
  (bounded, total 1000 ms) for the protocol task to drain space instead of dropping
  immediately. After the bound, drop and count as today. The drain side signals space.
  This runs in the websocket task, so blocking it backpressures TCP. Never block while
  holding the spinlock.
- Chunk processing and per-chunk Ack are unchanged.
- Host tests: if `core/net_ring` logic changes, extend `firmware/host_tests`. Hardware-
  independent logic stays in `core/`.

## 3. Host (`companion/crates/server/src/runtime_device.rs`, `companion/crates/app-core/src/asset_sync.rs`)
- Today `send_asset_chunk` does `connected_request` then `require_ack` per chunk, and
  `asset_sync.rs` loops chunks sequentially.
- Add a windowed path used only when the device advertises bit 12: send up to
  `ASSET_CHUNK_WINDOW = 8` chunks, then for each Ack received (matched by request id,
  in order) send the next.
- Any error/timeout (the existing 2000 ms per-request timeout, measured per chunk from
  its send) abandons the transfer exactly as today, with the same log lines and error
  type.
- `DeviceEvent`s arriving meanwhile must still be demultiplexed and delivered, exactly as
  during a single outstanding request.
- Without bit 12 behaviour is byte-for-byte today's.
- Find how the transport and request/response correlation work (`connected_request`,
  pending-request handling, event demux), and extend them minimally. Do not restructure
  unrelated paths.
- Tests:
  - a fake device that acks out of a window verifies at most 8 in flight;
  - correct ordering and offsets;
  - abandon on a mid-window error or timeout;
  - events delivered while chunks are in flight;
  - window 1 without the bit.
  `crates/server/tests/hostile_device.rs` and existing runtime tests show the fakes.

## Rules
- Do not commit. Do not run `idf.py` (sandbox denies it; the orchestrator builds
  firmware). Do not touch `firmware/version.txt`.
- Gates you can run:
  - `make -C firmware/host_tests clean test` and `make -C firmware/host_tests sanitize`;
  - from `companion/`: `cargo fmt --all --check`,
    `cargo clippy --workspace --all-targets -- -D warnings`, and
    `cargo test --workspace --all-targets`.
  Note: `export PATH="$HOME/.cargo/bin:$PATH"`, and server integration tests that bind
  loopback may be denied in your sandbox. Say which ones you could not run; do not mark
  them passing.
- Report: files changed, tests added (names), gate results verbatim, and every decision
  this brief did not settle.


## Implementation report — 2026-10-08

Status: implementation complete, uncommitted. No `idf.py`, firmware version change,
flash, deployment, or hardware verification was performed. Part 2 is not implemented.
The firmware build, binary layout comparison, on-board OTA download, and measured
throughput remain for the orchestrator/board session.

### Files changed

- `CLAUDE.md`
- `companion/apps/deskmate/src/lib/types.contract.ts`
- `companion/apps/deskmate/src/lib/types.ts`
- `companion/apps/deskmate/tests/serialCodec.test.ts`
- `companion/crates/app-core/src/asset_sync.rs`
- `companion/crates/app-core/src/runtime/link.rs`
- `companion/crates/app-core/src/state.rs`
- `companion/crates/protocol/src/lib.rs`
- `companion/crates/protocol/src/message.rs`
- `companion/crates/protocol/tests/fixtures.rs`
- `companion/crates/server/src/app_api/contract.rs`
- `companion/crates/server/src/runtime_device.rs`
- `docs/protocol/v2.md`
- `firmware/host_tests/test_protocol.c`
- `firmware/main/core/protocol_message.h`
- `firmware/main/link/net_link.c`
- `protocol/fixtures/v2/manifest.txt`
- `protocol/fixtures/v2/status_response.bin`
- `protocol/fixtures/v2/status_response_networked.bin`
- `protocol/fixtures/v2/status_response_ota_failed.bin`
- `companion/crates/server/src/runtime_device/chunks.rs`
- `companion/crates/server/src/runtime_device/chunk_tests.rs`
- `docs/superpowers/plans/2026-10-08-pipelined-asset-chunks.md`

The three status fixtures and their manifest were regenerated with the Rust fixture
generator. Historical brightness observations retain their historical capability word.
The pre-existing spec and the separately-created local-tap-views brief were left alone.

### Tests added

All five tests run the production socket actor over an in-memory WebSocket message
stream (no loopback listener or second correlation implementation):

- `runtime_device::chunk_tests::chunk_window_is_eight_with_ordered_offsets_and_event_delivery`
- `runtime_device::chunk_tests::chunk_window_correlates_out_of_order_acks_without_releasing_earlier_slots`
- `runtime_device::chunk_tests::chunk_window_abandons_on_mid_window_error_and_ignores_late_acks`
- `runtime_device::chunk_tests::chunk_window_timeout_is_measured_from_each_send`
- `runtime_device::chunk_tests::chunk_window_is_one_without_capability_and_eight_with_it`

Existing Rust/C capability and fixture assertions now pin 8160 and bit 12 (4096),
and the Web Serial fixture assertion was updated. `core/net_ring` is unchanged.

### Decisions not settled by the brief

- Replace the existing `s_rx_ready` binary-semaphore pointer with an event-group
  pointer of the same size. Two bits independently signal data and space; no source
  static/global was added or grown. The ring still allocates in PSRAM first with
  internal fallback. The 1000 ms budget covers the whole callback, including partial
  writes; only the final overflow reaches the existing drop counter. Binary layout
  and real websocket-task backpressure are not claimed verified by host tests.
- Add `RuntimeDevice::send_asset_chunks` with a sequential default. The WebSocket
  implementation checks its negotiated capability and submits one bounded transfer
  to its existing actor. It owns a copy of this transfer's wire bytes; other commands
  cannot overlap it. Existing AssetSync error type, failing offset, and log messages
  are preserved.
- Correlate out-of-order Acks by ID but release window slots in send order. Duplicate
  and unmatched Acks consume no slot. An error at any matching ID aborts immediately;
  late replies remain unmatched and cannot poison the following request.
- Measure chunk deadlines from send start and cap a stalled send at the earliest
  outstanding deadline. The synchronous whole-transfer wait is also bounded at
  chunk count × 2000 ms + the existing 2000 ms caller margin; each actor-side chunk
  still has its own 2000 ms deadline.
- Name the capability in the existing app-core/API/TypeScript capability contract,
  so the current advertised bit is not reported as unknown.
- Make the actor's stream/sink generic internally so the real production actor can
  be tested without sandbox-denied loopback sockets. Production still uses the same
  Axum WebSocket, decoder, event router, and request-ID allocator.

### Gate results

- `make -C firmware/host_tests clean test`: exit 0. Verbatim output includes
  `test_protocol: OK` and `test_net_ring: OK`.
- `make -C firmware/host_tests sanitize`: exit 0. Verbatim output includes
  `test_protocol: OK` and `test_net_ring: OK`; no sanitizer diagnostics.
- `cargo fmt --all --check`: exit 0, no output.
- `cargo clippy --workspace --all-targets -- -D warnings`: exit 0. Final line:

```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 14.09s
```

- `cargo test --workspace --all-targets`: exit 101, stopped at server's library
  tests. Verbatim result:

```text
test result: FAILED. 396 passed; 7 failed; 2 ignored; 0 measured; 0 filtered out; finished in 14.21s
```

- `cargo test --workspace --all-targets --no-fail-fast`: exit 101. Aggregate
  804 passed, 115 failed, 2 ignored. Every failure is a loopback-listener
  denial, with this verbatim OS error:

```text
Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }
```

The affected targets, verbatim:

```text
error: 10 targets failed:
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

The seven denied library tests are `data_cards::latency_bench::a_socket_tap_wakes_an_idle_runtime_before_its_next_tick`
and the six `egress::tests` real-connection tests (`post_form_enforces_the_body_cap_against_a_server_that_never_declares_a_length`,
`post_form_enforces_the_response_body_cap_over_a_real_connection`,
`post_form_resolves_then_pins_and_sends_the_form`,
`post_form_returns_a_redirect_as_itself_and_never_follows_it`,
`post_form_returns_the_response_status_for_a_non_2xx_response`, and
`reqwest_resolve_override_connects_to_the_pin_and_preserves_the_host_header`).
None of these denied tests is marked passing.

- Targeted runtime-device suite, verbatim:

```text
test result: ok. 34 passed; 0 failed; 0 ignored; 0 measured; 371 filtered out; finished in 0.41s
```

  All five new tests also pass in the final workspace run.
- `bun run check` in the companion app: exit 0.
- `bun test` in the companion app: exit 0. Verbatim result:

```text
 242 pass
 0 fail
 1316 expect() calls
Ran 242 tests across 17 files. [6.95s]
```

Full raw gate logs from this run: `/tmp/pipeline-firmware-test.log`,
`/tmp/pipeline-firmware-sanitize.log`, `/tmp/pipeline-fmt.log`,
`/tmp/pipeline-clippy.log`, `/tmp/pipeline-workspace-tests.log`,
`/tmp/pipeline-workspace-all-tests.log`, `/tmp/pipeline-runtime-tests.log`,
`/tmp/pipeline-ts-check.log`, and `/tmp/pipeline-web-tests.log`.
