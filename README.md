# deskmate

Monitor-clip AMOLED desk display. Spec: docs/superpowers/specs/2026-08-03-deskmate-design.md

**Status: M3 complete; M4 v1 completion active at Task 3.** M0 was
verified on the target board (Waveshare ESP32-S3-Touch-AMOLED-1.8
v2, CO5300 panel + CST820 touch): display bring-up over QSPI (LVGL 9 via
`esp_lvgl_port`), capacitive touch, brightness control, 180-degree rotation, and a
standalone clock screen that runs with no host connection. Details and known quirks are
in `docs/hardware/board-notes.md`.

What works today:
- Display: physical 368x448 CO5300 panel over QSPI, driven through
  `esp_lcd_co5300` + LVGL 9 (`lvgl` / `esp_lvgl_port`), presented as a 448x368
  landscape canvas, RGB565.
- Touch: CST820 (CST816S-family driver) over the shared I2C bus, registered as an LVGL
  input device.
- Brightness: 0-255 levels via a QSPI-wrapped DCS `0x51` (write-display-brightness)
  command.
- Rotation: landscape 90°/270° software rotation (LVGL's `sw_rotate` path), with touch
  coordinates correctly remapped by LVGL core for both orientations.
- Standalone clock screen: renders HH:MM + date with no phone/host pairing required.
  The HH:MM/date formatting core (`firmware/main/core/timefmt.c`) is host-testable and
  has its own test suite under `firmware/host_tests/`.

M1 added a protocol-only native USB Serial/JTAG channel, COBS + CRC-32C framing, canonical
CBOR messages, status/liveness handling, host time sync, bounded in-RAM push data, and a
Rust CLI. Its C/Rust test suites, ESP-IDF build, adversarial-link pass, 30-minute soak,
visual checks, and ten-cycle physical unplug/replug acceptance all pass.

M2 Tasks 1-4 now provide the frozen configuration/event protocol, fixed-capacity widget
model, full/standard digital-clock, progress-ring, and row-list templates, status strip,
local carousel gestures, bounded device-event/UI queues, and priority-interrupt firmware.
Tasks 5-7 add the long-lived Rust session, ICS provider, pomodoro/interrupt engine, and M2
CLI harness. M2's host tests, ESP-IDF build, native double-flash, malformed-link
recovery, 100 config swaps, 1,000 field patches, flat heap, keepalive idle, corrected
carousel gestures, pomodoro completion/dismissal, full-power replay, both landscape
orientations, event burst, and 30-minute mixed soak pass. The user explicitly waived
nine repeated M2 power cycles after one observed corrected cycle. M3 now has the strict
typed app/config contract, cross-platform atomic config store, single-owner background
runtime, pinned Tauri v2 desktop shell, narrow typed IPC, and the complete settings UI.
The app configures and previews the three proven widgets, arranges screens with pointer
or keyboard, chooses between the two landscape mounting orientations, exposes
provider/device/persistence state, and keeps the runtime alive in the tray when settings
closes. Widgets use the complete 448x368 canvas with no status strip. Its software,
macOS debug/release, physical UI/replay, sleep/wake, and tray-resident soak gates pass.
M4 Task 1 freezes application config schema v2, lossless M3 migration, bounded closed
types for the remaining v1 surface, and an additive protocol-v1 capability handshake.
See `docs/config/v2.md`, `docs/protocol/v1.md`, `docs/providers/v1.md`, and the active
M4 plan. Task 2 delivered the bounded weather/JSON/RSS providers and wider ICS
recurrence; Task 3 now owns the remaining device templates.

## M3 companion app

Install the locked frontend dependencies and launch the desktop app:

    cd companion/apps/deskmate
    bun install --frozen-lockfile
    PATH="$HOME/.cargo/bin:$PATH" bun run tauri dev

Use Settings to add a digital clock, pomodoro, or ICS calendar from a URL or local file,
choose USB-below or USB-above landscape mounting, then arrange its screens and choose
Save & apply. Closing Settings hides the window; the
tray process continues provider refreshes, timer handling, reconnect, and full replay.
Pomodoro duration and label persist across app restarts. A live timer survives settings
close/reopen and unrelated saves, but deliberately restarts idle after the entire app
quits so elapsed time is never guessed from process downtime.

Run the frontend checks from the same directory:

    bun run format:check
    bun run lint
    bun run check
    bun test
    bun run build

## Firmware build

    cd firmware
    idf.py set-target esp32s3
    idf.py build
    idf.py -p /dev/cu.usbmodem* flash monitor

Requires ESP-IDF >= 5.3 exported in the shell.

Developed and verified against ESP-IDF v5.5.5, installed to `~/esp/esp-idf`
(`. ~/esp/esp-idf/export.sh` before running `idf.py`).

## M1 CLI

Build the CLI with stable Rust:

    cargo build --manifest-path companion/Cargo.toml --release -p deskmate-cli

Then, with the native `303A:1001` application firmware connected:

    companion/target/release/deskmate-cli status
    companion/target/release/deskmate-cli time-sync
    companion/target/release/deskmate-cli push-data --widget weather --revision 1 \
      --field summary=s:Clear --field temp=i:23 --field ok=b:true

Add `--json` for machine output or `--port /dev/cu.usbmodem...` to bypass discovery.
The CLI confirms identity with a v1 status handshake rather than relying on a volatile
device-node suffix. Protocol details and bounds are in `docs/protocol/v1.md`.

## M2 CLI demo

The checked sample uses all three M2 templates and both legacy full/standard size
classes. Current firmware renders either class as a clean full canvas. Build
the CLI, then inspect and apply it before pushing widget data:

    cargo build --manifest-path companion/Cargo.toml -p deskmate-cli
    companion/target/debug/deskmate-cli inspect-config \
      --config companion/examples/m2-carousel.json
    companion/target/debug/deskmate-cli apply-config \
      --config companion/examples/m2-carousel.json
    companion/target/debug/deskmate-cli time-sync
    companion/target/debug/deskmate-cli push-clock --widget clock --title Desk
    companion/target/debug/deskmate-cli push-calendar --widget calendar \
      --ics /path/to/calendar.ics --timezone Asia/Tbilisi
    companion/target/debug/deskmate-cli select-screen --screen pomodoro-screen
    companion/target/debug/deskmate-cli pomodoro --widget pomodoro --duration-seconds 60

For a power-cycle-safe full demo, use one long-lived process so the reconnect owner has
the layout, time, and latest data for all three widgets:

    companion/target/debug/deskmate-cli demo \
      --config companion/examples/m2-carousel.json \
      --ics /path/to/calendar.ics --timezone Asia/Tbilisi \
      --duration-seconds 60

The demo command handles device taps, pushes one-second authoritative pomodoro
snapshots, reconnects and replays the complete carousel after unplug or power loss, and
sends one completion interrupt. Individual short-lived commands remain useful for
manual inspection, but one process cannot replay state owned only by an earlier process.
For standalone event inspection or manual interrupt testing:

    companion/target/debug/deskmate-cli events --json
    companion/target/debug/deskmate-cli trigger-interrupt \
      --widget calendar --token 1 --reason "Meeting starting"

Use `--json` for machine-readable command output and JSON Lines events. Long-running
commands stop cleanly on Ctrl-C; once keepalives stop, firmware returns to its standalone
clock on the normal 10-second timeout. Exit 3 means local config validation failed, 4 is
a provider failure, and 10-15 retain the device/transport/rejection classes.

Run all host-side checks with:

    make -C firmware/host_tests clean test
    cargo fmt --manifest-path companion/Cargo.toml --all --check
    cargo clippy --manifest-path companion/Cargo.toml --workspace --all-targets -- -D warnings
    cargo test --manifest-path companion/Cargo.toml --workspace

The repeatable on-device parser recovery check is:

    cargo run --manifest-path companion/Cargo.toml -p device \
      --example hardware_acceptance -- --port /dev/cu.usbmodem...
