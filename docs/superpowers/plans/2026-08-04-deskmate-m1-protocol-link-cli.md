# Deskmate M1 - Protocol, USB Link, and CLI Harness Implementation Plan

> **STATUS — HISTORICAL IMPLEMENTATION RECORD.** This plan describes the system and
> paths at the time of its dated work; it is not current architecture or build guidance.

**Status:** Complete and tagged `m1`. Software, adversarial-link, soak, visual, and
ten-cycle physical unplug/replug verification pass. Because M2 work began in the same
shared worktree before the final physical carryover closed, the tagged checkpoint also
contains the completed M2 foundation present at M1 exit.

**Goal:** From a terminal, discover a connected Deskmate and run
`deskmate-cli status`, `deskmate-cli time-sync`, and `deskmate-cli push-data`; malformed
traffic cannot crash the device, and loss of the host restores the standalone-clock
state.

**Architecture:** Use the ESP32-S3's native USB Serial/JTAG CDC exclusively as a
log-free application channel serviced by a FreeRTOS protocol task, leaving LVGL on its
existing task and preserving the hardware-assisted `idf.py flash` reset path. Firmware
diagnostics stay on UART0 and are not present in the USB byte stream. A bounded COBS-delimited,
CRC-protected binary envelope carries versioned CBOR payloads. The Rust `protocol` crate
owns message definitions and golden fixtures; the firmware parser and handlers conform
to the same fixtures. A Rust `device` crate owns discovery and request/response behavior,
and a thin `deskmate-cli` crate is the M1 user interface.

**Out of scope:** Templates, carousel navigation, status strip, persistent widget
configuration, providers, Tauri UI, asset transfer, and firmware update. M1 may retain
only the latest pushed sample in RAM; M2 will decide how widget data binds to UI.

## Global constraints

- Preserve every M0 behavior and the board-layer rules in `CLAUDE.md`.
- Protocol input is untrusted. Every buffer, text value, map, and frame has a fixed
  maximum; malformed data produces an error or is discarded and resynchronized.
- The protocol channel contains frames only. ESP-IDF logs use UART0; the USB
  Serial/JTAG secondary console must remain disabled.
- USB callbacks do bounded reads/writes and queue work only. Parsing, CBOR handling, and
  command execution run in the protocol task; LVGL changes run in LVGL context.
- Message and framing code under `firmware/main/core/` remains host-buildable. ESP-IDF
  transport glue stays under `firmware/main/link/`.
- Use test-first development for framing, parsing, validation, time conversion, and Rust
  request/response logic. Record real board observations in `docs/hardware/board-notes.md`.
- Use stable Rust and commit `Cargo.lock`. The M1 toolchain is installed under
  `/Users/rodion/.cargo`; invoke it by explicit path in shells where profile mutation has
  not added it to `PATH`.

## Wire contract to freeze in Task 2

Each wire frame is `COBS(decoded_frame) || 0x00`. The decoded frame is:

| Field | Encoding |
|---|---|
| wire version | `u8` (M1 = 1) |
| message type | `u8` |
| flags | `u16` little-endian; zero in M1 |
| request ID | `u32` little-endian; responses echo the request |
| payload length | `u16` little-endian |
| payload | canonical CBOR, exactly `payload length` bytes |
| checksum | CRC-32C over every preceding decoded byte |

The maximum decoded frame is 2048 bytes. A receiver must discard an overlong partial
frame through the next `0x00`, reject length/checksum/version errors, and continue with
the following frame. Unknown message types receive `UnsupportedMessage`; an unsupported
wire version receives `VersionMismatch` when enough of the envelope is valid to reply.

M1 message types and payloads:

- `StatusRequest` / `StatusResponse`: protocol and firmware versions, uptime, free heap,
  display size/brightness/rotation, link state, and latest pushed revision.
- `TimeSync` / `Ack`: Unix seconds plus a signed UTC offset in minutes. Firmware stores
  UTC via `settimeofday`; clock rendering applies the offset through a host-tested helper.
  Fixed-offset sync is sufficient for M1; the companion daemon must refresh it across
  DST changes in a later milestone.
- `PushData` / `Ack`: `widget_id`, monotonic `revision`, and a bounded string-keyed map of
  text/integer/boolean scalar fields. M1 validates and retains the latest value in RAM.
- `Heartbeat` / `HeartbeatAck`: liveness only. A valid request also refreshes liveness.
- `Error`: stable numeric code plus a bounded diagnostic string intended for the CLI.

Exact numeric type IDs, CBOR keys, field/count limits, CRC test vectors, and timeout
values must be written in `docs/protocol/v1.md` and frozen by golden fixtures before
firmware transport integration.

---

### Task 1: Baseline, Rust prerequisite, and USB transport proof

**Files:**
- Modify: `firmware/sdkconfig.defaults`
- Create: `firmware/main/link/usb_link.c`
- Create: `firmware/main/link/usb_link.h`
- Modify: `docs/hardware/board-notes.md`

- [x] Record the available clean M0 software baselines and explicitly record that a
  connected M0 image was unavailable for a fresh enumeration/standalone-clock run.
- [x] Install stable Rust with `rustup` if still absent; record `rustc` and `cargo`
  versions used for M1.
- [x] Prove the board's USB-C connector is wired to GPIO19/GPIO20 and evaluate the
  official `espressif/esp_tinyusb` dual-CDC path on hardware.
- [x] Prove the native USB Serial/JTAG CDC is a stable, bidirectional application
  interface on macOS and contains no boot or ESP-IDF logs. Diagnostics use UART0.
- [x] Verify the normal flash path and the physical BOOT/reset recovery path still work,
  then record port identity, descriptor choice, and recovery steps in board notes.
- [x] Remove the rejected TinyUSB spike; retain only a small `usb_link` API over the
  native bounded USB Serial/JTAG driver rings.

**Evidence so far (2026-08-04):**

- Pre-change M0 host tests passed (`test_timefmt: OK`) and ESP-IDF 5.5.5 built
  successfully (`deskmate.bin` `0xb4210` bytes, 30% app partition free). The board was
  not connected during this pass—there was no `/dev/cu.usbmodem*` device—so current
  enumeration and a fresh standalone-clock boot observation remain part of the first
  unchecked item.
- Installed stable Rust with the minimal rustup profile and no shell-profile mutation:
  `rustup 1.29.0`, `rustc 1.97.1 (8bab26f4f 2026-07-14)`, and
  `cargo 1.97.1 (c980f4866 2026-06-30)`.
- `espressif/esp_tinyusb ^2.0.0` resolves to **2.2.1** with
  `espressif/tinyusb 0.21.0~1` under ESP-IDF 5.5.5. The Waveshare schematic traces the
  USB-C connector's `USB_N`/`USB_P` nets through R19/R20 (22 ohm) to ESP32-S3
  GPIO19/GPIO20, respectively; Espressif defines those internal-PHY pins as USB D-/D+.
- The dual-CDC spike did enumerate on macOS as `303A:4002`, with diagnostics and
  protocol nodes derived from factory serial `A4CB8FDB3328`. It was rejected after the
  real-board test: its static buffers consumed enough internal RAM that LVGL's third
  58,880-byte software-rotation buffer failed allocation, causing a reboot loop, and
  replacing the native Serial/JTAG peripheral also broke the normal `idf.py flash`
  reset path. Physical BOOT plus reset recovered the board successfully.
- Per explicit user direction, M1 now keeps the native `303A:1001` USB Serial/JTAG
  peripheral that made M0 flashing reliable. Its one CDC is protocol-only; logs are
  routed to UART0 by disabling the USB secondary console. The native driver is installed
  only after LVGL allocates its display buffers. The pre-exit build succeeded at `0xb7350`
  bytes (28% app partition free). Normal automatic flashing succeeded twice, including
  a second flash initiated from the running M1 image, and a 30-minute open protocol
  session remained bidirectional without application log contamination.
- The ESP32-S3 ROM can emit pre-application startup text through native Serial/JTAG.
  Permanently disabling that path requires burning `DIS_USB_SERIAL_JTAG_ROM_PRINT`, which
  M1 deliberately does not do. The serial client clears stale input immediately after
  opening and then requires a framed status handshake; once the app owns the peripheral,
  all firmware/bootloader diagnostics use UART0. An unexpected reset during a live
  request is treated as malformed/disconnected traffic and retried by reconnecting.

**Acceptance:** One cable exposes an isolated bidirectional byte stream and the native
automatic flash/recovery path without regressing display, touch, or the standalone clock.

---

### Task 2: Freeze protocol v1 and build the Rust protocol crate

**Files:**
- Create: `docs/protocol/v1.md`
- Create: `companion/Cargo.toml`
- Create: `companion/Cargo.lock`
- Create: `companion/crates/protocol/`
- Create: `protocol/fixtures/v1/`

- [x] Write the complete numeric wire registry, canonical CBOR schemas, bounds, error
  codes, liveness timeout, and request/response rules. Keep transport framing separate
  from semantic payload definitions.
- [x] Create the Rust workspace and `protocol` crate with typed messages, deterministic
  encode/decode, incremental deframing, and explicit errors.
- [x] Add tests for fragmented/coalesced reads, embedded zero bytes, maximum frames,
  overflow resynchronization, bad CRC/length/version/type, invalid CBOR, duplicate map
  keys, out-of-range values, and request-ID correlation.
- [x] Generate and commit golden binary frames plus human-readable expected values for
  every M1 message and every parser error class. Fixture generation must be deterministic
  and exposed as a checked command, not an opaque one-off script.
- [x] Run `cargo fmt --all --check`, Clippy with warnings denied, and workspace tests.

**Acceptance:** The Rust crate can encode, incrementally decode, and validate all M1
messages, and the checked-in spec fully explains each golden byte sequence.

---

### Task 3: Firmware framing and message conformance on the host

**Files:**
- Create: `firmware/main/core/protocol_frame.c`
- Create: `firmware/main/core/protocol_frame.h`
- Create: `firmware/main/core/protocol_message.c`
- Create: `firmware/main/core/protocol_message.h`
- Create: `firmware/host_tests/test_protocol.c`
- Modify: `firmware/host_tests/Makefile`
- Modify: `firmware/main/idf_component.yml`
- Modify: `firmware/main/CMakeLists.txt`

- [x] Add the official Espressif TinyCBOR component for bounded payload decoding; do not
  build CBOR through string parsing or an ad hoc serializer.
- [x] Test-drive the C incremental frame decoder against every Rust-generated fixture
  and corruption/resynchronization case from Task 2.
- [x] Test-drive bounded decode/validation for all M1 request payloads. Unknown CBOR keys
  may be skipped for compatible evolution, but missing required keys, wrong types,
  duplicates, and exceeded limits must fail deterministically.
- [x] Add C encoders for status, acknowledgement, heartbeat acknowledgement, and error
  responses; compare their output byte-for-byte with golden fixtures.
- [x] Run all host tests under both the default compiler and sanitizers where available.

**Acceptance:** Rust and plain-C implementations consume the same fixtures with identical
results, including failures, without ESP-IDF or board hardware in the framing core.

---

### Task 4: Protocol task and USB integration

**Files:**
- Modify: `firmware/main/link/usb_link.c`
- Create: `firmware/main/link/protocol_task.c`
- Create: `firmware/main/link/protocol_task.h`
- Modify: `firmware/main/main.c`
- Modify: `firmware/main/CMakeLists.txt`

- [x] Add fixed-size RX/TX queues and a protocol task pinned away from LVGL. Define queue
  overflow behavior and counters; no callback may block indefinitely or allocate based
  on host-provided sizes.
- [x] Feed arbitrary USB chunks into the incremental parser and dispatch complete valid
  requests. Serialize complete responses before handing them to USB TX.
- [x] Track valid-frame, malformed-frame, CRC, overflow, and dropped-response counters
  for `StatusResponse`; rate-limit diagnostic logs for hostile input.
- [x] On hardware, send split frames, multiple frames in one write, corrupted frames,
  garbage, and overlong frames, then confirm the next valid status request succeeds and
  the LVGL clock remains responsive.

**Acceptance:** The device survives and resynchronizes after adversarial serial input,
and protocol traffic does not stall rendering or touch.

---

### Task 5: Status, time sync, pushed data, and host-loss behavior

**Files:**
- Create: `firmware/main/core/link_state.c`
- Create: `firmware/main/core/link_state.h`
- Create: `firmware/host_tests/test_link_state.c`
- Modify: `firmware/main/link/protocol_task.c`
- Modify: `firmware/main/ui/clock_screen.c`
- Modify: `firmware/main/ui/clock_screen.h`

- [x] Test-drive link-state transitions: standalone on boot, online after a valid request,
  refreshed by heartbeat/requests, and standalone after the documented timeout.
- [x] Implement status fields from live device state and monotonic counters.
- [x] Implement validated time sync with a host-tested UTC-offset conversion. Reject
  impossible epochs/offsets without changing the last good clock state.
- [x] Validate and retain the latest `PushData` revision in bounded RAM. Reject stale
  revisions and oversized/invalid fields; do not build M2 template bindings here.
- [x] Send link-state changes to the LVGL context so the clock's connection hint hides
  online and returns on timeout. The clock and touch must continue working throughout.
- [x] Hardware-verify sync, valid/stale/invalid pushes, unplug timeout, and replug.

**Acceptance:** All handlers acknowledge or return a stable error, the displayed clock
uses host-synced time, and unplugging visibly returns to the standalone clock state.

---

### Task 6: Device crate and CLI

**Files:**
- Create: `companion/crates/device/`
- Create: `companion/crates/deskmate-cli/`
- Modify: `companion/Cargo.toml`

- [x] Define a transport trait and test request correlation, timeouts, partial I/O,
  malformed responses, and disconnects with an in-memory fake before using a serial
  implementation.
- [x] Discover candidate serial interfaces by USB metadata, then confirm identity with a
  v1 status handshake. Never rely on `/dev/cu.usbmodem*` numbering; support an explicit
  `--port` override for recovery and development.
- [x] Implement `status`, `time-sync`, and `push-data` with useful human output and
  `--json` machine output. `push-data` accepts a widget ID, revision, and repeated bounded
  scalar fields matching the v1 schema.
- [x] Use nonzero exit codes for no device, timeout, version mismatch, rejected command,
  and malformed response; never print protocol bytes as successful output.
- [x] Run formatting, Clippy with warnings denied, and all workspace tests.

**Acceptance:** Each roadmap command works through the real device crate rather than
special CLI-only serial code, and failure modes are scriptable.

---

### Task 7: M1 exit verification and handoff

**Files:**
- Modify: `README.md`
- Modify: `docs/hardware/board-notes.md`
- Modify: `docs/superpowers/plans/2026-08-03-deskmate-roadmap.md`
- Modify: `CLAUDE.md` only if durable rules/state changed
- Create: M2 implementation plan

- [x] Run C host tests, Rust formatting/lint/tests, and a clean ESP-IDF build.
- [x] Flash the release build and execute the three CLI commands from a clean terminal.
- [x] Run a 30-minute connected heartbeat soak, malformed-input recovery pass, and at
  least ten unplug/replug cycles. Record heap floor, fixed native-ring capacities and
  available pressure counters, parser error counters, and observed enumeration behavior.
- [x] Verify boot and host-loss fallback, correct time after reconnect, touch
  responsiveness, brightness, and 90/270-degree landscape rotation.
- [x] Update README status and usage, close every completed checkbox with evidence,
  write the M2 plan from M1 findings, and mark M1 complete in the roadmap.
- [x] Create an `m1` tag only after the exit checklist and review fixes are complete.

**M1 exit gate:** A clean checkout can build both sides, shared fixtures prove protocol
conformance, the CLI demo works on the physical board, corrupted input recovers, and the
standalone clock remains the reliable failure mode.

## Software verification evidence (2026-08-04)

- The frozen v1 spec and deterministic generator produced nine valid and seven invalid
  complete-wire fixtures. Rust has 11 protocol unit tests plus two fixture suites.
- Plain C decodes and re-encodes every valid Rust fixture byte-for-byte, rejects the
  invalid fixtures by class, and covers fragmented/coalesced reads, maximum frames, and
  overlong-frame resynchronization. `test_timefmt`, `test_protocol`, and
  `test_link_state` pass under the default compiler and Address/UndefinedBehavior
  Sanitizers.
- Maximum-frame decode scratch is retained in the fixed protocol context rather than on
  the task stack, and dispatch reuses the decoded-message buffer for replies. An Xtensa
  object-code audit shows the remaining large nested encode frames total 4,336 bytes
  (`protocol_message_encode` 2,256 + `protocol_frame_encode` 2,080), leaving deliberate
  headroom in the 8 KiB protocol task instead of stacking another frame and message.
- The device crate has eight fake-transport tests covering partial I/O, correlation,
  timeout, disconnect, malformed response, wrong response type, and version mismatch.
  `cargo fmt --all --check`, workspace Clippy with `-D warnings`, and all workspace tests
  pass on Rust 1.97.1. CLI help exits zero; device absence produces JSON error output and
  the documented exit code 10.
- `cargo run -p device --example heartbeat_soak -- --seconds 1800` is the checked M1
  sustained-connection harness. It sends one heartbeat per second, checks monotonic
  device uptime, samples status/heap/counters each minute, and fails on any request or
  reset; it is ready for Task 7 once the native image is on the board.
- `cargo run -p device --example hardware_acceptance -- --port <device>` is the checked
  adversarial-link harness. It proves split and coalesced request handling, injects the
  golden bad-CRC and garbage fixtures plus an overlong frame, checks invalid time/data
  and stale-revision errors, then requires parser-counter deltas and a clean status
  response without reconnecting.
- The original queue-high-water exit metric applied to the rejected application-owned
  TinyUSB queues. ESP-IDF's selected native Serial/JTAG driver fixes both rings at 4096
  bytes but exposes neither occupancy nor an RX-overflow count through its public API;
  M1 therefore records those capacities, reports `rx_dropped_bytes=0` as explicitly
  unavailable, and uses parser overflow/dropped-response counters plus heap floor as the
  observable pressure metrics. This is a documented transport limitation, not a zero-drop
  claim.
- ESP-IDF 5.5.5 builds the integrated firmware successfully. After restoring explicit
  M1 touch controls for the brightness/rotation exit check, the current image is
  `0xb77e0` bytes with `0x48820` bytes (28%) free in the smallest app partition.
  LVGL is pinned to CPU0 and the bounded protocol task to CPU1.
## Hardware verification evidence (2026-08-04)

- The board enumerated through native USB Serial/JTAG as `303A:1001` on
  `/dev/cu.usbmodem1101`. A normal `idf.py -p /dev/cu.usbmodem1101 flash` installed M1,
  verified its image hash, and hard-reset normally. Repeating the same command from the
  running M1 application also succeeded, proving automatic reflashing was restored.
- `status --json` returned protocol v1, display `368x448`, brightness 200, rotation 0,
  free heap 8,462,035 bytes, and clean counters. `time-sync --json` acknowledged Unix
  time with UTC offset +240 minutes, and `push-data --json` accepted widget `weather`
  revision 1. A fresh status after the second flash also succeeded. A later status call
  without `--port` discovered `/dev/cu.usbmodem1101` from USB metadata and completed its
  v1 identity handshake, proving the normal discovery path rather than only the override.
- The checked hardware-acceptance harness passed without reconnecting:
  `valid=6->14 malformed=0->3 crc=0->1 overflow=0->1 dropped=0 rx_drops=0`. It covered
  split/coalesced frames, bad CRC, garbage, an overlong frame, invalid push/time values,
  stale revision rejection, and recovery to a final valid status response.
- The active 30-minute heartbeat soak passed with 1,791 heartbeats and no device reset:
  final uptime 2,332,440 ms, heap floor 8,462,035 bytes, 1,827 valid frames, and zero
  malformed, CRC, overflow, dropped-response, or RX-drop counters. The host suspended
  once between the 11- and 12-minute reports; device uptime advanced about 554 seconds
  while harness active time advanced 60 seconds. The same open session recovered and
  completed, so this is recorded as a host-suspension interval rather than claimed as
  uninterrupted one-hertz traffic.
- Ten physical unplug/replug cycles passed. Cycle 1 re-enumerated from
  `/dev/cu.usbmodem3101` to `/dev/cu.usbmodem1101` and completed a clean v1 status
  handshake. Cycles 2-10 were observed as distinct disconnect/reconnect edges, stayed
  disconnected for 5-7 seconds each, and consistently returned as
  `/dev/cu.usbmodem1101`. Immediately after cycle 10, one explicit-port status attempt
  raced macOS device-node readiness and returned `ENOENT`; time sync, a retried status,
  and normal metadata discovery then succeeded without another cable action. Final
  status reported heap `8,521,431`, `valid=18 malformed=3 crc=1 overflow=1 dropped=0
  rx_drops=0`. The earlier long unplug/host-loss observation and final visual
  display/touch/rotation check had already passed after the CO5300 alignment correction.
- The final 2026-08-04 software rerun passed all nine plain-C host tests, Rust format,
  Clippy with warnings denied, 20 Rust unit tests, three fixture tests, doc tests, and the
  ESP-IDF build. The current M2-inclusive firmware image is `0xbdb80` bytes with
  `0x42480` bytes (26%) free in the smallest app partition.
- A final host-side rerun after hardware testing passed all three plain-C tests under
  both the default compiler and ASan/UBSan, then removed the test binaries. Rust format,
  workspace Clippy with warnings denied, 20 unit tests, two fixture suites, and doc tests
  also passed.
- The first human exit attempt exposed an acceptance-harness gap rather than a failed
  board setter: M0's temporary tap-zone screen had been intentionally removed when the
  standalone clock was introduced, while M1 added no replacement way to invoke
  brightness or rotation. The clock gained diagnostic release zones: top cycles
  25/50/100% brightness and bottom flips orientation. The user then confirmed both
  brightness changes and the flip worked, but photographed cyan remnants left by the
  transient feedback label on the partial-refresh screen. That diagnostic overlay was
  removed. Per user direction, the default UI is now a 448x368 landscape canvas at 90
  degrees (USB cable down), flipped at 270 degrees; status schema validation accepts all
  cardinal rotations. The landscape build flashed normally and returned clean status
  with flat heap. The complete hostile-input hardware harness then passed again at
  `valid=4->12 malformed=0->3 crc=0->1 overflow=0->1 dropped=0 rx_drops=0`; final human
  visual confirmation initially remained pending.
- The next landscape photos showed that corruption was broader than the removed label:
  a distorted clock, persistent green blocks, and stale text/regions. Two attempted
  corrections were explicitly falsified by unchanged hardware photos: switching LVGL
  from two draw buffers to one, and forcing synchronous full-canvas invalidation. The
  board-specific difference was instead found in Waveshare's v2 BSP: the same commit
  that changed this product from SH8601 to CO5300 added a callback that rounds every
  dirty rectangle outward to two-pixel boundaries. Without it, the 29,440-pixel buffer
  splits a 448-pixel-wide full landscape repaint into 65-row strips; 90-degree rotation
  turns those into odd-width CO5300 column writes. M1 now installs the equivalent
  `rounder_cb`, which makes LVGL select 64-row strips and aligns all partial windows.
  The aligned image built, flashed, returned clean 448x368/90-degree status with
  `8,521,431` bytes free, and accepted time sync. The user then confirmed the display
  was clean at both landscape orientations, brightness and touch controls worked, and
  no artifacts remained. Single buffering is retained conservatively for this verified
  M1 image, but is not treated as the root-cause fix.
