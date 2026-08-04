# deskmate

Monitor-clip AMOLED desk display. Spec: docs/superpowers/specs/2026-08-03-deskmate-design.md

**Status: M2 active; M1 exit verification complete.** M0 was
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
Host tests and the ESP-IDF build pass. The long-lived Rust session, ICS/engine layer, M2
CLI demos, and physical exit checks remain in progress; see the active plan for exact
evidence and open gates.

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

Run all host-side checks with:

    make -C firmware/host_tests clean test
    cargo fmt --manifest-path companion/Cargo.toml --all --check
    cargo clippy --manifest-path companion/Cargo.toml --workspace --all-targets -- -D warnings
    cargo test --manifest-path companion/Cargo.toml --workspace

The repeatable on-device parser recovery check is:

    cargo run --manifest-path companion/Cargo.toml -p device \
      --example hardware_acceptance -- --port /dev/cu.usbmodem...
