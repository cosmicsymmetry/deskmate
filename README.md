# deskmate

Monitor-clip AMOLED desk display. Spec: docs/superpowers/specs/2026-08-03-deskmate-design.md

## Firmware build

    cd firmware
    idf.py set-target esp32s3
    idf.py build
    idf.py -p /dev/cu.usbmodem* flash monitor

Requires ESP-IDF >= 5.3 exported in the shell.

Developed and verified against ESP-IDF v5.5.5, installed to `~/esp/esp-idf`
(`. ~/esp/esp-idf/export.sh` before running `idf.py`).
