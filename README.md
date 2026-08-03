# deskmate

Monitor-clip AMOLED desk display. Spec: docs/superpowers/specs/2026-08-03-deskmate-design.md

**Status: M0 complete.** Verified on the target board (Waveshare ESP32-S3-Touch-AMOLED-1.8
v2, CO5300 panel + CST820 touch): display bring-up over QSPI (LVGL 9 via
`esp_lvgl_port`), capacitive touch, brightness control, 180-degree rotation, and a
standalone clock screen that runs with no host connection. Details and known quirks are
in `docs/hardware/board-notes.md`.

What works today:
- Display: CO5300 panel over QSPI, driven through `esp_lcd_co5300` + LVGL 9 (`lvgl` /
  `esp_lvgl_port`), 368x448, RGB565.
- Touch: CST820 (CST816S-family driver) over the shared I2C bus, registered as an LVGL
  input device.
- Brightness: 0-255 levels via a QSPI-wrapped DCS `0x51` (write-display-brightness)
  command.
- Rotation: 180-degree software rotation (LVGL's `sw_rotate` path), with touch
  coordinates correctly remapped by LVGL core for the rotated orientation.
- Standalone clock screen: renders HH:MM + date with no phone/host pairing required.
  The HH:MM/date formatting core (`firmware/main/core/timefmt.c`) is host-testable and
  has its own test suite under `firmware/host_tests/`.

Not in M0 (deferred to later milestones): USB protocol to a host app, time sync, the
template engine, screen carousel, and status strip — see the roadmap in the design spec.

## Firmware build

    cd firmware
    idf.py set-target esp32s3
    idf.py build
    idf.py -p /dev/cu.usbmodem* flash monitor

Requires ESP-IDF >= 5.3 exported in the shell.

Developed and verified against ESP-IDF v5.5.5, installed to `~/esp/esp-idf`
(`. ~/esp/esp-idf/export.sh` before running `idf.py`).
