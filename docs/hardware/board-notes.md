# Waveshare ESP32-S3-Touch-AMOLED-1.8 (v2) — board notes

Board under bring-up: **Waveshare ESP32-S3-Touch-AMOLED-1.8, v2 hardware revision**
(panel controller **CO5300**; the v1 revision uses **SH8601** and is a different chip —
see the "v1 vs v2" section below for every place this matters).

## Sources consulted

1. Wiki page: https://www.waveshare.com/wiki/ESP32-S3-Touch-AMOLED-1.8
   (fetched via self-hosted Firecrawl `/v1/scrape`, markdown+links, 2026-08-03).
   This page's prose and Arduino demo code describes the **v1 (SH8601 + FT3168)** board only —
   it never mentions "v2" or "CO5300" anywhere in the body text. The "Pinout Definition"
   section is an embedded JPEG image (not extractable as text), so it was not usable as a
   pin source directly; the schematic PDF and demo source (below) were used instead.
2. Schematic PDF: https://files.waveshare.com/wiki/ESP32-S3-Touch-AMOLED-1.8/ESP32-S3-Touch-AMOLED-1.8.pdf
   (linked from the wiki page's "Schematic Diagram" section; downloaded and read directly).
   This schematic is authoritative for net-to-GPIO wiring and matches the v2/CO5300 assembly's
   GPIO numbering (confirmed against the demo source below — no discrepancy found).
3. Waveshare GitHub demo repo: https://github.com/waveshareteam/ESP32-S3-Touch-AMOLED-1.8
   README explicitly documents both revisions:
   > "The repository supports both display and touch revisions: the original SH8601/FT3168
   > board and the newer CO5300/CST820 board."
   ESP-IDF examples live under `examples/esp-idf/`; the most relevant is
   `examples/esp-idf/00_bsp_quickstart`, which depends on the managed component
   `waveshare/esp32_s3_touch_amoled_1_8` (version `^2.0.3`).
4. Managed BSP component source (the actual pin definitions):
   https://github.com/waveshareteam/Waveshare-ESP32-components
   `bsp/esp32_s3_touch_amoled_1_8/include/bsp/esp32_s3_touch_amoled_1_8.h` and
   `bsp/esp32_s3_touch_amoled_1_8/esp32_s3_touch_amoled_1_8.c` (fetched at git ref `main`,
   component registry version 2.0.3 as pulled by the ESP-IDF example's `idf_component.yml`).
   This is Waveshare's official, currently-shipping ESP-IDF board support package for this
   exact board, and its dependency list confirms the **v2/CO5300 + CST816S-family touch**
   pairing (deps include `esp_lcd_co5300`, `esp_lcd_touch_cst816s`, `esp_lcd_touch_ft5x06` —
   the BSP probes both touch chips at runtime and uses whichever ACKs on the I2C bus, so one
   component source covers both hardware revisions).
5. Component registry API (https://components.espressif.com/api/components/...) used to
   pull exact version/download URLs for the BSP and its touch/IO-expander/RTC/IMU
   sub-components, and to read their header-defined I2C addresses directly rather than
   relying on memory.

## Final pin table (v2 / CO5300 board — what `board.h` encodes)

| Constant | Value | Schematic net | Source |
|---|---|---|---|
| `BOARD_LCD_H_RES` | 368 | — | Wiki ("368×448" resolution, repeated everywhere) + BSP `display.h` `BSP_LCD_H_RES` |
| `BOARD_LCD_V_RES` | 448 | — | Same as above, `BSP_LCD_V_RES` |
| `BOARD_LCD_QSPI_HOST` | `SPI2_HOST` | — | BSP `BSP_LCD_SPI_NUM` |
| `BOARD_LCD_PIN_PCLK` | `GPIO_NUM_11` | `QSPI_SCL` | Schematic (ESP32-S3 pin GPIO11 net `NLQSPI0SCL`) = BSP `BSP_LCD_PCLK` |
| `BOARD_LCD_PIN_CS` | `GPIO_NUM_12` | `LCD_CS` | Schematic (GPIO12 net `NLLCD0CS`) = BSP `BSP_LCD_CS` |
| `BOARD_LCD_PIN_D0` | `GPIO_NUM_4` | `QSPI_SIO0` | Schematic (GPIO4 net `NLQSPI0SIO0`) = BSP `BSP_LCD_DATA0` |
| `BOARD_LCD_PIN_D1` | `GPIO_NUM_5` | `QSPI_SI1` | Schematic (GPIO5 net `NLQSPI0SI1`) = BSP `BSP_LCD_DATA1` |
| `BOARD_LCD_PIN_D2` | `GPIO_NUM_6` | `QSPI_SI2` | Schematic (GPIO6 net `NLQSPI0SI2`) = BSP `BSP_LCD_DATA2` |
| `BOARD_LCD_PIN_D3` | `GPIO_NUM_7` | `QSPI_SI3` | Schematic (GPIO7 net `NLQSPI0SI3`) = BSP `BSP_LCD_DATA3` |
| `BOARD_LCD_PIN_RST` | `GPIO_NUM_NC` | `LCD_RESET` | Schematic shows `LCD_RESET` tied to `EXIO0` (TCA9554 IO-expander port P0), **not** a direct ESP32 GPIO. BSP agrees: `BSP_LCD_RST = GPIO_NUM_NC`. |
| `BOARD_I2C_PORT` | `I2C_NUM_0` | — | Chosen for this project; see note below — the demo's own Kconfig defaults to I2C port **1**, but the peripheral index is a software choice, not a wiring fact (see discrepancy note). |
| `BOARD_I2C_PIN_SDA` | `GPIO_NUM_15` | `ESP32_SDA` (also labeled `TP_SDA`†) | Schematic (GPIO15, shared bus net `ESP32_SDA`) = BSP `BSP_I2C_SDA` |
| `BOARD_I2C_PIN_SCL` | `GPIO_NUM_14` | `ESP32_SCL` (also labeled `TP_SCL`†) | Schematic (GPIO14, shared bus net `ESP32_SCL`) = BSP `BSP_I2C_SCL` |
| `BOARD_TOUCH_PIN_INT` | `GPIO_NUM_21` | `TP_INT` | Schematic (GPIO21 net `NLTP0INT`) = BSP `BSP_LCD_TOUCH_INT` |
| `BOARD_TOUCH_PIN_RST` | `GPIO_NUM_NC` | `TP_RESET` | Schematic shows `TP_RESET` tied to `EXIO2` (TCA9554 IO-expander port P2), **not** a direct ESP32 GPIO. BSP agrees: `BSP_LCD_TOUCH_RST = GPIO_NUM_NC`. |

Every LCD/touch/I2C pin above was cross-checked two ways — schematic net trace *and*
Waveshare's own shipping BSP source — and both agreed in every case. No pin-level
discrepancy was found between the schematic and the demo code for this board revision.

† **I2C net-name clarification (Task 2 deferred minor, resolved here):** the schematic
labels this same shared I2C bus pair differently in different sheets/blocks — the
sheet nearest the ESP32-S3 labels the nets `ESP32_SDA`/`ESP32_SCL`, while the sheet
block nearest the touch controller labels the identical physical nets `TP_SDA`/`TP_SCL`.
This is not a wiring conflict: both label pairs trace to the same GPIO15/GPIO14 pins on
the same shared bus (touch, RTC, IMU, IO-expander all hang off it), and the *values* in
the table above were correct under either label. Recorded here so a reader cross-checking
against a different part of the schematic doesn't mistake the second label for a second,
different net.

## Discrepancy: I2C port index (not a wiring fact)

The task's `board.h` template pre-filled `BOARD_I2C_PORT` as `I2C_NUM_0`. Waveshare's own
BSP Kconfig (`bsp/esp32_s3_touch_amoled_1_8/Kconfig`) defaults `BSP_I2C_NUM` to **`1`**
(range 0–1). This is **not** a schematic/wiring disagreement — which I2C peripheral index
(0 or 1) a driver binds to a given pair of GPIOs is a software-only choice on the ESP32-S3
(any I2C peripheral can be routed to any GPIO via the GPIO matrix); it says nothing about
how the board is physically wired. Kept `I2C_NUM_0` per the task template; noted here so a
later task doesn't mistake this for an unresolved pin conflict.

## Discrepancy: touch chip identity — brief said "FT3168", v2 hardware uses CST820/CST816S-family

The task brief's Step 1 instructions say to record "touch I²C pins + address (FT3168)".
**FT3168 is the v1-board touch chip.** Independent confirmation that v2 uses a different
touch chip, from two sources:

- Waveshare's own GitHub README (`waveshareteam/ESP32-S3-Touch-AMOLED-1.8`): "the newer
  CO5300/CST820 board."
- The v2 BSP source (`Waveshare-ESP32-components`) probes for touch controllers at runtime:
  it tries the **CST816S** driver first (I2C address `0x15`, from
  `esp_lcd_touch_cst816s.h`: `ESP_LCD_TOUCH_IO_I2C_CST816S_ADDRESS (0x15)`), and only falls
  back to the **FT5x06** driver family — which is what FT3168 is programmed as — (I2C
  address `0x38`, from `esp_lcd_touch_ft5x06.h`:
  `ESP_LCD_TOUCH_IO_I2C_FT5x06_ADDRESS (0x38)`) if the CST816S doesn't ACK. The BSP also
  applies a CST816S-only coordinate-gap quirk (`BSP_LCD_CST816S_X_GAP = 0x10`, a 16px X
  offset correction) that is not applied for the FT5x06 path — a real v1/v2 init-quirk
  difference for a later touch-driver task to replicate.
- The BSP's own comment on the CST820 chip: "Some bundled Arduino sources retain
  `Arduino_CST816x` family or API identifiers for compatible driver code; those identifiers
  do not describe the fitted V2 touch chip" — i.e. Waveshare's own docs warn that chip-family
  naming is inconsistent across their example code, and the actual silicon marked "CST820"
  is driven through the CST816S-family driver/protocol.

**Net effect on `board.h`:** the pins (`BOARD_TOUCH_PIN_INT`, `BOARD_TOUCH_PIN_RST`) are
identical for v1 and v2 — only the I2C address and driver differ, and the brief's constant
list doesn't include an address constant, so `board.h` needed no change. Recorded here so
the touch-driver task doesn't wire up an FT3168/FT5x06 driver against v2 hardware by
following the brief's parenthetical literally — **use CST816S-family @ 0x15 for this v2
board**, with FT5x06 @ 0x38 (FT3168) only relevant if bringing up a v1 unit.

## LCD panel init quirks (from the v2 BSP source, for the display-driver task)

From `esp32_s3_touch_amoled_1_8.c`, `lcd_init_cmds[]` (CO5300 vendor init sequence sent
over QSPI after reset/sleep-out):

- `0xFE 0x00` — select command page 0 (CO5300 vendor page-select convention)
- `0xC4 0x80` — QSPI interface enable bit set (vendor register, needed for QSPI mode)
- `0x3A 0x55` — pixel format = RGB565
- `0x35 0x00` — tearing-effect line ON, mode 0
- `0x53 0x20` — write-control register, brightness-control-block enable bit
- `0x51 0xFF` — display brightness set to max (0xFF) during init
- `0x63 0xFF` — CE (high-brightness/CABC-related) register set to max
- `0x2A 00 00 01 6F` — column address set, 0..0x016F (0..367) → matches `H_RES=368`
- `0x2B 00 00 01 BF` — row address set, 0..0x01BF (0..447) → matches `V_RES=448`
- `0x11` sleep-out, then 100 ms delay
- `0x29` display-on

`reset_gpio_num` passed to `esp_lcd_new_panel_co5300()` is `BSP_LCD_RST` (`GPIO_NUM_NC`
here) — the driver calls `esp_lcd_panel_reset()` regardless; with no GPIO wired, this is a
soft/command-only reset for this board (the panel's own power-on reset plus the `0x11`
sleep-out sequence is what actually brings it up cleanly). A later display-driver task
should NOT expect a hardware reset pulse to do anything on this board.

## Other onboard chips (facts for future milestones — not required by `board.h`)

All of the below sit on the **same shared I2C bus** as the touch controller
(`BOARD_I2C_PIN_SDA` / `BOARD_I2C_PIN_SCL`, GPIO15/GPIO14).

| Chip | Function | I2C address | Source |
|---|---|---|---|
| **TCA9554PWR** | 8-bit I2C GPIO expander. Drives `LCD_RESET` (EXIO0), `TP_RESET` (EXIO2), and other board-internal signals (SD card CS = EXIO7 per schematic, plus expansion pins). | `0x20` (all address pins strapped low, "000" variant) | Schematic (A0/A1/A2 pins) + `esp_io_expander_tca9554.h`: `ESP_IO_EXPANDER_I2C_TCA9554_ADDRESS_000 (0x20)`, and the BSP itself calls `esp_io_expander_new_i2c_tca9554(i2c_handle, BSP_IO_EXPANDER_I2C_ADDRESS, ...)` with that macro. |
| **PCF85063ATL** | RTC (real-time clock), backed by AXP2101 + backup battery pad for retention across main-battery swaps. | `0x51` | `waveshare/pcf85063a` component, `pcf85063a.h`: `#define PCF85063A_ADDRESS 0x51`. Used by ESP-IDF example `examples/esp-idf/91_pcf85063_rtc`. |
| **QMI8658C** | 6-axis IMU (3-axis accel + 3-axis gyro). | `0x6A` or `0x6B` (address-pin selectable; schematic shows `0X6B` strapped) | `waveshare/qmi8658` component, `qmi8658.h`: `QMI8658_ADDRESS_LOW (0x6A)` / `QMI8658_ADDRESS_HIGH (0x6B)`; example `92_qmi8658_imu` probes both and schematic net label shows `0X6B` next to U5, consistent with the high-address strap. |
| **AXP2101** | PMU (power management: battery charge/discharge, multiple regulated rails, IRQ on `AXP_IRQ`/`PWRON`/`PWROK`). | `0x34` (standard AXP2101 address, per the XPowersLib-based demo `examples/esp-idf/90_axp2101_pmu`, which references `AXP2101_SLAVE_ADDRESS` from the external XPowersLib — **not independently re-verified against a header in this repo**, flagged as lower-confidence than the other addresses in this table.) | Demo `examples/esp-idf/90_axp2101_pmu/main/main.cpp` (uses `PMU.begin(AXP2101_SLAVE_ADDRESS, ...)`); schematic sheet "POWER Supply" confirms AXP2101 (`UP1`) presence and IRQ/PWRON/PWROK nets. |
| **ES8311** | Low-power audio codec (speaker + mic path), driven over I2S (separate from the shared I2C bus's GPIO14/15 — I2S pins are `GPIO_NUM_9`/`16`/`45`/`8`/`10` per schematic/BSP `BSP_I2S_*` macros, plus `GPIO_NUM_46` power-amp enable). Codec control register access is over the shared I2C bus too. | `ES8311_CODEC_DEFAULT_ADDR` (standard ES8311 address, `0x18`) — used directly via the `es8311_codec_new()` / `esp_codec_dev` component in the v2 BSP; not independently re-verified against the `es8311` component header in this task. | BSP `esp32_s3_touch_amoled_1_8.c` (`ES8311_CODEC_DEFAULT_ADDR` used for both speaker and mic codec device config); schematic sheet "Codec"/"PA&SPEAKER&MIC". |

Not investigated further in this task (out of scope for `board.h`): NOR flash (16 MB,
SPI, dedicated `W25Q128JVSIQ` on the ESP32-S3's own SPI0/1 flash bus per schematic, not
user-addressable GPIO), microSD (SDMMC pins `GPIO_NUM_3`/`1`/`2` — `BSP_SD_D0/CMD/CLK` in
the BSP — relevant to a future storage task, not this one), BOOT/PWR buttons, and the
7-pin GPIO/UART/USB expansion pad.

## v1 vs v2 — everything that's known to differ

| Aspect | v1 | v2 (this board) |
|---|---|---|
| Panel driver IC | SH8601 | **CO5300** |
| Touch controller (silicon) | FT3168 | **CST820** |
| Touch controller (driver family / I2C addr) | FT5x06-compatible, `0x38` | **CST816S-compatible, `0x15`** |
| Touch coordinate quirk | none noted | 16px (`0x10`) X-axis gap correction applied in the v2 BSP |
| LCD/touch/I2C GPIO pin numbers | Same, per Waveshare's GitHub README table ("Original" vs "V2" example dirs use the same board pinout; only the driver chips differ) | Same |
| Arduino example/library set | `examples/arduino/` (`GFX_Library_for_Arduino` w/ SH8601, `Arduino_DriveBus` FT3168 touch) | `examples/arduino-v2/` (separate library set for CO5300/CST820) |
| ESP-IDF managed component deps | (v1 not exercised by the `esp-idf/00_bsp_quickstart` example, which pins to BSP `^2.0.3`) | `esp_lcd_co5300`, `esp_lcd_touch_cst816s` (+ `esp_lcd_touch_ft5x06` kept for v1 fallback probing in the same binary) |

No GPIO pin-number differences between v1 and v2 were found in any source consulted —
only the panel/touch silicon and their I2C addresses/quirks differ. This deskmate project
targets v2 exclusively; `board.h` encodes v2 values, and the FT3168/`0x38` path exists in
the BSP purely as a same-binary fallback for v1 units, which we do not need to support.

## Task 3: display bring-up — components, reset sequencing, verified on hardware

**Components used** (resolved via `idf.py add-dependency`, locked in
`firmware/dependencies.lock`): `espressif/esp_lcd_co5300` **2.1.0** (the CO5300 driver
exists in the registry — no SH8601 fallback needed), `lvgl/lvgl` **9.5.0** (`^9`),
`espressif/esp_lvgl_port` **2.8.0~1** (`^2`), `espressif/esp_io_expander_tca9554`
**2.0.3** (pulled in `espressif/esp_io_expander` 1.2.1 as its dependency).

### Correction to this task's own briefing: the Waveshare BSP does NOT pulse EXIO0 for LCD reset

This task's instructions asserted that "Waveshare's own BSP ... does exactly this
[pulses EXIO0 before panel init]" and told the implementer to read the BSP for the
pulse timing/levels. **That assertion does not hold** — verified by fetching and
reading the actual current BSP source directly
(`raw.githubusercontent.com/waveshareteam/Waveshare-ESP32-components/main/bsp/esp32_s3_touch_amoled_1_8/esp32_s3_touch_amoled_1_8.c`
and its header, same files Task 2 already cited). Findings:

- `bsp_display_new()` builds the panel with `.reset_gpio_num = BSP_LCD_RST`
  (`GPIO_NUM_NC`) and simply calls `esp_lcd_panel_reset(panel_handle)` — no IO-expander
  call anywhere in that function.
- `bsp_io_expander_init()` exists and creates the TCA9554 handle, but it is never
  called from `bsp_display_new()` or `bsp_touch_new()` in the file as fetched; nothing
  in the BSP source writes to EXIO0's direction or level register.
- This actually matches what **Task 2's own board-notes.md already said** (see the
  "LCD panel init quirks" section above, written before this task started): *"with no
  GPIO wired, this is a soft/command-only reset for this board ... A later
  display-driver task should NOT expect a hardware reset pulse to do anything on this
  board."* Task 2 had already found this; the Task 3 briefing's premise disagreed with
  it without new evidence.
- Confirmed in the `esp_lcd_co5300` driver source
  (`esp_lcd_co5300_spi.c:panel_co5300_reset()`): when `reset_gpio_num < 0` the driver
  sends `LCD_CMD_SWRESET` (0x01) over QSPI and delays 80 ms — a real, working software
  reset path, which is what the shipping Waveshare BSP actually relies on.

**What was implemented anyway, and why:** `board_i2c.c`/`display.c` still bring up the
I2C bus and the TCA9554 expander and pulse EXIO0 (output, low 10 ms, then high, settle
150 ms — timing borrowed from the co5300 driver's own *GPIO*-reset branch, the only
concrete timing fact available, since no EXIO0-specific timing exists anywhere) before
calling `esp_lcd_panel_reset()`/`esp_lcd_panel_init()`. Rationale: (a) the I2C bus +
expander are required regardless for Task 4's touch reset (EXIO2) and for probing the
CST816S touch controller, so bringing them up now costs nothing extra; (b) an extra,
real electrical reset ahead of the driver's own software reset cannot hurt and is cheap
insurance against a panel left in an unknown state by a previous firmware/power cycle.
The driver's software SWRESET still runs afterward unconditionally (`reset_gpio_num`
stays `GPIO_NUM_NC` in `board.h`, per the schematic). Both paths ran cleanly on hardware
with no errors — see log below.

### Brightness / display-on: no manual DCS command needed

The task briefing said to send DCS `0x51` (brightness) manually via
`esp_lcd_panel_io_tx_param` if the screen stays black. Reading
`esp_lcd_co5300_spi.c:vendor_specific_init_default[]` shows the CO5300 driver's
*default* init command table (used automatically whenever `co5300_vendor_config_t
.init_cmds` is left `NULL`, which `display.c` does) already includes `0x53 0x20`
(brightness-control-block enable) and `0x51 0xFF` (brightness = max) followed by sleep
-out (`0x11`) and display-on (`0x29`). So `board_display_init()` does not send any
manual brightness command — the driver's own default init already sets it. If a future
task wants a *specific* (non-max) brightness, use the driver's exposed
`esp_lcd_panel_co5300_set_brightness(panel, brightness_percent)` helper, or send DCS
`0x51` directly via `board_display_io()` — both are equivalent since that function just
wraps the same `tx_param(..., 0x51, ...)` call.

### Hardware verification

Flashed and captured serial boot log via a small pyserial script (DTR/RTS toggle +
8 s non-interactive read — `idf.py monitor` is interactive and was not used). Board
enumerated at **`/dev/cu.usbmodem1101`**, not `/dev/cu.usbmodem3101` as stated in the
task brief (USB port re-enumerated between Task 2 and Task 3 sessions — a session/OS
detail, not a hardware change). Boot log, no errors, single clean boot (no reset loop):

```
I (787) main_task: Calling app_main()
I (797) deskmate: deskmate M0 boot
I (807) deskmate: board: 368x448 LCD, QSPI CS=12 PCLK=11 D0-D3=4,5,6,7
I (807) board_i2c: I2C bus ready on port 0 (SDA=15, SCL=14)
I (977) board_display: LCD panel reset via TCA9554 EXIO0
I (977) board_display: Initialize QSPI bus
I (977) board_display: Install panel IO
I (977) board_display: Install CO5300 panel driver
I (977) co5300: version: 2.1.0
I (977) co5300_spi: LCD panel create success, version: 2.1.0
I (1167) board_display: Initialize LVGL port
I (1167) LVGL: Starting LVGL task
I (1187) board_display: display init complete (368x448)
I (1187) deskmate: board_display_init OK, io=0x3fce9ed0
I (1187) deskmate: test screen drawn, LVGL task running
I (1187) main_task: Returned from app_main()
I (1217) board_display: first LVGL flush completed
```

`first LVGL flush completed` is logged from an `LV_EVENT_FLUSH_FINISH` callback
registered on the `lv_display_t*` returned by `lvgl_port_add_disp()` — it firing 30 ms
after the label was drawn confirms a real QSPI color-data transaction completed and the
panel IO's `on_color_trans_done` callback (registered internally by `esp_lvgl_port`)
fired. This is the strongest evidence obtainable from logs alone that the display
pipeline is functioning end-to-end. **Human-verified since:** the user photographed the
running board — text rendered correctly, but with a bright green ~16px strip on the
right edge (uninitialized panel GRAM, the CO5300 x-gap quirk not yet applied at this
point in the bring-up). That defect was root-caused and fixed in Task 4 (see the x-gap
fix-round section below) and re-confirmed gone in that task's human verification pass.

### Post-review fix: the TCA9554 expander handle must be shared, not re-created

Review caught a real bug in the first version of `display.c`:
`board_lcd_expander_reset()` called `esp_io_expander_new_i2c_tca9554()` directly,
creating its own local, throwaway `esp_io_expander_handle_t` for the duration of the
reset pulse.

**Why that's wrong, verified in `managed_components/espressif__esp_io_expander_tca9554/esp_io_expander_tca9554.c`
(constructor at line ~54, `reset()` at line ~151):** `esp_io_expander_new_i2c_tca9554()`
unconditionally calls `reset(&tca9554->base)` as its last construction step, which
writes `DIR_REG_DEFAULT_VAL` (`0xFF`) and `OUT_REG_DEFAULT_VAL` (`0xFF`) to the
**physical chip's** direction and output registers — every single time the constructor
runs, not just the first time. Each `esp_io_expander_handle_t` only tracks its own
in-memory shadow of those registers and never re-reads silicon on construction. So if
Task 4 (or any future code) calls `esp_io_expander_new_i2c_tca9554()` again to drive
EXIO2 (touch reset), it would silently reset the whole chip — including EXIO0
(LCD_RESET) — back to all-input/all-high, and neither the display code's handle nor the
touch code's handle would know the other's handle had done this. Two independent
handles for one physical chip is unsafe by construction, not just by convention.

**Fix:** added `esp_io_expander_handle_t board_io_expander(void)` to
`board_i2c.h`/`board_i2c.c` (same file/pattern as `board_i2c_bus()` — both are
lazy-init, app-lifetime singletons). `display.c`'s `board_lcd_expander_reset()` now
calls `board_io_expander()` instead of the constructor directly. **Rule going forward:
all TCA9554 access (EXIO0 LCD_RESET, EXIO2 TP_RESET for Task 4, EXIO7 SD-card CS for any
future storage work) MUST go through `board_io_expander()`.** Nothing outside
`board_i2c.c` should ever call `esp_io_expander_new_i2c_tca9554()` directly. The
singleton handle is intentionally never freed/deleted — it needs to live exactly as
long as the process does, so there is no corresponding `board_io_expander_deinit()`.

Re-verified on hardware after the fix: same clean single-boot log as before, including
`LCD panel reset via TCA9554 EXIO0` and `first LVGL flush completed` — see the "Fix
verification" entry in `task-3-report.md` for the fresh capture.

## Task 4: touch bring-up — CST816S component, reset sequencing, verified on hardware

**Correction inherited from task briefing:** the task's own brief still named FT3168/
`esp_lcd_touch_ft5x06` (copied from the Waveshare wiki's v1-only prose); per the "touch
chip identity" discrepancy already recorded above, the controller task issued a
correction ahead of implementation directing use of **`espressif/esp_lcd_touch_cst816s`**
(CST820 is a member of the CST816S protocol family) at I2C address `0x15`, sharing the
bus/expander singletons from `board_i2c.h`. No new correction was needed beyond that —
implementing exactly per the corrected brief worked first try.

**Component used** (resolved via `idf.py add-dependency`, locked in
`firmware/dependencies.lock`): `espressif/esp_lcd_touch_cst816s` **1.1.1~2**, pulling in
`espressif/esp_lcd_touch` **1.2.1** (the base touch-controller abstraction, same
component family as `esp_lcd_co5300`'s panel abstraction) as a transitive dependency.

### TP_RESET (EXIO2) reset sequencing and I2C probe behavior

Read `esp_lcd_touch_cst816s.c` (`managed_components/espressif__esp_lcd_touch_cst816s/`)
before implementing: `esp_lcd_touch_new_i2c_cst816s()`'s internal `touch_cst816s_reset()`
only toggles a GPIO if `config.rst_gpio_num != GPIO_NUM_NC` — since TP_RESET lives on the
TCA9554 expander's EXIO2 (not a raw ESP32 GPIO, `BOARD_TOUCH_PIN_RST` is `GPIO_NUM_NC`,
same situation as `BOARD_LCD_PIN_RST`/EXIO0 in Task 3), the driver's own reset path is a
guaranteed no-op for this board. **The touch driver task must pulse EXIO2 itself before
probing**, exactly mirroring `display.c`'s `board_lcd_expander_reset()` pattern via the
shared `board_io_expander()` singleton (never a second
`esp_io_expander_new_i2c_tca9554()` call — see the Task 3 "Post-review fix" section
above; `touch.c`'s `board_touch_expander_reset()` calls `board_io_expander()`, not the
constructor).

Timing used: 10 ms assert-low (same as the EXIO0/LCD_RESET pulse, no board-specific
timing fact exists for TP_RESET either) followed by a **200 ms** settle delay after
release — longer than EXIO0's 150 ms, because CST816-family chips are commonly slower to
respond on I2C after reset than the CO5300 panel is.

**Observed on hardware: the probe succeeded on the very first attempt, no retry
needed.** `esp_lcd_touch_new_i2c_cst816s()` immediately read back a valid chip-ID
register (`CST816S: IC id: 183`), confirming real I2C ACK from the CST820 silicon at
address `0x15` right after the single reset pulse — the "CST816-family gotcha" (chip
silent until touched/freshly-reset) noted in the task briefing did not manifest here;
the retry-after-second-reset-pulse code path (`board_touch_init()`'s `if (err != ESP_OK)`
branch) exists in `touch.c` for robustness but was never exercised in this bring-up.

`BOARD_TOUCH_PIN_INT` (GPIO 21) is passed through to the touch config as-is;
`esp_lvgl_port`'s `lvgl_port_add_touch()` auto-detects a non-`GPIO_NUM_NC` `int_gpio_num`
and switches the LVGL input device to `LV_INDEV_MODE_EVENT` (interrupt-driven) instead of
polling, installing its own GPIO ISR via `esp_lcd_touch_register_interrupt_callback_with_data()`
— no manual ISR wiring was needed in `touch.c`.

### Hardware verification

Flashed and captured serial boot log via the same pyserial DTR/RTS-toggle script used in
Task 3 (12 s non-interactive read, `idf.py monitor` not used — same
`/dev/cu.usbmodem1101` enumeration as Task 3). Clean single boot, no errors, touch init
completes ~200 ms after the display's first LVGL flush (the EXIO2 settle delay):

```
I (809) board_i2c: I2C bus ready on port 0 (SDA=15, SCL=14)
I (819) board_i2c: TCA9554 IO expander ready at 0x20
I (979) board_display: LCD panel reset via TCA9554 EXIO0
...
I (1219) board_display: first LVGL flush completed
I (1399) touch: touch controller reset via TCA9554 EXIO2
I (1399) CST816S: IC id: 183
I (1399) touch: CST816S touch controller initialized, handle=0x3fcec440
I (1399) touch: touch registered as LVGL input device, indev=0x3fc9d098
I (1399) deskmate: board_touch_init OK
I (1409) main_task: Returned from app_main()
```

No further log lines appeared for the remaining ~10.6 s of the 12 s capture window
(board sat untouched on a desk) — no spurious touch events, no I2C errors, no reset
loop. This satisfies the "quiet idle" verification requirement.

A throttled (`ESP_LOGI`, max ~5/sec, tag `touch`) coordinate log line is wired into
`main.c`'s `screen_pressed_cb()` (registered on `LV_EVENT_PRESSING`) alongside a green
20x20px dot (`lv_obj_t *s_dot`) that jumps to the LVGL press point — both were exercised
in the sense that they compile and are wired to a live `lv_indev_t*`. **Human-verified
since:** after the fix round below (dot no longer marked `CLICKABLE`, panel x-gap
applied), the user confirmed on hardware that the dot tracks the finger smoothly with
correct axis directions and no swap/mirror needed — no further axis correction was
required. (One cosmetic item was observed and deferred, not axis-related: brief
touch-drag trail artifacts, self-clearing after refresh, suspected double-buffer
dirty-region sync — flagged as a watch item for M2 real widgets, see the M0 exit notes
at the end of this file.)

**Known open item for the human verification pass:** the earlier "touch chip identity"
research (above) found the Waveshare BSP applies a **16px (`0x10`) X-axis gap
correction** specific to the CST816S path (`BSP_LCD_CST816S_X_GAP`) that this task's
`touch.c` does **not** replicate (no `process_coordinates` callback is set in the
`esp_lcd_touch_config_t`). If the human finger test shows the green dot consistently
offset by ~16px on the X axis (dot appears shifted right/left of the actual finger
position by a small, constant amount, not a full mirror/swap), that gap correction is
the likely fix — add a `process_coordinates` callback that subtracts/adds 16 from `x`
before passing coordinates to LVGL.

### Fix round (post-review + human hardware verification): `BSP_LCD_CST816S_X_GAP` is a PANEL gap, not (only) a touch-coordinate quirk

The above paragraph undersold what `BSP_LCD_CST816S_X_GAP` (0x10 = 16) actually does.
Re-reading the cached Waveshare BSP source in full
(`bsp/esp32_s3_touch_amoled_1_8/esp32_s3_touch_amoled_1_8.c`, functions
`bsp_display_set_x_gap()` and `bsp_touch_new()`):

```c
static esp_err_t bsp_display_set_x_gap(uint16_t x_gap) {
    panel_x_gap = x_gap;
    if (panel_handle != NULL) {
        return esp_lcd_panel_set_gap(panel_handle, panel_x_gap, 0);
    }
    return ESP_OK;
}
// ...in bsp_touch_new(), after probing which touch chip ACKs:
if (CST816S detected) { x_gap = BSP_LCD_CST816S_X_GAP; /* = 16 */ }
// ...
ESP_RETURN_ON_ERROR(bsp_display_set_x_gap(x_gap), TAG, "");
```

So the BSP calls **`esp_lcd_panel_set_gap(panel, 16, 0)`** — a real CO5300 *panel*
column-address-window correction (`esp_lcd_co5300_spi.c:panel_co5300_set_gap()` shifts
every `CASET`/`RASET` window by `x_gap`/`y_gap` before writes) — and it keys the decision
of *whether* to apply that 16px panel gap off of *which touch chip probed successfully*.
That's because the BSP's binary supports both hardware revisions at once: v1
(SH8601+FT3168) needs no panel gap, v2 (CO5300+CST820) does, and probing the touch chip
is the BSP's only runtime signal for which revision is attached. It is **not** primarily
a touch-coordinate-processing quirk — the original wording above ("X-axis gap correction
... specific to the CST816S path") was misleadingly touch-centric; the effect lands on
the **panel's own GRAM addressing**, not on touch `x`/`y` values.

**Symptom this caused:** without calling `esp_lcd_panel_set_gap()`, `display.c` was
writing pixel data to CO5300 GRAM columns `[0, 368)`, but this panel's visible area
apparently maps to a GRAM window offset by 16 columns — so the rightmost ~16px of the
368px-wide visible panel showed raw, never-written GRAM content (a bright green
vertical strip), confirmed by the user's photo of the running test screen.

**Fix applied:** since this project only targets the v2/CST820 hardware (confirmed;
no v1 support needed), `display.c` hardcodes the gap rather than probing for it —
no dependency on touch bring-up order. Added `#define BOARD_LCD_X_GAP 16` and a call
`esp_lcd_panel_set_gap(panel, BOARD_LCD_X_GAP, 0)` right after `esp_lcd_panel_init()`
and before `esp_lcd_panel_disp_on_off()`, matching the BSP's own ordering (reset → init
→ set_gap → disp_on_off). `y_gap` stays 0 — no vertical strip was observed or expected
(the BSP's own call also always passes `y_gap = 0` for this board).

Re-verified on hardware after this fix (see Task 4's fix report in `task-4-report.md`
for the full capture): clean boot, no errors, `first LVGL flush completed` still
present. **Human-verified since:** the user confirmed on the physical panel that the
green strip is gone — the mechanism matches exactly what Waveshare's own shipping BSP
does for this board revision, and the fix is confirmed effective, not just
log-consistent.

### LVGL framebuffer placement

The two LVGL draw buffers configured in `lvgl_port_display_cfg_t`
(`buffer_size = BOARD_LCD_H_RES * 80` pixels × `sizeof(uint16_t)` = **58,880 bytes
each**, `double_buffer = true`) are allocated in **internal DMA-capable SRAM**, not
PSRAM — `.flags.buff_dma = 1` with `.flags.buff_spiram` left unset (0). This is
deliberate: partial-refresh flushes over QSPI go through GDMA, and internal SRAM gives
the highest, most predictable DMA throughput for that traffic. PSRAM (8 MB, confirmed
present in the boot log: `esp_psram: Found 8MB PSRAM device`) is left as headroom for
larger allocations later (fonts, images, a full-frame buffer) rather than used for these
partial-refresh line buffers.

## Task 5: brightness control + 180-degree rotation

**Brightness (`board_display_set_brightness()`):** a single, direct call to
`esp_lcd_panel_io_tx_param(s_io, 0x51, &level, 1)` — DCS "Write Display
Brightness", one byte, 0-255. `board_display_init()` now calls this function
itself (with `BOARD_LCD_INIT_BRIGHTNESS = 255`) right after LVGL setup
completes, and logs it (`board_display: brightness set to 255/255` in the
boot capture below). This does **not** replace the co5300 driver's own
internal `0x51 0xFF` inside its default init cmd table (see the Task 3 note
above) — that one still fires unconditionally inside `esp_lcd_panel_init()`,
which this project doesn't override with a custom `init_cmds` table. The
init-time call to `board_display_set_brightness()` is a deliberate follow-up
so that every future caller (config code in later milestones, this task's
own tap-zone test) has exactly one function to call and one log line to grep
for, rather than needing to know the driver sends its own copy internally
too. Sending the same DCS command twice at boot is harmless (idempotent).

**Rotation path chosen: LVGL 9 software rotation, not CO5300 MADCTL mirror.**
The brief's suggested Step 2 (`esp_lcd_panel_mirror(s_panel, on, on)`) was
evaluated first but **not implemented**, for a reason specific to this
board's `BOARD_LCD_X_GAP` fix (see the Task 4 fix-round section above):

- Read `esp_lcd_co5300_spi.c`: `panel_co5300_mirror()` only flips MADCTL bits
  6/7 (`LCD_CMD_MADCTL`) and sends that one command — it does not touch
  `co5300->x_gap`/`y_gap` at all. `panel_co5300_draw_bitmap()` *always*
  applies `x_start += co5300->x_gap` (16, fixed via `esp_lcd_panel_set_gap()`
  once after init) to every `CASET` window it sends, **regardless of mirror
  state**. Whether flipping MADCTL's MX/MY bits changes which *physical*
  glass columns that same numeric CASET range (16..383) lands on is a
  controller-specific hardware behavior this driver's source does not
  resolve either way, and there is no way to determine it from logs alone —
  it can only be confirmed by looking at the panel. Getting it wrong risks
  moving the already-fixed green uninitialized-RAM strip to the *other* edge
  in the rotated orientation, which is exactly the failure mode the task's
  acceptance test (no green strip in *either* orientation) is designed to
  catch, and exactly the scenario the task instructions call out as the
  trigger for falling back to software rotation.
- Read `managed_components/espressif__esp_lvgl_port/src/lvgl9/esp_lvgl_port_disp.c`
  (`lvgl_port_disp_rotation_update()`, `lvgl_port_add_disp_priv()`,
  `lvgl_port_flush_callback()`): `esp_lvgl_port` already implements *both*
  paths behind one config flag. With `disp_cfg.flags.sw_rotate` left `0`
  (false), calling `lv_display_set_rotation(disp, LV_DISPLAY_ROTATION_180)`
  makes `esp_lvgl_port` itself call
  `esp_lcd_panel_mirror(panel, true, true)` — i.e. the brief's suggested
  code, just invoked through the "idiomatic" LVGL rotation API instead of a
  direct call. With `disp_cfg.flags.sw_rotate = 1` (what this task sets),
  `lvgl_port_disp_rotation_update()` returns immediately without touching
  the panel's MADCTL/mirror/swap_xy *at all*
  (`if (disp_ctx->flags.sw_rotate) { return; }`), and instead
  `lvgl_port_flush_callback()` calls `lv_draw_sw_rotate()` on the rendered
  pixel buffer for the dirty rectangle, then `lvgl_port_rotate_area()` maps
  the flush target rectangle back into the *same* physical coordinate space
  as always (still `[0,368) x [0,448)` before the driver's own fixed
  `x_gap` is added in `panel_co5300_draw_bitmap()`, completely unchanged).
  This sidesteps the mirror/gap composition question entirely: the CO5300's
  own addressing convention, and the human-verified (Task 4) `x_gap = 16`
  fix, are never touched by rotation at all.
- This project has no PPA (`LVGL_PORT_PPA`; ESP32-S3 doesn't have the PPA
  peripheral, that's ESP32-P4-only), so `sw_rotate = 1` takes
  `esp_lvgl_port`'s plain-malloc software path: one extra
  `buffer_size * 2 bytes` (58,880 bytes, matching each existing flush
  buffer) internal-DMA-SRAM scratch buffer, allocated once at
  `lvgl_port_add_disp()` time. Confirmed on hardware this fits: the boot
  capture below shows `lvgl_port_add_disp()` (`board_display: display init
  complete`) succeeding with no allocation-failure log, on top of the two
  existing 58,880-byte flush buffers (176,640 bytes internal SRAM total now,
  up from 117,760 before this task).

**Touch consistency under rotation — corrected after code review.** The
first version of this task shipped a bug here: it claimed (based on a grep
of `lv_indev.c` for the substrings `rotat`/`ROTATION`) that nothing in this
stack auto-remaps touch coordinates for display rotation, and had
`board_display_set_rotation_180()` call `esp_lcd_touch_set_mirror_x/y()` on
the shared CST816S handle to compensate manually. **The grep conclusion was
wrong** — the actual function name is `lv_display_rotate_point()` (contains
"rotate", not "rotation"), which the `rotat`/`ROTATION` search pattern missed
entirely. Re-reading the source properly:

- `managed_components/lvgl__lvgl/src/indev/lv_indev.c`, `indev_pointer_proc()`
  (called from `indev_proc()` for every pointer indev, unconditionally) calls
  `lv_display_rotate_point(i->disp, &data->point)` on every read, for every
  pointer indev, with no flag gating it.
- `managed_components/lvgl__lvgl/src/display/lv_display.c`,
  `lv_display_rotate_point()`: for `LV_DISPLAY_ROTATION_180`, does exactly
  `point->x = disp->hor_res - x - 1; point->y = disp->ver_res - y - 1;` — a
  point reflection, remapping the touch controller's raw physical coordinate
  into the logical coordinate frame LVGL's widget tree is laid out in.

This is a completely different subsystem from `esp_lvgl_port`'s
`sw_rotate` flag (which only controls how *pixel output* reaches the panel —
MADCTL mirror vs. software buffer rotation, see above); LVGL core's own
indev processing remaps pointer input for the display's rotation
**unconditionally**, regardless of that flag. So the touch-mirror calls
added on top of this were a **second, redundant reflection**: LVGL core
already reflected the raw touch point once (correctly), and
`esp_lcd_touch_set_mirror_x/y()`'s software adjustment path in
`esp_lcd_touch.c` reflected it a second time — the two cancelled out,
leaving effective touch coordinates un-rotated while the rendered image
flipped 180°. Net symptom this would have caused: after toggling rotation,
the green dot (and any tap-zone hit test) would land at the *mirror-opposite*
point from the finger instead of under it.

**Fix:** removed the `esp_lcd_touch_set_mirror_x/y()` calls from
`board_display_set_rotation_180()` entirely, and removed the
`board_touch_handle()` accessor added to reach the touch handle (it had no
other caller once the mirror calls were gone — `touch.c` no longer stores a
`static esp_lcd_touch_handle_t` for this purpose either). LVGL core's own
`lv_display_rotate_point()` is correct and sufficient on its own; no
driver-level touch transform should be layered on top of it for this
project's rotation handling. `board_display_set_rotation_180()` now only
calls `lv_display_set_rotation()` — nothing touch-specific.

### Hardware verification

Flashed and captured serial boot log via the same pyserial DTR/RTS-toggle
script used in Tasks 3/4 (8 s non-interactive read, same
`/dev/cu.usbmodem1101` enumeration). Clean single boot, no errors, no reset
loop, brightness set at init through the new function:

```
I (810) board_i2c: I2C bus ready on port 0 (SDA=15, SCL=14)
I (820) board_i2c: TCA9554 IO expander ready at 0x20
I (980) board_display: LCD panel reset via TCA9554 EXIO0
...
I (1170) board_display: display init complete (368x448)
I (1170) board_display: brightness set to 255/255
I (1170) deskmate: board_display_init OK, io=0x3fce9ed0
I (1180) board_display: first LVGL flush completed
I (1200) deskmate: test screen drawn, LVGL task running
I (1410) touch: touch controller reset via TCA9554 EXIO2
I (1410) CST816S: IC id: 183
I (1410) touch: CST816S touch controller initialized, handle=0x3fcec440
I (1410) touch: touch registered as LVGL input device, indev=0x3fc9d0d0
I (1410) deskmate: board_touch_init OK
I (1420) main_task: Returned from app_main()
```

No rotation-toggle log line appears in this capture, as expected — the
board sat untouched, so `board_display_set_rotation_180()` was never called,
and rotation stays at its default (`LV_DISPLAY_ROTATION_0`, mirror off).

**What this capture can and cannot prove:** it proves brightness now routes
through `board_display_set_brightness()` (logged), the extra SW-rotation
scratch buffer allocated successfully, and the boot sequence is otherwise
identical to Tasks 3/4's already human-verified state (no new errors, no
reset loop). It **cannot** prove: that the three brightness levels are
visibly distinct on the physical panel, that 180-degree rotation actually
flips the visible image with no green strip on either edge, or that the
touch dot still tracks the finger correctly post-rotation. All three require
a human tapping the physical top/bottom halves of the panel and are flagged
as pending human verification in `task-5-report.md`.

### Fix round 2 (human hardware verification): rotation+touch confirmed working; brightness invisible + drag-scrolled the screen

Human verification of the touch-mirror-removal fix (commit `a25f34d`) confirmed the
Critical fix worked: 180-degree rotation flips the image and the green dot tracks the
finger correctly in both orientations. Two new findings came back from the same pass.

**Finding 1: brightness had zero visible effect, despite `ESP_OK` and a logged level.**
Root cause: this is a **QSPI** panel, and `esp_lcd_panel_io_tx_param()` on a QSPI LCD IO
handle expects the "command" argument to be a full 32-bit **command envelope**, not a
bare DCS command byte. Confirmed two ways:

- `managed_components/espressif__esp_lcd_co5300/esp_lcd_co5300_spi.c`: every single
  command the driver itself ever sends — including its own default init table's `0x53
  0x20` (BCTRL enable) and `0x51 0xFF` (init brightness) — goes through the driver's
  internal `tx_param()` helper, which does this whenever `use_qspi_interface` is set
  (true for this board, `co5300_vendor_config_t.flags.use_qspi_interface = 1` in
  `board_display_init()`):
  ```c
  lcd_cmd &= 0xff;
  lcd_cmd <<= 8;
  lcd_cmd |= LCD_OPCODE_WRITE_CMD << 24;   // LCD_OPCODE_WRITE_CMD == 0x02
  ```
  i.e. the real 32-bit word is `(0x02 << 24) | (cmd << 8)`, not just `cmd`.
- Cross-checked against Waveshare's own BSP, fetched directly
  (`raw.githubusercontent.com/waveshareteam/Waveshare-ESP32-components/main/bsp/esp32_s3_touch_amoled_1_8/esp32_s3_touch_amoled_1_8.c`):
  `bsp_display_brightness_set()` builds the *exact same* 32-bit word by hand for this
  exact board (`lcd_cmd = 0x51; lcd_cmd &= 0xff; lcd_cmd <<= 8; lcd_cmd |= 0x02 << 24;`)
  before calling `esp_lcd_panel_io_tx_param(io_handle, lcd_cmd, &param, 1)` — confirming
  this isn't a co5300-driver-specific convention but the actual protocol this panel's
  QSPI command decoder requires.

  The BSP does **not** re-send `0x53`/BCTRL inside `bsp_display_brightness_set()` either —
  consistent with BCTRL only needing to be enabled once, which both the co5300 driver's
  default init table and this project's (unmodified, `init_cmds = NULL`) init already do
  before `board_display_set_brightness()` is ever reachable.

  The earlier `board_display_set_brightness()` sent a **bare** `esp_lcd_panel_io_tx_param(s_io, 0x51, ...)`
  — a different (invalid, unenveloped) 32-bit command word from the panel decoder's point
  of view. The SPI transaction itself completes fine (hence `ESP_OK` and a clean log
  line), but the panel has no idea a brightness write was intended, so nothing visibly
  changed. Note the co5300 driver *also* exposes a ready-made public API for this
  (`esp_lcd_panel_co5300_set_brightness(panel, percent_0_100)`, backed internally by the
  same enveloped `tx_param()`), but it takes a 0-100 percentage rather than this project's
  0-255 `level` interface, so wrapping the command by hand (matching the driver/BSP
  convention exactly) was used instead of introducing a lossy percent round-trip.

  **Fix:** `firmware/main/board/display.c` now builds
  `BOARD_LCD_QSPI_CMD(0x51) = (0x02UL << 24) | ((0x51UL) << 8)` and sends that as the
  `esp_lcd_panel_io_tx_param()` command argument, with `level` (0-255) as the one-byte
  parameter — unchanged.

**Finding 2: swiping a finger dragged the "deskmate M0" label across the screen.**
Root cause: `lv_screen_active()`'s root object is `LV_OBJ_FLAG_SCROLLABLE` by default in
LVGL 9 (like any `lv_obj_t`), and it has content (the label) that can visually appear to
"scroll" under a drag even with no scrollbar rendered. **Fix:** `firmware/main/main.c`
now calls `lv_obj_remove_flag(scr, LV_OBJ_FLAG_SCROLLABLE);` immediately after obtaining
`scr`, before any styling or child objects are added. **This must be repeated for Task
6's clock screen** (and any future screen/container that has no intended scroll content)
— note added here so it isn't missed.

**Deferred, not touched this round:** the green dot's transient trail artifacts, flagged
by the human tester as a watch item but explicitly out of scope for this fix round.

Re-verification (rebuild + reflash + boot-log capture) for this round is recorded in
`task-5-report.md`'s fix-round-2 section. **Human-verified since:** the user confirmed
brightness now visibly steps across the three tapped levels, and the screen no longer
drags/scrolls under a touch swipe. Combined with the earlier fix round's confirmed
rotation+touch-tracking result, all of Task 5's human-only checks (rotation flip, dot
tracking under rotation, brightness steps, no drag) are now confirmed on hardware.

## Task 6: standalone clock screen

No new hardware quirks — reuses the already-bring-up-verified display/touch/brightness/
rotation stack as-is. `firmware/main/ui/clock_screen.c` builds a plain LVGL screen (big
HH:MM label in Montserrat 48, a smaller date line, a dim static hint line), clears
`LV_OBJ_FLAG_SCROLLABLE` on its own active-screen object (same fix Task 5 applied,
required again here since this is a different `lv_obj_t`), and updates its labels only
on minute rollover (not every 1 Hz tick) via a check against the previously-logged
minute. A `clock`-tagged `ESP_LOGI` free-heap line is emitted on that same once-per-minute
gate — this is the heap log the Task 7 soak below watches. `firmware/main/core/timefmt.c`
(HH:MM/date string formatting) has zero ESP-IDF/LVGL includes and is compiled and run
standalone by `firmware/host_tests/` on the host, independent of the target build —
satisfying the spec's host-testable-core split for this milestone.

## M0 exit — 30-minute soak test (Task 7)

Soak observed the already-flashed, already-running clock firmware (commit `49c6cf0`) via
a detached pyserial logger attached to `/dev/cu.usbmodem1101` at 115200 baud without
touching DTR/RTS (so the attach itself does not reset the board) — see
`task-7-report.md` for the exact log excerpts and pass/fail table.

**Result: PASS.** Observation window 2026-08-03 22:59:01 -> 23:32:49 host time
(~33.8 minutes, exceeding the 30-minute requirement), watching the clock firmware
(commit `49c6cf0`) that was already running before this soak started (no reflash).

- **No reboot:** the ESP-IDF uptime tag in every log line (`I (millis) clock: ...`)
  increased monotonically for the entire window with no reset back to a small value
  and no bootloader banner / repeated `app_main()` line appeared. One capture gap
  in the host-side log exists (`23:10:49` -> `23:29:49`, ~19 minutes with no lines
  appended) — cross-checked against the uptime counter either side of the gap
  (`1172054` ms -> `2311334` ms = 1,139,280 ms elapsed) against the real host-clock
  gap (~1,140,000 ms): they match to within noise, proving the board kept running
  continuously through the gap rather than rebooting. The gap itself is attributed to
  the host-side detached logger process/sandbox pausing (e.g. host idle/sleep
  behavior), not the device — the device-side evidence (monotonic uptime, identical
  heap before and after) is what actually establishes "no reboot," independent of
  that host-side capture hiccup.
- **No error lines:** `grep -inE "rst:0x|Guru Meditation|abort\(\)|E \(|panic|CORRUPT HEAP|assert failed"`
  across the full captured log returned zero matches.
- **Heap log present (`clock` tag, ~once/min) and floor flat:** 16 samples captured,
  every single one reporting the exact same value, **8,493,783 bytes free** — no
  decline, no leak, across the whole window (and matching the value already seen in
  Task 6's own capture, i.e. this is the same long-lived boot session Task 6 verified,
  now soaked for 30+ minutes past that point with zero drift).

**Observed free-heap floor: 8,493,783 bytes** (flat for the entire soak window).

No visual artifacts could be checked by this agent (no camera/screen access to the
physical board) — the pass criteria above are the log-observable ones per the M0 exit
task's scope; any residual visual concern is the touch-drag trail item captured below,
which is a human-observed (not soak-observed) watch item.

## Known open item carried into M1/M2

**Transient touch-drag trail artifacts:** first observed by the human tester on the
Task 4/5 test screen (a temporary green dot + label used only for touch/rotation
verification, since removed) — brief visual trail/smear artifacts appeared at touched
spots, self-clearing after the next refresh. Harmless on that throwaway test screen and
explicitly deferred rather than root-caused during Tasks 4/5/6. Suspected cause: LVGL's
double-buffered partial-refresh dirty-region sync (each of the two 58,880-byte flush
buffers only has the current frame's dirty rectangle redrawn, so a stale pixel from the
other buffer's previous frame can briefly show through at the seam between two flushes).
**Watch item for M2:** re-observe this on real widgets (not a throwaway test screen) once
M2 introduces more dynamic UI; if it reproduces, the fix is likely either forcing a
full-buffer invalidate on the affected widget's redraw or switching that widget's
containing screen to `LV_DISPLAY_RENDER_MODE_FULL` instead of partial.

**M1 investigation:** landscape at 90 degrees reproduced this much more severely: the
clock itself distorted and persistent green rectangles plus stale text appeared across
the panel. Switching from double to single buffering did not change the photographed
corruption, disproving the shared rotation-scratch race as its cause. A forced
synchronous full-canvas repaint also made no visual difference.

The board-specific omission was found by comparing against Waveshare's current v2 BSP.
Waveshare commit `144d255da2ba93ddae2a976ba653a4358a1a295d` ("Update ESP32-S3
AMOLED 1.8 display and touch drivers") changed this product from SH8601 to CO5300 and
added an LVGL rounder in the same patch. It expands every invalid area to an even x/y
start and odd x/y end. This is not cosmetic with landscape partial buffers: 29,440
pixels divided by the 448-pixel logical width yields 65 rows. Without the rounder, a
full repaint is transmitted as 65-row pieces which software rotation turns into
odd-width CO5300 column windows. The rounder makes LVGL's buffer-fit calculation step
down to 64 rows and keeps every panel window and payload two-pixel aligned. M1 now
supplies that callback through `lvgl_port_display_cfg_t.rounder_cb`. The aligned build
flashed and passed protocol boot status/time-sync. The user then confirmed the physical
display was clean at both 90 and 270 degrees, brightness and touch remained functional,
and the green blocks, diagonal text, and other artifacts were gone.

The similar-looking Espressif software-rotation report `esp-bsp#400` was also traced to
its exact merged fix, commit `3e6a581b31c9504801b5ea7a71291f1a09540849`. That patch
corrected the 90/270 enum passed to `lv_draw_sw_rotate`; the local `esp_lvgl_port` 2.8.0
already contains it, so it was ruled out rather than reapplied.

## M1 Task 1: USB transport proof

### USB-C data wiring

The official one-page Waveshare schematic traces both orientations of the USB-C data
pair into `USB_N`/`USB_P`, through series resistors R19/R20 (22 ohm), and then to the
ESP32-S3R8's GPIO19/GPIO20 pins. Espressif's ESP32-S3 USB device documentation defines
GPIO19 as the internal-PHY USB D- pin and GPIO20 as USB D+. This establishes that the
board's only USB-C connector is electrically usable by the ESP32-S3 USB-OTG peripheral,
not just for power. M0's earlier flashing/log capture through `/dev/cu.usbmodem1101`
also established real data connectivity, although it used the ROM/USB-Serial-JTAG path
rather than the new TinyUSB application descriptors.

Sources:

- Waveshare schematic:
  https://files.waveshare.com/wiki/ESP32-S3-Touch-AMOLED-1.8/ESP32-S3-Touch-AMOLED-1.8.pdf
- ESP-IDF USB Device Stack documentation:
  https://docs.espressif.com/projects/esp-idf/en/v5.5/esp32s3/api-reference/peripherals/usb_device.html

### Rejected TinyUSB dual-CDC spike and selected native transport

- Component manifest constraint: `espressif/esp_tinyusb ^2.0.0`.
- Locked versions under ESP-IDF 5.5.5: `espressif/esp_tinyusb` **2.2.1** and its
  `espressif/tinyusb` dependency **0.21.0~1**.
- Development descriptor IDs use Espressif's VID **0x303A** and esp_tinyusb's automatic
  dual-CDC PID **0x4002**. These IDs are appropriate for development only and require a
  product-owned VID/PID decision before distribution.
- Product string: `Deskmate Desk Display`. The 12-hex-digit serial string comes from the
  ESP32-S3 factory base MAC, so multiple development units do not share the component's
  default placeholder serial.
- CDC 0 string: `Deskmate diagnostics`; standard output/error and ESP-IDF logs are
  redirected here after TinyUSB initialization.
- CDC 1 string: `Deskmate protocol`; its callback only performs a bounded read into a
  4096-byte static stream and queues bytes. It never prints logs or echoes traffic.
- The protocol TX buffer is 4096 bytes so one maximum protocol v1 COBS wire frame
  (2058-byte worst case including delimiter) fits atomically when empty.

Software verification on 2026-08-04: pre-change M0 native host tests passed; the
pre-change ESP-IDF build produced `0xb4210` bytes (30% free). After adding dual CDC, the
firmware compiled and linked successfully at `0xbd9e0` bytes (26% free). A second,
isolated build using a fresh temporary `sdkconfig` confirmed that the checked-in defaults
independently enable both CDC interfaces. Rust installed for M1 is `rustc 1.97.1`,
`cargo 1.97.1`, `rustup 1.29.0`.

The board was connected later on 2026-08-04. Before flashing, it enumerated through the
native USB Serial/JTAG peripheral as `303A:1001` at `/dev/cu.usbmodem1101`. The unrelated
test firmware present on the board initialized the v2 peripherals and display cleanly;
it was not the Deskmate M0 image, so this did not count as an M0 regression run.

The TinyUSB image enumerated at full speed as `303A:4002`, product `Deskmate Desk
Display`, serial `A4CB8FDB3328`, with macOS nodes ending in `...33281` (diagnostics,
interfaces 0/1) and `...33283` (protocol, interfaces 2/3). It then rebooted repeatedly.
A temporary five-second diagnostic window captured the deterministic failure:

```
E (...) LVGL: lvgl_port_add_disp_priv(471): Not enough memory for LVGL buffer (rotation buffer) allocation!
```

The spike also could not enter the ROM downloader through esptool's normal DTR/RTS
sequence because custom USB-OTG CDC line state does not preserve the native peripheral's
automatic reset behavior. Physical recovery was verified: hold BOOT, reset/power-cycle,
release BOOT, flash `/dev/cu.usbmodem1101`, then reset once without BOOT.

The selected M1 transport is therefore the ESP32-S3 native USB Serial/JTAG CDC using
`usb_serial_jtag_driver_install()`, with fixed 4096-byte RX/TX rings. It is initialized
after LVGL secures its DMA and rotation buffers. `CONFIG_ESP_CONSOLE_SECONDARY_NONE=y`
keeps boot and ESP-IDF logs off the protocol CDC; diagnostics remain on UART0. This
preserves the known-good `303A:1001` descriptor and normal `idf.py flash` workflow.
TinyUSB and its managed dependency were removed.

### M1 integrated native-USB hardware pass

The complete M1 firmware now uses only the native driver, with 4096-byte bounded RX/TX
rings and a protocol task pinned to CPU1; LVGL is explicitly pinned to CPU0. The task
dispatches the frozen v1 fixtures, exposes live display/link/parser status, applies UTC
time plus the host's fixed offset, retains the latest monotonic push revision, and
returns the clock hint after ten seconds without a valid request. UI mutations are
queued through `lv_async_call()` and therefore execute in LVGL context.

Software verification on 2026-08-04 passed the default and sanitizer C suites, Rust
format/lint/tests, and the ESP-IDF 5.5.5 build. The integrated `deskmate.bin` is
`0xb77e0` bytes after the M1 landscape/artifact correction, leaving `0x48820` bytes (28%) of
the smallest app partition free.

The test board returned to native `303A:1001` enumeration at
`/dev/cu.usbmodem1101`. A normal `idf.py -p /dev/cu.usbmodem1101 flash` installed the
integrated M1 image in one pass with image-hash verification and automatic hard reset.
The same normal flash command then succeeded a second time from the running M1 image,
confirming that the rejected TinyUSB spike no longer affects the reset/download path.

The first clean `status --json` reported protocol v1, display `368x448`, brightness
200, rotation 0, free heap 8,462,035 bytes, latest revision 0, and all parser/transport
counters at zero. `time-sync --json` acknowledged Unix time with the local fixed UTC
offset of +240 minutes, and `push-data --json` accepted widget `weather`, revision 1.
After the second flash reset the volatile state, status, time sync, and the revision-1
push all succeeded again. A subsequent `status --json` invocation omitted `--port` and
correctly selected `/dev/cu.usbmodem1101` by USB metadata plus the v1 status handshake.

The checked `hardware_acceptance` harness passed in one serial session. It exercised
split and coalesced requests, bad CRC, garbage, an overlong frame, invalid push data,
invalid time, and stale revision rejection, and then received a clean final status:

```
PASS port=/dev/cu.usbmodem1101 uptime_ms=36653 heap=8462035 valid=6->14 malformed=0->3 crc=0->1 overflow=0->1 dropped=0 rx_drops=0
```

The 30-minute active heartbeat soak also passed without a device reset or heap loss:

```
PASS port=/dev/cu.usbmodem1101 elapsed=1800s heartbeats=1791 uptime_ms=2332440 heap_floor=8462035 valid=1827 malformed=0 crc=0 overflow=0 dropped=0 rx_drops=0
```

There was one host suspension between the 11- and 12-minute reports: device uptime
advanced about 554 seconds while harness active time advanced 60 seconds. The same open
serial session resumed and completed with flat heap and clean counters. This proves
recovery across that host pause, but the interval is not represented as uninterrupted
one-hertz traffic.

The selected native driver fixes both rings at 4096 bytes but exposes neither ring
occupancy nor an RX-overflow total. Accordingly, `rx_drops=0` denotes the unavailable
native metric, not a proof that the hardware ring never dropped a byte; parser overflow,
dropped response, heap-floor, and successful resynchronization are the observable
pressure evidence.

The final physical unplug/replug carryover passed on 2026-08-04. Cycle 1 changed the
macOS device node from `/dev/cu.usbmodem3101` to `/dev/cu.usbmodem1101` and was followed
by a clean v1 status handshake. Nine further cycles were observed as distinct removal
and enumeration edges; each remained disconnected for 5-7 seconds and returned on
`/dev/cu.usbmodem1101`. An immediate explicit-port status after the final enumeration
raced device-node readiness and returned `ENOENT`; a time sync, retried status, and
automatic-discovery status all succeeded without another cable action. Final status
reported uptime 10,822,821 ms, heap 8,521,431 bytes, `valid=18 malformed=3 crc=1
overflow=1 dropped=0 rx_drops=0`, with the display at 448x368/90 degrees. Together with
the previously verified long-unplug standalone fallback and visual checks, this closes
the M1 hardware exit gate.

The first visual exit attempt revealed that the clock screen had no user action wired
to the otherwise working brightness/rotation board APIs. This was inherited from M0:
the top/bottom tap zones were explicitly a temporary Task 5 test and were removed when
Task 6 replaced the test screen with the standalone clock. Asking for a visual M1 check
without restoring an invocation path was therefore invalid. M1 registered a release
handler on the clock screen: the upper half cycles raw brightness levels 64/128/255 and
the lower half flips orientation. The corrected image compiled and flashed normally; a
clean post-flash status reported brightness 200, rotation 0, 8,462,027 bytes free, and
zero parser/transport errors. The user then confirmed visible brightness steps and the
180-degree flip both worked.

That confirmation photo also showed two small cyan remnants at the former location of
the transient feedback text. Their color and position matched the M1-only cyan result
label, reproducing the partial-refresh text artifact previously carried as an M2 watch
item. Since the result label was diagnostic rather than product UI, it was removed; the
brightness and rotation changes themselves are sufficient feedback and do not require a
dynamic overlay.

Per subsequent user direction, the device UI now defaults to landscape: the physical
368x448 panel is software-rotated 90 degrees into a 448x368 logical canvas with the USB
cable down, and the flip control selects 270 degrees. LVGL continues to rotate touch
coordinates into logical space, and the tap zones compare against the live logical
height. Protocol status was extended compatibly to accept all cardinal angles and now
reports logical dimensions. The new build flashed through the normal native path; an
automatic-discovery status returned `display_width=448`, `display_height=368`,
`rotation=90`, brightness 200, free heap 8,462,035 bytes, and clean counters. Time sync
then succeeded. The full hardware-acceptance harness also passed again on this landscape
build (`valid=4->12 malformed=0->3 crc=0->1 overflow=0->1 dropped=0 rx_drops=0`).
Artifact-free landscape rendering and the 270-degree flip were confirmed after the
CO5300 alignment correction described below.

The subsequent photos did show severe landscape corruption: a malformed clock in the
first boot and, after the next flash, persistent green rectangles plus stale diagonal
text and other regions. This falsified the narrower theory that only the transient
feedback label was involved. Switching to one draw buffer recovered 59,396 bytes of
heap but the next photo was unchanged, disproving that change as the render fix. Forcing
full-screen invalidation and a synchronous boot refresh was also unchanged. Source
comparison then found the missing CO5300 two-pixel invalidation rounder described in the
earlier investigation section. The aligned image is `0xb78b0` bytes, flashed normally,
returned 448x368 at 90 degrees with free heap 8,521,431 bytes and clean counters, and
accepted time sync. The user confirmed both landscape orientations stayed clean,
brightness worked, and no display artifacts remained.

The native ESP32-S3 ROM may print startup text through USB Serial/JTAG before the
application starts. Disabling that source is an irreversible eFuse operation
(`DIS_USB_SERIAL_JTAG_ROM_PRINT`), so this project does not burn it. The M1 client clears
the input backlog on open before its framed status handshake. Application and bootloader
diagnostics are routed to UART0; a reset while a request is live is treated as a broken
session rather than protocol data.

## M2 partial hardware exit evidence (2026-08-04)

The clean M2-inclusive `0xbdb80` image flashed normally twice over the native
USB Serial/JTAG device. The second flash used `idf.py flash` without `-p` and
automatically selected `/dev/cu.usbmodem1101`. Post-flash status reported the logical
448x368 canvas at 90 degrees, 8,462,727 bytes free heap, and clean counters. The
application log identifies this as an M2 boot; the protocol's firmware-version field is
derived from `git describe` and therefore reported the nearest tagged dirty build as
`m1-dirty` before an M2 tag exists.

Automatic CLI discovery applied the checked three-screen carousel, and independent CLI
processes successfully seeded themselves from retained data/config revisions. The M1
adversarial hardware harness, updated to configure its test widget under the M2 model,
passed split/coalesced input, CRC/garbage/overlong recovery, invalid payload/time, valid
push, and stale-revision rejection. Its final counters were `valid=1->10 malformed=0->3
crc=0->1 overflow=0->1`, with zero response/RX/event/UI drops and UI queue high-water 3.

The M2 stress harness then completed 100 alternating config replacements and 1,000
mixed clock/progress/calendar field patches, with periodic screen activation. Across 20
status samples, initial, post-config baseline, final, and floor heap were all exactly
8,462,727 bytes. There were no resets or protocol/UI/event drops, and UI queue high-water
remained 3. A 65-second interval with no caller commands produced 21 session keepalives;
the device stayed online and heap remained unchanged. The checked carousel was restored
at data revision 1,003 and config revision 102. Human touch/visual checks, touch-event
pressure, ten M2-session unplug/replug cycles, and the 30-minute mixed soak are still
open and must not be inferred from this automated evidence.

The first M2 full-power-cycle attempt intentionally provided a stronger test than a
cable-only re-enumeration and exposed a host ownership defect. The device correctly
booted into its RAM-independent `00:00 / Connect deskmate app` fallback, but the running
pomodoro command had not issued the configuration or time sync itself; those belonged
to earlier short-lived CLI processes. It reconnected and then received `UnknownWidget`
while replaying its lone pomodoro push. The CLI now has a single long-lived `demo`
command that owns time, config, clock/calendar/pomodoro data, interaction state, and
interrupts in one replay cache. Its software checks pass and the corrected command has
loaded successfully on this board; corrected physical power-cycle replay remains to be
observed before the reconnect gate can close.

The first physical carousel swipes produced no navigation because the full-size
template content/root and status-strip objects inherited LVGL's default
`CLICKABLE | SCROLLABLE` flags. Those otherwise visual-only containers won hit testing,
so the screen-level carousel callback never received a press/release pair. The fix makes
all layout containers passive and explicitly keeps the owning screen clickable. The
resulting image retained the same `0xbdb80` size and flashed normally. Its live event
stream then reported clock -> pomodoro -> calendar -> pomodoro navigation, five
start/pause taps, exactly one pomodoro completion interrupt, and its dismissal restoring
`pomodoro-screen`; the user confirmed the visible gestures worked.

The corrected single-owner demo also passed its first full-power-cycle replay. The host
reported `reconnected on /dev/tty.usbmodem1101`, replayed time/config/clock/calendar/
pomodoro state after the reset, and the user confirmed the configured carousel visibly
returned. This is corrected cycle 1 of the required 10; nine more cycles remain.
The user explicitly deferred those repeated physical cycles so implementation could
continue; the hardware record does not treat unperformed cycles as evidence.

The subsequent mixed render/protocol soak passed for 1,800 seconds. It patched the
clock, progress ring, and calendar at 4 Hz, activated another carousel screen every five
seconds, and sampled status once per minute. Initial, post-config baseline, final, and
all 32 sampled heap values were exactly 8,462,727 bytes. Final status reported uptime
2,154,206 ms, `valid=6405 malformed=0 crc=0 overflow=0`, no reset, and zero response,
RX, device-event, UI-command, host-event, or sequence-gap drops. UI queue high-water was
4; device-event high-water was 1, with three physical events received during the run.

The final M2 rotation/pressure check used the standalone clock control to switch the
physical device to 270 degrees; status confirmed the same logical 448x368 canvas. The
full demo then rendered all three widgets and the status strip in that orientation. The
user confirmed clean rendering, responsive animation, taps, and swipes. Eight rapid
alternating navigation events arrived in order. Final status at uptime 2,690,320 ms
reported 8,462,727 bytes free heap, `valid=6794 malformed=0 crc=0 overflow=0`, zero
response/RX/device-event/UI-command drops, event queue high-water 1, and UI queue
high-water 4. Together with the earlier clean 90-degree interaction pass, this closes
M2's physical render/gesture gate.

## M3 companion-app hardware exit (2026-08-05)

M3's software implementation, host tests, ESP-IDF build, frontend checks, and macOS
desktop builds pass. The user explicitly deferred new physical testing until the next
hardware session. Nothing in this entry claims that the M3 app-to-board flow, real
sleep/wake behavior, or soak has been observed.

The remaining board session must:

1. Start from the settings UI, configure the clock, pomodoro, and ICS calendar (covering
   a local file and a URL between the functional checks), reorder the screens, and save
   without using the CLI to establish runtime state.
2. Observe all three clean-canvas templates at 90° and 270°. Change orientation only
   from Settings, confirm screen-edge/bottom-half touches do not rotate it, swipe the carousel,
   start/pause the timer by touch, let an accelerated timer complete, dismiss its
   interrupt, and confirm the previous screen returns.
3. Power-cycle the board while the app is the active owner. Confirm time, layout, latest
   clock/calendar/pomodoro fields, active screen, and any live interrupt replay from that
   one process without manual reapply.
4. Exercise a real host sleep/wake and settings close/reopen or webview reload. Confirm
   there is still one runtime owner, no duplicate provider refresh or interrupt, and the
   latest snapshot appears when settings returns.
5. Run a 30-minute mixed session with settings closed. Record host process memory/CPU,
   device heap and protocol/UI/event counters, provider/runtime queue pressure,
   reconnects, missed events, resets, and touch/render responsiveness.

These observations passed in the resumed session. Findings were fixed and regression
tested; the user accepted the completed morning soak and waived repeating it after the
later orientation/clean-canvas-only change. M3 is complete. No `m3` tag was created.

### M3 orientation and clean-canvas amendment (2026-08-05)

The product decision changed during the resumed hardware session: mounting orientation
belongs in the companion app, with exactly two landscape options. `90°` means USB below
and `270°` means USB above. Portrait modes and the standalone clock's former lower-half
rotation gesture are removed. The chosen rotation is persisted in app config, carried in
`ApplyConfig`, applied by firmware, and used by fallback rendering until reboot or a
later config.

The status strip is also removed because its contents are not readable at this panel
size. Digital clock, pomodoro, calendar, and interrupt views now use the complete
448x368 canvas. The protocol retains `standard` as a legacy M2 size enum, but current
firmware intentionally lays it out identically to `full`.

The amended `0xbd8d0` image was then flashed over native USB Serial/JTAG. Its first
status response reported 448x368 at 90°, 8,462,743 bytes free heap, revision/token zero,
and zero malformed, CRC, overflow, response, RX, event, or UI-command drops. From the
rebuilt release Settings app, the user selected USB-above and confirmed the display
flipped to 270° with a clean full-canvas widget. They selected USB-below and confirmed
it returned to 90°; repeated lower-half/edge taps did not rotate it, while carousel
swipes continued to work. The user then checked clock, pomodoro, and calendar: all three
used the full panel without a status strip, pomodoro start/pause and local advancement
worked, and calendar content rendered normally.

## M4 Task 1 capability-handshake deployment (2026-08-05)

The schema/negotiation task adds no new rendering behavior, but its firmware advertises
the already-proven rotation support so the new app does not conservatively treat an M3
image as core-only. ESP-IDF built `deskmate.bin` at `0xbd940` bytes with `0x426c0` bytes
(26%) free in the smallest app partition. The release Deskmate process was terminated
cleanly to release `/dev/cu.usbmodem1101`, the image flashed with verified hashes, and a
read-only CLI status query returned active protocol 1, maximum protocol 1, capabilities
`3` (`CoreWidgets | ConfigRotation`), logical 448x368 at 90°, 8,462,743 bytes free heap,
and zero malformed, CRC, overflow, response, RX, or event drops. The release app was
then reopened and reacquired the device port. This verifies deployment and negotiation;
the user-requested morning soak was not repeated.

## Card model (M4 Task 2B) — physical verification (2026-08-06)

Branch `feat/card-model` at `52bf9af`, board `/dev/cu.usbmodem1101`. All nine checks in
`docs/superpowers/plans/2026-08-06-deskmate-card-model.md` were run on the physical
board with the user observing the panel. Eight pass. One sub-check found a real gap and
one produced a non-reproducible first observation; both are recorded below rather than
smoothed over.

### 1. Firmware unchanged — PASS

`git diff eaf3b0c..HEAD` is empty for both `firmware/` and
`companion/crates/{protocol,device}`: the card-model work changed no firmware and no
wire code. The only commit touching `firmware/` since `ae05d7a` is the checkpoint
`eaf3b0c`.

Rebuilt from branch HEAD: `deskmate.bin` = `0xbd9d0` bytes, `0x42630` (26%) free in the
smallest app partition. This is 144 bytes larger than the `0xbd940` previously recorded,
which the shorter git-describe string (`m1-31-g52bf9af`, 15 ch, vs
`m1-2-gae05d7a-dirty`, 19 ch) does not fully account for. Equivalence was therefore NOT
assumed — the image was flashed, hashes verified.

Post-flash `StatusResponse`: protocol 1, max protocol 1, **capabilities 3**
(`CoreWidgets | ConfigRotation`) — identical to the pre-work M4 Task 1 record.
448x368, rotation 90, free heap 8,462,743, brightness 200, all drop/error counters zero.

### 2. Migrated legacy config — PASS

The subject was the user's **real live config**, not a fixture: the on-disk
`~/Library/Application Support/io.deskmate.companion/config.json` was genuine
`schema_version: 1` with `widgets[]`/`screens[]`, screen order clock -> pomodoro ->
calendar, orientation `landscape-flipped`. It was backed up first.

The migration result was predicted in writing before the app was launched, and matched
exactly:

| card | template | presence | alert |
|---|---|---|---|
| `clock` "Desk" | `digital-clock` | in-rotation, inherit | `none` |
| `pomodoro` "Focus" | `progress-ring` | in-rotation, inherit | `on-timer-finish` / `until-dismissed` |
| `calendar` "Up next" | `row-list` | in-rotation, inherit | `none` |

with `carousel.advance: manual` and orientation preserved. The alert column is the load-
bearing part: pomodoro gained an alert because its *historical* interrupt policy was
enabled, and calendar correctly did **not**, confirming on real data that migration
derives the alert from the old policy and never from the card kind.

Card order matched the previous screen order. On the panel the user observed the clock
at 270° (USB cable above) — the same first screen, correctly oriented.

Migration is read-time only: the v1 file stays on disk untouched until the next save,
which then writes v3. Behaviour, not a defect.

### 3. Timed rotation with distinct per-card dwells — PASS

Dwells 5 s / 10 s / 20 s (35 s loop). User observed the correct order, visibly distinct
dwell lengths, and a clean wrap.

### 4. Swipe during a dwell — PASS

With every card at 30 s, swiping ~20 s into a dwell gave the card swiped to a **full
30 s**, not the ~10 s remainder. Manual navigation cancels the pending advance rather
than inheriting it. See §10 for the counter evidence on "no duplicate `ActivateScreen`".

### 5. Pomodoro alert, `until-dismissed` — PASS

Full-screen takeover on timer finish, held indefinitely against a live 30 s rotation,
cleared only on tap, and restored the correct carousel card.

### 6. Bounded `AlertHold::Seconds` — LIMITATION CONFIRMED

**First observation was wrong and is recorded here deliberately.** With a 30 s hold and
30 s dwells, the panel appeared to self-clear at ~30 s. Two controlled retests
contradicted it:

- Rotation disabled entirely, 60 s hold: the alert **stayed until tapped**, never
  cleared on its own.
- Rotation at 10 s dwells with an `until-dismissed` hold: the alert **stayed put**; a
  carousel advance does not disturb an active interrupt.

So the documented limitation is real: protocol v1 has no host->device dismissal message,
`TriggerInterrupt` carries no duration, the spec states "M2 has no automatic interrupt
timeout", and firmware has no auto-dismiss path. A bounded hold bounds only host-side
bookkeeping. The first observation is **not explained** — it was not reproduced under
either controlled condition, and no mechanism was found that would clear the overlay.

The card editor's own copy already states this correctly ("stays on screen until you tap
it, regardless of this setting").

### 7. Rotation rule — PASS, stronger than specified

The plan expected a save-time rejection. The UI instead makes the state **unreachable**:
muting the second-to-last card moves it to "Alerts and muted", and on the last remaining
in-rotation card both "Alert only" and "Off" are disabled with the reason stated inline
("This is your only in-rotation card — muting or turning it off would empty the
rotation..."). There is no invalid save to reject.

The validation layer was then tested separately by hand-writing a config with every card
`presence: off`. The app refused it, showed "config has 1 validation issue(s)", and
**left the invalid file untouched** on disk.

Two defects in that fallback path, both recorded as follow-ups:

1. The banner reads "Using your last working settings", but what loads is the built-in
   default (1 card, `show_seconds` on), not the user's previous 3-card config. The copy
   claims something the behaviour does not do.
2. The fallback is **pushed to the device**: the user observed the panel drop to a
   single clock card. A corrupted config file on disk therefore replaces a working
   display, rather than the device keeping its last-applied configuration.

### 8. Both orientations — PASS

Re-ran rotation (5/10/20 s dwells) and the pomodoro `until-dismissed` alert at **90°**
(USB cable below). The display flipped correctly, rotation and wrap behaved identically,
and the alert took over, held until tapped, and restored the right card. Combined with
checks 3-5 at 270°, both orientations are covered.

### 9. Unplug / replug mid-rotation — PASS, with one gap

Unplugging mid-rotation dropped the panel to the standalone clock; replugging replayed
the configuration and rotation resumed normally, with no jumping or double-advance.

**Gap — an alert that fires while the device is unpowered is lost.** A 3-minute pomodoro
whose finish time fell entirely inside the unplugged window produced no alert on
reconnect; rotation simply resumed. Observed once, not reproduced.

This is not a sync-ordering bug: `synchronize_full` sends `apply_layout` first (which
the firmware's `dispatch_apply_config` follows with `interrupt_state_clear()`) and calls
`flush_interrupts` **last**, and `flush_interrupts` re-sends any interrupt that is not
`acknowledged`. The replay path exists and is correctly ordered, so the loss is
undiagnosed. No test covers it: `tests/runtime.rs` has
`cold_boot_and_two_power_resets_replay_the_complete_owned_state` and
`pomodoro_events_complete_once_and_dismissed_interrupts_do_not_replay`, but nothing
asserts that an interrupt raised while the device is absent reaches it on reconnect.

**Root cause and host fix — 2026-08-15.** The bounded-hold countdown was started when
the host scheduled the interrupt, before any device had displayed it. If the configured
hold expired during the unpowered window, the host retired that never-displayed
interrupt locally, leaving `flush_interrupts` nothing to deliver on reconnect. The
companion now starts a bounded hold only after `flush_interrupts` successfully delivers
the active interrupt; regression tests cover reconnect delivery for both bounded and
`until-dismissed` holds, and confirm that a bounded hold still expires from its delivery
time. The original single hardware observation did not record the alert's hold
configuration, so this closes the only persistent-loss mechanism present in the host
code, but the hardware scenario has not yet been re-verified on the board.

### 10. Device counters after the full session

Queried over the CLI with the app closed, after all nine checks:

```
uptime 2,146,636 ms   free_heap 8,462,743   rotation 90   capabilities 3
valid_frames 1257     malformed 0   crc_errors 0   overflow_frames 0
dropped_responses 0   rx_dropped_bytes 0   dropped_events 0
dropped_ui_commands 0 event_queue_high_water 0   ui_queue_high_water 3
latest_revision 388   config_revision 11   latest_interrupt_token 386
```

Two things worth naming. `free_heap` is **byte-identical** to the post-flash reading and
to every earlier record in this file — flat heap across rotation, alerts and reconnects.
And `ui_queue_high_water` reached only 3 with `dropped_ui_commands` at 0, which is the
evidence behind "no duplicate `ActivateScreen`" in checks 4 and 9: a duplicate-command
storm would raise that high-water mark and eventually drop commands.

### Not covered by this session

- "Fires exactly once per event" for calendar alerts was not separately isolated.

## Extended templates (M4 Task 3) — verified 2026-08-11

Observed on the physical board, firmware `m1-66-gbe54c85` (merged `main`, image
781,264 bytes / `0xbebd0`, hash verified on flash). Driven by the companion app for
config application and by `deskmate-cli push-data` for the icon sweep. The device
retained its config across the app→CLI host swap (`config_revision` stayed 7), which
is what made the CLI sweep possible.

- [x] `StatusResponse` reports capabilities 11 (core | rotation | extended templates).
      Observed 11 immediately after flash.
- [x] A weather card renders icon-left/text-right with a real temperature, summary
      and location, at both 90° and 270°. Confirmed at both orientations; text
      readable and unclipped in each.
- [x] Each of the 11 icon names renders its own distinct artwork, and an
      unrecognised name renders the hollow `unknown` ring rather than blank space.
      All twelve names (`sun`, `moon`, `cloud`, `cloud-sun`, `cloud-moon`, `rain`,
      `drizzle`, `snow`, `storm`, `fog`, `unknown`, plus a bogus `not-a-real-icon`)
      were pushed at 8s dwell with each icon's own name printed beside it. All 11
      read as clearly distinct at normal desk distance — including the pairs most at
      risk of collapsing, `rain`/`drizzle` and `cloud-sun`/`cloud-moon`. The bogus
      name fell back to the same hollow ring as `unknown`.
- [x] A json-feed card renders its mapped value as the hero number, and an absent
      value renders `--` rather than stale pixels from the previous card. Observed
      with `{"count": 42}` → `42`; `{"count": true}` → `true`; a 20-digit
      `12345678901234567890` → `1234567890123456` (first 16 bytes, clipped
      host-side, push NOT refused); and a payload with the path absent → `--`,
      replacing a previously displayed `42`, so no stale pixels.

      Note on the absent-value case: an unresolved mapping path is a whole-feed
      error (`parse_json_feed` returns `MalformedFeed("JSON mapping did not resolve
      to a value")`), so the card also shows an error line and goes stale. The `--`
      itself comes from the firmware's declared default because the provider emits
      no `value` field at all. Both behaviours are correct; the error line was not
      anticipated by this checklist item.

      Correction to the earlier wording of this item: `value` is **not** the only
      renderable data field `big-number-label` declares. `label` is also declared
      (64 bytes) and is user-mappable through a json-feed mapping — that is how the
      ellipsize check below was driven.
- [~] An analog-clock card tracks time, and `show_seconds` toggles the second hand.
      **Partially verified.** With `show_seconds` true: second hand present and
      sweeping, and the twelve ticks were confirmed concentric with the hands and
      hub (the border-inset pivot fix works). With `show_seconds` false: the second
      hand was correctly removed, but the displayed time was reported wrong and the
      cause was NOT diagnosed before the check was abandoned at the user's request.
      Do not describe the `show_seconds`-false path as verified.

      Leading untested hypothesis: the verification config pinned
      `"timezone": "UTC"` while the board sits at UTC+4, which would put the clock
      exactly 4 hours behind wall time. This was never confirmed. Against it being a
      template defect: `analog_clock_tick` computes the hour and minute angles
      (`hour12 * 30 + tm_min / 2` and `tm_min * 6`) with no dependence on
      `s_show_seconds`, which only hides the second-hand object — so the flag has no
      path to the hour/minute positions. The time was also not scrutinised in the
      `show_seconds`-true observation, where the question put to the observer
      emphasised tick concentricity.
- [x] A `big-number-label` `label` longer than its box ellipsizes on one line rather
      than wrapping onto a second and colliding with the state label at the card's
      bottom edge. Observed: a 94-character caption rendered on one line ending in
      an ellipsis. This is the check that matters most for the pinned-height fix —
      pinning width alone was not sufficient, and two earlier attempts got it wrong.
- [ ] `unknown_field_count` stays at zero across a weather push.
      **NOT VERIFIABLE ON HARDWARE — this item cannot be closed as written.**
      `unknown_field_count` never reaches the host: it is not a field in
      `StatusResponse`, and `widget_model_unknown_field_count()`
      (`firmware/main/core/widget_model.c:323`) has **no callers anywhere in the
      firmware**. The counter is accumulated and never read, so no host-observable
      behaviour distinguishes zero from nonzero. An unknown field is counted and
      ignored; unlike a type mismatch, it does not reject the push.

      What actually covers this today is the host test
      `test_weather_push_has_no_unknown_fields`
      (`firmware/host_tests/test_template_fields.c:184`), which asserts every field
      the weather provider emits is declared in the `icon-badge-text` registry. Its
      emitted list was re-checked against `crates/providers/src/weather.rs` on
      2026-08-11 and matches exactly (10 fields). That list is hand-maintained and
      can silently drift from the provider — it is not derived from it.

      To make this observable, `unknown_field_count` would need a `StatusResponse`
      field, which is an additive protocol change. Not attempted.
- [x] Heap stays flat and `ui_queue_high_water` stays low across a full rotation
      that includes all three new templates. After ~35 minutes uptime spanning
      8 config applies, 117 data revisions, the 12-icon sweep and roughly 14 full
      three-card rotations: `free_heap` 8,462,735 — **byte-identical to the reading
      taken immediately after boot** — and `ui_queue_high_water` 2. Every error and
      drop counter zero: `malformed_frames`, `crc_errors`, `overflow_frames`,
      `dropped_responses`, `rx_dropped_bytes`, `dropped_events`,
      `dropped_ui_commands`.

### Not covered by this session

- The `show_seconds`-false time reading, above.
- `unknown_field_count` on hardware, above — not observable over protocol v1.
- Weather data was fetched live from open-meteo for a guessed location
  (`Tbilisi`, taken from the repo's own test fixture). The layout was verified;
  the correctness of the forecast content itself was not.

### Code observation, not a verification result

`analog_clock.c:21` declares `static bool s_show_seconds = true;` at file scope, and
`analog_clock_tick` reads it. It is shared by every analog-clock view and persists
across teardown, so two analog-clock cards with different `show_seconds` values would
have whichever applied last win for both. Not exercised by this session — the
verification config never held two analog clocks — and not fixed.

## V1 Task 10: dev-only framebuffer capture — build verified, physical diff outstanding (2026-08-13)

No physical board was connected for this session. **Step 3 of the Task 10 brief (run
`framebuffer_diff.rs` against the board at both orientations and record per-case
results) did not run and is not verified.** Do not describe the capture/diff path as
observed on hardware. What was verified, all on host tooling:

- `idf.py -C firmware build` (plain release build) and
  `idf.py -C firmware -DDESKMATE_DEV_DIAG=1 build` (dev-diag build, separate build
  directory) both compile clean.
- The plain build's `.elf` contains neither the `dev_capture_handle_request` symbol
  (`nm` found no match) nor the `"dev_capture"` log-tag string (`strings` found no
  match), against a control check (`nm`/`strings` on the same `.elf` confirmed the
  tools work against this Xtensa target by finding known-present symbols). The
  dev-diag build's `.elf` contains both. `link/dev_capture.c` is always listed in
  `firmware/main/CMakeLists.txt`'s `SRCS`; its `.o` is a near-empty translation unit
  (1,232 bytes, debug info only) in the plain build and a full one (23,340 bytes) in
  the dev-diag build, confirming the `#ifdef DESKMATE_DEV_DIAG` guard — not
  conditional `SRCS` inclusion — is what keeps the handler out of release. Dev-diag
  `.bin` is 1,344 bytes larger than the plain `.bin`.
- `firmware/lv_conf.h`'s `LV_USE_SNAPSHOT` flip from 0 to 1 (needed by
  `lv_snapshot_take_to_draw_buf`, shared config for both the firmware and
  `lvgl-sim`) produced no pixel drift: `cargo test -p lvgl-sim` (golden-frame suite,
  ~70 committed PNGs) passed unchanged after the flip.
- `make -C firmware/host_tests clean test` — all 11 suites pass unchanged;
  `link/dev_capture.c` is link-layer, not `core/`, so it has no host-test target
  (consistent with `protocol_task.c`/`usb_link.c`, its siblings).
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
  warnings`, and `cargo test --workspace` (companion) all pass with
  `companion/crates/device/examples/framebuffer_diff.rs` and the `lvgl-sim` case
  table move (`tests/cases.rs` -> `src/cases.rs`, `pub mod cases`) in place.

What remains for the physical step, once a board running the dev-diag build is
available: `cargo run -p device --example framebuffer_diff -- [--port <path>]`
against both orientations, and an entry recording the observed per-case results
(pass/differ/error) here, titled "V1 framebuffer diff — <date>" per the brief. An
unexplained diff is a stop-the-line finding per the brief and must be diagnosed
before Task 11, not waved through.

## V1 physical acceptance — 2026-08-13 — BLOCKED: boot crash loop, Phase 1 fails

Board connected at `/dev/cu.usbmodem101`, HEAD `ee927cc`. **Full report:**
`.superpowers/sdd/2026-08-11-deskmate-v1-preview-typeface-redesign/task-11-report.md`.
Summary here; do not re-derive from memory, read that file for the complete evidence
trail (coredump register dumps, bisection table, ruled-out causes).

**The device does not boot to the standalone clock, or to any usable state, on this
branch.** It crashes and reboots in a continuous loop starting immediately after
flashing, on every single boot cycle observed (~20+), on both the dev-diag and the
plain release build. Framebuffer-diff (Phase 2) and the §6.4 acceptance gate/soak
(Phase 3) did **not** run — both are blocked by Phase 1's failure and produced no
results, pass or fail. The `show_seconds`-false analog-clock defect open since M4 is
therefore still open; nothing this session closes it.

- `idf.py -C firmware -DDESKMATE_DEV_DIAG=1 build` then
  `idf.py -C firmware -p /dev/cu.usbmodem101 erase-flash flash` both succeed cleanly.
  The new partition table's lines print correctly in the boot log (`ota_0`/`ota_1`
  4MB each, `assets` 6MB, `coredump` 64KB).
- Every boot cycle: clean ROM/bootloader/`app_init`/`heap_init`/PSRAM-pool-reserve
  log lines, then ~450ms later `esp_core_dump_flash: Save core dump to flash...` (a
  live panic write, confirmed against `espcoredump` source — not a boot-time
  integrity check), then `RTC_SW_CPU_RST` reset, then the cycle repeats. No `Guru
  Meditation` banner text appears in the non-interactive pyserial capture used (no
  TTY available in this environment for `idf.py monitor`), though the panic clearly
  did run (it wrote a coredump and rebooted).
- `idf.py coredump-info` decodes the crash: `exccause 0x0 (IllegalInstructionCause)`,
  fault `pc 0x20440fca` (not valid ESP32-S3 code space — bit-for-bit identical across
  the dev-diag build, the plain release build, and an `LV_USE_SNAPSHOT=0` diagnostic
  rebuild, despite each having different code layout). The return-address chain
  (register `a0`, window-bits restored and cross-checked against a second frame's
  independently-decoded address in the same coredump) traces cleanly through
  `lvgl_port_add_disp` (`esp_lvgl_port_disp.c:131`, inside
  `lvgl_port_disp_rotation_update(disp_ctx)`) called from `board_display_init`
  (`display.c:227`) called from `app_main` (`main.c:25`,
  `ESP_ERROR_CHECK(board_display_init())`). The exact instruction that jumps to the
  invalid `pc` was not pinned down — no debugger session was available.
- Ruled out this session: `DESKMATE_DEV_DIAG=1`/`dev_capture.c` (plain build crashes
  identically), stale NVS/partial erase (reproduced with and without `erase-flash`
  immediately prior), and `LV_USE_SNAPSHOT=1` (diagnostic flip to 0, rebuilt,
  reflashed, identical crash — binary size was unchanged either way in this build,
  0xbb560 both ways; reverted immediately after, `firmware/lv_conf.h` and `git
  status` both confirmed clean afterward).
- Bisected via disposable `git worktree` checkouts (both removed after use; main
  worktree never left `ee927cc`): commit `363893e` ("feat: custom partition table —
  OTA slots, asset region, coredump") — the **first commit ever to add a custom
  `firmware/partitions.csv`** on any branch (confirmed via `git log --all --oneline
  -- firmware/partitions.csv`, exactly one commit) — already crashes identically to
  HEAD. Its parent, `1a6f8d9`, does not: no panic-save line and no reboot loop
  appeared in a 15-second capture. The 40 commits of design/font/template work
  between `363893e` and HEAD are **not** required to reproduce the crash. This
  isolates the regression to the new partition table and/or the coredump-to-flash
  subsystem it newly enables (`CONFIG_ESP_COREDUMP_ENABLE_TO_FLASH=y` +
  `CONFIG_ESP_COREDUMP_CHECK_BOOT=y`, previously dead configuration without a
  `coredump` partition to target) as the most specific known variable — a hypothesis
  backed by a clean single-commit bisection, **not** a proven root cause. Why a
  partition-table/coredump change would produce a wild jump specifically inside
  LVGL's display-rotation-update path is unexplained.
- `cargo run -p deskmate-cli -- status --port /dev/cu.usbmodem101` → `error: device
  request timed out`, consistent with the device never reaching
  `protocol_task_start()` in `main.c`.
- Separately noted, not the cause: `firmware/sdkconfig` routes application-level
  `ESP_LOGI` console output to physical UART0 (`CONFIG_ESP_CONSOLE_UART_DEFAULT=y`),
  not the native USB-Serial/JTAG port this session's captures were read over. No
  `deskmate M2 boot` (or any app-tagged) log line appears in *any* capture in this
  session, crashing builds and the non-crashing pre-partition-table build alike —
  this is console routing, not evidence for or against the crash itself.

**This branch must not be tagged, merged, or described as physically verified until
this is root-caused and fixed, then this whole physical batch is re-run from Phase 1.**
Next step needs an interactive TTY against the board (`idf.py monitor` or a JTAG/GDB
session) to catch the panic banner live or single-step `board_display_init()` —
neither was available in this sandboxed session.

## Boot crash-loop root-caused — LVGL draw-buffer budget (observed 2026-08-13)

Supersedes the hypothesis in the previous entry. The crash-loop is **not** caused by
the custom partition table or by the coredump configuration. Both were tested and
excluded; see
`.superpowers/sdd/2026-08-11-deskmate-v1-preview-typeface-redesign/task-11-debug-report.md`
for the full evidence trail.

**Observed on the physical board** (Waveshare v2 at `/dev/cu.usbmodem101`, ESP-IDF
v5.5.5, HEAD `baf399f`, pyserial DTR/RTS-toggle captures):

- HEAD reproduces the loop on every boot: 9 complete cycles in a 12 s capture, panic
  at ~1317 ms each time, `RTC_SW_CPU_RST` between them.
- The panic banner had never been seen because
  `CONFIG_ESP_CONSOLE_UART_DEFAULT=y` + `CONFIG_ESP_CONSOLE_SECONDARY_NONE=y` send
  `panic_print_char()` output (and every `ESP_LOGI` line) to **UART0 on GPIO43/44
  only** — `components/esp_system/panic.c:129` dispatches solely to the compiled-in
  console backends. The `esp_core_dump_flash:` lines that *were* visible over USB
  come from `esp_rom_printf`, which follows
  `CONFIG_ESP_ROM_CONSOLE_OUTPUT_SECONDARY=y`. Adding
  `CONFIG_ESP_CONSOLE_SECONDARY_USB_SERIAL_JTAG=y` as a temporary, uncommitted
  overlay made the banner visible; the committed config is unchanged.
- With that visibility, the line immediately preceding the banner is the cause:
  `E (1254) LVGL: lvgl_port_add_disp_priv(471): Not enough memory for LVGL buffer
  (rotation buffer) allocation!`, then
  `Guru Meditation Error: Core 0 panic'ed (IllegalInstruction)`, `PC 0x20440fca`,
  `Backtrace: ... |<-CORRUPTED`.
- Measured immediately before `lvgl_port_add_disp()`, identical on every boot:
  largest free `MALLOC_CAP_DMA` block **110,592 B**, against **2 x 58,880 =
  117,760 B** required. Total free DMA memory was 186,583 B — the failure is
  contiguity, not capacity. Internal DMA RAM is three disjoint regions
  (`0x3FCBFA78` 171,160 B / `0x3FCE9710` 22,308 B / `0x3FCF0000` 32,768 B); once the
  flush buffer takes 58,880 B out of the 110,592 B block, no remaining block can hold
  the rotation buffer.

**Board fact to carry forward:** with `.sw_rotate = true`, `esp_lvgl_port` allocates
**two** internal-DMA draw buffers of `buffer_size` each (flush + rotation scratch),
not one. On this board the usable ceiling for that pair, measured at
`board_display_init()` time, is ~110 KiB. `display.c` now pins the budget at 96 KiB
(64 lines, 2 x 47,104 B) behind a `_Static_assert`, plus a runtime
`heap_caps_get_largest_free_block(MALLOC_CAP_DMA)` pre-flight check — needed because
`esp_lvgl_port`'s allocation-failure path frees `disp_ctx` and still returns a
non-NULL `lv_display_t`, so its caller dereferences freed memory and jumps through a
garbage function pointer instead of reporting an error.

**Verified on hardware 2026-08-14.** The fix boots the board cleanly and the
protocol answers. Observed on the committed `b90e711` build, rebuilt and flashed
against unmodified `sdkconfig.defaults` (`CONFIG_ESP_CONSOLE_SECONDARY_NONE 1`,
`CONFIG_ESP_COREDUMP_ENABLE_TO_FLASH 1` in `build/config/sdkconfig.h`):

- **Three consecutive resets, all clean:** each shows exactly one ROM banner with
  `rst:0x15 (USB_UART_CHIP_RESET),boot:0x2b (SPI_FAST_FLASH_BOOT)`, zero
  `Guru Meditation` banners and zero `esp_core_dump_flash: Save core dump to flash`
  lines — against the pre-fix signature of nine of each in a 12 s window.
- **Full boot chain** (captured once on a diagnostic-console overlay build of the same
  fix, since the committed config sends `ESP_LOGI` to UART0): `deskmate M2 boot` →
  `LCD panel reset via TCA9554 EXIO0` → `Install CO5300 panel driver` →
  `Initialize LVGL port` → `rotation set to 90 degrees` →
  `display init complete (368x448)` → `board_touch_init OK` →
  `first LVGL flush completed` → `clock_screen_show: screen ready` →
  `clock screen shown, LVGL task running` → `native USB Serial/JTAG protocol link
  ready` → `protocol task running`. The error that used to precede the panic,
  `lvgl_port_add_disp_priv(471): Not enough memory for LVGL buffer (rotation buffer)
  allocation!`, is **absent**.
- **Status readback:** `firmware m1-112-g65b5357 / protocol v1 (max v1, capabilities
  0x000000000000000b)`, `display 448x368, brightness 200, rotation 90°`,
  `uptime 18275 ms, free heap 8476515 bytes, link online`,
  `frames valid=1 malformed=0 crc=0 overflow=0 dropped_responses=0 rx_drops=0`.
  A second readback ~14 s later gave `uptime 32134 ms` with the **same**
  `free heap 8476515 bytes` — monotonic uptime rules out a hidden reboot, and the
  byte-identical heap rules out a leak across the interval.

**Board-handling fact, corrected and important.** The previous revision of this entry
claimed GPIO0 was held low by a stuck BOOT button. **That was wrong.** On this macOS +
USB-Serial/JTAG setup, `esptool`'s post-flash "Hard resetting via RTS pin" pulses RTS
while DTR is still asserted from its own connect sequence, so the chip re-enters ROM
DOWNLOAD mode (`boot:0x23 (DOWNLOAD(USB/UART0))`) instead of running the app. Every
`idf.py flash` and every DTR/RTS reset attempted from the host did this. Two
consequences worth keeping:

- **Never infer the physical GPIO0 level from `GPIO_STRAP_REG` read over esptool.**
  The register latches at reset, and esptool's connect deliberately drives GPIO0 low
  to enter download mode, so it always reads back "GPIO0 low". That misreading cost a
  full verification round.
- **To get a real run-mode boot, reset via the RTC watchdog**, which touches no
  DTR/RTS: unlock `RTC_CNTL_WDTWPROTECT_REG` with `RTC_CNTL_WDT_WKEY`, set
  `RTC_CNTL_WDTCONFIG1_REG`, enable via `RTC_CNTL_WDTCONFIG0_REG`
  `(1 << 31) | (5 << 28) | (1 << 8) | 2`, re-lock, then deassert DTR/RTS and close the
  port before it fires. USB re-enumerates across the reset, so the reader must reopen
  in a loop. This gives `boot:0x2b (SPI_FAST_FLASH_BOOT)` reliably. To tell a running
  app from a chip parked in download mode, probe with
  `esptool --before no_reset chip_id`: it connects **only** if the chip is in download
  mode.

**Still outstanding for the acceptance run.** The fix changes flush-chunk geometry
(80 -> 64 lines), so a full 448x368 landscape repaint is split into more, shorter
strips. The CO5300 even-window rounding rule (`board_lcd_rounder_cb`) is exactly what
keeps odd-width rotated column windows safe, so the acceptance run must re-eyeball
both orientations for stale text or shifted colour blocks at the new strip height.
That visual check needs human eyes and has not been performed.

## V1 physical acceptance — 2026-08-14 — Phases 1-3 PASS (soak waived at 27 min)

HEAD `05d04ce`, board at `/dev/cu.usbmodem101`. Full detail:
`.superpowers/sdd/2026-08-11-deskmate-v1-preview-typeface-redesign/task-11-report.md`.
Every result below was independently observed this session (rebuilt/reflashed/
re-read status directly), not transcribed from the prior debug session's claims.

**Phase 1 (dev-diag build).** Rebuilt clean, flashed, reset via the RTC-watchdog
method documented above (not DTR/RTS). Twelve-second capture: one ROM banner
(`boot:0x2b`), zero panic-save lines, zero `Guru Meditation` banners.
`deskmate-cli status`: `firmware m1-113-g05d04ce`, protocol v1, capabilities `0x0b`,
display 448x368 rotation 90°, uptime climbing, `free heap 8476515`, all frame counters
zero except one valid frame. PASS.

**Phase 2 (framebuffer diff, dev-diag build).**
`cargo run -p device --example framebuffer_diff -- --port /dev/cu.usbmodem101`, one
run (the case table already spans both orientations per case):
`SUMMARY total=56 identical=54 differing=0 errored=0 excluded=2`. The 2 excluded are
`row-list--truncation-boundary` at both orientations, excluded by design (its 128-char
field exceeds row-list's own 96-char registry maximum, so the device correctly rejects
that push — not a failure). **Zero unexplained diffs.**
`analog-clock--no-seconds--landscape` and `--flipped` are both `identical`, closing
the `show_seconds`-false defect open since M4: the pinned-instant golden proves the
hour/minute hand angles are correct and independent of the flag once the device
actually renders a frame, at both orientations. PASS.

**Phase 3 (plain release build).** `fullclean` then plain flash (no erase, no
`DESKMATE_DEV_DIAG`), RTC-watchdog reset: one ROM banner, zero panic lines.
`deskmate-cli status`: `free heap 8480619`, clean counters. Standalone clock boots.

- `hardware_acceptance.rs`: `PASS ... valid=2->11 malformed=0->3 crc=0->1
  overflow=0->1 dropped=0 rx_drops=0 events_dropped=0 event_high_water=0 ui_dropped=0
  ui_high_water=2` — split/coalesced frame handling, bad CRC, garbage, overlong,
  invalid-revision push (rejected `InvalidPayload`), pre-floor time-sync (rejected
  `InvalidTime`), and a stale-revision replay (rejected `StaleRevision`) all handled
  without a reboot, with the relevant counters advancing.
- Rotation 270 + `RowList` (a template `hardware_acceptance.rs` doesn't touch) +
  typical/maximal(96-char)/over-ceiling(97-char) `row0_title` pushes, via a temporary
  uncommitted diagnostic example deleted immediately after use (`git status` clean
  before and after): typical and maximal pushes accepted, the over-ceiling push
  rejected (not a crash), `free_heap` byte-identical before/after (`8480619`),
  `uptime_ms` monotonic across the run (no reboot). This was needed because
  `hardware_acceptance.rs` only exercises `DigitalClock` at rotation 90 on this exact
  binary; Phase 2's exhaustive template/rotation sweep ran on the dev-diag binary,
  which differs from plain only in `dev_capture.c` (unrelated to config/template
  code, per the Task 10 `nm`/`strings` check above).
- **30-minute mixed soak** — `m2_stress.rs --soak-seconds 1800` (config swaps, data
  patches, an idle window, then the soak):
  ```
  soak_elapsed=1620s heap=8480619 valid=6699 malformed=3 event_high_water=0 event_dropped=0 ui_high_water=2 ui_dropped=0
  ```
  **WAIVED at 27 of 30 minutes (1620s/1800s), not completed** — interrupted by
  explicit user direction; process killed and the serial port released immediately.
  Per this repo's M2 waiver convention, this is a waiver, not a pass: do not describe
  a completed 30-minute soak as observed. The partial data itself is clean: heap held
  byte-flat at `8480619` across every sample from the preceding 100 config swaps +
  1000 patches + 65s idle window through `soak_elapsed=1620s`, `valid` frames climbed
  monotonically (1466->6699) with no discontinuity, `malformed` held at `3` unchanged
  throughout the soak's own traffic, and `event_high_water`/`event_dropped`/
  `ui_dropped` stayed `0` with `ui_queue_high_water` flat at `2` — no reboot, no
  counter regression, no port drop at any sampled point up to the kill.

**Not evaluated — needs human eyes.** The CO5300 even-window rounding check flagged
above (new 64-line strip height, both orientations, watching for stale text or
shifted colour blocks) — Phase 2's diff runs through `lv_snapshot_take`, which
captures the pre-flush logical frame and bypasses the real QSPI flush/rounding path
entirely, so it cannot see this. Also not evaluated: AMOLED surface-module
rendering quality, storm icon readability, boot-clock visual appearance, general
color/contrast at desk distance.

## Unpowered-alert fix — hardware re-verification PASSED (2026-08-15)

Firmware `m1-113-g05d04ce` on `/dev/cu.usbmodem1101`, main at the alert-hold fix
(`4206a98`), user observing the panel. Driven by the sequenced harness
`companion/crates/app-core/examples/alert_replay_check.rs` (15 s pomodoro,
5 s bounded hold), which starts the timer only after the link drops so the
completion provably falls inside the disconnected window.

**Result — PASS.** Link down at T+56 s; timer started host-side; pomodoro
completed at T+71 s while disconnected; the 5 s bounded hold elapsed while still
disconnected; replug at T+91 s produced the interrupt takeover on the panel
within seconds (the completed pomodoro face, "no time left"), delivered by the
runtime's reconnect flush. The instrumented run reported no `RuntimeState`
errors and no card errors, and the device's `latest_interrupt_token` advanced,
confirming the firmware accepted the trigger. Before the fix this exact
sequence retired the interrupt host-side and the panel came back with nothing.

Two earlier attempts that day mis-executed the timing (the timer completed
while still connected); those runs instead demonstrated session-replay
re-delivery of a never-dismissed interrupt after link loss — also observed
working, but they are not evidence for this fix.

**Battery discovery.** The board has a battery attached: unplugging USB is
*link* loss, not power loss (`uptime_ms` kept counting across a 20 s unplug).
The 2026-08-06 "unpowered" observation was therefore most likely also a
link-loss case. The host-side mechanism and fix are identical either way, but
a true power-loss run (battery disconnected, device reboots, session replay +
reconnect flush after a fresh boot) has not been performed.

**UX findings (open, not defects in the alert state machine):**

1. The interrupt takeover face is visually indistinguishable from the completed
   pomodoro card beneath it — both render the same template showing "Done" — so
   a successful tap-dismiss looks like nothing happened. During re-verification
   the user repeatedly "dismissed" an overlay that was already gone.
2. Tapping a Completed pomodoro (tap action start-pause) is a deliberate host
   no-op (`engine::pomodoro::toggle` on `Completed` only refreshes), but the
   firmware applies optimistic local tap feedback that no authoritative push
   ever reconciles — the "done" word flashes red and stays red until the next
   tap. Confirmed by the event stream: such taps arrive as `tap start-pause`,
   the host processes them, state does not change, nothing is pushed back.
3. The host silently ignores an `InterruptDismissed` event whose token it no
   longer tracks (`runtime.rs`, the dismissal arm's `.is_ok()` gate) — benign
   today because the firmware restores its own saved screen on a validated
   dismissal, but a debug log would make future sessions easier to read.

Counters at session end: `valid=3261 malformed=0 crc=0 overflow=0`,
`events dropped=1` (one event emitted while the link was down, dropped by
design), `free_heap 8480619` — byte-identical to every prior record.

## CO5300 even-window rounding at the 64-line flush strip — PASSED (2026-08-15)

Closes the "needs human eyes" flag above. Firmware `m1-113-g05d04ce`,
`/dev/cu.usbmodem1101`, user observing at desk distance. Content: the default
ticking clock with seconds (`show_seconds: true`), whose per-second digit
updates exercise the partial-flush/rounding path continuously.

- **90° (default, USB down):** observed during the alert re-verification
  session's connected phases (~1 minute of ticking) — no flashing artifacts,
  no flicker bands, no shifted columns, no corrupted stripes.
- **270° (flipped, USB up):** held via a scratch runtime harness applying
  `orientation: landscape-flipped` (~1 minute of ticking) — clean flip, no
  mirroring or offset, no artifacts.

User verdict verbatim: "There are no flashing artifacts" (90°), "Everything is
fine" (270°). The other unevaluated visual-quality items from the V1 acceptance
(surface-module rendering quality, storm icon readability, boot-clock
appearance, color/contrast) were not part of this check and remain unevaluated.

## Webcam harness commissioned — 2026-08-15

The webcam verification harness (`tools/hwcam/`, design spec
`docs/superpowers/specs/2026-08-15-deskmate-webcam-harness-design.md`) passed
its live acceptance. Judging is agent-vision only — no CV/OCR code exists by
design. Session evidence: `~/deskmate-hw-sessions/2026-08-15-harness-acceptance/`
(frames named below; `NOTES.md` there holds the full session record).

- **Framing preflight:** PASSED on the third frame
  (`20260815T125708Z-preflight3.jpg` — sharp, centered, all panel labels
  readable) after the user adjusted the camera; the first two attempts were
  soft because the desk setup had shifted. One capture attempt failed loudly
  with "Could not lock device for configuration" while the OBSBOT Center app
  held the camera — a live validation of the harness's camera-exclusivity
  error path.
- **10-minute observation-mode timelapse:** `timelapse.sh <session> 25 soak`,
  killed at 600 s. 20 frames captured (expected 18–21), **0 FAILED** lines in
  `timelapse.log`, start/stop lines present. Sampling rule: every 5th frame
  plus the final one (#1, #6, #11, #16, #20). All sampled frames sharp and
  readable; the panel held an identical paused-pomodoro card (Focus 24:57,
  ELAPSED 00:03, STATUS Paused) throughout — the user had tapped the pomodoro
  during camera adjustment, and an active/paused pomodoro pins the takeover
  face, so no rotation was expected. No artifacts, no reboot, no
  standalone-clock fallback.
- **Incidental:** the board was found running user-flashed foreign firmware
  before acceptance (`20260815T124444Z-firmware-check.jpg`); Deskmate V1
  firmware was rebuilt and reflashed (user-authorized), and the companion app
  re-adopted the board (`20260815T124606Z-postflash.jpg`).

## Clock title removal — partial verification 2026-08-17

Software change: the identity chip is gone from all three clock surfaces and the
`DATE` eyebrow from the two date-bearing ones; each remaining stack is re-centred on
its own canvas. Plan
`docs/superpowers/plans/2026-08-17-deskmate-clock-title-removal.md`, spec
`docs/superpowers/specs/2026-08-17-deskmate-clock-title-removal-design.md`.
Firmware flashed from `idf.py -C firmware -DDESKMATE_DEV_DIAG=1 flash` at commit
`876033b`.

**PASSED — physical framebuffer diff, all 14 clock frames byte-identical.**
`companion/crates/device/examples/framebuffer_diff.rs` run twice against the board.
All eight `digital-clock--*` and all six `analog-clock--*` cases compared **identical**
in both runs, at both `landscape` and `flipped`. The panel therefore draws the
re-centred faces exactly as the re-blessed goldens claim; this covers the two card
faces at both orientations more strictly than a photograph could.

**Pre-existing harness defect found: `progress-ring--running-mid-countdown` is
timing-sensitive on hardware and cannot pass deterministically.** Both runs reported
`total=54 identical=51 differing=1 errored=0 excluded=2`, but the differing case was
`--landscape` on the first run and `--flipped` on the second, with an identical
signature each time (100 pixels, max channel delta 208). A deterministic rendering
fault would hit the same orientation every run; the same signature migrating between
orientations is a race. Root cause is in `progress_ring.c:39-48`: `current_remaining_ms`
subtracts elapsed `lv_tick_get()` time whenever `progress_running` is true, so a
*running* ring keeps counting down on the device after its fields are pushed, while
`Simulator::render` draws the case's pinned `remaining_seconds: 900` frozen. Whether the
capture lands before or after the device's next one-second tick decides the comparison.
Unrelated to this change — `progress_ring.c` was not touched, and the case pins
`running: true`. It also means V1 acceptance's recorded `0 differing` was luck rather
than proof, and that the expectation of `identical=52` in this plan was unattainable.
Not fixed here (out of scope); options when someone does address it are to pin
`running: false` for the compared case, or to exclude it in `exclusion_reason()` the way
the `row-list--truncation-boundary` pair already is.

**PASSED (with a stated limit) — the standalone fallback screen at 90°.** This is the one
surface the framebuffer diff cannot reach, because it has no golden case, so the webcam
harness is the only instrument. The first framing preflight FAILED unrecoverably (board at
the right edge of frame, backlit, 180° rotated —
`20260817T091111Z-fallback-90.jpg`); after the setup was corrected the panel was upright
and readable. Confirmed on `20260817T092738Z-fallback-waited.jpg`: **no `CLOCK` chip**
above the time (clear canvas where the pill used to sit), **no `DATE` eyebrow** above
"Mon, Aug 17", the date alone and vertically centred in a **single full-width module**
(the fallback's `CLOCK_MODULE_W` = 400, visibly distinct from the card's narrower date
module plus dial), and generous canvas above the hero consistent with the intended 88px
top margin. **Limit:** the camera's field of view crops the panel's bottom edge, so the
"88 above / 88 below exactly centred" arithmetic is confirmed only for the top half by
direct observation. The bottom half rests on the arithmetic over
`CLOCK_TIME_Y`/`CLOCK_MODULE_Y`/`CLOCK_MODULE_H` plus the fact that the two card faces —
same LVGL path, same board, same rounder callback — came back pixel-exact from the
framebuffer diff.

**PASSED — the fallback-to-card transition, both directions.** Four frames from an
unmoved camera, one minute apart: card at 09:26 (`20260817T092647Z-fallback-90-retry.jpg`,
dial module present), fallback at 09:27 (`20260817T092738Z-fallback-waited.jpg`, single
module, hero visibly lower), card again at 09:28
(`20260817T092832Z-card-reconnected.jpg`, dial module back, hero visibly higher). The
predicted 24px hero shift is real and observable when frames are compared side by side,
but in motion it does not read as a glitch: the whole layout changes at once (the dial
module appears or vanishes, the date module resizes), so the hero moving is part of a
wholesale screen swap rather than a jump. **Also observed:** the fallback does not appear
the instant the companion app quits — the device waits for its link keepalive to time out,
roughly 30 s here. A capture taken ~4 s after quitting still showed the card.

**NOT VERIFIED — the fallback screen at 270°.** Changing orientation requires the
companion's settings UI, which the agent cannot drive. The two *card* faces are verified
at both orientations by the framebuffer diff (`--landscape` and `--flipped` both
identical), and the fallback screen uses the same software-rotation path, but its 270°
rendering was not itself observed. Do not describe it as verified.

Incidental: the installed `/Applications/Deskmate.app` held `/dev/cu.usbmodem1101`
exclusively and was quit (gracefully, via `osascript`) to free the port for flashing and
for the diff runs. It was not running when these observations were taken.

## V2 Task 3 — NVS persistence and the tier gate, verified 2026-08-18

First firmware to persist anything across a reboot. Software change: an NVS-backed
network config store (`firmware/main/link/net_store.c`), handlers for the two new
provisioning message types, and the USB tier gate. Plan
`docs/superpowers/plans/2026-08-18-deskmate-v2-networked-device.md`, spec
`docs/superpowers/specs/2026-08-18-deskmate-v2-networked-device-design.md`.
Flashed from `idf.py -C firmware -p /dev/cu.usbmodem3101 flash` at commit `77dc868`
(`firmware_version` reported as `m1-170-g77dc868`, confirming the running image).
No radio involved in any of this — the device never joined a network.

Reboots were performed with `esptool --after hard_reset` rather than by unplugging USB.
That is the correct instrument here: the board has a battery, so a USB unplug is link
loss rather than power loss (recorded under the 2026-08-15 alert-replay entry), and NVS
lives in flash, which survives a reset and a power cut identically. Each reboot was
confirmed by a low `uptime_ms` in the following status.

**PASSED — all six checks.**

1. **Factory-fresh state reports local.** Before any provisioning, `status --json`
   reported `tier: local`, `capabilities: 11`, `rotation: 90`, `valid_frames: 1`,
   `malformed_frames: 0`.
2. **Provisioning is accepted and acknowledged.** `provision ... --tier networked` ACKed,
   exit 0, printing `(takes effect on next boot)`. The output named the ssid, server_url
   and device_id and **did not print the passphrase or the token** — the never-expose-
   secrets rule observed in practice, not only in review.
3. **The tier survived a reboot.** After a hard reset, `status` reported
   `tier: networked` at `uptime_ms: 12610`. This is the change's central claim and it
   holds.
4. **The cable is refused as an owner.** With the device in networked tier,
   `push-data` over USB was rejected: `device rejected request (WrongTier): device is
   owned over the network`, CLI exit code **13** (non-zero, so the failure is
   scriptable). This is the first physically observable proof of the single-owner
   invariant on the network tier.
5. **Factory reset returns the device to local.** `factory-reset` ACKed; after a hard
   reset `status` reported `tier: local` at `uptime_ms: 5807`.
6. **An all-empty local config round-trips.** Provisioning with every string field empty
   and `--tier local` ACKed, and after a reboot reported `tier: local` at
   `uptime_ms: 5811`. This settles a question that could not be answered off-hardware:
   `nvs_set_str` does accept the empty strings a factory-fresh local config writes.

Counters stayed clean throughout: `malformed_frames: 0`, `free_heap` ~8.474 MB at every
sample (8474155 / 8474123), `ui_queue_high_water: 1`. The panel showed no change at any
point, which is correct — this task adds no UI.

**Not covered by this session, and not claimed:** the radio, SNTP, the WebSocket
transport, and OTA are all later tasks. A genuinely torn NVS write (power cut mid-save)
was not induced; the code's mitigation is field ordering, with `tier` written last, so a
fresh device cannot half-become networked.

Incidental: `/Applications/Deskmate.app` held the serial port exclusively and was quit
gracefully via `osascript` before flashing, as in the 2026-08-15 session. It was not
running during any of these observations, and was left quit afterwards. The board
enumerated as `/dev/cu.usbmodem3101` this session rather than `1101`.

## V2 Task 4 — transport vtable refactor, verified 2026-08-18

Pure refactor: `protocol_task.c`'s four `usb_link_*` call sites now route through a
`link_transport_t` vtable, so Task 8's WebSocket transport can be a sibling rather than a
special case. No behaviour was intended to change, so the gate is equivalence, not new
function. Flashed at commit `660d226` (`firmware_version` `m1-172-g660d226`).

**PASSED — behaviour identical to Task 3's verified state.**

- Status in local tier reported `tier: local`, `malformed_frames: 0`, `valid_frames: 1`.
- `push-data` in **local** tier was rejected with `UnknownWidget` — the informative
  result, not merely a success: the tier gate passed the message through and the
  *handler* rejected it because no widget is configured. The message reached its
  destination.
- Re-provisioned to networked, hard reset: `tier: networked` at `uptime_ms: 6125`, so
  persistence still works across the refactor.
- `push-data` in **networked** tier was rejected with `WrongTier` at CLI exit 13 — a
  different rejection from the local-tier case, which is what proves the gate still
  discriminates by tier rather than simply blocking or simply passing.
- Restored to local tier with `factory-reset`.

The two distinct rejections are the substance of this check. A refactor that had
disturbed the gate's position would most likely have produced the same answer in both
tiers.

## V2 Task 5 — per-transport link deadline, sanity-checked 2026-08-18

`link_state`'s host-loss deadline is now a per-transport value rather than the
compile-time `PROTOCOL_LINK_TIMEOUT_MS`. USB keeps exactly 10000 ms; the 45000 ms value
for the network transport is defined but has no consumer until Task 8. Flashed at commit
`e257f33` (`firmware_version` `m1-174-ge257f33`).

The plan defines no physical step for this task — its gate is the host tests, which pin
the boundaries at 9999/10000 and 44999/45000. This was run anyway as cheap insurance,
because the task rewired how the USB deadline is initialised.

**PASSED — USB link healthy and unchanged.** `tier: local`, `online: true`,
`malformed_frames: 0`, `free_heap` 8474107 (flat against Task 3's 8474155/8474123 to
within normal variation). A second status three seconds later still reported
`online: true` with `valid_frames: 2`, so the 10-second USB deadline is intact and the
link is not flapping.

Not covered: the 45000 ms network deadline cannot be exercised until a network transport
exists (Task 8).

## V2 Task 6 — WiFi station and SNTP, verified 2026-08-18

The radio comes up for the first time. Flashed at commit `373b0ca` lineage
(`baac85f` firmware + CLI status fields), board on `/dev/cu.usbmodem3101`.

**A blocking defect was found and fixed first — see the entry below this one.** The
first WiFi build crash-looped before reaching the clock screen; `s_context` was moved to
PSRAM to make room. Everything here was observed on the fixed build.

**PASSED — network half, read from `status --json`.**

- `wifi_state: connected`, `ip: 192.168.8.168`, `wifi_rssi: -63`. The RSSI is a live
  sample via `esp_wifi_sta_get_ap_info()`, not the join-instant value — a review finding
  fixed before this run, because a frozen RSSI defeats the "wrong password versus out of
  range" judgement the FAILED classification exists to support.
- `tier: local`, correct for the free tier: the radio is up for SNTP and firmware only,
  and no control connection to any server exists.
- `uptime_ms: 282274` at the sample, ~4.7 minutes with no reboot. `malformed_frames: 0`.
  `free_heap` 8,376,795.

**PASSED — the standalone clock shows correct local time, observed by the repository
owner directly.** The panel showed the correct local time when checked against UTC+4 at
17:49 local / 13:49 UTC. The webcam harness was unavailable this session (see below), so
this is a direct human observation rather than a captured frame.

Worth stating precisely, because it is stronger than "no host attached": USB *was*
connected throughout as a status-polling link, but **no configuration and no time-sync
was ever pushed** — neither `status` nor `provision` sends one, and no config was ever
applied, so the device sat on the standalone clock screen for the whole session. The
displayed time can therefore only have come from SNTP supplying UTC plus the provisioned
`utc_offset_minutes` of 240 supplying the zone. That is exactly the property Task 6
exists to deliver.

**NOT VERIFIED — the network-loss transition.** That pulling the network moves
`wifi_state` to `connecting` without rebooting the device was not exercised; it needs the
AP taken away or the board moved out of range, neither of which was available. The
reconnect path is covered by review only.

**NOT VERIFIED — anything requiring the webcam.** The harness failed this session and
this exposed a latent defect in it: the physical **OBSBOT Meet 2 StreamCamera was absent
from the `avfoundation` device list entirely** (only MacBook Pro Camera `[0]`, **OBSBOT
Virtual Camera** `[1]` and Desk View `[2]` enumerated), while the OBSBOT Center system
extension was running. `tools/hwcam/capture.sh` resolves the camera by matching the first
video device whose name contains `OBSBOT`, which now matches the *Virtual* camera, and
opening it fails with `Error opening input file 1`. The script's own comment explains it
resolves by name because indices shift — but the match is too loose to tell the real
camera from the virtual one. **Fix when next touching the harness: exclude `Virtual` from
the match, and fail with a clear message when only the virtual device is present.**

## V2 — WiFi crash-loop root-caused and fixed, 2026-08-18

The first Task 6 build **crash-looped on every boot** and never reached the standalone
clock: `ESP_ERROR_CHECK failed: esp_err_t 0x101 (ESP_ERR_NO_MEM)` at `main.c:61`,
`expression: board_display_init()`.

**Root cause: the WiFi stack's static DIRAM squeezed the LVGL draw buffers out of
internal RAM.** `display.c` needs two internal-DMA buffers (flush plus rotation scratch,
because `sw_rotate = true`): 2 x 47,104 = 94,208 B, and its guard requires a free block of
95,232 B.

Measured on the board with temporary `esp_rom_printf` probes:

| | bytes |
|---|---|
| Largest contiguous `MALLOC_CAP_DMA` block at `board_display_init()` entry | **61,440** |
| Required | 95,232 |
| Shortfall | 33,792 |
| Same figure measured during V1 | 110,592 |

The block was identical at every probe point through expander reset, SPI bus init and
`lvgl_port_init`, so nothing inside display init consumed it — the loss predated
`app_main`. `idf.py size-components` attributed it: libpp 19,189 + libnet80211 12,422 +
libphy 8,498 + liblwip 3,790 + libwpa_supplicant 1,371 = **45,270 B** of WiFi-stack
DIRAM, against a measured drop of 49,152 B from V1.

**Fix: `s_context` (54,616 B) moved from internal `.bss` to PSRAM.** The map file ranks
internal `.bss` as `work_mem_int$0` 65,536 (LVGL's pool), `s_context` 54,616,
`s_queue` 10,232 — roughly 130 KB, none of it DMA-requiring, on a board with 8 MB of idle
PSRAM. Nothing in `protocol_context_t` is DMA'd: `usb_serial_jtag_write_bytes()` and
`esp_websocket_client_send_bin()` both copy, and the context is touched only from the
protocol task, never an ISR.

**Result, measured: largest contiguous DMA block 61,440 -> 118,784 B**, a margin of
23,552 B over the 95,232 requirement, with headroom for Task 8's TLS. One boot, zero
crashes.

Two notes for whoever hits this next:

- **LVGL's 64 KiB pool is the bigger block but the wrong target.** `firmware/lv_conf.h`
  is compiled by the `lvgl-sim` host harness, so changing LVGL's allocator would put V1's
  pixel-exact golden-frame pipeline and all 54 goldens at risk. `s_context` is confined to
  one file in `link/` and cannot affect the simulator.
- **The error was legible only because of V1's defensive guard.** The explicit
  `heap_caps_get_largest_free_block(MALLOC_CAP_DMA)` check added after the 2026-08-13
  crash-loop turned what would have been an `IllegalInstruction` panic with a corrupted
  backtrace into a named `ESP_ERR_NO_MEM` at one line. Note also that the `ESP_LOGE`
  carrying the actual figures was **invisible**: the post-scheduler console goes to UART0
  only, and even switching the primary console to USB Serial JTAG did not surface it, so
  `esp_rom_printf` probes were required. Same blindfold as 2026-08-13.

## V2 Task 8 — networked link root-caused and fixed, verified 2026-08-19

First physical execution of Task 8 Step 8. The board never reached the server, and
the cause was in the build, not the network.

### Symptom

Provisioned networked (`dev-0003`, `wss://deskmate.rodi.one/v1/device/link`), the
board reported `tier: networked`, `wifi_state: connected`, `ip: 192.168.8.168`,
`capabilities: 203` — but `online: false` and `ota_state: failed`, indefinitely.
`GET /v1/devices/dev-0003` returned `connected: false, last_seen_unix_ms: null`.
Uptime climbed monotonically past 119 s with `free_heap` byte-flat, so there was no
reboot loop; the device was stable and simply never connected.

### Root cause

`CONFIG_MBEDTLS_INTERNAL_MEM_ALLOC=y` — ESP-IDF's default — confines every mbedTLS
allocation to **internal DRAM**. Internal DRAM on this board is already committed to
LVGL's draw buffers and WiFi's static `.bss` (the same budget behind the V1 boot
crash-loop recorded above), so the 16 KB `MBEDTLS_SSL_IN_CONTENT_LEN` buffer could
not be obtained and `mbedtls_ssl_setup()` failed **before any socket work**:

```
E esp-tls-mbedtls: mbedtls_ssl_setup returned -0x7F00   (MBEDTLS_ERR_SSL_ALLOC_FAILED)
E websocket_client: transport_error=ESP_ERR_MBEDTLS_SSL_SETUP_FAILED
```

One cause produced every symptom: no WSS link, no OTA check reaching the server, no
traffic at the origin, no reboot, and a correctly widening reconnect backoff.

**`free_heap` is not a TLS health signal on this board.** It read 8,340,243 bytes
throughout — overwhelmingly PSRAM, the one pool mbedTLS was forbidden to touch. A
device here can report 8 MB free and still fail every handshake.

### Fix

`CONFIG_MBEDTLS_EXTERNAL_MEM_ALLOC=y` in `firmware/sdkconfig.defaults`, so TLS
allocates from the 8 MB PSRAM. Commented at the setting.

Verified on the board the same day: on the first boot after flashing, the server
logged `firmware check device_id=dev-0003` and `device link established
device_id=dev-0003`, and `GET /v1/devices/dev-0003` returned `connected: true` with a
full `AppSnapshot` (`port_name: "network:dev-0003"`, `valid_frames: 39`,
`reconnects: 0`). The panel left the standalone fallback and rendered the server's
`DigitalClock` card.

### Diagnosis required a temporary console build

The protocol link uses **USB-Serial-JTAG** (`usb_link.c`) and the console is UART0
with `CONFIG_ESP_CONSOLE_SECONDARY_NONE`, so no device log is reachable over the
cable in a shipping build. A diagnostic build with
`CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y` was used to read the mbedTLS error; it
interleaves log output with protocol frames and must never supply acceptance
evidence. `firmware/sdkconfig` is gitignored, so no tracked file changed. Note that
`idf.py fullclean` does **not** remove `firmware/sdkconfig`, and a stale one silently
overrides `SDKCONFIG_DEFAULTS` — delete it explicitly when switching config sets.

### Deployment defects found alongside, both on the live server

1. **`RUST_LOG` was unset**, so `tracing_subscriber::fmt::init()`'s `EnvFilter`
   defaulted to **ERROR** and discarded every device-link diagnostic. The only
   visible startup line is the `println!` in `main.rs:68`; the `tracing::info!`
   directly beneath it was being dropped, which made logging look alive while
   reporting nothing. **Task 8 Step 8's first observation — "the server logs an
   accepted connection" — could not have passed as deployed.** Set to
   `info,server=debug`; original saved as `/etc/deskmate/server.env.bak-20260819`.
2. **`DESKMATE_FIRMWARE_DIR` did not exist** while `DESKMATE_FIRMWARE_VERSION` was
   `1.0.0`, so `/v1/device/firmware` advertised an update whose image could not be
   downloaded, and the device failed the install on every boot. That — not a device
   fault — is what `ota_state: failed` meant. Directory created; version corrected.
   Must be set deliberately before Task 11.

### Two corrections to Task 8 Step 8's checklist

- Its third observation expects the panel to stay on the standalone clock because
  "no config has been sent yet". The server holds a **default** config
  (`config.origin: "defaults"`) and applies it on connect, so the panel leaves the
  fallback immediately. The wording is wrong, not the behaviour.
- Early-boot TLS failures are normal: `net_link_start()` runs before DHCP/DNS, so the
  first attempts fail `ESP_ERR_ESP_TLS_CANNOT_RESOLVE_HOSTNAME` (`getaddrinfo()
  returns 202`) at t≈1.5 s before the backoff succeeds. Do not confuse these with
  `ESP_ERR_MBEDTLS_SSL_SETUP_FAILED`.

Task 8's four observations are **not** all discharged by this entry: the accepted
connection and the status round-trips are evidenced above, but the killed-server
reconnect observation has not been run against a deliberately stopped server, and
the panel observation needs re-running under the corrected expectation.
