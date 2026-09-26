# Waveshare ESP32-S3-Touch-AMOLED-1.8 (v2) — board notes

**Status: HISTORICAL LAB RECORD.** Dated entries preserve the exact software and hardware
used for each observation; names of retired host software are evidence, not current setup
instructions.

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
      (`firmware/main/core/widget_model.c:323` at the time) had **no callers anywhere
      in the firmware**; the counter and getter were removed in the 2026-09-05 cleanup. The counter is accumulated and never read, so no host-observable
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
- To keep the generated diagnostic configuration disposable and isolated from a release
  build, create it inside its own build directory:

  ```sh
  idf.py -C firmware -B firmware/build-diag \
    -DSDKCONFIG=build-diag/sdkconfig \
    -DSDKCONFIG_DEFAULTS='sdkconfig.defaults;sdkconfig.diag' \
    -DDESKMATE_DEV_DIAG=1 build
  ```

  `sdkconfig.diag` routes diagnostic logs onto the protocol pipe. Active logging can
  corrupt COBS frames; this build must never ship and cannot supply acceptance evidence.
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

Retired 2026-09-19: `alert_replay_check` verified the deleted desktop serial host; the historical result below is preserved.

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

   > **The "nothing is pushed back" half is REFUTED in software, 2026-08-21. The
   > observation stands; its explanation does not.** `control_pomodoro` does not
   > gate on whether state changed — it inserts into `latest_fields`, marks the
   > widget dirty and pushes unconditionally — and `Session::push_fields` has no
   > unchanged-payload dedupe, so a real push goes out. `PomodoroUpdate.fields`
   > always carries `running`, which on `Completed` is `false`. On the firmware
   > side `progress_ring.c`'s patch calls `set_running_color(view, running)`
   > unconditionally (no early return; the early return in `render_remaining`
   > affects only the label text), and `carousel.c` states the intent outright:
   > "Optimistic feedback is intentionally reconciled by the next full
   > authoritative PushData snapshot". Pinned by
   > `a_tap_on_a_completed_pomodoro_still_pushes_authoritative_state`.
   >
   > Two hypotheses were checked and dropped. The tapped surface was **not** the
   > interrupt overlay: `carousel.c` returns early on an interrupt tap, emitting
   > `DISMISS_INTERRUPT` without ever calling
   > `template_view_apply_local_action`, so an overlay tap applies no optimistic
   > feedback at all. And the device layer does not dedupe identical pushes.
   >
   > **What remains, for the board.** The leading candidate is that the push did
   > not happen *on that occasion* rather than that it never happens:
   > `control_pomodoro` only pushes inline when
   > `state.connected && !preferences.paused`, so a paused config would leave the
   > red standing until the next authoritative push. The cheap check is to tap a
   > completed pomodoro and look for a `PushData` for that widget in the same
   > second — if it is there and the red persists, the defect is in firmware's
   > apply path and not in the host; if it is absent, read `preferences.paused`
   > and the connection state at that moment. Do not "fix" this host-side
   > without that evidence: the host is currently doing what both sides document.
3. The host silently ignores an `InterruptDismissed` event whose token it no
   longer tracks (`runtime.rs`, the dismissal arm's `.is_ok()` gate) — benign
   today because the firmware restores its own saved screen on a validated
   dismissal, but a debug log would make future sessions easier to read.

   > **ADDRESSED 2026-08-21, as a counter rather than a log.** `app-core` has no
   > logging facility at all, so a debug log would have meant adding a dependency
   > to emit something a hardware session may not even capture. The runtime
   > snapshot is what actually gets read on the board, so the signal went there:
   > `RuntimeDiagnostics.interrupt_dismissals_ignored` counts every
   > `InterruptDismissed` the host received and did not apply — unknown token, or
   > no token at all. Declining is still correct; what changes is that it is now
   > visible, so "the host saw my tap and declined it" and "the event never
   > arrived" stop looking identical. Reachable in **both** tiers without extra
   > work: it rides the existing `AppSnapshot`, so the networked tier gets it
   > under `snapshot.diagnostics` in `GET /v1/devices/{id}`.
   >
   > Two tests pin it, because a counter that always increments is not a signal:
   > one asserts an untracked token counts, and
   > `pomodoro_events_complete_once_and_dismissed_interrupts_do_not_replay` now
   > asserts a *valid* dismissal leaves it at zero. The field crosses the
   > cross-language IPC contract, so `types.ts`, the generated
   > `types.contract.ts` and the dev-harness fixture moved with it.

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
Not fixed here (out of scope).

> **FIXED 2026-08-21, and neither of the options guessed at above was the right one.**
> Pinning `running: false` on the compared case would have deleted the only pixel
> coverage of the running arc-indicator hue, and a plain exclusion would have lost the
> partial-arc geometry with it — the other two progress-ring cases sit at the arc's
> extremes (0 and full). A third option was available because the flakiness lives only in
> the *device-vs-simulator* comparison: the simulator's own goldens are deterministic, so
> the case is now **golden-only**, kept in `cases::golden_cases()` and excluded from
> hardware by name in `exclusion_reason()`. Two deterministic cases replace its hardware
> coverage: `paused-mid-countdown` (same 900/1500 partial arc, ring stopped, so both
> sides render the pinned value) and `running-at-zero` (the only running ring hardware
> can be compared on — `current_remaining_ms` clamps to 0 whenever
> `elapsed >= remaining_ms`, which at 0 holds for any elapsed; it covers the running
> status colour but not the arc hue, because a zero-length arc draws no indicator).
>
> Worth recording that **no running value could have worked**: the label's
> `(remaining_ms + 999) / 1000` ceiling flips a second every second by construction,
> whatever the duration, so "choose a steadier remaining_seconds" was never on the table.
> The expectation on this date, before C-template retirement, was
> `total=58 identical=54 differing=0 excluded=4`. Task 8's review restored those 58
> six-face rows as device-pushable scenes alongside 18 synthetic node-kind rows. Gate B's
> current expectation is therefore `total=76 identical=64 differing=0 excluded=12`:
> four face-row exclusions (the two orientations of this running case and the row-list
> truncation boundary) plus eight synthetic asset/field exclusions. Both name-based
> exclusions are reachable and active again. A differing case is a real disagreement,
> not something to recognise and wave through.

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

## V2 Task 11 — OTA download panicked before writing a byte, root-caused and fixed 2026-08-19

First physical execution of Task 11 Step 9. The happy path did not work.

### Symptom

With an update published and offered, the board cycled roughly every ten seconds:

```
10:13:34  ota=checking     up=4752
10:13:39  ota=downloading  up=10322
10:13:45  ota=checking     up=3973    <- rebooted
10:13:50  ota=downloading  up=9571
10:13:56  ota=checking     up=3053    <- rebooted
```

It never reached version B and always came back on version A.

**This failure mode is nearly invisible from the host.** After each panic
`ota_state` returns to `checking`, so a host polling status sees a device
perpetually *about to* update rather than one that failed. Nothing in Task 11's
checklist asks anyone to read `uptime_ms`, which was the only honest signal.

### Root cause

Console capture (diagnostic build) caught the panic immediately after
`esp_https_ota: Writing to <ota_1> partition at offset 0x420000`:

```
Guru Meditation Error: Core 1 panic'ed (LoadProhibited). Exception was unhandled.
Backtrace: 0x42098b2d 0x42099105 0x4200d5d1 0x4200d9cd 0x403802d1
rst:0xc (RTC_SW_CPU_RST)
```

Symbolicated against the matching ELF:

```
get_description_from_image   esp_https_ota.c:598
esp_https_ota_get_img_desc   esp_https_ota.c:645
install_update               firmware/main/link/ota.c:321
ota_task                     firmware/main/link/ota.c:464
```

`ota.c` passed `.staging` and `.final` as **the same partition pointer** — the
straightforward way to describe a single-partition update. `esp_https_ota_begin()`
(ESP-IDF v5.5.5, `esp_https_ota.c:501-512`) only assigns
`handle->partition.final` inside `if (ota_config->partition.staging !=
ota_config->partition.final)`, and that inner `if` has **no else branch**. Passing
them equal therefore leaves `handle->partition.final` unset, and
`get_description_from_image()` dereferences `handle->partition.final->type` at
line 602. LoadProhibited, before a single byte is written.

### Fix

Leave `.final` NULL in `esp_https_ota_config_t` so `esp_https_ota_begin()` takes its
own `final = staging` path, which is the intended semantics. One-field change in
`firmware/main/link/ota.c`, commented at the site.

### Notes for anyone repeating this

- The coredump partition holds a dump, but `idf.py coredump-info` refuses it when the
  build directory no longer contains the exact ELF that crashed
  (`coredump SHA256(...) != app SHA256(...)`). Keep the ELF, or reproduce under the
  console build instead.
- The console is UART0 with `CONFIG_ESP_CONSOLE_SECONDARY_NONE` and the protocol link
  owns USB-Serial-JTAG, so a shipping build cannot report a panic over the cable. A
  diagnostic build with `CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y` is currently the only
  way to see one; it interleaves logs with protocol frames and must not supply
  acceptance evidence.

## V2 Task 11 — two concurrent TLS sessions break the download, fixed 2026-08-19

Found while attempting Step 10's rollback test. The broken image C never installed:
the download failed reproducibly ~7 s in, three times, including under the console
build. The server was innocent — the offer was correct and
`GET /v1/firmware/<version>.bin` returned HTTP 200 with the right embedded version —
and it was not a timeout (those are 120 s no-progress, 1800 s total).

### Root cause

```
I esp_https_ota: Starting OTA...
I esp_https_ota: Writing to <ota_1> partition at offset 0x420000
E esp-aes: Failed to allocate memory
E esp-tls-mbedtls: read error :-0x0001
E esp-tls-mbedtls: read error :-0x7180
E esp_https_ota: data read -1, errno 0
W ota: firmware check failed: ESP_FAIL
```

`esp-aes` is the **hardware AES accelerator**. Its DMA buffers must come from internal
memory — PSRAM cannot serve DMA — and `CONFIG_SPIRAM_MALLOC_RESERVE_INTERNAL` reserves
only 32 KB against 228 KB of internal RAM already committed to LVGL and WiFi. With the
WSS protocol link live, a second TLS session for the download cannot get its buffers.

Same family as this session's first defect: moving mbedTLS's own allocations to PSRAM
(`CONFIG_MBEDTLS_EXTERNAL_MEM_ALLOC`) did **not** move the AES driver's DMA buffers,
and it cannot — they are DMA by definition.

### Fix

`install_update()` suspends the WebSocket for the duration of the download and resumes
it on every failure path (`0ad1a51`). Owner chose this over raising the internal
reserve (which would take RAM from the pool behind two previous incidents) or
disabling hardware AES with `CONFIG_MBEDTLS_HARDWARE_AES=n` (which would slow every TLS
operation).

Two implementation notes that matter:

- **`stop()`, not `close()`.** The client sets `enable_close_reconnect`, so a graceful
  close immediately re-dials and reintroduces the second session.
- **Every failure path resumes.** Only success skips it, because it reboots. A failure
  returning without reopening the link would leave the display server-less until the
  next reset — a recoverable download failure turned into a dead device.

### This retracts part of Step 9's earlier result

Step 9's happy path was recorded as PASS earlier the same day. It did complete an
update, but only because the WSS link happened not to be established while that
download ran — the log shows it still failing `CANNOT_RESOLVE_HOSTNAME` at the time.
**That pass was timing, not a working mechanism.** Step 9 must be re-run with the link
established before OTA can be called verified.

### Observability gap worth closing

The protocol reports OTA *state* but never an OTA *reason*: no field in
`StatusResponse`, none in the server's `AppSnapshot`. A device that fails to update
reports `ota_state: failed` and nothing else. Diagnosing this one required a
console-build reflash of the board on the desk; a fleet device could not be diagnosed
at all.

## V2 Task 11 Step 10 — rollback verified on the board, 2026-08-19

Deliberately broken image `m1-rollback-c2` (an early `abort()` before
`ota_mark_running_image_valid()`) published and offered to a device running
`m1-226-gaff8e7d`. Started from a full `idf.py flash` of `m1-225-g0ad1a51`, so the
bootloader on the board is the one carrying `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE`.

Observed, unattended end to end:

```
10:56:13  server: device link established
10:56:14  server: firmware check current=m1-226-gaff8e7d
10:56:14  server: device link closed          <- suspended for the download
10:56:17..35  ota=downloading (18 s)
10:56:41  m1-226-gaff8e7d  ota=checking  up=1941   <- back on B already
10:56:47  m1-226-gaff8e7d  ota=failed    up=8021
```

Flash and otadata afterwards:

```
ota_0: m1-rollback-c2      otadata[0]: ota_seq=3 -> ota_0  state=4 (ABORTED)
ota_1: m1-226-gaff8e7d     otadata[1]: ota_seq=2 -> ota_1  state=2 (VALID)
```

So C was really downloaded and installed, really booted, really failed to validate,
and the **bootloader really rolled back** — the device returned to B with no reflash
and no human intervention. The rollback path is now exercised rather than assumed.

Two things fell out of this run:

- The `device link closed` one second after the firmware check is the Task 11
  serialization fix (`0ad1a51`) visible from the server side.
- The trailing `ota=failed` is **not** a fault. After rolling back, the device checked
  again, found the server still advertising the image that had just failed, and
  refused to reinstall it — `reject_reinstall_of_failed_image()` doing its job. That
  is CLAUDE.md's "refused forever" rule observed rather than assumed, and it is why a
  corrected rebuild must carry a new version string.

## V2 Task 11 Step 11 — deferral FAILS on hardware, and why, 2026-08-19

**Result: the update began while a pomodoro was running.** Not a near miss — a clean
failure of the gate item.

Setup: pomodoro confirmed `state: running, remaining_seconds: 268` server-side, version
`m1-227-gf14f241` published and offered, board reset to trigger its boot check.

```
11:32:04  reset (pomodoro running, 268 s left)
11:32:14  ota=checking      up=7101
11:32:20  ota=downloading   up=13136
11:32:32  ota=downloading   up=25221
11:32:46  m1-227-gf14f241   ota=idle   <- updated anyway
```

### Root cause: the device has no link when the check runs

```
11:32:16  WARN device link refused: owner already live  device_id=dev-0003
11:32:16  DEBUG firmware check                          current=m1-226-gaff8e7d
11:32:17  WARN device link idle timeout, closing
11:32:17  INFO device link closed
11:32:44  INFO device link established                  <- 28 s after the reboot
```

The board rebooted and reconnected faster than the server noticed the old socket was
dead, so **the server refused its own device** ("owner already live"). The stale link
timed out a second later, but by then the device had backed off. The firmware check
therefore ran with **no link at all**, so no host state had been replayed, so
`protocol_task_ota_blocked()` saw no running pomodoro and the download proceeded.

The deferral logic is not broken. The state it depends on is simply absent at the only
moment a check ever happens — boot. Since the scheduled interval is 24 h, the guard as
built protects a timer started *between* boots and essentially never protects one at
boot, which is when the check actually fires.

### Two systemic findings from the same run

1. **A quick reboot costs ~28 s of disconnection.** The server's single-owner refusal
   does not distinguish "a second device is impersonating this one" from "this device
   just rebooted and its old socket is stale". The device is refused, backs off, and
   waits. Reconnection is delayed by the guard meant to protect it.
2. **A link drop discards a running pomodoro.** Before the reboot the timer read
   `running, 268 s`; after reconnect it reads `idle, 300 s`. Task 9 Step 4 specifies
   "on close, shut the runtime down so a reconnect starts cleanly", so the runtime is
   torn down and rebuilt from stored config on every link close, taking live timer
   state with it. Combined with the earlier finding that a config apply also resets a
   running timer, **a networked pomodoro survives neither a config edit nor a link
   blip** — and link blips are routine.

### Not fixed in this session

All three are entangled: the deferral needs both a link and replayed state at check
time; the link is delayed by the refusal; and the state would be discarded by the
reconnect anyway. Fixing the deferral alone would not make it work. The candidate
directions — let a reconnecting device take over its own stale socket, persist or
replay live timer state across a reconnect, or delay the boot check until state has
arrived — are design decisions with real consequences, not local patches, and were
left for the owner rather than chosen unilaterally.

## V2 — correction: the ~28 s reconnect was the device's backoff, not the server, 2026-08-19

The Step 11 entry above says the server's single-owner refusal "delays the device it is
meant to protect". That misattributes it, and the record should not stand as written.

The server behaved well: it refused the duplicate at 11:32:16.772 and had closed the
stale link by 11:32:17.034 — **262 ms**. It was not slow to notice.

The delay was the device's own reconnect curve. `net_link_start()` runs at
protocol-task init, before DHCP completes, so the first attempts of every boot fail on
`ESP_ERR_ESP_TLS_CANNOT_RESOLVE_HOSTNAME` by construction. `configure_next_reconnect()`
advanced the backoff on each of those, so by the time the network could carry a
connection the curve was already several doublings up the 1 s → 60 s ramp. The 409
refusal then cost one more doubling on an already-large delay. Refusal was the last
straw, not the cause.

**Fix:** hold the backoff at its 1 s floor while `wifi_station_state()` is not
`CONNECTED`, so failures that happened when there was no network to fail against do not
penalise the first attempt that could have worked. Failures *after* the station has an
IP still back off normally.

The general lesson is worth keeping: a retry curve that starts before its dependencies
are up spends its patience on failures that were never informative.

### Measured on the board after the fix (2026-08-19 11:59 UTC)

Reset-to-`device link established`, from the server's own log:

| run | reset | established | elapsed |
|---|---|---|---|
| baseline (before fix) | 11:32:04 | 11:32:44 | ~40 s, one `refused` |
| 1 | 11:59:28 | 11:59:38 | **~10 s** |
| 2 | 11:59:39 | 11:59:48 | **~10 s** |

Run 2 reset immediately after run 1's link came up, so it is the same stale-socket
scenario that produced the 409 before. **No `device link refused` line appears for
either run** — the device now reconnects before its own stale link matters, so the
refusal never fires. The single-owner guard is unchanged; it simply stopped being
provoked.

## V2 — two loose ends closed honestly, 2026-08-19

### The "rotation stalls on the pomodoro entry" anomaly is RETRACTED

Re-run deliberately with known timestamps: timed playlist, 15 s dwells, entries
clock / weather / pomodoro, applied 12:23:48.

```
12:24:08  clock       12:24:38  pomodoro
12:24:18  weather     12:24:47  pomodoro
12:24:28  weather     12:24:57  clock
                      12:25:07  weather
```

Two full cycles, each entry dwelling ~15 s, wrapping cleanly **past** the pomodoro.
Rotation does not stall. The earlier suspicion rested on five frames and an inferred
config-apply time, and the inference was simply wrong. No defect; nothing to fix.

### Task 8 observation 4's "widening gaps" is still NOT observed on a shipping build

An attempt to evidence it indirectly — vary the server outage length, measure how long
the device takes to return — **does not work, and the method should not be repeated.**
When the server returns, the device sits at a random point inside its current retry
interval, so the measured delay is a uniform sample in [0, T], not T itself:

| outage | time to return |
|---|---|
| 40 s | 32 s |
| ~600 s | **10 s** |

The longer outage returned *faster*. That is not evidence against widening; it is
evidence the measurement carries almost no information. Two samples from two
distributions cannot show one is wider.

What *is* supported: the doubling curve is unit-tested (`test_reconnect_backoff`), the
code is a plain 1 s → 60 s ramp, and the diagnostic console build showed consecutive
gaps of 8.8 s → 13.6 s directly. What is missing is the observation under acceptance
conditions, which needs the device's own retry log — i.e. a console build, which by
this session's own rule cannot supply acceptance evidence.

Also confirmed during those outages: `uptime_ms` climbed continuously past 1,187,131 ms
with no reboot and no spin, and the device reconnected unattended after both a 40 s and
a ~10 minute outage. That is the other half of the observation, and it does pass.

## V2 Task 11 Step 11 — blocker fixed in software, NOT yet observed, 2026-08-19

The deferral failure recorded above has been root-caused into two remaining defects
and both are fixed. **Nothing in this entry was seen on the board.** It exists so the
next board session knows what to re-run and what to expect; the step stays open.

### What was actually wrong

Two independent defects, either of which alone would have kept the deferral useless:

1. **The boot check waited on the radio, not on the owner.** `perform_check()` gated
   on `wait_for_wifi()` (`wifi_station_state() == PROTOCOL_WIFI_CONNECTED`), while
   the deferral gate `protocol_task_ota_blocked()` is computed from *host-pushed*
   state — a live interrupt or a running progress timer in the widget model. The
   station gets an IP seconds before the link is up and long before any state has
   been replayed, so the guard read a variable that was provably empty at the only
   moment it is consulted.
2. **A link drop discarded the running pomodoro.** The server built a `RuntimeHandle`
   per WebSocket and shut it down on close, so live timer and interrupt state was a
   property of a transport connection rather than of the device.

Fixing either alone would not have made the deferral work: the first gives the guard
a link to wait for, the second gives it a running timer still there to see.

### The fixes

`3f83911` (firmware) adds `protocol_task_owner_state_ready()` — `link.online` **and**
`widget_model_config(...) != NULL` — and a bounded `wait_for_owner_state()` of
`OTA_OWNER_WAIT_MS` = 60 s after the WiFi wait, in both tiers. **The wait fails open
by design:** on timeout the check proceeds anyway, because a device whose owner can
never become ready must stay updatable rather than be stranded by a bad config. Same
reasoning as the local-only image validity gate.

`12e5f4d` (server) makes the runtime per-device and long-lived, with sockets
attaching as replaceable transports pinned by a generation counter. A reconnect runs
`connect()` and replays time, layout, latest fields, active screen and live
interrupts. `connected` in `GET /v1/devices/{id}` now reports socket liveness rather
than runtime existence, so a retained runtime cannot make an unplugged device look
attached; single ownership still refuses a second concurrent owner, now by
`compare_exchange` on the per-device entry.

### What to expect when re-running it

Start a pomodoro server-side, confirm `state: running`, publish a newer version,
reset the board. The check should now stall at `ota=checking` while the timer runs
rather than advancing to `downloading`, and proceed once the timer finishes. The
reboot is the point — it is the only moment a check ever fires, since the interval is
24 h.

Two things worth watching, neither yet observed:

- The bounded wait adds up to 60 s before a check when no owner appears. On a healthy
  networked device the link now lands around 10 s, so the wait should be short.
- With the runtime retained, `GET /v1/devices/{id}` returns a `snapshot` even while
  `connected` is false. That is intended and is an improvement — the timer stays
  visible while the board is offline — but it changes what the field means.

### Carried forward as a V3 item

`device_links` entries are now never removed, so a runtime is retained for every
device that has ever connected, and `MAX_CONCURRENT_LINKS` no longer bounds the
number of retained runtimes. For a single-tenant V2 with one board this is what was
wanted. It needs a lifetime rule before more devices exist.

## V2 Task 11 Step 11 — deferral PASSES on hardware, 2026-08-19

The failure recorded earlier today is closed. Both halves of the step were observed on
the board, on a shipping build, in one continuous run.

Setup: device on `v2.0.0-gate2` (carrying the owner-wait fix), firmware catalog pinned
to `v2.0.0-gate1` so an update was genuinely available in both directions, config
temporarily reduced to a single-entry pomodoro playlist so the card could not rotate
away from a tap. Timer started by a human tap, confirmed `running, 290` server-side,
board hard-reset over USB at 18:08:48 with the timer running.

### It did not begin

| time | ota | uptime | timer |
| --- | --- | --- | --- |
| 18:08:26 (pre-reset) | idle | 15748866 | running 290 |
| 18:09:03 | idle | 13292 | running 253 |
| 18:10:36 | idle | 107299 | running 159 |
| 18:13:20 | idle | 269411 | completed 0 |

`ota` never left `idle` and the version never moved, across the whole five-minute
timer. The morning run was `downloading` by uptime 13136. `idle` rather than `checking`
is what the code does: the owner wait returns `ESP_ERR_INVALID_STATE`, `perform_check`
sets `PROTOCOL_OTA_IDLE`, and `ota_task` enters its deferral poll.

### The timer survived the reboot

`running 290` before the reset, `running 253` after it — counting continuously across a
device reboot that took uptime back to 13 s. The morning run read `idle, 300` at this
point. This is the retained-runtime fix observed on hardware, and it is the half that
made the deferral meaningful: without it there would have been no running timer left
for the guard to see.

### It proceeded once the state cleared

The completion alert is `on-timer-finish` with `hold: until-dismissed`, so
`interrupt_live` stayed true and the deferral correctly held past the timer's end as
well. One dismissing tap released it:

```
20:25:44  DEBUG firmware check  device_id=dev-0003  current=v2.0.0-gate2
20:25:45  INFO  device link closed                  device_id=dev-0003
20:26:11  INFO  device link established             device_id=dev-0003
20:26:14  DEBUG firmware check  device_id=dev-0003  current=v2.0.0-gate1
```

The check fired within a second of the dismissal, the link was suspended for the
download, and the device was back 26 s later on the new image. Afterwards:
`version v2.0.0-gate1`, `ota idle`, `uptime 57356`, `connected true`, `reconnects 3`,
every error counter 0. The panel showed the pomodoro card replayed after the update —
`Focus 00:00`, `ELAPSED 05:00`, `STATUS Done` — matching the server's `completed 0`,
so **the retained runtime survived the firmware update as well as the reboot.**

### Three things to carry forward

1. **An until-dismissed alert postpones firmware updates indefinitely.** Correct
   behaviour — do not update while something is demanding attention — but it means an
   unattended device with an undismissed alert will never update.
2. **`GET /v1/devices/{id}` exposes no interrupt or alert field.** `snapshot` carries
   config, runtime, device, providers, pomodoros, card_data, card_errors, persistence
   and diagnostics, so "is an interrupt live" cannot be read from the wire and had to
   be inferred. Same observability family as the missing OTA failure reason.
3. **The sequence reads as a fault to an observer.** The owner reported the panel
   "starting to reboot", then a disconnected period, then the pomodoro reappearing at
   zero, and took all three for a malfunction. They are, in order: the update takeover
   installing, the deliberate link suspension, and the retained runtime replaying
   `completed` to a freshly booted device.

### Trap found while staging this

**A flashed build is silently reverted by the catalog.** `FirmwareCatalog::check()`
offers its pinned version whenever the device's differs — in either direction — so a
freshly flashed `v2.0.0-gate1` was replaced by `m1-233-g024b1ce` within a minute of
`idf.py flash`. This is by design (`firmware.rs`: "Config, not a database: V2 has
exactly one image live at a time"), but it means a downgrade path exists by design and
that a board session must update the catalog pin, not just flash. Also note
`firmware/version.txt` now pins the version explicitly, because `git describe` served a
stale cached string from a dirty tree — the built binary carried `m1-233-g024b1ce`
while `git describe` said `m1-239-g7de45c2-dirty`.

Evidence: `~/deskmate-hw-sessions/2026-08-19-v2-exit-gate/`, frames
`181106Z-step11-deferred-timer-running`, `181445Z-step11-timer-completed`,
`202735Z-step11-updated-to-gate1`. Note the camera moved between the second and third,
so those two must not be compared as if fixed.

## V2 exit gate — unattended run, 2026-08-19 evening

Run with the owner asleep, so only items needing no hands were attempted. Session
evidence: `~/deskmate-hw-sessions/2026-08-19-v2-exit-gate/`.

### Step 2 baseline (freshly booted networked device, 20:52)

- Image **1,548,256 bytes** (`0x179FE0`), 63% of the app partition free.
- Free heap **8,310,267**, byte-identical across three samples 40 s apart.
- RSSI −37, `ui_queue_high_water` 1, `event_queue_high_water` 0, all error counters 0.

### Gate item 3 — link loss and recovery: server half PASSES

Server stopped 20:53:23; standalone fallback observed 20:54:38 (`00:54`, `Thu, Aug 20`,
"Connect deskmate app" — local time correct for UTC+4, so SNTP holds the clock with no
server); server started 20:55:06; link back 20:55:31, about 25 s.

**"Without a reboot" is proven by uptime continuity, not by the panel** — the two look
identical. The board was reset at 20:51:44 and reported `uptime 224371 ms` at 20:55:31
against 227 s of wall clock, so the counter never restarted.

Two traps for whoever reads these samples. `reconnects` reads **0** after recovery
because that is the freshly started *server* process's counter, not the device's. And
the running timer resets to `idle/300` across a server restart: runtime retention
survives a link drop, not a restart of the process holding the runtime.

**The WiFi half was NOT run.** It needs router access or a hand. The tempting software
substitute was rejected on inspection: Caddy on that host reverse-proxies about
seventeen unrelated services, so stopping the tunnel would have taken the owner's whole
homelab down overnight to test one gate item.

### Gate item 2 — tier switch: NOT RUN, and it costs a device identity

`NetworkConfig` carries `token`, and the server stores only SHA-256 digests, so the
plaintext exists exactly once, at mint time. Returning a device from local tier to
networked therefore requires minting a **new identity**: the board returns under a new
`device_id` with a fresh config and the previous entry is orphaned. That follows from
"secrets are never readable back over the wire" rather than being a defect, but it makes
this the wrong item to attempt unattended — a failure leaves the board with no owner.

### Gate item 6 — 30-minute soak: PARTIAL

20:57:42 → 21:27:55 UTC, 30 min 13 s, 31 samples at 60 s, 15 timelapse frames.

| metric | result |
| --- | --- |
| connectivity | `conn=True` in all 31, never dropped |
| free heap | 28-byte band, 8310703–8310731, no trend, first and last identical |
| `ui_queue_high_water` | 2, constant |
| `event_queue_high_water` | 0, constant |
| malformed / crc / overflow / dropped | 0 throughout |
| uptime | 356472 → 2169118 ms continuous, matching wall clock; no reboot |
| rotation | clock 11 / weather 11 / pomodoro 9 |

**Two qualifications that matter.** The heap is not "flat at the Step 2 baseline" as the
checklist words it: the fresh-boot baseline was 8,310,267 and steady state sits about
456 bytes higher once link and providers are up. There is no leak — no drift in either
direction, and the run ends where it began — but those are different quantities.
And **taps and interrupts were not exercised**, so this is not the mixed soak specified.

### OTA failure reason — works on hardware, and is not yet good enough

Provoked a real remote failure by pinning the catalog to `v2.0.0-nofile`, for which no
image exists, so the URL returns HTTP 404. Board reset 21:33:05:

```
21:33:17  ota=checking  up=9369
21:33:32  ota=failed    up=24584   last_ota_error = "begin: ESP_FAIL"
```

Where this morning's identical failure showed `ota_state: failed` and nothing else —
which is what forced a console reflash of a working board — the cause is now readable
over the wire without a cable.

**But `begin: ESP_FAIL` still hides the 404.** `esp_https_ota_begin()` performs the first
request itself and fails before the download stage, and ESP-IDF collapses the HTTP error
into a bare `ESP_FAIL`, so the most likely remote failure — a missing, moved or
unreadable image — still tells an operator nothing actionable. Follow-up: read
`esp_http_client_get_status_code()` through `http_client_init_cb` so it renders
`begin: HTTP 404`.

Worth stating plainly: this weakness was invisible to a fully green test suite and
surfaced only because the check was carried through to the board.

`v2.0.0-nofile` is now permanently burned as a version string by
`reject_reinstall_of_failed_image()`.

## V2 — the OTA failure-reason commit broke OTA downloads, 2026-08-19 late

**`3f2aa03` is a regression and must not be trusted as it stands.** Every OTA download
from firmware carrying it fails:

```
last_ota_error = "download: ESP_ERR_MBEDTLS_SSL_READ_FAILED"
```

### Evidence that isolates it

| observation | result |
| --- | --- |
| `v2.0.0-gate1` (before `3f2aa03`) downloading `v2.0.0-gate3` | **succeeded**, 21:31 |
| `v2.0.0-gate3` (at `3f2aa03`) downloading `v2.0.0-gate4` | failed, 21:46 and 21:49 |
| `v2.0.0-gate3` downloading `v2.0.0-gate1.bin`, a known-good, previously-installed image | failed, 21:52, same error |
| server serving that file on the LAN | HTTP 200, 1,548,256 bytes |
| server serving it through the public Cloudflare tunnel | HTTP 200, 1,548,256 bytes, 0.43 s |

So it is not the image, not the server, not the tunnel and not the network. The only
firmware change between the working and broken builds is `3f2aa03`.

### Mechanism: NOT established. The first hypothesis was wrong.

The initial suspect was that `3f2aa03`'s custom HTTP event handler (`download_http_event`)
displaced one `esp_https_ota` relies on to read the image body. **That is refuted by the
IDF 5.5.5 sources**, and the refutation should be kept so nobody re-derives it:

- `esp_https_ota_begin()` passes the supplied config straight to
  `esp_http_client_init()`; it never calls `esp_http_client_set_event_handler()`.
- `esp_https_ota_perform()` reads the image with `esp_http_client_read()` directly.
- `http_on_body()` copies the body *first*, then dispatches `HTTP_EVENT_ON_DATA`, and
  ignores the callback's return value.
- `HTTP_EVENT_ERROR` is dispatched only after the transport read has already failed.

So a user event handler cannot displace an OTA body reader, because there is no
event-driven body reader to displace.

The remaining concrete suspect is **internal DRAM**: `3f2aa03` adds ~105 bytes of it
(`s_last_error` plus its `portMUX_TYPE`), and `0ad1a51` already established that this
board starves the AES accelerator's DMA allocations out of internal RAM. That is
plausible and unproven; no second fix was invented for it.

The isolation to `3f2aa03` is empirical and stands on the table above. What does not
stand is any explanation of *why*.

### The handler hypothesis is now refuted by experiment too

A build with the custom handler and all its status-capture machinery removed
(`v2.0.0-gate5`) was flashed and tested at 22:09. **It still fails:**

```
gate5 (handler removed) -> failed   last_ota_error = "download: ESP_FAIL"
```

So removing the handler does not fix the download, matching what the IDF sources already
said. Two hypotheses are now dead: the handler is not the mechanism, and neither is
anything else that removal touched.

Note the reason string got *worse* — `download: ESP_FAIL` instead of
`download: ESP_ERR_MBEDTLS_SSL_READ_FAILED` — because the removed `prefer_transport_error`
machinery was what surfaced the specific mbedTLS error. That machinery is worth keeping
for its diagnostic value; it is not implicated in the regression.

### Where a morning session should start

The regression is somewhere inside `3f2aa03` and has not been bisected. It is a ~159-line
diff to one file, so bisecting it is a bounded job:

1. Build a variant with only the wire/protocol half of `3f2aa03` (the `StatusResponse`
   key 30 plumbing) and none of the `ota.c` instrumentation. If downloads work, the
   cause is in `ota.c`.
2. Then add back the `ota.c` pieces in two halves — the static state
   (`s_last_error` + its `portMUX_TYPE`, ~105 bytes of **internal DRAM**) versus the
   failure-path string formatting.
3. The DRAM half is the standing suspect, because `0ad1a51` established that this board
   starves the AES accelerator's DMA allocations out of internal RAM, and `free_heap`
   reports PSRAM so it cannot show this. `heap_caps_get_free_size(MALLOC_CAP_INTERNAL)`
   around the download would settle it — and if it is the cause, the field's storage
   simply needs to move out of DRAM.

Do not attempt this by reasoning alone; every step needs a flash and a download attempt,
because no host test reaches this path.

### Why this matters beyond the bug

1. **Every gate was green.** Host tests, `idf.py build`, clippy and the full cargo
   workspace all passed, before and after. Nothing in the software suite can reach this
   path; only the board can. Treat "all tests green" on any `ota.c` change as saying
   nothing about whether OTA still works.
2. **The feature that broke OTA is what made the break diagnosable.** Without
   `last_ota_error` this would have read `ota_state: failed` and nothing else — the
   exact blindness the field was added to remove, and it would have cost another console
   reflash to find. The field earns its keep even in the commit that introduced the bug.
3. It also means the earlier hardware result stands: `last_ota_error` genuinely works.
   What must be removed is only the event-handler mechanism behind the
   `begin: HTTP <status>` refinement.

### Board state

Restored to `v2.0.0-gate1` over USB (otadata reset to initial, app written at 0x20000),
because gate1 carries both verified fixes — the owner wait (`3f83911`) and the retained
runtime (`12e5f4d`) — and none of this regression. Confirmed healthy afterwards:
connected, `ota=idle` after a clean check, rotation running, every error counter 0,
heap steady at ~8,310,600. The catalog is pinned to `v2.0.0-gate1` so it stays put.

**Do not ship `3f2aa03` or the version strings `v2.0.0-gate3`/`v2.0.0-gate4` to a board.**
Note `v2.0.0-nofile` and `v2.0.0-gate4` are now burned as version strings on this device
by `reject_reinstall_of_failed_image()`.

## V2 gate item 3 — third failure variant: network path down, 2026-08-20

Run with the owner's explicit authorisation to take the shared reverse proxy down
("используй пока caddy, потом поменяем"), having first refused to do it unasked because
Caddy fronts about twenty of their unrelated containers.

**This is not the WiFi-drop variant.** The radio stays up; what breaks is the path to
the server (Cloudflare → cloudflared → **Caddy**). It is a third, genuinely distinct
failure mode from the server-process stop already recorded, because the failure appears
at a different layer — an upstream that refuses rather than a socket that closes
cleanly. **The true WiFi-drop half still needs router access and remains unrun.**

| event | time (UTC) |
| --- | --- |
| Caddy stopped | 05:18:58 |
| standalone fallback observed | 05:20:34 |
| Caddy restarted | 05:20:40 |
| link re-established | by 05:21:06 |

Outage bounded to **102 s** by making the restart part of the same scripted step, so a
slow operator could not extend someone else's downtime.

- `052034Z-item3b-path-down` — standalone fallback: `09:20`, `Thu, Aug 20`, "Connect
  deskmate app". Local time correct for UTC+4.

**No reboot, again proven by uptime continuity:** 25,579,321 ms at 05:18:44 and
25,722,382 ms at 05:21:06 — a delta of 143 s against 142 s of wall clock. `reconnects`
went 0 → 1, and every error counter stayed 0, including `malformed_frames`, `crc_errors`
and `dropped_responses`. `valid_frames` continued climbing without a gap.

Recovery took about 26 s from the path returning.

### Incidental: a seven-hour clean run

Before this test the board had been up **25,579,321 ms (7.1 hours)** on `v2.0.0-gate1`
with `reconnects: 0`, `valid_frames: 14546` and **every error counter at zero** — no
link drop, no malformed frame, no dropped response, heap steady around 8,310,500. That
is a longer continuous observation than any soak in the plan asks for, and it came free.

## V2 gate item 2 — the non-destructive half PASSES, 2026-08-20

The cable was exercised against a device in networked tier.
`net_config_usb_message_allowed()` permits only `STATUS_REQUEST`, `NETWORK_CONFIG` and
`FACTORY_RESET` there — the cable is the configurator, the server is the owner — and
that is what the board does:

| message over USB | result |
| --- | --- |
| `STATUS_REQUEST` | allowed; reported `tier networked`, `capabilities 0x…cb` (203), wifi connected, ip 192.168.8.168 |
| `TIME_SYNC` | `error: device rejected request (WrongTier): device is owned over the network` |
| `PUSH_DATA` | same typed refusal |
| `ACTIVATE_SCREEN` | same typed refusal |
| `TRIGGER_INTERRUPT` | same typed refusal |

The device was unharmed by the attempts: `valid_frames` kept climbing, `malformed`,
`crc`, `overflow` and `dropped_responses` all stayed 0, and the link stayed online.

**Tooling note:** `deskmate-cli apply-config` could not be used for this. That subcommand
dates from M2 and still expects the old `widgets`/`screens` document, so it rejects a
schema-v4 config locally and never reaches the wire. `APPLY_CONFIG` is in the refused set
by construction and travels the identical gate as `TIME_SYNC`, which was exercised — but
to satisfy the checklist's wording literally, the CLI needs a v4-aware apply.

### The destructive half is blocked on a secret, by design

Going to local tier and back cannot be completed without the **WiFi SSID and PSK**.
`net_store_erase()` calls `nvs_erase_all()`, so a factory reset wipes the credentials
along with everything else, and `NetworkConfig` requires them to provision back. They are
not recoverable from the device — the "secrets are never readable back" rule working as
intended.

Combined with the identity cost already recorded (returning to networked needs a
plaintext token, so a new identity must be minted), **a tier round-trip costs one device
identity and requires the WiFi password supplied out of band.** Neither is a defect; both
make this the most expensive item in the gate.

Checked while looking: the repository contains only `<ssid>`/`<psk>` placeholders. No real
credentials are committed.

## V2 — OTA download regression BISECTED: 105 bytes of static DRAM, 2026-08-20

Root cause found, isolated to a single variable, with a control. **The cause is not the
instrumentation `3f2aa03` added. It is the storage.**

### The experiment

Three builds from the identical pre-`3f2aa03` base, each flashed and asked to download
`v2.0.0-gate2` from the catalog:

| build | what was added | download |
| --- | --- | --- |
| `v2.0.0-bisectC` (**control**) | nothing | **succeeded** — installed gate2 and rebooted into it |
| `v2.0.0-bisectA` | `char[97]` + `portMUX_TYPE` statics, touched under `taskENTER_CRITICAL` | failed |
| `v2.0.0-bisectB` | the same statics, touched without any critical section | failed |

A exonerates nothing on its own; B exonerates the spinlock; C proves the base still
works, so the bisect is valid rather than an environmental drift. What remains is **~105
bytes of `.bss` in internal DRAM**.

Also ruled out earlier, and worth not re-testing: the custom HTTP event handler (refuted
from IDF sources *and* by removing it and still failing), the image, the server, the
tunnel and the network.

### It is layout, not capacity

`idf.py size` on the control build:

```
DIRAM   219307 used  (64.17%)   122453 remain   341760 total
IRAM     16384 used  (100.0%)        0 remain    16384 total
```

There are **122 KB of static DIRAM headroom**, so 105 bytes cannot be exhausting a
budget. The effect must therefore be positional: shifting `.bss` by ~105 bytes moves the
runtime heap's blocks, and something on the TLS/AES path — which `0ad1a51` established
must allocate DMA-capable **internal** RAM — stops finding what it needs. The contiguity
mechanism is inference; what is proven is that the 105 bytes are causal and that total
capacity is not the constraint.

This is the same class as V1's boot crash-loop, recorded above, where "the custom
partition table itself was not the cause, only a contributing memory-layout shift".

Note also **IRAM is 100% full with zero remaining.**

### What this means, beyond one feature

**Any addition to firmware statics can break OTA downloads, unpredictably, with every
test green.** That is the standing hazard, not the `last_ota_error` feature. Before
adding static state to firmware — especially anything reached from `ota.c` — assume the
download path is at risk and verify it on the board.

For `last_ota_error` specifically, the fix is to keep the reason out of internal DRAM:
carry a compact numeric code (stage, `esp_err_t`, HTTP status) and format the string in
the protocol task into the status structure that already lives in PSRAM, rather than
holding a 97-byte buffer in `.bss`. Whether even a few bytes are safe is unknown, which
is itself the point — the margin is not a number anyone currently knows.

The deeper question the owner may want to answer first: *why* is the internal-RAM
situation this brittle, and can headroom be bought back (LVGL buffers, WiFi/TLS
tuning) so that adding a variable stops being a risk?

## V2 gate item 3 — a three-hour outage survived in place, 2026-08-20 (unplanned)

A mains power cut took the homelab down while the board stayed on USB power and its own
battery. This produced a far longer link-loss observation than any test in the plan, and
it passes.

| event | time (UTC) |
| --- | --- |
| last healthy network sample | 05:39:59 |
| server host lost power | some point after that |
| server process started again | 08:50:41 |
| tunnel reachable again (Cloudflare 401, not 530) | 08:52:45 |
| **device link established** | **08:52:46** |

Device state throughout, read over the cable while the link was down: `wifi connected`
(rssi −43, ip 192.168.8.168), **`link standalone`**, `ota idle`, every error counter 0.

**No reboot across ~3¼ hours without an owner**, proven by uptime continuity on the same
build: `up=203612` at 05:39:59 and `up=11992401` afterwards — a delta of 11,788,789 ms
(3 h 16 m) against the same elapsed wall clock. The radio stayed associated the whole
time; only the owner was gone, which is exactly the designed failure direction.

### Two things worth keeping

1. **The device reconnected one second after the path became reachable** — not after the
   server started. `deskmate-server` had been listening since 08:50:41, but Cloudflare
   answered 530 until 08:52:45, and the link came up at 08:52:46. For a networked-tier
   device, "the server is up" means **the tunnel is connected**, not that the process is
   running; the board dials the public URL, never the LAN address. A power-cut recovery
   checklist should verify the tunnel, not just the unit.
2. `reconnects` read **0** afterwards — again the freshly started *server* process's
   counter, not the device's. This is the second time that has looked like a
   contradiction; it is not.

This supersedes nothing already recorded, but it is much stronger evidence than the
engineered 102-second Caddy outage: three hours versus one hundred seconds, and it cost
nothing to obtain.

## V2 — OTA download regression FIXED and verified on the board, 2026-08-20

`5699f1d` removes the regression `3f2aa03` introduced, keeping the feature.

The reason is no longer stored as a string. Stage, `esp_err_t` and HTTP status pack into
a 64-bit logical value held as **two lock-free 32-bit atomic halves**, with the existing
OTA state atomic supplying the generation that makes a torn read detectable. The string
is formatted only on read, into the caller's `protocol_status_response_t` buffer, which
already lives in PSRAM.

**Measured, not estimated:** `nm` on `ota.c.obj` shows `.bss` going **9 → 17 bytes**
(`s_last_error` is exactly 8), against roughly 114 before. Baseline symbols for
comparison are `s_task_starting` 1, `s_state` 4, `s_task` 4.

A `atomic_uint_least64_t` was the obvious shape and is **wrong on this silicon**: GCC
reports 64-bit atomics as not always lock-free, emits `__atomic_load_8`/`__atomic_store_8`,
and ESP-IDF backs those with a **global `portMUX_TYPE`** — which would have reinstated a
lock, and a system-wide one at that. Worth remembering before reaching for a 64-bit
atomic anywhere in this firmware.

### Hardware verification

| check | result |
| --- | --- |
| download an image (`v2.0.0-gate6` → `v2.0.0-gate1`) | **succeeded**, ~55 s, reset 09:06:06 → running gate1 09:07:01 |
| missing image provokes a reported failure | `ota_state failed`, `last_ota_error = "begin: ESP_FAIL"` |
| a successful check clears a stale reason | after reset with a valid catalog: `ota idle`, `last_ota_error = null` |

The three builds immediately before this one could not download at all, so the download
is the decisive evidence rather than the test suite, which was green throughout the
regression.

The `begin: HTTP <status>` refinement stays dropped: it needed a custom event handler on
the download client, and a working update mechanism beats a better diagnostic about the
update mechanism.

**Board left on `v2.0.0-gate6`**, catalog pinned to it, `ota idle`, `last_ota_error null`,
all error counters 0. Version strings burned on this device by
`reject_reinstall_of_failed_image()`: `v2.0.0-nofile`, `v2.0.0-gate4`, `v2.0.0-absent`.

**The standing hazard is unchanged and still applies:** 105 bytes broke this, so any
addition to firmware statics can break OTA downloads with every test green. Verify a real
download on the board after touching firmware statics.

## V2 gate item 1 — the headline demo PASSES, 2026-08-20

USB physically unplugged at ~09:32, Mac companion app not running (checked: no Tauri
process; the only node process is the parallel UI session's vite dev server, which never
opens the port). `/dev/cu.usbmodem*` absent for the duration.

The panel kept updating from the server throughout:

| frame | what it shows |
| --- | --- |
| `093226Z-item1-cable-out-1` | weather card, **30°**, "Partly cloudy" — 31° the day before, so the provider data is live rather than frozen |
| `093323Z-item1-cable-out-2` | rotation advanced |
| `093347Z-item1-cable-out-3` | digital clock reading **13:33:50** with a ticking seconds field and the analog sub-dial — correct to the second for UTC+4 against a 09:33:47Z capture |
| `093411Z-item1-cable-out-4` | rotation advanced again |

Server side across the same window: `connected: true` continuously, `uptime` climbing
1,285,241 → 1,397,265 ms with no discontinuity, `ota idle`, `reconnects` unchanged at 1,
and `malformed`/`crc`/`overflow`/`dropped_responses`/`dropped_events` all 0. Rotation
observed cycling clock → weather → pomodoro in the samples.

So with no cable and no Mac app, the device is owned by the server over WiFi and the
display keeps working — which is the whole point of V2.

**The board has a battery, so this is link independence, not power loss.** Do not record
it as a power test; the true power-loss variant remains a separate thing.

## V2 — the battery is short-lived, which qualifies "unplug is link loss", 2026-08-20

The board ran flat while unplugged and powered itself off. Observed: after the gate item 1
cable pull the board was left on battery, and some hours later it had gone — no USB port,
no reply to `ping 192.168.8.168` from the server host, no link re-established after the
path came back, and a webcam frame that was solid black because the panel was genuinely
off rather than because the virtual camera misfired.

**This qualifies a claim already in the record.** "USB unplug is link loss, not power
loss" is true only over a short window. It held for the minutes of the item 1 demo; it
does not hold for hours. Anything left unplugged long enough becomes a power test whether
or not that was the intent, and the transition is silent — the device simply stops.

Two practical consequences:

- Keep the cable in for long-running work (soaks, overnight observation) unless the point
  *is* to drain it.
- When a board goes unreachable on every channel at once — no serial port, no ping, no
  link — check power before diagnosing anything else. Every other explanation costs more
  time and this one is free to test.

An attempt at gate item 7 (alert replay across a reconnect) was in progress when this
happened and is recorded as **not run**, not as a failure: reconstructing the timeline
afterwards showed the board had already rebooted at ~14:15:12 — before the test window
opened at 14:17:22 — so its premise never held. It is being re-run.

One reading trap worth naming, because it nearly caught me: with the runtime retained, a
`GET /v1/devices/{id}` snapshot is still served while the device is gone, and its `device`
fields are the **last values received**, not current ones. `up=66761` looked like a live
uptime from a freshly booted board; it was a frozen sample from before the link closed.
`connected: false` beside it is the only thing that says so.

> **Closed 2026-08-21: the snapshot now says how old it is, inside `device`.**
> `snapshot.device.observed_age_seconds` is injected next to the values it qualifies, by
> the same mechanism that already puts `last_ota_error` there. The information was not
> strictly absent before — `last_seen_unix_ms` sits at the top level — but as raw epoch
> milliseconds several screens away from the numbers it governs, which is why it was
> missed. A frozen `uptime_ms` beside `observed_age_seconds: 412` cannot be read as live.
> `null` means never heard from, deliberately not `0`, which would read as "just now";
> the subtraction saturates, because `SystemTime` is not monotonic and an NTP step
> backwards would otherwise report an age of half a billion years.
>
> **The sibling trap in the same response is closed the same day, by renaming:
> `counters.reconnects` is now `counters.host_reconnects`.** Every other counter in that
> struct is reported by the device in its `StatusResponse`; that one is the host's own
> tally, incremented when the session re-establishes and replays, so it resets when the
> server process restarts while the device keeps running. Reading 0 beside a device that
> has plainly reconnected is correct, and the 2026-08-19 note below already had to
> explain exactly that in prose. The struct already carried `host_dropped_events`, so the
> prefix is the convention rather than a new one. **Observations recorded before
> 2026-08-21 name it `reconnects`; the number means the same thing.**

## V2 gate item 7 — alert replay across a reconnect PASSES, 2026-08-20

Bounded-hold alert, fired while the link was down, delivered on reconnect. This is the
path that has produced real defects twice, so it is worth the detail.

Setup: pomodoro card, `duration_seconds` 120, alert `on-timer-finish` with a **bounded**
hold of 600 s (not `until-dismissed` — the bounded variant is the one that broke before,
when its countdown started at schedule time rather than delivery). Single-card playlist so
the card could not rotate away from the tap.

| event | time (UTC) |
| --- | --- |
| timer started by tap, confirmed `running/98` server-side | 18:40:56 |
| path broken (Caddy stopped) — link closed | 18:40:58 |
| timer completed **while offline** (98 s later) | ~18:42:34 |
| path restored | 18:43:15 |
| device link re-established | 18:44:06 |

**Evidence of delivery is wire-level, not visual:** the device's `latest interrupt token`,
read over the cable, went **60 → 85**. Before the test it was 60; after the reconnect it
was 85, so an interrupt reached the device that had not before. Uptime was continuous
across the whole window (1,268,941 → 1,346,860 ms), so this was a reconnect and not a
reboot, and every error counter stayed 0.

The path was broken rather than the server stopped, deliberately: restarting the server
process destroys the runtime and the live timer with it, which would test "the server
forgot" instead of "the server remembered while it could not reach the device".

### Two things not resolved

1. **The token jumped by 25, not 1. ROOT-CAUSED AND FIXED 2026-08-21 — it was a host
   defect, not token churn, and no interrupt was ever lost.** `update_device_status` in
   `companion/crates/app-core/src/runtime.rs` advanced the arbiter's interrupt counter to
   `max(latest_interrupt_token, latest_revision, config_revision)` on *every* status
   response. Only the first of those counts interrupts: `latest_revision` is the
   data-push counter (`link_state.c` stores `push->revision`) and `config_revision` is
   the config counter, and both climb with ordinary traffic. So the next interrupt was
   minted at *data revision + 1*. The device's 60 was its last accepted interrupt
   (`interrupt_state.c` stores the host's token verbatim, and only on acceptance); its
   data revision had reached 84 across the session's clock and weather pushes; the alert
   was therefore minted 85. The gap was the distance between two unrelated counters.

   The code's own comment described the revision term as a floor for "an older v1 image
   that omits it", but it was applied unconditionally and forever. The fix gates it on
   `latest_interrupt_token == 0` — a pre-M3 image (key 21 absent decodes to 0) or a
   device that has accepted no interrupt yet, the only cases with no interrupt counter to
   follow. Both branches are pinned by tests in `crates/app-core/tests/runtime.rs`:
   `interrupt_tokens_do_not_follow_the_unrelated_revision_counters` reproduces this exact
   observation in-process — it failed with `left: [85]`, the board's number — and
   `an_absent_interrupt_counter_still_takes_the_revision_floor` keeps the legacy floor.

   Two things this does **not** change: gate item 7 still passes (the counter moved, which
   was always the evidence of delivery), and
   `busy_retry_and_reconnect_replay_reuse_identical_tokens` was never contradicted — the
   token was minted high once, not churned. Host-only Rust, so firmware statics are
   untouched and the OTA download hazard does not apply.
2. **No panel frame exists for this run.** The webcam harness produced solid black on
   every attempt, including with a 60-frame warmup, while the board was demonstrably
   alive (link online, uptime climbing, CLI answering over USB). The physical OBSBOT is
   held by its app and the virtual device returns black, so both paths are dead until the
   app is sorted. This is the harness's own documented failure mode — "the virtual camera
   can return a convincing wrong frame" — and it also weakens an earlier inference in this
   session where a black frame was read as "the panel is off". That conclusion happened to
   be right, but it was carried by the absent serial port and the failed ping, not by the
   frame.

## Reconnect backoff does not widen on the WSS path — observed 2026-08-23

Observed on the physical board while away from the home network, so the device was
failing to reach `deskmate.rodi.one` continuously. Captured by reading the USB serial
line directly (see the second finding below for why the CLI could not be used).

**Task 8's owed exit-gate observation is answered, and the answer is no.** Over ~60
seconds of uninterrupted failure the reconnect interval stayed flat at roughly
0.85–1.2 s with jitter, and never widened:

```
E (126651) esp-tls: couldn't get hostname for :deskmate.rodi.one: getaddrinfo() returns 202
W (126671) net_link: WebSocket TLS/transport failure, status=0, error_type=tcp_transport
I (126681) websocket_client: Reconnect after 1130 ms
...
I (129031) websocket_client: Reconnect after 1090 ms
I (130051) websocket_client: Reconnect after  980 ms
I (132891) websocket_client: Reconnect after  990 ms
I (141141) websocket_client: Reconnect after 1160 ms
```

The gap between successive `net_link: WebSocket disconnected after an error` lines is
~1 s throughout, from uptime 126 s to 142 s.

`net_link.c` does use the backoff module — `s_reconnect_backoff` at :37,
`reconnect_backoff_next_delay_ms()` at :105 — but `:247` passes
`reconnect_backoff_peek_delay_ms()` into the websocket client's `.reconnect_timeout_ms`
**at client construction**. `esp_websocket_client` then runs its own auto-reconnect on
that initial value, so a widening computed later never reaches the thing actually
scheduling the retries. `core/reconnect_backoff.c`'s host tests pass because the module
is correct in isolation; it simply is not governing this path.

Practical consequence: a device that cannot resolve its server retries DNS about once a
second indefinitely. Not diagnosed further here and **not fixed** — recorded so the gate
item is answered by evidence rather than left open.

Also confirmed in the same capture: `wifi_station: WiFi disconnected, reason=201`
(`NO_AP_FOUND`) — the device was not associated at all, it was looking for an SSID that
was not present, which is why every `getaddrinfo` failed.

## A diagnostic build puts console logs on the protocol pipe — observed 2026-08-23

The device was running `v2.0.0-swaes6` (the software-AES experiment build; the repo's
`firmware/version.txt` was pinned to it, and the clean `f6cb0ea` tree pins
`v2.0.0-gate6`). `deskmate-cli status` failed every attempt with
`malformed device response: Cobs`, and a raw read of `/dev/cu.usbmodem3101` showed
ESP-IDF log text rather than framed packets.

`link/usb_link.c` carries the protocol over **`usb_serial_jtag`**, and
`sdkconfig.defaults` deliberately keeps the console on UART0
(`CONFIG_ESP_CONSOLE_UART_DEFAULT=y`, `CONFIG_ESP_CONSOLE_SECONDARY_NONE=y`). The diag
build evidently routes the console onto the USB-Serial-JTAG instead, so logs and protocol
frames interleave on one pipe and COBS framing cannot survive.

This violates the repo's own standing rule — keep diagnostic logs out of the machine
protocol byte stream.

**Corrected later the same day:** the impact is narrower than first written. Once the
device came back online and stopped logging, `deskmate-cli status` decoded cleanly
against the *same* `v2.0.0-swaes6` build. So the collision only corrupts framing **while
the device is actively emitting log lines** — at the time of the first observation it was
failing DNS roughly once a second, so every frame was interleaved with error output. A
quiet diag build is usable over the cable; a diag build in a fault loop is not, which is
precisely when you most need to reach it. Treat it as a hazard, not an absolute block.

## Task 14 minimal OTA gate — INCONCLUSIVE on hotel WiFi, 2026-08-23

Stage 1's hardware gate was reduced by owner direction to a single check: does a build
carrying the asset store's static footprint still download and install over the air.

**Partially answered yes, incidentally.** `v2.0.0-gate6` (= `f6cb0ea`, which contains
Task 9 and therefore the full `.bss` growth) was downloaded and installed over the
network by this device earlier the same day, twice. So the asset store's static footprint
does not by itself break OTA.

**The fix-wave delta is unproven.** `v2.0.0-gate11` was built from `fe1356f` (all four
whole-branch fixes), published, and pinned. Two attempts, both `ota_state: failed`, device
remained on `gate6`.

**Not recorded as a regression, because the evidence points at RF:**

- `.bss` at `gate11` is **102,608 — byte-identical to `gate6`**, which downloaded fine.
  The fix wave's new `static asset_release_fn s_release` landed inside existing linker
  alignment padding, which also resolves the "unexplained slack" noted earlier.
- WiFi RSSI was **-72 dBm** when `gate6` downloaded successfully, **-75** at the first
  `gate11` attempt and **-93** at the second. -93 dBm is borderline unusable for a
  sustained 1.5 MB TLS transfer.
- The server logged the firmware *check* for both attempts but **no download request at
  all**, consistent with failing before or during connection setup rather than a
  mid-transfer abort.

Retest on a stable network before drawing any conclusion. Do **not** bisect on this
evidence.

### Real observability defect found regardless of cause

`ota_state` read `failed` while `last_ota_error` was **absent** on both attempts. That
field exists precisely so an OTA failure is diagnosable — it is what made `3f2aa03`'s own
regression findable. Only two code paths set `PROTOCOL_OTA_FAILED` (`ota.c:224`, `:236`)
and both pack a stage, so a `FAILED` state with `OTA_FAILURE_STAGE_NONE` should not be
reachable. One candidate worth checking: `clear_last_error()` (`ota.c:215-218`) stores a
zero packed value while preserving the *current* state, so if it runs while the state is
already `FAILED` it produces exactly this — failed with no reason. Not diagnosed further.

The catalog was reverted to `v2.0.0-gate6` so the device does not retry a failing 1.5 MB
download daily over a weak link. `v2.0.0-gate11.bin` remains published and ready to pin
when the retest happens.

## The board's identity is `dev-0005`, and the registry is the only source for it — 2026-08-23

Configuring the Mac app against the server failed twice in a row this morning for two
unrelated reasons. Both are recorded because both cost real time and neither is
discoverable from the app's own error text.

### The current device identity is `dev-0005`

This file said `dev-0003` (see the 2026-08-18 entry), and it was three identities stale.
The live registry at `/var/lib/deskmate/configs/device-identities.json` holds
`dev-0001` … `dev-0005` with `next_sequence: 5`, and the server log shows the board
dialling in continuously as `dev-0005`:

```
09:39:18  INFO server::device_link: device link established  device_id=dev-0005
```

Every re-pair mints a new identity — that is the "a tier round-trip costs a device
identity" rule in CLAUDE.md, seen from the other side. **Read the registry, not this
file, when you need the board's current id.** A prose note in a document cannot track a
value that changes whenever someone re-pairs.

Pushing a config to a stale-but-minted id **succeeds and reports success**. `PUT
/v1/devices/{id}/config` accepts a config for any minted identity whether or not a device
is attached, because a device must be configurable while unplugged; nothing in the
protocol distinguishes "stored for later" from "delivered". The app's confirmation was
truthful — the save reached the server — and the panel simply never changed.

The reliable tell is `GET /v1/devices/{id}`: **`connected: false` together with
`last_seen_unix_ms: null` and no `snapshot` means the identity has never been used at
all.** Since `12e5f4d` made runtimes per-device and long-lived, any identity that has ever
connected retains a runtime and still returns a snapshot while disconnected. Absent
snapshot is therefore a much stronger signal than a false `connected`.

Orphans left in place, not pruned: `dev-0003.json` and `dev-0004.json` configs, plus the
dead `dev-0001`/`dev-0002` identities. Deleting an identity is irreversible — only digests
are stored — so this needs an explicit decision rather than a tidy-up.

### The deployed server was a schema version behind the app

Before the id problem was visible, every save was rejected with:

```json
{"path":"schema_version","code":"unsupported-version",
 "message":"schema version 4 is not supported; expected 5"}
```

The app and the server carry **independent** copies of `CURRENT_SCHEMA_VERSION`. Schema v5
landed 2026-08-22 in `0c7c467`; the deployed binary was built 2026-08-21 13:23. Redeploy
the server whenever the schema moves — the mismatch surfaces only as a validation failure
on the first save, which reads like a config problem rather than a deploy problem.

Redeploy notes worth keeping: reach the VM over **Tailscale** (`docker-vm`,
100.93.166.123), because `~/.ssh/config` pins its LAN address `192.168.8.20`, which is
unreachable from any other network. The build container must match
`companion/rust-toolchain.toml` (**`rust:1.98-bookworm`** since the 1.98.0 pin), and the
previous deploy leaves a root-owned `companion/target/` — keep it and replace only the
sources, which turns a cold build into ~40 seconds. Verify without writing anything by
PUTting a deliberately old-versioned body and reading which version the server names back.

### Unrelated observation, not diagnosed

`DESKMATE_SERVER_BIND=192.168.8.20:8443` — a LAN interface, not the loopback the runbook
specifies. The tunnel is the only thing that needs to reach the server, so this exposes the
admin surface, guarded by one bearer token, to the whole LAN. Also `systemctl` reported
`Consumed 46min 24.911s CPU time` across roughly two days with no device attached for most
of it. Neither was changed.

## Scene renderer OTA + boot — verified 2026-08-25

First hardware run of the scene-renderer branch (`v2.0.0-scene1`, built at
`55d1a97`). **The OTA download works with the moved `.bss`.** That was the one
check the branch could not close on software grounds: `.bss` went 102,608 →
102,624 (+16, `scene_view.c`'s three existing pointers becoming reachable once
Task 10 linked the file), and this board has twice lost days to a memory-layout
shift breaking OTA outright with every test green (`3f2aa03`).

Observed, `dev-0005` over `deskmate.rodi.one`:

- **Attempt 1 FAILED** — `ota_state: failed`, `last_ota_error: "download: ESP_FAIL"`,
  device rolled back and stayed on `v2.0.0-gate6`. RSSI -77.
- **Attempt 2 SUCCEEDED** — identical image, same location, same signal band
  (-75..-82). Downloaded, installed, rebooted onto `v2.0.0-scene1`, and marked
  itself valid (uptime passed the rollback window and stayed on the new slot).

**Ruling: the first failure was transient, not the `.bss` hazard.** The
layout-shift failure mode documented for `3f2aa03` is *deterministic* — three
builds from an identical base decided it, and the bad one failed every time,
"whether or not a critical section touches them". A retry of the identical
image succeeding is not that signature. Same inconclusive-then-fine pattern as
the `gate11` attempt on 2026-08-23. Ruled out by direct test, not inference:
the image fetched clean over the public URL from the Mac (HTTP 200, 1,599,376
bytes, md5 `768e1351…` matching the local build, 21 MB/s), and `ota_0`/`ota_1`
are 4 MB against a 1.6 MB image.

Also confirmed on the new firmware: `capabilities` carries bit 8
(`unknown_capability_bits: 0x0000000000000100` — the DEPLOYED SERVER PREDATES
this branch and has no name for it, which is why it reads as unknown rather
than as `scene-render`; the device is advertising 491 correctly). Protocol v1,
tier networked, rotation 270, `active_screen_id: clock`. Counters clean after
~2 minutes: 0 `crc_errors`, 0 `malformed_frames`, 0 `overflow_frames`, 75
`valid_frames`, `ui_queue_high_water` 1.

### Two operational findings from this session

1. **The device checks for firmware exactly twice a day and cannot be asked.**
   `ota.c:43` sets `OTA_CHECK_INTERVAL_MS` to 86,400,000 (24 h, ±1 h jitter),
   and `ota_task` checks once immediately at boot. The firmware *has* a
   force-check path — `ota_check_now()` (`ota.c:809`), reached by a request
   carrying `OTA_CHECK_STATUS_REQUEST_ID` (`protocol_task.c:1468`; the sentinel was
   removed in the 2026-09-05 cleanup because nothing ever sent it) — but
   **nothing host-side sends it**: the server has only three admin routes
   (create device, get device, put config), and neither the Mac app nor the
   `device`/`protocol` crates reference it. So publishing an image and waiting
   does nothing; a power cycle is the only way to trigger an update. Note a USB
   unplug will NOT do it — the board has a battery, so that is link loss, not
   power loss.
2. **Nothing reports free INTERNAL DRAM, which is the one number that would
   diagnose this failure class.** `free_heap` reads ~8.3 MB, which is PSRAM and
   already recorded as useless as a TLS health signal on this board. When an OTA
   download fails, there is currently no way to distinguish "internal DRAM
   exhausted on the TLS/AES path" from RF without bisecting builds.

### Not verified

The scene renderer itself has **not** been exercised on the panel. All four of
the asset-GC teardown observations require pushing a scene with an asset font,
and **the server cannot push a scene** — no file under
`companion/crates/server/src/` references `PushScene`, `Scene` or
`encode_scene_payload`, and `build_digital_clock_scene`'s only caller is the
parity test. The device can receive and render one; nothing sends one yet.

## The scene renderer draws on the panel — verified 2026-08-25

**First time a host-built scene has ever rendered on the physical display.** The
server pushed `build_digital_clock_scene`'s `DigitalClock` scene to `dev-0005`
over `deskmate.rodi.one` via the new `POST /v1/devices/{id}/scene`, on
`v2.0.0-scene1` in networked tier — the release image, over the shipping path,
with no flash and no tier change.

**Observed: a ticking seconds field, and no visual artifacts.** That is
conclusive on its own. The saved config carries `show_seconds: false` (confirmed
in the server's snapshot for the clock card), so the shipped C `digital_clock`
template cannot draw seconds. Seconds on the panel can only have come from the
pushed scene, which means `scene_decode()` → `scene_model_validate()` →
`scene_view.c` → LVGL all ran correctly on the device against real baked fonts.

### Not verified — do not describe these as observed

- **The date line was not confirmed.** The scene pinned `local_now` to
  `2019-07-04`, so a correct render reads `Thu, Jul 4`; the observer did not
  report the date either way. The date is the host-computed static, so confirming
  it would have pinned `BakedFontMetrics::measure()` and the date box on hardware.
- **Only 270 degrees was exercised.** The device's saved rotation is 270. 90 was
  never pushed, so nothing here speaks to the other mount.
- **No pixel claim of any kind.** This was an eyeball observation over a webcam-less
  session. Byte-exact interpreter parity is Task 11a's harness, which is written
  and still unrun.

### The defect this found: a late reply fails the NEXT request

Pushing every 3 s produced 7 acks and then 12 straight failures; pushing every
20 s produced this:

```
07:37:27 rev 100: HTTP 200
07:38:09 rev 102: HTTP 502 -- response request ID mismatch (expected 472, received 471)
07:38:32 rev 103: HTTP 502 -- response request ID mismatch (expected 485, received 478)
07:39:16 rev 105: HTTP 502 -- device request timed out
```

Root-caused to `SocketPeer`'s read loop taking the pending waiter **before**
comparing request ids, so a late reply to an abandoned request failed whichever
request was in flight — self-sustaining until traffic stopped. The drift from one
behind to seven behind is the cascade widening. Fixed in `95ed9eb`; the same
shape was latent in `DeviceClient` and `DeviceSession`, i.e. over the cable too.

**The device was never at fault.** `malformed_frames: 0`, `overflow_frames: 0`,
`ui_queue_high_water: 2`, no reboot across 2 h of uptime, and full recovery every
time traffic stopped. Two device-side facts are worth carrying forward anyway:

1. `dropped_responses` reached 11, and it increments only when
   `protocol_message_encode` fails or `write_frame` misses
   **`PROTOCOL_WRITE_TIMEOUT_MS`, which is 200 ms** (`protocol_task.c:53`). An Ack
   is far too small to fail encoding, so the device genuinely could not write
   those replies inside 200 ms. Whether that is marginal RF (RSSI -77 here) or
   internal-DRAM pressure on the TLS/AES path is **undetermined** — see below.
   The correlation fix stops the cascade; it does not make the device faster.
2. **The missing internal-DRAM number blocked this diagnosis, for the second time
   in 24 hours.** The 2026-08-25 OTA entry already recorded that nothing reports
   free internal DRAM and that it is "precisely the number that would separate
   this failure class from RF without bisecting builds". `free_heap` reads ~8.3 MB
   of PSRAM and says nothing. Adding it to `StatusResponse` is additive and cheap.

Operational notes for the next session: the active playlist rotates every **50 s**
(`default_dwell_seconds`), and each tick sends `ActivateScreen`, which puts a
template view back over a pushed scene — so a scene must be re-asserted inside that
window, or the playlist paused (`preferences.paused` gates the device send, though
rotation still advances locally). `card_id` need not name a configured card:
`dispatch_push_scene` uses it only to scope `field.`/`timer.` binding lookups.

### Reliability after the correlation fix — 2026-08-26

Re-measured with the fixed server deployed (`6a51dc2`), same device, same link, same
scene: **20 of 20 pushes accepted at 5 s intervals, `dropped_responses` 0.** Compare
the pre-fix runs: 7 of 19 at 3 s, and 4 of 9 at 20 s. The host-side cascade was the
entire failure.

**The 200 ms write budget is therefore an unproven suspect, and is deliberately NOT being
changed.** It remains a genuine asymmetry worth knowing about: `net_link_write_frame`
gives `esp_websocket_client_send_bin` `PROTOCOL_WRITE_TIMEOUT_MS` = 200 ms
(`protocol_task.c:53`) to deliver a reply the host will wait a full 2000 ms for, so any
transient TLS stall or websocket-client lock contention destroys a reply the host would
still have accepted. The clean fix, if it is ever needed, is a per-transport write budget
beside `link_transport_t`'s existing `link_timeout_ms`, leaving USB at 200 ms. It is not
being made now because a firmware change costs an OTA-download re-verification, which on
this board needs a physical power cycle (the device checks twice a day, cannot be asked,
and the battery means a USB unplug is not a power loss) — and there is currently no
evidence the timeout ever bites at realistic cadence. The 11 drops seen on 2026-08-25
occurred during a 3 s burst that was also mid-cascade, so they cannot be attributed to
the timeout alone.

Note the device rebooted between the two sessions (uptime 2 h 08 m → 1 h 00 m), so the
2026-08-26 counters are fresh rather than cumulative.

## Scene Label + RotRect software layout — hardware check owed 2026-08-26

Task 1b added scene kinds 8 (`Label`) and 9 (`RotRect`) without adding any
file-scope storage. The release-config `idf.py build`/`idf.py size` comparison is
byte-flat in every memory-layout figure that has mattered to the OTA hazard:

- `.bss`: **102,624 → 102,624 bytes** (delta 0)
- `.data`: **23,128 → 23,128 bytes** (delta 0)
- IRAM: **16,384/16,384 → 16,384/16,384 bytes**, 0 remaining (delta 0)

Flash grew, as expected for decoder/renderer code: image-size accounting moved
1,599,259 → 1,603,703 bytes, while `deskmate.bin` moved 0x186790 → 0x1878f0.
No OTA download or panel observation was performed here; the plan assigns that
single power-cycle/download check to the owner, and unchanged static figures do
not waive it.

### DIRAM `.text` checked too, and it is flat — 2026-08-26

Following up the entry above, because `.bss` is not the whole hazard.
`idf.py size` splits DIRAM into three components, and **DIRAM `.text` — code
resident in internal RAM — is carved from the same 341,760-byte pool as `.bss`**,
so it reduces the runtime heap in exactly the way static data does. `3f2aa03` was
about internal DRAM available to the heap, not about `.bss` specifically.

Measured on the same tree, before and after Task 1b:

- DIRAM `.text`: **93,635 → 93,635 bytes** (delta 0), total DIRAM 219,387 both times.

`deskmate.map` attributes every byte of the new code to flash-mapped text
(`0x420…`), with **zero** DIRAM/IRAM text from `protocol_message.c.obj`,
`scene_decode.c.obj`, `scene_view.c.obj` and `scene_model.c.obj`:
`rotation_for_time` at `0x42012124`, the inlined label and rotated-rect builders
inside `build_nodes` at `0x420121c4`, the encoder arms inside
`protocol_message_encode` at `0x42019a48`, the decoder arms inside
`scene_decode_map` at `0x4201c1a0`, and `scene_model_parse_rotation_binding` at
`0x4201d624`. No new function reached internal RAM through `IRAM_ATTR`, through
inlining from an IRAM-resident caller, or through a linker-fragment rule.

**A stale figure caused a false alarm here, which is worth not repeating.**
`CLAUDE.md` recorded total DIRAM as 219,307 at `5699f1d`; it now reads 219,387.
Task 10 of the previous plan explains +16 of that. The remaining ~64 bytes
accumulated between those commits and are **not** attributable to the scene
nodes. A fresh total compared against a figure captured at a different commit is
not evidence about a recent change — only a before/after on the same tree is.

### Horizontal-anchor follow-up before the owed download — 2026-08-26

Before the unchecked Task 1b Step 9 was run, `Label` gained optional horizontal
anchoring so BigNumberLabel's content-sized pill can be centred by LVGL rather
than measured by the host. No board check was performed here; this is intended
to ride the same not-yet-tested image and the hardware item remains owed.

The release-config same-tree comparison is still byte-flat in all four internal
memory figures: `.bss` **102,624 → 102,624**, `.data` **23,128 → 23,128**, IRAM
**16,384 → 16,384**, and DIRAM `.text` **93,635 → 93,635**. Flash code and total
image size each grew by 144 bytes; `deskmate.bin` moved `0x1878f0 → 0x187980`.

## Stage 2b OTA check — PASSED 2026-08-26

`v2.0.0-scene2`, the image carrying scene node kinds 8 (`Label`) and 9
(`RotRect`) plus the label's horizontal anchor, downloaded and installed on
`dev-0005` over `deskmate.rodi.one` on the first attempt. Observed immediately
after: `firmware_version: v2.0.0-scene2`, `uptime_ms: 110116`, `ota_state: idle`,
`last_ota_error: null`, link connected. It survived the rollback window on the
new slot, the same evidence standard the 2026-08-25 entry used.

**This is the whole hardware cost of stage 2b.** Every remaining task in
`docs/superpowers/plans/2026-08-26-deskmate-scene-templates.md` is host-only.

Two things this run confirms beyond the download itself:

1. **Flat internal RAM predicted a clean OTA, for the first time deliberately.**
   `.bss` 102,624, DIRAM `.text` 93,635, `.data` 23,128 and IRAM 16,384 were all
   byte-flat across the change, verified before and after on the same tree. The
   two failures this repository has paid for — the V1 boot crash-loop and
   `3f2aa03` — were both internal-DRAM layout shifts. Holding all four figures
   flat and then seeing a first-attempt download is the first time that
   relationship has been used as a prediction rather than read backwards from a
   failure. It is one data point, not a law: keep running the check.
2. **A power cycle alone proves nothing if nothing is published.** An earlier
   cycle the same day found no update, because `DESKMATE_FIRMWARE_VERSION` still
   named the version the device was already running. The device checked, matched,
   and correctly did nothing. Publishing is three steps and all three are
   required: put `<version>.bin` in `$DESKMATE_FIRMWARE_DIR`, set
   `DESKMATE_FIRMWARE_VERSION` to that string, restart the server. Then power
   cycle — the device checks once at boot and twice a day, cannot be asked, and a
   USB unplug is link loss rather than power loss because the board has a battery.

## The server was pinning a CPU core, and had been for days — fixed 2026-08-26

Reported by the owner. The deployed server was at **100% of one core**, memory flat
at 6-11 MB across every instance, so a spin rather than a leak. systemd's
per-instance accounting shows how long it had been going on:

| Instance started | Wall | CPU | Duty |
| --- | --- | --- | --- |
| Aug 24 07:59 | 21h07m | 20h08m41s | **95%** |
| Aug 25 05:06 | 13h40m | 11h20m11s | 83% |
| Aug 25 18:46 | 13h13m | 10h28m34s | 79% |
| Aug 26 10:37 | 7h26m | 5h49m47s | 78% |

**This entry corrects an earlier one.** The 2026-08-25 note recorded
"`Consumed 46min 24.911s CPU time` across roughly two days with no device attached
for most of it" and treated it as an aside. That was 46 minutes over **2h24m** —
a 32% duty cycle, not a rounding error. The observation that should have caught
this filed it as trivia. When a note quotes a cumulative CPU figure, quote the
wall time beside it or the number means nothing.

### Root cause

`run_scheduled_work` returned early when `!state.connected`, **above** the calls to
`Scheduler::status_due()` and `time_sync_due()` — and those calls are what advance
the deadlines, through `take_deadline`'s side effect. So while the device was away,
both deadlines stayed permanently in the past. `wait_duration()` takes `.min()`
across every deadline and calls `saturating_duration_since(now)`, which returns
**zero** for a past instant, so `command_receiver.recv_timeout(0)` returned
instantly, forever. The paused early-return had the same shape, and provider
deadlines could sit in the past while a previous job was in flight.

Fixed in `e06d6ab`: a deadline is consumed whenever a tick examines it, before any
gate decides whether the I/O happens. `wait_duration` was deliberately **not**
clamped to a minimum sleep — that would mask a past-due deadline instead of
resolving it, and hide the next bug of this shape.

The naive fix regresses something worth keeping: `next_status` sitting in the past
is also what makes a status fire promptly on reconnect, and this device drops its
link on idle timeout routinely. The deadlines are re-armed at the connect
transition instead, so a reconnect still refreshes promptly — which a dropped link
needs, since the device may have rebooted and its clock drifted while away.

### Why it looked intermittent

**A server that has never seen a device does not spin.** Runtimes are per-device
and long-lived, so with no device attached there is no worker loop at all. The
spin needs a runtime that exists but has no live socket — created on connect, then
stranded on disconnect. A fresh restart therefore always looks healthy, and
degrades only once a device has connected and gone away.

### Verification status: NOT yet proven on the deployment

The fix is deployed and the server sits at 0.0% — but **that proves nothing on its
own**, because the device has been offline since 13:50 and no runtime has been
created in this instance. The honest check is a full cycle: let the device
connect, let the link drop, and confirm CPU stays flat afterwards. Until that is
observed, this is fixed in test and unproven in the field.

## Stage 2b OTA check — PASSED 2026-08-27, `v2.0.0-scene3`

One image carrying **four** firmware needs downloaded, installed and rebooted on `dev-0005`
over `deskmate.rodi.one`. Owner-reported: the panel showed no artifacts.

One image carries all four firmware needs stage 2b surfaced, because it was never
published between them:

| Need | Wire | Unblocks |
| --- | --- | --- |
| `SceneArc.opacity` | key 9, omission = `LV_OPA_COVER` | all 8 `ProgressRing` parity rows |
| external rot-rect pivot | no new key; a relaxed, axis-wise canvas bound | all 8 `AnalogClock` rows |
| `SceneRect.clip` | key 7 | STORM's 14 clipped pixels |
| `SceneRotRect.clip` | key 10 | `AnalogClock`'s 8 tick-edge pixels |

Protocol stays v1 and additive, `PROTOCOL_CURRENT_CAPABILITIES` stays 491, schema stays v5.

**Internal RAM is byte-flat across all of it**, on a same-tree before/after taken at
`09404ca` and re-measured after each firmware commit — `.bss` **102,624 → 102,624**,
DIRAM `.text` **93,635 → 93,635**, `.data` **23,128 → 23,128**, IRAM
**16,384/16,384 → 16,384/16,384 with 0 remaining**, DIRAM total **219,387 → 219,387**.
Only flash `.text` moved, **1,140,180 → 1,141,824**, and that is off-chip. Flat internal
figures have predicted a clean download exactly once (2026-08-26), which is one data
point and not a law — hence this check.

### What was published, and verified

`deskmate.bin` 1,605,600 bytes, md5 `4aa60a714d841a7e504cfcf16ee04cf7`, published as
`v2.0.0-scene3`. All three required steps were done and each was checked rather than
assumed:

1. The image is in `/var/lib/deskmate/firmware/v2.0.0-scene3.bin`, owned by
   `deskmate-server`, and `md5sum` on the VM matches the local build.
2. `DESKMATE_FIRMWARE_VERSION` in `/etc/deskmate/server.env` reads `v2.0.0-scene3`.
   `firmware/version.txt` was moved to match in `26e4246` — required, because the catalog
   offers its version in **either** direction, so a device left on a different string is
   offered a change within the minute.
3. `deskmate-server.service` was restarted and is active.

Then end-to-end through the public tunnel, not just on the VM:
`GET https://deskmate.rodi.one/v1/firmware/v2.0.0-scene3.bin` returned **200**,
1,605,600 bytes in 0.9 s, md5 `4aa60a714d841a7e504cfcf16ee04cf7` — byte-identical to the
local build. So the image a device would fetch is provably the image that was built.

**Reaching the VM needs Tailscale (`100.93.166.123`), not the `~/.ssh/config` host**,
whose pinned LAN address `192.168.8.20` times out from any other network. That cost a
round trip here and will again.

`GET /v1/devices/dev-0005` immediately after the restart: `connected: false`,
`snapshot: null`. The retained runtime does not survive a server restart, so this says
nothing about the board's health — only that the link is down, which is expected while
the board is off or asleep.

### The result

Owner power-cycled; the device downloaded and installed. `GET /v1/devices/dev-0005`
immediately after, quoted rather than paraphrased:

```
firmware_version   v2.0.0-scene3      ota_state          idle
last_ota_error     null               last_network_error null
connected          true               wifi_state         connected
uptime_ms          69670              wifi_rssi          -76 dBm
rotation           270                tier               networked
free_heap          8299971            protocol_version   1
```

Every counter clean: `crc_errors` 0, `malformed_frames` 0, `overflow_frames` 0,
`dropped_events` 0, `dropped_responses` 0, `dropped_ui_commands` 0, `rx_dropped_bytes` 0,
`valid_frames` 43, `event_queue_high_water` 1, `ui_queue_high_water` 1. `host_reconnects`
is 1, which is the server restart during publishing, not a link fault.

**This is the whole hardware cost of stage 2b, and it bought four firmware needs**:
`SceneArc.opacity`, the external rot-rect pivot, `SceneRect.clip` and `SceneRotRect.clip`.
Deferring each one as it was found rather than fixing it in place is what collapsed four
verifications into one.

**Flat internal RAM has now predicted a clean download twice** (2026-08-26, 2026-08-27).
That is two data points, not a law — the failure mode it guards against is *deterministic*,
so a pass says the layout did not move somewhere fatal, not that it never can. Keep running
the check.

Note `free_heap` reads ~8 MB. That is PSRAM and is **not** a TLS health signal on this
board.

**What this does NOT prove.** The download and the absence of artifacts, nothing more. No
scene was pushed, so the four new node capabilities were not exercised on the panel — arc
opacity, an external pivot, and either clip have never drawn on hardware. The parity gate
that proves them is simulator-to-simulator, and real 270° geometry is only ever provable by
looking at the panel. Stage 3 owes that observation.

### Two things learned while publishing

**The server binary was deliberately not rebuilt**, and did not need to be: nothing in
stage 2b changed the server, and the check exercises the firmware catalog and the device's
updater, neither of which moved. `POST /v1/devices/{id}/scene` is present in the deployed
binary — confirmed by probing it against controls (422 on the scene route, 405 on a
known-present route, 404 on a bogus one).

**A `strings`-based conclusion about that binary was wrong, and the controls caught it.**
`strings` is not installed on the VM, so `strings … | grep -c` returned 0 for every pattern
including ones certain to be present. A zero from a tool that is not there looks exactly
like a zero from a tool that ran. Calibrate a negative against a positive control before
believing it.

### A defect this check surfaced, unrelated to the OTA

The same query reports `unknown_capability_bits: 0x0000000000000100` — bit 8,
`CAPABILITY_SCENE_RENDER` — and lists only six named capabilities. The device correctly
advertises 491; **the host cannot name bit 8**. This is not stale deployment:
`companion/crates/protocol` defines and exports `CAPABILITY_SCENE_RENDER`, but
`app-core`'s `DeviceCapability` enum has eight variants and never learned it, so `bit()`,
`label()`, `from_bits()` and `known_bits()` all omit it. A freshly built server reports the
same. The consequence is exactly what that enum exists to prevent — its own doc comment
says a raw bitmask "tells the user nothing about what to change or which firmware to
install", and every scene-capable device now trips that path.

## Stage 3a Gate A — OTA download failed once, then PASSED on retry, 2026-08-28

`v2.0.0-live1` was published and offered; the board power-cycled and **did not install
it**. It remains on `v2.0.0-scene3`.

```
firmware_version  v2.0.0-scene3     ota_state         failed
last_ota_error    download: ESP_FAIL
wifi_state        connected         wifi_rssi         -76 dBm
uptime_ms         45828             tier              networked
```

Counters otherwise clean: `crc_errors` 0, `malformed_frames` 0, `overflow_frames` 0,
`rx_dropped_bytes` 0, `valid_frames` 35. `dropped_responses` is **1**, and
`host_reconnects` 1.

### Where the failure actually is, read from `ota.c` rather than guessed

`last_ota_error` is `download: ESP_FAIL` — stage DOWNLOAD, `esp_err_t` `ESP_FAIL`. Only
one path produces that combination: `firmware/main/link/ota.c:669`, reached after the
`esp_https_ota_perform()` loop exits with a non-`ESP_OK`, non-`IN_PROGRESS` result. That
places the failure **inside the body transfer**, and it means everything before it
succeeded:

- `esp_https_ota_begin()` returned `ESP_OK` — otherwise the stage would read `begin`.
  **So TLS was set up.** That is the step the internal-DMA/AES memory hazard breaks
  (`62e5aea`, `0ad1a51`), and it did not break here.
- `esp_https_ota_get_status_code()` was **200** — otherwise `set_http_failure` would have
  recorded an HTTP status instead of an `esp_err_t`.
- The image size passed the partition bound, and `esp_https_ota_get_img_desc()` both
  succeeded and **version-matched** — otherwise the stage would read `verify`.

So the device reached the server, was served the right image, read its descriptor, and
then lost the transfer partway through.

**This is not the documented memory-layout signature.** That failure mode kills
`mbedtls_ssl_setup()` before any socket work, and would surface as stage `begin`. The
same-tree measurement supports that reading: `.bss` 102,624, DIRAM `.text` 93,635,
`.data` 23,128, IRAM 16,384/16,384 and DIRAM total 219,387 are all **byte-flat** against
the `05500ef` baseline, across all five firmware tasks. Only flash `.text` moved
(1,141,824 → 1,145,532).

The server log corroborates the sequence and its speed:

```
05:15:17  device link established
05:15:21  firmware check  current=v2.0.0-scene3
05:15:23  device link closed        <- install_update() suspends the WSS link
05:15:25  device link established   <- resumed on the failure path
```

Two seconds from link-suspend to link-resume. A 1.6 MB body over this link cannot
complete in that window, so the transfer died early rather than timing out —
`ota_policy_download_timed_out` would have recorded `ESP_ERR_TIMEOUT`, not `ESP_FAIL`.

### A methodological note, because it cost two wrong turns

Neither the server journal nor the Caddy container log showed any request for
`/v1/firmware/v2.0.0-live1.bin`, which looked like strong evidence the device never asked
for it. **It was not evidence at all**: a control request issued by hand returned HTTP
200 and *also* produced zero log lines, so that path simply is not logged. This is the
second time in two days that a cheap negative has been misleading here — the first was
`strings` not being installed on the VM. Calibrate a negative against a positive control
before believing it.

### Precedent, and what is owed

There is a direct precedent: the 2026-08-25 stage-2a check failed once and **succeeded on
a retry of the identical image at the same signal**, and was recorded as transient
precisely because the layout-shift mode is deterministic. `-76 dBm` is the same weak link
board-notes already describes as "download daily over a weak link".

So the next step is a **retry of the identical image**, which is decisive either way:

- succeeds → transient, and the reading above holds;
- fails identically → deterministic, and it must be bisected on the board across builds
  from an identical base, as `3f2aa03` was, rather than retried further.

The device checks at boot and twice a day and cannot be asked, so a retry means another
power cycle. Nothing was republished between attempts: the catalog still offers
`v2.0.0-live1`, md5 `d2a8d5e0b8b5d829b812d65287d18f96`.

**Do not describe stage 3a Gate A as passed.** Nothing has been observed on the panel;
the renderer's live bindings have still never drawn on hardware.

### The retry installed it — the first failure was transient

Second power cycle, **identical image, nothing republished**:

```
firmware_version  v2.0.0-live1      ota_state         idle
last_ota_error    null              last_network_error null
wifi_state        connected         wifi_rssi         -77 dBm  (was -80 at boot)
uptime_ms         51238             tier              networked
```

Counters clean: `crc_errors` 0, `malformed_frames` 0, `overflow_frames` 0,
`dropped_events` 0, `dropped_responses` 0, `rx_dropped_bytes` 0. `host_reconnects` 3
covers the two failed/retried cycles and the server restart.

It downloaded, installed, rebooted onto the new slot and survived the rollback window —
the same evidence standard the 2026-08-25 and 2026-08-26 entries used.

**So the reading of the first failure was right**: `esp_https_ota_perform()` lost the body
partway through on a weak link, and it was not the memory-layout mode. That mode is
deterministic and would have failed the retry identically at stage `begin`.

**The transient-download tally is now two out of four** on this board — 2026-08-25 and
2026-08-28 both failed once and installed on a retry of the identical image, both at
roughly -76 to -80 dBm. That is no longer a curiosity; at this signal a single failed
download should be **retried before it is investigated**, and only a second identical
failure justifies a bisect. Diagnosing the first one cost real time here, though the
`ota.c` stage/error reading is what made the retry decision confident rather than hopeful,
so it was not wasted.

**Flat internal RAM has now preceded a clean download three times** (2026-08-26,
2026-08-27, 2026-08-28) — still not a law, and still not a substitute for the check.

### What Gate A still owes

**Nothing has been observed on the panel, and the live bindings have still never drawn.**
That is not merely unobserved, it is currently *unobservable*: the deployed server binary
predates every one of Tasks 1-7, so nothing is pushing scenes that use the new bindings.
It also predates the `SceneRender` capability naming fix, which is why the device — which
correctly advertises 491 — is still reported with `unknown_capability_bits: 0x100` and
only six named capabilities.

Gate A's panel observations therefore require the **server** to be redeployed first.


### Server redeployed so Gate A becomes observable — 2026-08-28

Built from a `git archive HEAD` export in `rust:1.98-bookworm` against the retained
root-owned `companion/target/`: **20.6 s**. Installed over
`/usr/local/bin/deskmate-server` with the previous binary kept as
`deskmate-server.bak-20260828`. The device reconnected 2 s after the restart.

**The bin target is `server`, not `deskmate-server`** — `cargo build -p server --bin
deskmate-server` fails with "no bin target named". The *installed* file is renamed on the
way in. Worth recording; it cost a build cycle.

Immediately confirmed live, and this is the first end-to-end proof of the capability fix
from `05500ef`:

```
unknown_capability_bits  0x0000000000000000   (was 0x100)
capabilities             core-widgets, config-rotation, extended-templates,
                         asset-transfer, firmware-update, networking, scene-render
```

The device has advertised 491 all along; the host can finally name bit 8. Counters clean
after the reconnect: `crc_errors` 0, `malformed_frames` 0, `dropped_responses` 0,
`host_reconnects` 0, `valid_frames` 105.

### What the panel can and cannot settle, which matters for how Gate A is judged

**A scene and its C template are pixel-identical by construction** — that is precisely
what the 132-row byte-exact gate guarantees — so *looking at the panel cannot distinguish
which path drew it*. Neither can the admin API: `active_screen_id` is the same either way,
and `app-core` has no tracing on the scene path (raising `RUST_LOG` to `app_core=debug`
produced nothing, and was reverted).

So the panel observation is not "is this a scene?". It is **"is what the device draws
correct, at both orientations, over time"** — the clock crossing a minute with its date and
both dial hands right, a timer whose STATUS word and indicator colour follow its state, and
a tap that moves the timer with the link down. Which path produced it is settled by the
negotiation logic and its tests, not by eyes.

Stage 2a had a decisive visual tell — a ticking seconds field against a saved config with
`show_seconds: false`, which the C template *cannot* draw. That tell is not available here:
the automatic path builds the scene from the same card, so both halves agree about seconds
by design. Do not go looking for it.

### Gate A panel observation — PASSED, with three items still owed — 2026-08-28

Owner-observed on `v2.0.0-live1` with the redeployed server pushing scenes: **the panel
renders correctly with no artifacts**, at the board's current **270°**.

That is the substance of Gate A. Combined with the clean OTA install and the negotiation
logic's own tests, the renderer's live-binding path is now known to draw correctly on real
hardware — which it never had before. Arc opacity, the external rot-rect pivot and both
clip forms have now drawn on the panel for the first time.

**Do not read more into it than was said.** Three of Task 6 Step 4's observations were not
separately confirmed and remain owed:

1. **A minute boundary crossing** — that the date line and both small-dial hands, which
   were literals before this stage, actually step. A still frame cannot show this, and it
   is the specific thing the `date` and `time:angle:*` bindings exist to fix.
2. **90°.** The board sits at 270°. Real 270° geometry is only ever provable by looking,
   and so is 90°; the simulator's flipped framebuffer is an exact index reversal and proves
   neither.
3. **A `ProgressRing` timer advancing** with its STATUS word and indicator colour
   following state, and **a tap moving the timer with the link down** (Task 4).

**Ruling: these three are folded into Gate B (Task 9) rather than gating Task 8.** Gate B
needs its own power cycle regardless, so bundling costs nothing, and the risk Gate A exists
to isolate — a rendering fault being confused with a memory-layout fault — is already
substantially retired by a clean install and a clean panel. If any of the three then fails,
the C templates are gone and the diagnosis is harder; that is the accepted cost, and it is
accepted deliberately rather than by omission.

### Gate B — the OTA download with the C templates removed — PASSED — 2026-08-28

The check the 15,496-byte `.bss` shrink demanded. Session
`~/deskmate-hw-sessions/2026-08-28-stage3a-gate-b/`.

Evidence is the server journal on docker-vm, not the admin API: this session had no
`DESKMATE_ADMIN_TOKEN`. Quoted rather than paraphrased:

    08:27:24Z  INFO server::device_link: device link established device_id=dev-0005
    08:27:26Z DEBUG server::firmware:    firmware check device_id=dev-0005 current=v2.0.0-live1
    08:27:27Z  INFO server::device_link: device link closed device_id=dev-0005
    08:28:33Z  INFO server::device_link: device link established device_id=dev-0005
    08:28:36Z DEBUG server::firmware:    firmware check device_id=dev-0005 current=v2.0.0-live2

The one-second gap between the `live1` check and the close is `install_update()`
suspending the link for the download — the `0ad1a51` behaviour, forced by the hardware
AES accelerator's DMA buffers needing internal RAM, which is why two concurrent TLS
sessions cannot coexist on this board. Sixty-six seconds later the device is back
reporting `v2.0.0-live2`.

**It downloaded and installed on the first attempt.** No earlier failed attempt appears
in the journal after the 07:54:41Z server restart that moved the catalog. Stage 2a's
equivalent check failed once and succeeded on a retry of the identical image; the
publish commit (`045f432`) explicitly pre-authorised one retry here. None was needed.

The link established at 08:28:33Z was still open, with no intervening close, at
08:35:03Z — 6.5 minutes continuous on the new image. Catalog and device now both read
`v2.0.0-live2`, so no change is offered in either direction.

Published artifact re-verified through the tunnel during the session:
`GET https://deskmate.rodi.one/v1/firmware/v2.0.0-live2.bin` -> 200, 1,597,168 bytes,
md5 `47686777a8725924d16b2a6d85c31422`, matching the build recorded in `045f432`.

This is the third consecutive time flat-or-moved internal RAM has predicted a clean
download (2026-08-26, 2026-08-27, today), and the first time the movement was a *shrink*
— 15,496 bytes, roughly 140x the ~105 bytes that broke downloads outright in `3f2aa03`,
in the opposite direction. **Three data points are still not a law.** The hazard is
movement, not exhaustion, and the check stays mandatory after any change to firmware
statics.

**Not observed, and not to be described as verified:**

1. `ota_state` and `last_ota_error` from `GET /v1/devices/dev-0005`. The version
   transition and the sustained link establish that the download completed and the image
   is running; the two status fields themselves were not read.
2. **Rollback-window survival across a second boot.** The image is running and has been
   for minutes, but `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE=y` acts at the *next* boot.
   Only another power cycle settles whether `ota_mark_running_image_valid()` ran.
3. Everything in Task 9 Steps 3-4 — one card per template at both orientations, and the
   standalone clock on host loss — plus the three items Gate A folded in here (a minute
   boundary crossing, 90 degrees, and a `ProgressRing` advancing with a tap while the
   link is down). The panel was not observed in this session: the OBSBOT Meet 2 was
   physically disconnected, absent from both `system_profiler SPUSBDataType` and the
   avfoundation device list, with only its virtual-camera extension loaded publishing a
   "camera off" placeholder.

### Gate B Steps 2-3 and a new defect — 2026-08-28 (continued)

Same session as the OTA entry above, resumed with admin-API access.

**Step 2's remaining fields, now read.** `GET /v1/devices/dev-0005`:
`firmware_version v2.0.0-live2`, `ota_state idle`, `last_ota_error null`,
`last_network_error null`, `connected true`, `tier networked`, `rotation 270`,
`wifi_state connected`, `wifi_rssi -56`, capabilities `core-widgets, config-rotation,
extended-templates, asset-transfer, firmware-update, networking, scene-render`,
`unknown_capability_bits 0x0`, and every counter zero — `crc_errors`,
`malformed_frames`, `dropped_responses`, `dropped_events`, `dropped_ui_commands`,
`overflow_frames`, `host_reconnects` — over 1,286 valid frames.

**`wifi_rssi -56` deserves recording against the first-attempt download.** Both prior
downloads on this board that failed and then succeeded on retry sat around -76 to -80 dBm.
A link 20 dB stronger is a more ordinary explanation for the clean run than anything about
the 15,496-byte `.bss` shrink. Do not credit the shrink with the smooth download.

**`uptime_ms` 2,415,110 (~40 min) shows no reboot since the install**, so rollback survival
across a second boot is still unproven, exactly as recorded above.

**Step 3 — one card per template at both orientations — PASSED.** The saved config only
had cards for four of the six templates, so a test config added `clock-analog`
(analog-clock) and `weather-big` (big-number-label), put all six in one playlist at a 12 s
dwell, and — deliberately — set the pomodoro `alert` to `none`, because an until-dismissed
alert postpones firmware updates indefinitely and a gate session must not strand the device
that way. All six faces render correctly at **270** and at **90**, the latter inverted in
camera against the physically fixed board, which is what a real 180-degree UI rotation
looks like. Session frames `*-six270.jpg` and `*-six90.jpg`.

#### DEFECT: the standalone clock flashes for ~220-250 ms at every carousel transition

Found as an unexplained frame during the 270 sweep, reproduced, then measured. **The link
is up throughout**; this is not the host-loss fallback.

`show_current_content()` in `protocol_task.c` queues `UI_COMMAND_SHOW_CARD_FALLBACK` when
the host activates a screen, and `core/ui_command_policy.c` maps that to
`UI_COMMAND_ACTION_SHOW_STANDALONE_CLOCK`. `dispatch_push_scene` then calls
`ui_runtime_discard_card_fallbacks()` to pull the pending fallback back off the queue
before the scene loads — its comment says so in as many words. **That only wins if the
scene beats the LVGL command timer, and `UI_COMMAND_POLL_MS` is 20 ms.** Across
Cloudflare -> cloudflared -> Caddy -> WSS, the scene never does.

**This could not happen before stage 3a.** `git show 7d47ab6^:firmware/main/ui/ui_runtime.c`
carries `UI_COMMAND_SHOW_VIEW`/`UI_COMMAND_PATCH_VIEW`: the device drew the next card
immediately from its compiled-in C template plus retained field data, with no host round
trip at all. `SHOW_CARD_FALLBACK` arrived in `35c46d2`. Retiring the C templates converted
"render the next card locally, now" into "show the standalone clock until the server sends
a scene".

Measured from `20260828T091832Z-flash.mp4` (45 s, 59.94 fps) with ffmpeg scene detection
over the panel crop; transitions come in pairs, 12 s apart, matching the dwell:

    7.482 -> 7.699    217 ms
    19.450 -> 19.679  229 ms
    43.624 -> 43.874  250 ms
    31.563            one change only — it entered the DigitalClock card, which resembles
                      the standalone clock too closely to cross the threshold

Frames at t=7.40/7.58/7.80 show `BigNumberLabel` -> **standalone clock** -> `RowList`. The
middle face is positively identified as `clock_screen.c`: line 223 hides the "Connect
deskmate app" hint when `s_online` is true, which is why the flash carries no hint where
the genuine host-loss fallback did.

Counters stayed clean throughout, so nothing is being rejected — this is a latency race,
not a decode failure. **Not fixed here:** any firmware change costs an OTA re-verification
cycle, so the fix is the owner's call.

**The 132-row byte-exact parity gate is structurally blind to this.** It compares one
rendered scene against one C template. It says nothing about what the panel shows in the
gap between two of them, which is where this lives. Add it to the list of things that gate
cannot prove, beside shared code and injected binding inputs.

#### Gate A's folded item 3: the ProgressRing state machine — PASSED (online), offline tap owed

Pomodoro pinned as the sole playlist entry so it stayed on screen; owner tapped. Frames
`*-tap.jpg` in the session directory, 09:23-09:25Z, at 270 degrees. Observed in order:

    Ready    full lavender ring, STATUS "Ready" white, ELAPSED 00:00
    Running  ring RED and shrinking, STATUS "Running" RED, 00:59->00:43, ELAPSED 00:01->00:17
    Paused   ring lavender again at its partial sweep, STATUS "Paused" white, frozen 00:39
    Running  red again, 00:37->00:09, ELAPSED 00:23->00:51
    Done     00:00, ELAPSED 01:00, STATUS "Done" white, indicator arc at zero sweep

**This is the first hardware evidence for five of stage 3a's six additions at once:**
`timer.status`, `timer.elapsed`, `timer.total`, `timer.permille` and `running_color`.
Note the arc **shrinks** as the timer runs down — the ledger's inverted-percent defect
(the device using elapsed where the C arc used remaining, which would have made a native
ring grow) is genuinely fixed on the board, not just in tests.

**The offline tap is NOT observed and remains owed** — but the reason is subtler than
"no tap arrived", and the distinction is the point. Three attempts were made, dropping the
link at 09:25:57Z, 09:28:17Z and 09:30:16Z. The owner reports tapping during the third and
seeing the panel switch to the clock card immediately. **That switch does not require a tap
to explain it.**

A fourth run settled it: link dropped at 09:34:39Z, 60 s recorded at 60 fps, and the owner
did not tap at all — scene detection over the whole clip finds nothing but the transition
itself (three crossfade frames at t=38.87/38.89/38.91, no finger anywhere). **With no tap,
the panel still left the pomodoro scene for the standalone clock at t=38.9 s.** In the
third attempt the link dropped at 09:30:16Z, putting that timeout at ~09:30:55, and the
last still frame was 09:30:56 — so the tap and the timeout coincided to within a couple of
seconds. The owner's observation was real; the causal reading is what does not survive.

This also sharpens the retained-scene window to **~39 s from link close**, measured at
60 fps, replacing the ~42-45 s inferred from 3 s-spaced stills earlier in the session.
That is the budget any future offline-interaction check must work inside, and it is why
the first three attempts were built wrong.

**Owner reports the offline tap works.** On the final attempt (link dropped 09:37:32Z)
the owner tapped and the ring started counting — the local action doing exactly what it
should with no host. Recorded as an **owner observation**, on the same footing as Gate A's.
One honest caveat, stated once and not laboured: the 60 fps clip covering 09:37:32-09:38:02
contains no finger and no panel change, so the tap fell outside that window, and the link
was restored at 09:38:07 — the recording does not independently corroborate it. Treat this
as owner-observed, not instrument-confirmed.

The local-action path exists and was read — `carousel.c:79` -> `scene_view_apply_local_action`
-> `scene_timer_apply_local_action` (`core/scene_binding.c:95`), which toggles `running`
unconditionally so a tap from Ready or Done both produce a visible change — but reading
the code is not observing the panel.

Two incidental facts worth keeping:

- **The retained-scene window before the standalone fallback is ~45 s.** Measured at
  09:03:36Z link close -> ~09:04:21Z fallback, and corroborated by attempt 3, where a
  40 s window did not fall back. That is the budget any future offline-interaction check
  has to work inside.
- **Reconnect (server-up to link-established) was 4 s, 47 s and 29 s across the three
  drops.** Variable, not monotonically widening, so this is **not** evidence for Task 8's
  widening backoff — recorded so a future session does not mistake one long reconnect for
  that observation. Outages were 71 s, 91 s and 60 s respectively.

**Device restored to its saved configuration** at the end of the session and verified via
`GET /v1/devices/dev-0005`: the original five cards (test cards `clock-analog` and
`weather-big` removed), playlist clock/weather/rss at 50 s dwell, orientation
`landscape-flipped` / `rotation 270`, and the pomodoro `alert` re-armed to
`on-timer-finish` with `until-dismissed` hold. Closing state: `v2.0.0-live2`,
`ota_state idle`, `last_ota_error null`, `wifi_rssi -54`, `uptime_ms` ~63 min with no
reboot. `dropped_events` moved 0 -> 1 during the deliberate link drops; every other
counter is still zero.

## Stage 3b Task 9 — PARTIAL, blocked on the panel; a link-timeout asymmetry found, 2026-08-29

Session on `v2.0.0-live2`, device `dev-0005`, networked tier, rotation 270. **The gate did
NOT close.** No plugin face was ever seen on the panel, because the OBSBOT app held the
physical camera while publishing a muted "video off" frame on its virtual device, so every
capture returned that placeholder rather than the board. Nothing below is a panel
observation, and the visual half of Task 9 Steps 3-5 remains entirely unobserved.

**What did pass, and is real:**

- **The server redeploy and plugin load.** Built from a `git archive HEAD` export in
  `rust:1.98-bookworm` on docker-vm (49 s warm, `target/` preserved), installed to
  `/usr/local/bin/deskmate-server`. Startup logged
  `plugin registry loaded path=/var/lib/deskmate/plugins plugin_count=2 failure_count=0`.
  `DESKMATE_PLUGINS_DIR` was NOT set and did not need to be: `DEFAULT_PLUGINS_DIR` is
  already `/var/lib/deskmate/plugins`.
- **Task 9 Step 2's digest comparison, on the server side.** `GET /v1/plugins` reported
  digests identical to `sha256sum` run independently on the VM and `shasum -a 256` run on
  the Mac: `aqi/icons.ttf` = `40bbbac715465adf7ba539f53a0cb16991a2c1f73a4fb57af209d39e0c62b327`
  (4320 bytes), `agenda/badge.rgb565` =
  `e19db5d47bbdae46bf80e5a7df400795bce84973817c9f124642bc76bf41d33a` (812 bytes).
  `icons.ttf` is byte-identical to `crates/lvgl-sim/assets/Inter-subset.ttf`, confirming
  what CLAUDE.md claims.
- **A real plugin scene compiled and reached the device.** `POST /v1/devices/dev-0005/scene`
  with `template: "plugin"`, `plugin_id: "aqi"` and the committed fixture's inner
  `payload` returned **HTTP 200**; `card_errors` stayed `[]`, `commands_processed` went to
  1, and the device's `latest_revision` advanced 185 -> 191 with `malformed_frames`,
  `crc_errors`, `overflow_frames` and `dropped_ui_commands` all still 0. So the registry
  lookup, manifest compile with real assets, `validate_message`, and delivery to physical
  hardware all work. **Whether it drew correctly is unknown.**

**Task 9 Step 2 cannot be closed as written, and this is structural.** It asks to confirm
"the digests the device reports". `StatusResponse` carries no asset inventory — the device
reports holding a digest only implicitly, through `AssetBegin`'s `already_present` ack.
Same class as M4's `unknown_field_count` item. The server side is confirmed above; the
device side needs either a panel observation or a new status field.

**Finding: the server reaps an idle link 4.5x faster than the device notices it is gone.**
`server/src/runtime_device.rs` sets `IDLE_TIMEOUT = protocol::LINK_TIMEOUT_MS` = **10 s**
with `PING_INTERVAL` 3 s, while the device's networked transport uses
`NET_LINK_TIMEOUT_MS` = **45 s** (`firmware/main/link/net_link.c:25`; the USB path uses
`PROTOCOL_LINK_TIMEOUT_MS` = 10 s, so this asymmetry is specific to the network link).
Observed consequence, twice: the server logged `device link idle timeout, closing` while
the device continued to report `online: true`, `wifi_state connected`, and every frame
counter clean. Timestamps: established 14:30:38, closed 14:51:45; re-established 14:53:36,
closed 14:53:57 — the second link survived **21 s**. One reconnect gap measured at
**111 s** (14:51:45 -> 14:53:36). A later reconnect had not completed within 30 s of
watching, consistent with a widening backoff but **not** an observation of one: a single
gap does not show widening, and V2 Task 8's backoff item stays open.

Signal was poor throughout and is the most likely proximate cause: `wifi_rssi` read
**-91 to -93 dBm** early in the session and **-81 dBm** later. Do not read the drops as a
proven defect in the link code; do read the 10 s / 45 s asymmetry as real and worth
deciding about, since it means the server can abandon a device that still believes it is
connected.

Two scene pushes failed on that instability: one `502 {"kind":"runtime","message":"device
request timed out"}` and one `502 ... "device is disconnected"`. The agenda plugin — the
one with the `image` node — was therefore **never** successfully pushed, so the image-node
and `board_lcd_rounder_cb` observations are unattempted, not failed.

The board did not reboot at any point (`uptime_ms` rose monotonically 340,646 ->
1,997,191, ~33 min) and `free_heap` stayed flat at ~8.30-8.31 MB (PSRAM; not a TLS health
signal on this board). `dropped_responses` moved 0 -> 1 across the drops; every other
counter stayed 0. The USB serial port re-enumerated `/dev/cu.usbmodem3101` ->
`/dev/cu.usbmodem1101` mid-session with no reboot, consistent with the battery making a
USB replug a link event rather than a power event.

**Still owed from this stage:** every panel observation (Steps 3-5), the asset-GC teardown
(Step 6), and the device-side half of the digest check.

## Stage 3b Task 9, second attempt — plugin faces STILL unobserved; the 200 ms write budget bites for real, 2026-08-29

Camera working this time (physical `OBSBOT Meet 2 StreamCamera` at 3840x2160, after the
app released it). Same device `dev-0005`, `v2.0.0-live2`, rotation 270, no reboot across
the whole session (`uptime_ms` reached 19,311,571, ~5.4 h). **No plugin face was seen. The
gate still does not close**, but the reason is now diagnosed rather than unknown.

**Two faces WERE observed rendering correctly, and both are host-pushed scenes** (stage 3a
retired every C template, so anything on the panel is the scene renderer):

- The **`RowList`/rss face**: "Headlines" chip, a truncated headline row
  (`Sat, ...  The Internet Is Ki...`), and the count ring showing `1`. Clean, no artifacts.
- The **`DigitalClock` card face**: hero `19:42`, a `Sat, Aug 29` date box, and the analog
  dial complication with orange hands and tick marks. Clean, no artifacts, correct layout.

**Closed, unplanned: the standalone fallback clock at 270 degrees, bottom margin included.**
While the link was down the panel showed `clock_screen.c`'s fallback — hero time, date box,
and the dim `Connect deskmate app` hint (`clock_screen.c:221`) — with no title chip and no
`DATE` eyebrow, matching the clock-title-removal spec. The bottom margin is fully visible
and uncropped in the frame. **Both items CLAUDE.md listed as unverified from that work (the
fallback at 270 degrees, and its bottom margin) are now observed.** Note the fallback showed
`23:43` where the host-driven face showed `19:42`: the fallback renders local UTC+4 while
the host was pushing UTC, consistent with the board sitting at UTC+4.

**Finding: the deliberately-unfixed 200 ms write budget DOES bite at realistic cadence.**
CLAUDE.md records `PROTOCOL_WRITE_TIMEOUT_MS` = 200 ms for the device to deliver a reply the
host waits 2000 ms for, and says "No evidence it bites at realistic cadence." **There is now
evidence.** Over this session `dropped_responses` climbed **1 -> 15** while
`malformed_frames`, `crc_errors` and `overflow_frames` all stayed **0** — the link was not
corrupting anything, the device simply could not write replies inside 200 ms. Host-side this
appeared as repeated `502 {"kind":"runtime","message":"device: device request timed out"}`
on `POST /v1/devices/{id}/scene`.

The two timeouts compose into a flap loop: the device drops a reply (200 ms), the server
sees no activity and reaps at `IDLE_TIMEOUT` 10 s, the device reconnects in ~2 s
(`19:43:58` closed -> `19:44:00` established), and it repeats. `wifi_rssi` was **-77 dBm**
here, improved from the -91/-93 earlier, so this is not purely a fringe-signal artifact.

**Why no plugin face could be captured, mechanically.** The pushes were not lost: server
`diagnostics.commands_processed` reached **7** with `command_queue_full: 0` and
`card_errors: []`, so the runtime accepted and processed every `PushScene`; only the acks
timed out. But an operator-pushed scene is transient — on every reconnect the runtime
replays the active card's own scene, which overwrites it, and with the link flapping on a
~10 s period the pushed face is wiped before it can be photographed. Rotation compounds it
(`my-playlist` is timed, 50 s dwell, clock -> weather -> rss).

**Deliberately NOT done: raising the server's `IDLE_TIMEOUT` to stop the flapping.** It
would very likely have unblocked the capture, but a gate observed on a hand-modified
timeout is not evidence about the shipping configuration, and this session's whole purpose
is evidence. The asymmetry recorded in the previous entry (server 10 s vs device
`NET_LINK_TIMEOUT_MS` 45 s) and the 200 ms write budget are now both supported by
measurements and want a real decision, not a workaround applied mid-gate.

**Still owed, unchanged:** every plugin-face observation (Steps 3-5), the asset-GC teardown
(Step 6), and the device-side half of the digest check. Nothing about the plugin faces is
hardware-verified.

## Stage 3b Task 9 — PASSED 2026-08-30: both plugin faces on the panel at both orientations

Device `dev-0005`, `v2.0.0-live2`, networked tier, server at `0b9075f`. **The plugin faces
drew on the panel.** Steps 1-5 pass; Step 6 (asset-GC teardown) was not run.

**What made it possible.** The 2026-08-29 sessions failed on a link that flapped every
~10 s. `0b9075f` moved the server's `IDLE_TIMEOUT` off `protocol::LINK_TIMEOUT_MS` (10 s,
the USB value) to 30 s, derived as `DEVICE_NETWORK_LINK_TIMEOUT_SECS - IDLE_TIMEOUT_MARGIN_SECS`
so it sits between a healthy link's silence and the device's own 45 s
(`net_link.c`'s `NET_LINK_TIMEOUT_MS`). Result, measured: **0 idle timeouts in 15 minutes**
against a prior baseline of links lasting 21-24 s, and operator scene pushes went **0/3 ->
3/3 acked**. Signal was also better (`wifi_rssi` -67 to -69 vs -93 earlier) and
`dropped_responses` stayed **0** for the whole session, so the fix is not the only variable
— but the flapping stopped and stayed stopped.

**Step 2 - digests.** `GET /v1/plugins` matched independent `sha256sum` (VM) and
`shasum -a 256` (Mac): `aqi/icons.ttf` =
`40bbbac715465adf7ba539f53a0cb16991a2c1f73a4fb57af209d39e0c62b327` (4320 B),
`agenda/badge.rgb565` = `e19db5d47bbdae46bf80e5a7df400795bce84973817c9f124642bc76bf41d33a`
(812 B). The device-side half is confirmed **transitively and unavoidably**: `StatusResponse`
has no asset inventory (see the 2026-08-29 entry), but the aqi glyph could only resolve if
the device held those exact bytes under that digest — and it drew.

**Steps 3-4 - the aqi face, both orientations.** Every node rendered as the manifest
specifies: `MODERATE` (`upper(data.current.category)`), an orange `M` glyph from the
uploaded `icons.ttf` **at pixel_size 72, a size no baked tier provides** — which is the
whole point of `SceneFont::Asset` and Step 4's requirement — a `42` hero
(`data.current.aqi`), `AQI`, `42` + `PM2.5`
(`default(round(data.current.pollutants.pm25.value, 0), "--")`), and `field.title`.
No first-render hitch was visible at the capture cadence used; §6's 96 px glyph-cache-miss
timing gate was **not** measured and is still owed.

**`field.*` drew a real value for the first time in this project.** The aqi face's
`{{ field.title }}` node rendered **"Headlines"** — the title of the `rss` card the scene
was pushed to. The binding CLAUDE.md and the parity ledger both recorded as having no
producer is now live end to end: `ServerProviderRefresher` emits the card's `title` ->
`PushData` -> device retains it -> `field.title` resolves it at render. Note the reach is
still what the ledger says: a plugin card's wire template is always `DigitalClock`, so only
its four registered fields can ever resolve.

**Step 5 - the agenda face and the image node, both orientations.** `2026-08-28`
(`data.date`), the `badge.rgb565` **image node** as a white square, and exactly **five**
event rows from a fixture carrying **six** — `MAX_REPEAT_ITEMS` enforced on hardware. The
long first title truncated to `Standup with the who...` (a real `truncate()`), and the
event with null time and null title rendered `--:--  (untitled)`. **No artifacts at either
orientation with an image node present**, which is the `board_lcd_rounder_cb` observation
§6 deferred out of stage 3a.

**Real 90-degree geometry is now observed, not inferred.** Orientation was flipped by
`PUT /v1/devices/{id}/config` with `preferences.orientation` `landscape-flipped` ->
`landscape`, confirmed by the device reporting `rotation: 90`, and both faces were captured
again. Both appeared correctly inverted in the camera with identical layout and no
clipping. This is the claim no host gate can make: the parity gate and the `0x7E` capture
both carry `flipped(A) == flipped(B)` blindness. Orientation was restored to
`landscape-flipped` afterwards and verified both live and in
`/var/lib/deskmate/configs/dev-0005.json`.

**Finding: the operator scene route's caller-supplied `revision` shares one counter with
`PushData`, and a high value stalls the runtime.** `link_state.c:116-120` keeps a single
`latest_revision` that both `PushScene` and `PushData` are checked against. Pushing scenes
at revisions 910-932 left the runtime's own `next_scene_revision` (~243) below it, and
every card went `data-refused (StaleRevision)`. It does not self-heal by design:
`next_scene_revision` starts at 0 and only increments, and the status-based adoption at
`runtime.rs:3049` is deliberately for the **interrupt token** only (a 2026-08-20 note
explains that coupling it to revisions misread 24 lost interrupts). Restarting the server
makes it worse — `WorkerState::new` sets the counter to 0. What actually cleared it here
was `ApplyConfig`, which reset the device's baseline: `latest_revision` read **248**
afterwards and live weather resumed rendering. A device power cycle would also clear it
(`link_state_init` memsets). **Treat a large operator `revision` as a footgun**; prefer
values just above the runtime's current counter.

**Also observed again: a request whose work succeeded but whose ack was lost.** The
orientation-restore `PUT` returned `504 runtime command response timed out` while the
device had already applied it (`rotation: 270`, and the stored config shows
`landscape-flipped`). Same 200 ms `PROTOCOL_WRITE_TIMEOUT_MS` shape as 2026-08-29; the
30 s idle timeout does not address it and was not intended to.

**Not done:** Step 6, the asset-GC teardown (release, compaction, font-registry reset,
retained-scene rebuild, and the `BUSY`/OTA-owner interaction). It remains without an
automated test and without a hardware observation. Stage 4 makes it more urgent because
rasterization turns volatile assets into the common case.

## Stage 4 Task 7 Phase A — asset-GC teardown, PARTIAL (2026-09-06)

Board on `v2.0.0-live2`, networked tier, `dev-0005`, capabilities **491** (no bit 9 yet
— correct for the predecessor image). WiFi re-provisioned over the cable to
`Slate7Legacy` earlier this session; same device identity/token, no identity cost. Server
redeployed 2026-09-05 from `f40958a`, registry 5/0
(`agenda, aqi, claude-limits, svg-aqi, svg-live-clock`).

**Method.** Stage 3b Task 9 Steps 1-5/7 already passed 2026-08-30, so Phase A is only
Step 6, the teardown. Drove a registry-wide desired-asset change: moved
`/var/lib/deskmate/plugins/aqi` aside (dropping its `icons.ttf`, digest `40bbbac71546`,
from the desired set), restarted the server, let the device reconnect so `synchronize_full`
issues an `AssetRelease` with the shrunk keep-set; then restored `aqi` and restarted again.

**Observed (webcam clip `20260905T201625Z-gc-teardown.mp4`, 2 fps frames):**
- **Scene rebuilt live, no reboot.** Across both registry changes (aqi out 20:16:30Z,
  aqi back 20:20:09Z) the device uptime ran continuously to **749,961 ms** with
  `valid_frames` climbing 237→456 and **0 malformed / 0 crc / 0 overflow**; `free_heap`
  flat at ~8.315 MB (PSRAM). The panel tore down and rebuilt its scene without a blank
  reboot: link re-established 2.4 s after each restart, `card_errors: []` throughout.
- **Compaction did not wipe.** The device stayed healthy after the release — the empty-set
  wipe hazard (`AssetRelease { digests: [] }`) did not fire, and the retained agenda badge
  digest (`e19db5d47bbd`) stayed in the keep-set. This is the `synchronize_full` +
  `compose_asset_keep_set` behaviour the wire-sequence test (`hostile_device.rs`) pins,
  now exercised on hardware.
- **`claude-limits` drew on the panel for the first time** (frame
  `20260905T202312Z-phaseA-settled.jpg`): SESSION 40% / WEEKLY 19% / RESETS Sun 1:40 AM /
  Wed 8:00 PM / "Max 5×", a clean Native scene that survived the teardown intact. Closes
  the runbook's "claude-limits on the panel" debt.

**NOT observed, and why (honest gaps, not failures):**
- **Font-registry reset with a displayed registry font, and the glyph disappearing.**
  None of `dev-0005`'s cards (clock, weather, rss, pomodoro, claude-limits) display a
  registry font/image — the released `icons.ttf` was held but not on screen — so the
  font-release-on-destroy step is not panel-visible on this config. The rebuild path that
  contains `font_registry_reset()` did run (the scene rebuilt correctly with its baked-font
  cards); the specific glyph-vanishes moment was not reproduced. Re-observing it would mean
  re-treading 2026-08-30's operator-route glyph render purely to tear it down.
- **Asset-store compaction counters.** The device exposes no asset-used-byte / DEAD-record
  counter through `status` or the admin snapshot (`diagnostics` covers queues/providers
  only), so "absent digest marked DEAD" is confirmed by wire contract and the
  did-not-wipe/did-not-break evidence above, not by a device-reported number.
- **The `BUSY`/OTA-owner interaction.** Needs an OTA in flight, which Phase A cannot have
  without publishing firmware. Folded into Phase B Step 10's "repeat one release while OTA
  owns the panel", recorded there as completing this clause — an explicit, planned
  deviation from strict phase ordering (see the session runbook).

**Verdict: Phase A did not fail** — the teardown/release/rebuild mechanism works on
hardware and nothing regressed — but Step 6 is only **partially** observed. The
font-vanish and BUSY/OTA pieces are deferred (the latter into Phase B by design; the
former is not reproducible on this device's card set). Proceeding to Phase B is therefore
justified, with the BUSY/OTA observation owed inside it.

## Stage 4 Task 7 Phase B — PASSED on hardware (2026-09-06)

Published `v2.0.0-raster1` (built from `f40958a`, sha
`29f15a6f7ee1f6f5717deaa09a21a3bdb75bd19c6ddeae5dd2b51ade1afc3b15`) by flipping
`DESKMATE_FIRMWARE_VERSION` on docker-vm and restarting. Baseline memory (same tree):
DIRAM total 203,867 (`.text` 93,635, `.bss` 87,104, `.data` 23,128), IRAM 16,384/16,384
(0 remaining) — internal RAM byte-flat vs the cleanup tree.

**B5 — OTA download + rollback survival: PASSED.** The device only checks firmware at
boot (24 h interval otherwise), so a USB RTS reset re-ran the boot check. Sequence (server
UTC): reset 20:54:41 → boot, `ota_state: checking` at uptime 13,002 ms → downloaded and
installed → rebooted into the new slot → `v2.0.0-raster1`, `ota_state: idle`, uptime reset
to 8,807 ms at 20:55:29. **Installed on the first attempt**, `last_ota_error: None`
throughout. Rollback-window survived: uptime then climbed 24k→86k ms and stayed on
raster1 with no revert to live2, so the image marked itself valid. The documented
memory-layout OTA hazard did not bite.

**B6 — capability truth: PASSED.** Device and server report numeric **1003**; the server
decodes the bit set by name as `core-widgets, config-rotation, extended-templates,
asset-transfer, firmware-update, networking, scene-render, volatile-assets`;
`unknown_capability_bits: 0x0`. A volatile transfer was accepted (see B7).

**B7 — native vs raster: PASSED at 270°.** The native weather card
(`icon-badge-text`) drew cleanly. Pushing `svg-aqi` (a manifest-v2 SVG template →
`RasterOnly`) via the operator route produced a server-side `resvg` render → one 448×368
RGB565 volatile frame → one-node scene: the panel showed "GOOD / ⬤ / 42 / AQI".
`free_heap` dropped ~334 KB on the push (one 329,740-byte RGB565 frame in PSRAM),
confirming the volatile transfer. (The circle reads bright/white on the webcam — likely
exposure of the bright fill; not diagnosable from the camera.) **90° not yet captured in
this checkpoint.**

**B8 — refuse rule: PASSED (mandatory rollout observation).** `svg-live-clock` (an SVG
face binding `{{ time:HH:mm }}`) pushed via the operator route *with data*, so the only
possible refusal cause is the live binding. Result: HTTP 200, **no raster asset and no
frozen scene sent**, and a typed `scene-refused` card error naming `time:HH:mm` ("…a
server-rendered image of it would freeze, so it is refused instead…"). The panel showed
the **live standalone clock** (observed ticking 21:03→21:08), i.e. the device kept drawing
time itself rather than freezing a rastered clock — the exact intended behaviour.

**B9 — 30-second floor: core behaviours PASSED (mandatory rollout observation).** Through
the operator route, three distinct `svg-aqi` snapshots pushed at t≈0/5/11 s (AQI 11 / 88 /
199), all HTTP 200. Observed (1 fps frames of `…-floor.mp4`): (1) the first frame (11)
rendered immediately; (2) no new frame appeared before 30 s; (3) at the 30 s boundary
(push A 21:03:25 → 21:03:55) the **newest** snapshot (199 "UNHEALTHY") appeared — the
intermediate 88 was coalesced away and never shown. Floor timing was exact.
- **One caveat, root-caused as a test-setup artifact, not a floor defect:** during the
  deferral window the panel fell back to the standalone clock instead of holding the
  prior frame (11). Cause: `svg-aqi`'s source is `https://example.invalid/` (unfetchable
  by design), so its provider fetch kept failing (`providers[svg-aqi-card].state = error:
  dns resolution failed`, `provider_jobs_started` 3→5) and the resulting data-less error
  snapshot made the SVG evaluation fail, clobbering the operator-injected good data
  between pushes. With a real fetchable source this would not occur. Worth noting: that
  SVG-evaluation failure surfaced only in the `providers` block, not as a `card_errors`
  entry.

Config note: these observations used a temporary manual single-card playlist `svg-test`
with added plugin cards `svg-aqi-card`/`svg-live-clock-card`; dev-0005's original config
is saved and is restored at the end of the session.

### Phase B continued — B7 both orientations, B10 churn, restore (2026-09-06)

**B7 — raster at BOTH orientations: PASSED.** The same `svg-aqi` raster
("GOOD / ⬤ / 42 / AQI") drew correctly at 270° (`landscape-flipped`, upright to the
camera) and at 90° (`landscape`, 180°-inverted to the camera because the board sits
optimally for 270°). The full scene renders under both software rotations; the physical
transform and rounder handle the volatile RGB565 frame. Capturing a single still needed
the 30 s floor to be clear and a push-then-capture within a few seconds, because
`svg-aqi`'s unfetchable `example.invalid` source repeatedly clobbers operator data with
an error snapshot (see the B9 caveat).

**B10 — volatile churn: core PASSED.** 20 raster revisions pushed via the operator route
at ~31 s intervals (AQI 39→210), all HTTP 200. `free_heap` stayed flat at **7,981,503**
(±20 bytes; two transient dips to ~7,979,900 caught a sample mid-swap with the incoming
slot briefly allocated) across the whole ~10-minute run — **PSRAM did not trend down**, so
old volatile frames are freed as new ones display; no leak. Uptime ran continuously
(no reboot), `valid_frames` 456→1196, 0 malformed/crc/overflow/dropped-ui-commands. The
final raster drew with no tearing/artifact. When the svg cards were later removed from
config, `free_heap` returned to 8,317,395 — the held volatile frame was released on
teardown, a bonus confirmation of release-on-card-removal.
- **Limits:** the device exposes no flash asset-used-byte counter, so "decoded frames
  never touch the flash counter" is inferred from `free_heap` being PSRAM and no durable
  `AssetRelease` growth, not read directly. And "no standalone-clock flash on an ordinary
  old→new swap" could not be isolated, because the `example.invalid` provider clobbers to
  the clock between the 31 s-spaced pushes regardless of the swap.

**Restore.** dev-0005's original production config was PUT back (generation 4): rotation
270°, original cards (`claude-limits, clock, pomodoro, rss, weather`), no card errors.
The device remains on the shipping image **v2.0.0-raster1** in networked tier with its
dev-0005 identity — never left networked tier, so no re-provision was needed.

**Still owed on hardware (Phase B remainder), each needing its own setup:**
- **B11 / Task 6 Step 5 — the on-target `framebuffer_diff` byte comparison** (expected
  96/8/88). Needs a `DESKMATE_DEV_DIAG=1` flash and a local-tier round-trip (networked
  tier refuses `PushScene` over the cable), which takes the device off the shipping image
  temporarily; the diag image is built (`firmware/build-diag`, sha
  `f815edf07cef7981698304d964f468ed7cde6296d7a1783c4f32843f16fb1dbc`). Deliberately not
  run at the tail of this session to avoid a fatigued mis-restore; the plaintext token in
  `pass` means it restores to the same dev-0005 identity.
- **The BUSY/OTA-owner variant** (carried from stage 3b Task 9 Step 6 / Phase A): observe
  a raster release while an OTA owns the panel. Needs a pending OTA in flight (i.e. a
  freshly published newer version) concurrent with a push; `ota_state` stayed `idle`
  through churn, so it was not observed.

**Phase B verdict: the shipping stage-4 image passed every gate that does not require the
diag build** — OTA download/install/rollback survival, capabilities 1003, native and
raster at both orientations, the typed refuse rule, the 30 s floor, and 20-revision
volatile churn with flat PSRAM. Stage 4 is confirmed working on the physical board.

## Stage 4 Task 7 — B11 / Task 6 Step 5: on-target `framebuffer_diff` — PASSED (2026-09-06)

The one Phase B item that needs the diag build. Ran later the same day.

**Setup.** The staged diag image (`f815edf…`) turned out to be **stale** — an Aug-21
build predating the scene renderer: on boot it reported `v2.0.0-swaes6`, capabilities
`0xcb` (203). Caught by reading `deskmate-cli status` before trusting it; comparing the
current simulator against that firmware would have been meaningless. The "no source drift
since the runbook" check had verified the source, not that the staged *binary* matched it
— the runbook's own hash was of a leftover. Rebuilt the diag image fresh from HEAD
(`idf.py -C firmware -B firmware/build-diag -DDESKMATE_DEV_DIAG=1 build`, sha
`6eae2138702332cb4663d4e415465bd19f9dbb7b3c281310ef6a25459a829491`), which reported
`v2.0.0-raster1` / caps `0x3eb` (1003) as expected. The release restore artifact was
rebuilt too and **reproduces the shipping image byte-identically** (sha
`29f15a6f7ee1f6f5717deaa09a21a3bdb75bd19c6ddeae5dd2b51ade1afc3b15`), so the diag build
(same source + `DESKMATE_DEV_DIAG`) is trustworthy for the comparison. Device taken to
**local tier** over the cable (`provision --tier local`), which is what lets it accept
`ApplyConfig`/`PushScene` over USB.

**Result: `total=96 identical=86 differing=0 errored=0 excluded=10` — exit 0.** Every
includable case is byte-identical, both orientations, no tolerance. The log is at
`docs/hardware/media/2026-09-06-task7/framebuffer_diff-2026-09-06.log`.

Post-manifest-removal update (2026-09-11): that remains the historical observation. The
current inventory's unobserved software prediction is **78 total / 8 excluded / 70
identical**; the next hardware session must run the matrix fresh.

**The predicted split was 96/8/88; the first hardware run corrected it to 96/10/86 by
surfacing three test-harness fidelity issues — none a firmware or renderer defect:**

1. **`plugin-v2-timer--remaining-357-of-1000` (both orientations): time sync rejected.**
   The case pinned `now_unix_seconds: 0`, below the device's `PROTOCOL_MIN_UNIX_SECONDS`
   (1577836800 = 2020-01-01) sanity floor, so every push failed at `TimeSync` with
   `InvalidValue("unix seconds")`. The timer scene binds only `timer.permille` /
   `timer.remaining`, never `time:`/`date`, so the instant is irrelevant to the frame —
   the harness just time-syncs every case unconditionally. **Fixed** by pinning a valid
   instant (`SCENE_NOW`); both rows now render identically on the board. (Firmware is
   correct — rejecting a nonsense clock is the intended bound.)

2. **`plugin-aqi--empty` (both orientations): 36 px differ (14×3 strip at the
   `field.title` box).** The simulator drew `"--"`, the device drew nothing. `aqi`'s empty
   state pushes no `title` field and its scene binds `{{ field.title }}`. `scene_binding.c`
   writes the `"--"` placeholder only when `context->field()` returns **NULL**. The
   simulator gets an empty field array → `title` absent → NULL → `"--"`. The device has
   `title` **registered** (a configured card always registers its template's fields; the
   harness's `apply_case_config` uses `DigitalClock`) → returns `""` (empty, non-NULL) →
   renders nothing. Same C, different field input; the device *cannot* reproduce the
   placeholder for a name its registry knows. **Excluded from hardware, kept in the golden
   suite** (its deliberate placeholder coverage) — the same shape as the existing
   `field.status` (`scene-text`/`scene-label`) exclusions. This is the "injected/absent
   binding-input" blindness the ledger warned of, made visible.

3. **`scene-image--flipped`: `InvalidPayload "scene could not be rendered"` — a teardown
   race, not a renderer defect.** The synthetic image case has **four** image nodes
   (agenda has one, and never tripped it). Run alone at either orientation the case is
   byte-identical; it failed **only** when the same 4-image scene was re-rendered at 270°
   immediately after 90° (the matrix flips orientation between a case and its
   counterpart). The device frees the previous scene's image buffers asynchronously on the
   UI tick, so back-to-back heavy-image renders outpaced the teardown and the new render
   failed to allocate. `free_heap` was 8.4 MB (PSRAM) throughout — not gross exhaustion.
   A post-config settle before an asset-bearing re-render removes it (500 ms sufficed;
   `CONFIG_SETTLE` = 700 ms for margin). The renderer itself draws multi-image scenes
   correctly at both orientations. *Worth noting for possible future firmware hardening:*
   a runtime orientation change with a heavy-image card live can transiently fail one
   re-render before the async teardown frees memory; production orientation changes carry
   natural latency and the device recovers, so this was recorded, not fixed.

The three fixes are test-only (`cases.rs`, `examples/framebuffer_diff.rs`) — no firmware,
protocol, or config change — so no OTA re-verification is owed. The harness unit test now
pins the historical 96/10/86 split with the per-reason exclusion breakdown; the current
post-removal test pins the unobserved 78/8/70 inventory instead.

**Restore.** Re-provisioned networked (`dev-0005`, WiFi `Slate7Legacy`, offset 240,
plaintext token from `pass`), then full-flashed the verified release image (sha
`29f15a6f…`). Device came up `v2.0.0-raster1`, caps `0x3eb` (1003), tier networked, WiFi
connected (`192.168.8.168`), rotation 270°; the server confirms `dev-0005` **connected**,
`v2.0.0-raster1`, `volatile-assets` present — **same identity, no re-mint** (the plaintext
token authenticated). Production config restored automatically on reconnect.

**Still owed on hardware:** only the **BUSY/OTA-owner variant** (a raster release while an
OTA owns the panel) — needs a pending OTA in flight, a distinct setup, not run this
session. B11 is now closed. (V2 Task 8's widening-backoff observation on the shipping
build was discharged 2026-09-06 — see the entry at the end of this file.)

## V2 Task 8 — widening-backoff observation on the shipping build, PASSED (2026-09-06)

Board `dev-0005`, shipping image `v2.0.0-raster1` (caps 1003), networked tier, WiFi up
throughout, USB attached. This closes the item that had been stuck as a catch-22: the
shipping build routes its console to UART0 only
(`CONFIG_ESP_CONSOLE_UART_DEFAULT=y`, `SECONDARY_NONE`), so `websocket_client: Reconnect
after N ms` never reaches the USB cable, and the indirect "outage-length -> return-time"
method is provably invalid (uniform sample in [0,T]). Both are recorded above as the
reason prior sessions could not discharge it.

**Method that worked — observe the retry schedule at the server socket.** Stopped
`deskmate-server` and bound a bare TCP listener to its port `192.168.8.20:8443` that
accepts each connection, logs a UTC-ms timestamp, and closes. Caddy proxies the device's
WSS reconnect attempts to it, so the interval between logged connections is the device's
own reconnect delay. Two sources appeared: a constant 2.01 s health probe from the host
IP (104 conns, filtered out) and the device's proxied retries via the docker bridge
`172.26.0.2`.

**Widening curve observed (device retries, seconds between attempts):**
`3.1 -> 5.1 -> 10.7 -> 16.6 -> 35.0 -> 50.3 -> 66.2` — doubling and capping at the 60 s
ceiling, the last two being jittered draws of the capped 60 s base (±20% -> [48,72]).
This is the genuine backoff schedule: the failure shape only affects what happens after
TCP connect, not when the device dials, and the clean doubling confirms it. Log:
`docs/hardware/media/2026-09-06-task8/listener-conns.log`.

**Panel behaviour.** ~30 s in, the retained Weather card still showed (link-timeout
fallback is 45 s). Past 45 s the panel fell to the standalone clock
(`panel-standalone-fallback.jpg`) — no reboot, no spin. After the server was restored
the device reconnected and live weather was pushed back down
(`panel-restored-weather.jpg`).

**A confound found and resolved — the "kill the server" clause was then run properly.**
The listener run *also* rebooted the device ~181 s in (`uptime_ms` dropped; no fresh
coredump was written — the flash coredump is a stale `4160f1cb8` image, not raster1's
`d663a6a58`, so the reboot was not a panic/abort). The accept-then-close listener is a
half-open failure shape a real outage does not produce. So a second run used a **plain
`systemctl stop` (real HTTP-502 outage, no listener)** for 210 s with a non-resetting
USB serial logger attached. The boot ROM prints its banner on USB-JTAG at every reset;
the serial log showed **only the baseline attach-reset banner
(`rst:0x15 USB_UART_CHIP_RESET`) and no new banner during the outage window** — the
device did **not** reboot under a genuine outage, stayed on the standalone clock, and
reconnected on restore. Log: `plain-outage-serial.log`. Every device reset seen this
session was either `USB_UART_CHIP_RESET` (my serial logger's control lines — the
attach/detach-resets-the-board hazard, now confirmed by reason code) or the accept-close
listener's induced reboot; none was a crash/watchdog/brownout and none occurred under a
plain 502 outage.

**Verdict.** V2 Task 8's "killing the server leaves the device retrying with visibly
widening gaps rather than rebooting or spinning" is **satisfied on the shipping build**:
widening intervals observed (listener run) and no reboot under a genuine outage (plain
run). Two robustness/ops notes, neither a V2 blocker: (1) a peer that repeatedly accepts
then closes the socket half-open can drive the device to reboot after ~3 min (~7
attempts) — likely internal-RAM churn on repeated TLS setup; worth a follow-up but
distinct from the normal outage path; (2) the device briefly displayed an apparently
wrong wall-clock time during the outage (standalone clock ~6 h off real local) — the VM
clock is NTP-synced and correct now, so glance at the TimeSync/offset path later.

## Cable pull with the Mac app running — PASSED 2026-09-09

Verification for `f976f42` ("bound every wait on the device session worker"), the fix
for the defect where a device that disappeared from under an open serial fd wedged the
whole companion app and made saving to the server impossible. Host-side change only —
no firmware was rebuilt or flashed, so no OTA re-verification is owed. Board `dev-0005`
on `v2.0.0-raster1`, networked tier, server `deskmate.rodi.one`.

**Method.** Attach the board over USB with the app running; save a config change to the
server; pull the cable; then re-check the app. The runtime worker was sampled with
`sample <pid>` throughout, because the failure signature is a stack, not a log line: the
broken build parks 100% of samples at one `run_runtime` offset inside
`SerialRuntimeDevice::status -> DeviceSession::request -> recv`, while a healthy worker
shows several distinct offsets as it cycles.

| Check | Observed | Result |
| --- | --- | --- |
| App connects over USB in networked tier | tray "Device: Connected" | PASS |
| Save to server, cable attached | `dev-0005.json` 09:56:04Z, `show_seconds: true` | PASS |
| Cable pulled 13:57:04 local | node `/dev/cu.usbmodem1101` gone | — |
| Worker at t+3/6/9 s after the pull | 4 distinct `run_runtime` offsets, **0** frames in `status` | PASS |
| Runtime notices the disconnection | tray "Device: Standalone" | PASS |
| Save to server, cable pulled | `dev-0005.json` 09:58:15Z, `show_seconds: false`, UI "Saved to the server" | PASS |
| Quit with the cable pulled | **0.1 s** | PASS |
| Config left as found | byte-identical to the pre-test copy | PASS |

**Why the last two rows matter.** Before the fix the same quit hung for over 30 s and
only completed when macOS finished tearing the device node down, because `Drop` joined a
thread blocked in an uninterruptible read. And "Device: Standalone" is the visible proof
that a stalled session reports `Transport(Disconnected)` rather than `Timeout`:
app-core's `is_disconnect` does not count `Timeout`, so the wrong classification would
have left the runtime holding a dead session and silently never reconnecting.

**Not covered here.** Re-attaching the cable and confirming the app picks the device up
again on a fresh session was not exercised; `SerialRuntimeDevice::connect` discards a
stalled session and opens a new one, and that path has unit coverage but no board
observation yet.

## Picture cards on the panel — 2026-09-10

First picture card drawn on `dev-0005`. Observed, not inferred: the loop rotated
`clock -> plugin -> picture` at 50 s dwell across two full cycles with
`card_errors: 0`, the picture holding its dwell each time.

Getting there took five defects, each hidden behind the last, plus one stale
binary. Recorded because four of them are the kind that recur.

1. **A durable asset could not be compressed, in BOTH halves.** A 448x368 frame is
   329,740 bytes, and at `MAX_ASSET_CHUNK_BYTES` (1920) that is 172 sequential
   chunk round trips. It died at offset 161,280 -- exactly chunk 84. The host
   hard-coded `ASSET_ENCODING_RAW` on the durable path and the firmware's durable
   `AssetBegin` ignored `encoding` outright; only the volatile tier ever
   compressed. Picture cards chose durable for lifetime reasons (two PSRAM slots
   cannot serve a loop) and nobody noticed durable also forfeited the compression
   stage 4 added for exactly this cost.
2. **The failure named the wrong card.** `record_plugin_asset_sync_refusals`
   iterated plugin cards only, so a picture card's own transfer failure was
   recorded against `plugin-2` -- a card that was fine -- and the picture card
   reported nothing. That is why this presented as silence rather than a fault,
   and it is what cost the session.
3. **`AssetRelease` was budgeted like a message.** Its handler runs the mark-dead
   scan and compacts the flash blob region, and a 330 KB frame makes that move far
   more than the curated fonts ever did. It timed out at 2 s while the device
   reported `dropped_responses`, `malformed_frames` and `crc_errors` all ZERO --
   the reply was late, not lost. The many-round-trip chunk phase, which does no
   bulk flash work, succeeded in the same pass; that is what ruled out a flaky
   link.
4. **The volatile-only rule was written in THREE places** -- `protocol`'s
   `validate_asset_begin`, and the firmware's decode AND validate paths -- so
   fixing one shipped `v2.0.0-durable1`, an image that ADVERTISED capability bit
   10 while still refusing the transfers the bit promises. Same failure as bit 7
   sitting defined-but-dark. The firmware test that looked like it pinned the rule
   was decoding a fixture that omitted `AssetBegin` key 3, so the durable case
   never reached the tier check at all.
5. **The durable decode fed the passthrough header to the decoder.** The host
   copies the 12-byte LVGL header verbatim and encodes only the pixel body; the
   volatile path mirrors that by initialising the decoder at `bytes + 12`. The new
   durable path did neither, and said so: `asset chunk decode failed`.

**And the companion app is a third deployment boundary.** `/Applications/Deskmate.app`
was a build from the previous day, so it compiled `CURRENT_SCHEMA_VERSION = 6`,
could not read the v7 config the redeployed server held, and rendered no cards at
all. That reads as a broken window rather than a stale binary. `docs/config/v7.md`
now lists all three boundaries in order.

**OTA and memory.** Three OTA downloads (`durable1`, `2`, `3`) each installed and
rebooted in ~30 seconds, on a device that only re-checks firmware at boot.
Internal RAM stayed BYTE-FLAT across every build -- DIRAM 203,867, `.bss` 87,104,
`.data` 23,128, IRAM 16,384/16,384 with 0 remaining -- verified before and after
on the same tree rather than against a remembered number. That is because the
protocol context is `heap_caps_calloc`'d from PSRAM, so the decoder state it
gained costs no static RAM. Flat memory has now predicted a clean download five
times; it is still not a law, and the check stays.

**`PROTOCOL_CURRENT_CAPABILITIES` is now 2027** (bit 10, `DurableAssetEncoding`,
`+1024`). Bit 9 could not carry the promise: a deployed bit-9 build writes durable
wire bytes to flash verbatim, so compressing without a new bit would have stored
compressed bytes as pixels on every device in the fleet.

## Protocol v2 — OWED, not yet observed (branch `refactor/wave-c-protocol-v2`)

**Nothing in this section has been run on hardware.** It is the stub the Wave C plan asks
for: what a session must observe before protocol v2 is trusted, written down while the
reasons are fresh.

`refactor/wave-c-protocol-v2` was **merged to `main` on 2026-09-12** by owner direction,
ahead of this session. **It is not deployed, and the server must not be redeployed until
the board is flashed** -- the live binary must keep running its pre-v2 build until then,
because a v2 server cannot talk to the v1 firmware on `dev-0005` at all.

### Why an on-board check is mandatory here

Firmware statics moved. Measured before and after on the same tree:

| | Before (2026-09-05 cleanup) | After protocol v2 | Delta |
|---|---:|---:|---:|
| `.bss` | 87,104 | 87,000 | **−104** |
| `.data` | 23,128 | 23,128 | 0 |
| DIRAM total | 203,891 | 203,763 | −128 |
| IRAM | 16,384 / 16,384, 0 remaining | 16,384 / 16,384, 0 remaining | 0 |

**−104 bytes of `.bss` is within one byte of the ~105 bytes that broke OTA downloads in
`3f2aa03` with every test green.** That failure was a memory-layout shift, not a capacity
problem, and a shrink moves layout exactly as a growth does. Treat this as the same class
of hazard, and do not read "it got smaller" as reassurance.

Where the 104 bytes went: `protocol_device_event_t` lost its second 33-byte id array and
`carousel_binding_t` lost three of its six, so `device_event_queue_t` fell from 616 to 552
bytes (the host test prints the figure) and `protocol_task`'s binding shrank with it. The
per-template field registry (`core/template_fields.c`) and `widget_model`'s field state
went entirely.

### The rollout order is NOT the usual one, and getting it wrong bricks the link

Protocol v2 is **not additive**. A v2 server and a v1 device do not half-work: the frame
envelope's version byte makes every frame `VersionMismatch` in both directions. So the
device must be flashed **first**, over the cable, and the server redeployed second.
Between the two the device is unreachable from the live server — that window is expected,
not a fault.

1. `idf.py -C firmware flash` the v2 image over USB. **The version string is
   `v2.1.0-proto2`**, bumped from `v2.0.0-durable3` on 2026-09-12 precisely so this image
   is not published under a string an earlier, different image already used — the OTA
   refusal keys on the string, not on image content, so a reused one can never be
   corrected. `FirmwareCatalog::check` is string equality and offers its version in either
   direction, so the catalog will revert the board within a minute unless step 2 moves
   `DESKMATE_FIRMWARE_VERSION` to `v2.1.0-proto2` as well.
2. Redeploy the server (the merge already happened, 2026-09-12), with
   `DESKMATE_FIRMWARE_VERSION=v2.1.0-proto2` and the matching image in the firmware
   directory. A server built before this change speaks v1 and cannot talk to the flashed
   device at all -- which is also why the live binary must stay on its pre-v2 build until
   step 1 is done.
3. Verify the link comes back, then run the checks below.

### What the session must observe

- [ ] **The OTA download.** The check this whole section exists for. Publish a second v2
      image and watch it download, install and survive the rollback window.
- [ ] **Capabilities read 2016**, by name, with no unknown bits. Bits 0-4 are retired;
      a device still advertising them is running pre-v2 firmware.
- [ ] **A card face draws** at 270° and 90° — the scene path is unchanged, so this is a
      regression check, not new coverage.
- [ ] **A pomodoro counts down between pushes.** `PushTimer` replaced the field bag; the
      `timer.*` bindings must still tick locally with no host traffic.
- [ ] **A tap reports one card id.** `DeviceEvent` lost its second identifier; confirm a
      tap and a swipe both reach the host and act on the right card.
- [ ] **`framebuffer_diff` on target.** The last observed split was `96 total / 10
      excluded / 86 identical` (2026-09-06). That matrix is gone: Wave A retired the three
      orphan faces with the template oracle (−34 rows) and schema v9 removed the curated
      plugins (−18). The software inventory is now **44 rows**, pinned by
      `gate_b_inventory_is_the_real_counted_split_not_an_assumed_one`. Protocol v2 then
      closed the four `field.*` exclusions — there is no field namespace to fall outside
      of — leaving **one** exclusion, the `progress-ring--running-mid-countdown` pair,
      which is a push-to-capture timing race no wire change can fix. So the expected split
      is **`44 total / 2 excluded / 42 identical / 0 differing`**. That is a software
      prediction; run it fresh rather than trusting it.

Software gates that are green on this branch, and what they do not cover: the full
companion workspace set, `make -C firmware/host_tests clean test` and `sanitize`, and
`idf.py -C firmware build`. None of them can see an OTA layout failure. That is the whole
lesson of `3f2aa03`.

### The server half is DEPLOYED as of 2026-09-11 21:36 UTC — the board is now the missing half

Deployed by owner direction (*"deploy as well"*), ahead of the flash, which inverts the
order the section above prescribes. The consequence is the documented one and it is now
the live state: **`deskmate.rodi.one` speaks protocol v2 and `dev-0005` is still flashed
with v1, so the two cannot talk at all.**

What was deployed, from `31e9042` via `git archive HEAD` (never the working tree):

| | |
|---|---|
| binary | built in `rust:1.98-bookworm` on docker-vm, 17.4 s against the retained `target/` |
| rollback target | `/usr/local/bin/deskmate-server.bak-20260911T213610Z` |
| env backup | `/etc/deskmate/server.env.bak-20260911T213610Z` |
| `DESKMATE_FIRMWARE_VERSION` | `v2.0.0-durable3` → **`v2.1.0-proto2`** |
| published image | `/var/lib/deskmate/firmware/v2.1.0-proto2.bin`, sha256 `34484a79e8a9e8ae…`, byte-identical to the local build |
| verified | `GET /v1/firmware/v2.1.0-proto2.bin` → 200, 1,593,984 bytes through the tunnel; `/v1/device/link` → 401 unauthenticated |

**Observed, not inferred:** `dev-0005` was connected and reporting
`current=v2.0.0-durable3` at 21:26:04, the restart closed its link at 21:36:18, and it has
not reconnected since. Whether that is the protocol mismatch, a widening backoff, or the
board simply being powered off **cannot be told apart from the server side** — the log
records no link attempt at all, and a board at rest looks identical. Do not write this up
as a confirmed mismatch observation; confirm it at the board.

**Rollback, if the panel is needed before the flash session** — both halves, together:

```sh
ssh rodion@100.93.166.123
sudo -n cp -a /usr/local/bin/deskmate-server.bak-20260911T213610Z /usr/local/bin/deskmate-server
sudo -n cp -a /etc/deskmate/server.env.bak-20260911T213610Z /etc/deskmate/server.env
sudo -n systemctl restart deskmate-server
```

The env file must go back too: a pre-v2 binary with `DESKMATE_FIRMWARE_VERSION=v2.1.0-proto2`
would offer a v1 device an image it cannot run.

**What the flash session still owes is unchanged** — every checkbox above, with the OTA
download the one that matters. Step 2 of the rollout order is already done; the session is
now step 1 and steps 3-5.

---

## 2026-09-12 — the board is flashed and linking (server-side observation)

**Everything above this line about the flash session being owed, and about `dev-0005` not
having reconnected, is superseded.** The board is on protocol v2 and talking to the live
server.

Observed at 20:11 UTC while redeploying the server for the server-rendered card faces —
not as a hardware session, which is why the scope of what this proves is narrow:

| | |
| --- | --- |
| board firmware reported | `firmware check device_id=dev-0005 current=v2.1.0-proto2` |
| published pin | `DESKMATE_FIRMWARE_VERSION=v2.1.0-proto2` — matches, so nothing is offered in either direction |
| link | `device link established device_id=dev-0005`, 2.1 s after `systemctl restart` |
| version mismatch | none in the log, before or after the restart |
| server binary at the time | 18,415,416 bytes, built from `5d14cd4` |

### What this proves, and what it does not

It proves **the link**: the protocol v2 handshake completes, the board runs the published
image, and the host and device halves agree. A v1 board against this server would answer
every frame `VersionMismatch`, and none appears.

It proves nothing else, and this is the distinction the repo's rule about hardware claims
exists for — **this is server-side evidence, observed from a log, not from the board.**
Still owed, and now runnable for the first time since the flash:

- [ ] **The OTA download.** Mandatory, not a formality: `.bss` moved −104 bytes
      (87,104 → 87,000), within one byte of the shift that broke OTA downloads in
      `3f2aa03`. A board that links fine can still fail to pull an image.
- [ ] Capabilities reading **2016** by name at the board.
- [ ] A face at both mountings (90° and 270°).
- [ ] A pomodoro counting down between pushes.
- [ ] A tap reporting one card id.
- [ ] The framebuffer matrix — 44 rows / 2 excluded / 42 comparable. Last observed
      96/10/86 on 2026-09-06, which predates v9, Wave A and Wave C.

### Newly owed, from the same deploy

- [ ] **A server-rendered face on the panel.** The weather/RSS/token faces ride the
      picture card's durable-asset path, which is proven for a producer's PNG but has
      never carried a frame the server drew itself. Owed: one of each on `dev-0005` at
      both mountings, **plus the measured RLE565 transfer size per frame** — the
      flat-fill/no-gradient reasoning in `docs/images/server-rendered-cards.md` predicts
      ~10 KB against ~330 KB raw, and that number has never been measured for these
      faces.

### Rollback for the current binary

The pre-flash rollback block above is now wrong in one respect: the env file does **not**
need to go back, because the pin already matches the flashed image and did not move in
this deploy. The binary alone:

```sh
ssh rodion@100.93.166.123
sudo -n cp -a /usr/local/bin/deskmate-server.bak-20260912T201058Z /usr/local/bin/deskmate-server
sudo -n systemctl restart deskmate-server
```

That backup is the 2026-09-11 protocol-v2 binary (8,934,672 bytes), which speaks the same
wire to the same board — so this rollback is binary-only and does not disturb the fleet.

## V3 deployed to `deskmate.rodi.one` — 2026-09-15 14:46 UTC

V3 sub-projects 1-4 (`fc3e82a`, merge `fb15060`). **This changes nothing the device
sees**: V3 adds no config field, no wire message and no firmware byte, so the pending
flash is unaffected in either direction. `DESKMATE_FIRMWARE_VERSION` stays
`v2.1.0-proto2` and the env file was not edited.

| | |
|---|---|
| commit | `fc3e82a` via `git archive HEAD` (never the working tree) |
| binary | built in `rust:1.98-bookworm` on docker-vm, **41.4 s** against the retained `target/` |
| sha256 | `2b50d217699feee5d6e48676f68c7823c5360bfc5650d13e42ecf501db662851` |
| rollback binary | `/usr/local/bin/deskmate-server.bak-20260915T144613Z` (sha256 `f5ff2b70…`) |
| rollback env | `/etc/deskmate/server.env.bak-20260915T144613Z` (identical to the running one) |

Verified through the tunnel after restart:

| Route | Before | After |
|---|---|---|
| `GET /v1/manage` | 404 | **401** (exists, gated) |
| `GET /v1/manage/login` | 404 | **200**, renders |
| `POST /v1/integrations/{id}/token` | 404 | **401** |
| `GET /v1/device/link` | 401 | 401 |
| `POST /v1/images` | 401 | 401 |
| `GET /v1/firmware/v2.1.0-proto2.bin` | 200, 1,593,984 B | 200, 1,593,984 B |

The dashboard renders real state: `dev-0001`..`dev-0005`, all **Not connected**, no
integrations, no picture sources. A board at rest is the resting state, not a fault.

### The rollback block in the section above was two deploys stale — corrected here

That block names `deskmate-server.bak-20260911T213610Z` as the rollback target. It is not
the binary that was running. **An undeployed-by-this-record deploy happened
2026-09-12T20:10:58Z**, leaving `deskmate-server.bak-20260912T201058Z`, and the binary
actually running until today was `f5ff2b70…`, which matches neither. Its provenance is
not recorded anywhere and could not be reconstructed from the VM — the build tree is a
`git archive` export with no commit marker. **Identified by this merge:** that deploy was
`5d14cd4` from `feat/server-side-cards`, whose own record two sections above states the
same timestamp and an 18,415,416-byte binary. It was replaced by the V3 deploy below, so
the server-rendered faces have not been live since 2026-09-15 14:46 UTC. **Whoever deploys next: read the live
`sha256sum` rather than trusting a rollback block, and write the commit down.**

Also observed in the journal: `device link established device_id=dev-0005` at
2026-09-12T20:11:03Z, followed by `device link idle timeout, closing` 54 s later. That is
consistent with a v1 device opening the WebSocket and then failing every frame on
`VersionMismatch` — the link is established before any protocol frame is exchanged — so
it is **not** evidence the board is flashed. Confirm at the board.

### One thing that does not work on this deployment

**Browser form login returns 503 while no Google OAuth client is configured**, so
`/v1/manage` is reachable today only by sending `Authorization: Bearer <admin token>`
(curl works; a browser cannot sign in). Observed live: correct token -> 503, wrong token
-> 401, so the token check runs first and nothing leaks.

The cause is a seam, not a bug in the page: the session signer lives on
`IntegrationRuntime` (sub-project 2), and `main.rs` only constructs that when
`DESKMATE_GOOGLE_CLIENT_ID` is set. Setting the Google client config fixes it with no
code change. Fixing it properly means moving `SessionSigner` onto `ServerState` so the
operator session does not depend on an integration existing.

---

## Server-rendered card faces deployed — 2026-09-15 22:32 UTC

`feat/server-side-cards` merged to `main` as `a1e5a5f` and deployed. **This changes
nothing the device sees**: schema stays v10, the wire is untouched, firmware is untouched,
and no capability moves. `DESKMATE_FIRMWARE_VERSION` stays `v2.1.0-proto2` and the env
file was not edited, so **rollback is binary-only**.

| | |
|---|---|
| commit | `a1e5a5f` via `git archive HEAD` (never the working tree) |
| binary | built in `rust:1.98-bookworm` on docker-vm, **26.42 s** against the retained `target/` |
| size | 19,088,472 bytes |
| sha256 | `1672065a3ebda88efa2ff6c6ef6f3d923fd6e547c8e7b383793b2c2daf01945f` |
| rollback binary | `/usr/local/bin/deskmate-server.bak-20260915T223212Z` (sha256 `2b50d217…`, the V3 build from `fc3e82a`) |
| rollback env | none needed — the env file did not move |

Two image sources were minted and a spec file written to
`/var/lib/deskmate/configs/data-cards.json` (0600, `deskmate-server`), so this deploy is
**not** behaviour-neutral the way the branch's own record anticipated:

| source_id | face | refresh |
|---|---|---|
| `image-a9773eb40f03457f37a9b6d8` | `weather`, Dubai, metric | 900 s |
| `image-8e361aaf9eb90ddd8d4b685e` | `rss`, Hacker News | 900 s |

Both refreshers spawned on start and pushed a frame within 1.3 s of the restart, with
zero `error`/`warn`/`panic` lines in the journal since:

```
INFO server::data_cards: server-rendered card refreshing source_id=image-a9773eb40f03457f37a9b6d8 kind="weather" refresh_seconds=900
INFO server::data_cards: server-rendered card refreshing source_id=image-8e361aaf9eb90ddd8d4b685e kind="rss"     refresh_seconds=900
```

Frames landed at **329,740 bytes** each — `448 x 368 x 2` plus the 12-byte LVGL header,
byte-identical in size to the working `Claude limits` card, which is the evidence the
rasterizer produced a full frame rather than a partial one.

`dev-0005`'s stored config was migrated v7 -> v10 and gained two picture cards naming
those sources (backup: `dev-0005.json.bak-20260915T223846Z`). Note that
`POST /v1/images` alone is NOT enough: `config.rs:413` requires a picture card's
`source_id` to also appear in that document's own `image_sources`, so the source must be
minted **and** listed.

### Still owed — nothing here was seen on the panel

**The board was powered off for all of this.** Every observation above is server-side.
What the branch's own record already owed remains owed, and this deploy adds nothing to
the evidence:

- A server-rendered face **on the panel**, at either mounting.
- **The measured RLE565 transfer size.** `docs/images/server-rendered-cards.md` predicts
  ~10 KB against the ~330 KB raw frame on the flat-fill argument. The stored frame is the
  raw 329,740 bytes; the compression happens on the wire and has never been measured.
- The OTA download against the -104-byte `.bss` shift, which is unrelated to this deploy
  but is still the check that matters most.

---

## Schema v11 deployed — first-class weather/RSS/token cards, 2026-09-16 12:03 UTC

`e2b103c`. The owner rejected the interim shape: three tiles reading
`PICTURE / PNG` above a dropdown that re-points a card at another source. Weather,
RSS and token are now card kinds with their own settings, and `data-cards.json` is
gone. **Schema bump, so the order was server binary -> `Deskmate.app` -> save.**
No wire change, no firmware change, no capability moves;
`DESKMATE_FIRMWARE_VERSION` stays `v2.1.0-proto2` and the env file was not edited,
so **rollback is binary-only**.

| | |
|---|---|
| commit | `e2b103c` via `git archive HEAD` |
| binary | `rust:1.98-bookworm` on docker-vm, **25.45 s** against the retained `target/` |
| size | 19,211,032 bytes |
| sha256 | `fa7486d7a35c1ae85a588269c5b5f172f050d41ac2b805fdd717b5b9715e4727` |
| rollback binary | `/usr/local/bin/deskmate-server.bak-20260916T120328Z` (`1672065a…`, the v10 build from `a1e5a5f`) |
| retired | `data-cards.json` -> `data-cards.json.retired-20260916T120328Z` (inert: `load_specs` is deleted) |
| Mac app | rebuilt and installed from the same commit, 16,959,568 bytes |

### The observation that matters: a config PUT re-derives the refreshers

The server restarted at **12:03:28** and spawned NO refreshers, which is correct --
`dev-0005` still held v10 picture cards and the spec file was retired. The
`PUT /v1/devices/dev-0005/config` at **12:05:15** spawned both, two minutes after
the restart:

```
INFO server::data_cards: server-rendered card refreshing source_id=card-weather     kind="weather" refresh_seconds=900
INFO server::data_cards: server-rendered card refreshing source_id=card-hacker-news kind="rss"     refresh_seconds=900
```

That gap is the evidence that an edit in the companion takes effect without a
process restart, and that `replace_data_card_refreshers` aborts the previous set.
The derived sources `card-weather` and `card-hacker-news` were created
server-owned and tokenless, and each pushed a **329,740-byte** frame within a
second. Zero `error`/`warn`/`panic` lines since. Frame files are named
`card-<sha256>.bin`: source ids are hashed into filenames so one containing path
characters cannot escape the frame directory.

The v10 -> v11 migration ran on the real stored document (`origin=MigratedV10`),
not a fixture.

### A migration consequence worth remembering

v11 is additive, so a picture card previously fed by `data-cards.json` SURVIVES
the migration with nothing refreshing it -- it keeps its last frame forever and
looks fine. Both interim cards were replaced with real `weather`/`rss` cards by
hand here. Their minted sources (`image-a9773eb4…`, `image-8e361aaf…`) are now
orphaned in `image-sources.json`; they hold no card reference and are harmless,
but they are the residue to clear if that file is ever tidied.

### Still owed — nothing was seen on the panel

**The board was powered off throughout.** Every observation above is server-side.
A server-rendered face on the panel at either mounting, and the predicted ~10 KB
RLE565 transfer size against the 329,740-byte raw frame, both remain unmeasured,
as does the OTA download against the -104-byte `.bss` shift.

---

## Schema v11 REVERTED — 2026-09-17 08:09 UTC

The section above records a deploy that no longer stands. `e2b103c` was reverted as
`9b20b53` on owner direction: it answered a request about what the settings window looked
like with a schema bump, six card kinds and an ordered two-stage deploy, against a rule
the repo already stated in `324c981`, `e137294` and `CLAUDE.md`. The rule now sits in
"Product rules the owner has set" as a prohibition, with the cost attached.

**`faces/weather.rs`, `faces/rss.rs`, `faces/token.rs` and the rasterizer are untouched by
the revert.** They came from `c637295` and v11 never edited them. The weather face design
is owner-approved and settled.

| | |
|---|---|
| binary now running | `1672065a3ebda88e…` (the v10 build from `a1e5a5f`), restored from `deskmate-server.bak-20260916T120328Z` |
| `data-cards.json` | restored from `data-cards.json.retired-20260916T120328Z` |
| `dev-0005.json` | restored from `dev-0005.json.bak-20260916T120515Z` (schema 10, four cards) |
| v11 state kept | `dev-0005.json.v11-*`, `image-sources.json.v11-*` |

### The rollback was NOT binary-only, and the claim that it would be was wrong

Restoring the binary alone put the server in a restart loop:

```
panicked at crates/server/src/lib.rs:139: failed to load the image-source store:
Io { message: "metadata contains an invalid source id" }
```

v11 derived its source ids as `card-{card_id}` and wrote them into
`configs/image-sources.json`. **v10 rejects that id shape**, so the older binary could not
open its own store. Recovery was to delete the two `card-*` entries and their frame files.

**This is a class of risk that is not on the list of expensive boundaries.**
`CURRENT_SCHEMA_VERSION`, the wire and firmware statics are all versioned and negotiated.
`image-sources.json` carries a `schema_version` but nothing validates a *new* version's
ids against an *older* reader, and the file is shared state that both binaries open at
startup and panic on. **Before claiming a rollback is binary-only, check what the new
version writes into unversioned or laxly-versioned state that the old one must read.**

### `dev-0005` is powered on and linked

`device link established device_id=dev-0005` at 08:09:55 UTC, on the v10 binary. The board
being up is unusual -- it is normally off -- and the long-owed hardware checks are
runnable right now: a server-rendered face on the panel at either mounting, the measured
RLE565 transfer size against the 329,740-byte raw frame, the capability word read by name,
and the OTA download against the -104-byte `.bss` shift. **None of them has been run**;
this note records only that the link came up.


## Serial session subtraction — 2026-09-19

`companion/crates/app-core/examples/alert_replay_check.rs` is retired on owner
instruction. It was the only remaining consumer of the long-lived serial runtime
capabilities, verified the desktop serial host deleted in `18a9779` / `64a25aa`,
and is not among CLAUDE.md's "Hardware verification still owed". Its 2026-08-15
PASS and the unperformed true-power-loss variant remain historical observations.

Removed with it: serial acknowledged-state replay, event routing and its bounded
queue, keepalive heartbeats, serial diagnostic counters, automatic timer/config
revision allocation, and the unused time-sync, timer/config, scene, asset,
interrupt and activation methods on `DeviceSession`. The minimal reconnect
transport handoff and idle-read loop remain because removing them would invalidate
the unchanged reconnect-stall and idle-blocked Drop regressions. Reconnect no
longer replays state. Shared replay, event and diagnostic types used by the server
remain intact, as do the other hardware examples.

The CLI's cable path is unchanged code: status, time-sync and push-timer still use
`DeviceClient`; provision and factory-reset still use `connect_session` and the
existing worker. Its bounded reply wait, sticky stalled-session marking,
`Transport(Disconnected)` on a missed reply and bounded, non-joining Drop on a
blocked worker remain. FakeTransport characterization passed before subtraction
and covers all five messages, partial I/O, response correlation, ACK validation,
capability refusal before writes, and unsolicited events around replies.

NO hardware verification of this change was observed (the board was off).
The next cable session should run `deskmate-cli status` and one provision dry-run
before trusting it. There is currently no `provision --dry-run` option; the exact
non-mutating dry-run procedure still needs to be settled before that check. The
initial status handshake in `connect_session` remains synchronous and outside the
worker's stall protection; this subtraction does not make connection establishment
bounded.

### 2026-09-19 — the last pre-v10 configs leave the live server (server-side, no hardware)

Config schema versions v4-v9 stopped being readable today (`refactor!: config schema v10 is
the only readable version`). Before that binary shipped, the only two documents still at
those versions were removed from docker-vm, on the owner's authorization.

Observed over SSH at 14:44 UTC, with the pre-retirement binary still running:

- `/var/lib/private/deskmate/configs/dev-0003.json` was `schema_version` 5 (cards: clock,
  pomodoro, calendar) and `dev-0004.json` was `schema_version` 4 (clock, weather, pomodoro).
  Both belong to identities that have not linked since August. They were MOVED, not deleted,
  to `/var/lib/private/deskmate/retired-configs-20260919/` (mode 0700, same owner).
- `dev-0005.json` -- the panel on the desk -- was `schema_version` 10 and its SHA-256 was
  identical before and after (`e683b3d614f9257c…`). The service was not restarted; it reads
  configs lazily. `GET /` answered 200 afterwards.
- The identities `dev-0003` and `dev-0004` are STILL in `device-identities.json`. There is no
  retirement route, and hand-editing that registry (sequence counter, digests, recovery
  path) was not worth the risk for two entries that cannot link to anything harmful: if one
  ever did, it would get the safe fallback config. Retiring them is a separate decision.

To restore one: move its file back. The current binary will refuse it as
`UnsupportedVersion` (visibly, and without overwriting it); only a pre-2026-09-19 binary can
migrate it.

## 2026-09-23 -- tap to face (track C1, phase 1) deployed; the tap itself is NOT yet observed

The server, the browser companion and the faces were deployed from
`track-c1-tap-to-face` at 10:11 UTC. What follows is **server-side evidence only**, in the
sense `CLAUDE.md` means it: a frame in the store is not the panel, and no finger has
touched the glass yet.

Observed:

- **The new lowering did not disturb the board.** Every picture card's wire `CardConfig`
  changed with this build (all of them now lower to `StartPause`, which is the only way
  protocol v2 reports a tap at all). The link closed with the old binary at 10:11:12 and was
  established again at 10:11:15 -- three seconds -- and no `StaleRevision`, no
  `VersionMismatch` and no refusal of any kind appears in the unit's log since.
- **The envelope reaches the real faces package.** The weather face refreshed at 10:11:13
  through the new `{png, state}` path and its frame came back **digest-unchanged**
  (`the face is unchanged`), which is the strongest available proof that base64 round-trips
  byte-for-byte: the same face drawn by the old bare-PNG path produced the same digest.
- **`dev-0005` is linked**, Wi-Fi connected at -70 dBm, IP 192.168.8.168, update idle,
  mounting "landscape flipped" (270 degrees), reported by the live companion.
- **The live window states the tap.** The weather card's editor reads "Tap the panel for the
  coming days." where it previously read "Tapping this card does nothing." That sentence is
  the face's own, carried from `describe` through `FaceDescriptor`.

**Still owed on the board, and not claimable without it:**

- A **tap on the panel** doing anything at all. This is the whole point of the track and it
  needs a person at the desk. The weather card on `dev-0005` is a weather face, so it is the
  card to tap: one tap should redraw it as tomorrow plus the four days after, and a second
  tap should return it to the current conditions.
- The same at **both mountings** (the board is currently at 270).
- The **tap-to-redraw latency**, wall clock. Nobody has measured it. A weather frame is about
  32 KB / 17 chunks; Hacker News is 61 KB / 33 and would be the slower case.
- **Hacker News paging on the panel.** The card named "Hacker News" in the live loop is an
  *RSS* face pointed at the Hacker News feed, not the `hackernews` face, so it does not take
  taps. Exercising the paging on hardware needs a `hackernews` picture card added to the
  loop, which is the owner's to add.

## 2026-09-24 -- the first tap on the glass: it works once, then the link falls over

The owner tapped the weather card on `dev-0005` (270 degrees, firmware `v2.1.0-proto2`).
**This is the first tap ever observed on hardware, and it found a defect.** Reported from the
desk: nothing happened for a few seconds, so they tapped several more times; the panel fell
back to the standalone clock, then showed the tomorrow view, then went back to the clock,
went back and forth, and then **the device rebooted**.

Server-side timeline, from `deskmate-server`'s journal and `face-state.json` (UTC):

| time | event |
|---|---|
| 13:02:28 | `device link established dev-0005`, firmware check `v2.1.0-proto2` |
| 13:08:07.273 | `face-state.json` written `{"view":"days","tappedAt":"...13:08:07.273Z"}` -- a tap-driven render was accepted |
| 13:08:32.253 | WARN `the frame is stored but the device was not notified` ... `runtime command response timed out` |
| 13:08:32.978 | the same warning again, 0.725 s later |
| 13:08:50.375 | WARN `device link refused: owner already live` |
| 13:08:52.290 | the same refusal again |
| 13:08:52.943 | `device link closed` -- the stale session finally released |
| 13:08:55.618 | `device link established` (the rebooted device) |
| 13:08:57.302 | firmware check, `v2.1.0-proto2` |

Two measurements reframe that timeline, and both were taken off the board:

- **A weather render costs 0.66 s** end to end on the VM -- `bun` start, the Open-Meteo
  fetch and the resvg raster included, measured three times at 0.66 / 0.68 / 0.64 s for a
  23,801-byte envelope. So the seconds the owner waited are **not** the face, and two renders
  finishing 0.725 s apart needs no hidden concurrency: the refresher loop is sequential and
  simply ran twice.
- **`ImageSourceUpdated` carries the 25-second `SYNCHRONIZING_COMMAND_TIMEOUT`**, not the
  short budget. Both warnings are therefore a *full budget* expiring: the notifications were
  submitted at about 13:08:07.25 and 13:08:07.98 and timed out 25 s later. The device stopped
  answering the moment the first tap-driven push began.

What that makes true, and what it does not:

- **A tap does reach the face and does change what is drawn.** The state file flipping to
  `"days"` proves the whole seam -- touch, firmware, wire, runtime, sink, coalescer, the
  faces subprocess, the state store. That much of C1 phase 1 is real.
- **The tap is NOT what broke the push.** That was the first reading and it was wrong; this
  correction is the most important line in the entry. The same unit's journal across the
  whole day shows the identical failure with **no taps involved**: the Hacker News source
  (`image-8e361aa...`, the biggest frame at about 61 KB / 33 chunks) went un-notified at
  13:28:09, 15:28:16 and 15:43:17, and the token source failed at 15:57:54 with
  `device: AssetCommit ... failed: device request timed out`. Per-day counts of
  push-timeouts / link-closes / `owner already live`, remembering that the board is normally
  off so a zero may only mean it was unplugged: 09-20 `0/1/0`, 09-21 `2/4/2`, 09-22 `0/0/0`,
  09-23 `1/3/1`, 09-24 `6/8/5`. **09-21 predates the tap feature**, so the mechanism is older
  than Track C1. A tap is merely the first time anyone stands at the desk waiting for a push,
  which is why the tap got the blame.
- **A second tap's frame still cannot be delivered while the first push is in flight.** That
  part is a real weakness of the tap design: `notify_image_source_changed` is fire-and-forget
  (`spawn_blocking`) and `ImageSourceUpdated` carries the 25 s `SYNCHRONIZING_COMMAND_TIMEOUT`,
  so a second notification queues behind the first at the runtime worker. Robustness defect,
  not the cause of anything observed here.
- **A rebooted device is locked out of its own link for about 4.5 s.** Its first two
  reconnect attempts were refused `owner already live` because the server still held the
  previous session's lease; the lease was released at 13:08:52.943 and the link came back at
  13:08:55.618. Not tap-specific, and it recurred at 15:03 and 16:26 the same day.
- **Why the device rebooted is NOT observed and must not be claimed.** The server cannot see
  it, the shipping console is UART0-only, and attaching USB serial reboots the board, so that
  boot's crash is gone.

What the device's own counters say, read from the admin snapshot afterwards (`/v1/devices/dev-0005`):

- `crc_errors: 0`, `malformed_frames: 0`, `dropped_events: 0`, `dropped_responses: 0`,
  `overflow_frames: 0`, `rx_dropped_bytes: 0`, `valid_frames: 143` -- **nothing is corrupting
  or overflowing.** Whatever fails, fails as a stall, not as bad bytes.
- `free_heap: 8358839` -- about 8 MB free. **Heap exhaustion is not indicated**, which is the
  first hypothesis this entry originally reached for.
- `host_reconnects: 9`, and the runtime's last state is `error: device request timed out`.
- The host-side `command_queue_full: 0`, so nothing backed up in the command channel.

**The strongest remaining hypothesis, and it is still only that:** asset transfers to this
board stall on the network path. `dev-0005` reaches the server at
`wss://deskmate.rodi.one/v1/device/link` -- Wi-Fi, router, the public internet, Cloudflare,
cloudflared, Caddy -- **even though the board and the VM share a LAN**. The largest asset
fails most often, which is what a per-chunk stall predicts and what neither a heap nor a
protocol fault predicts. `CLAUDE.md` already flagged that "whether the tunnel carries the
Hacker News face's 33 chunks is not known"; it often does not. Confirming this needs
per-chunk reply latency, which is a server-side instrumentation change and needs no board
and no flash.

Also found while reading this path, unrelated to the incident: **`taps_dropped` never reaches
the admin snapshot.** It has a tested accessor (`RuntimeHandle::taps_dropped`), so it is not
write-only, but `RuntimeDiagnosticCounters::snapshot()` does not copy it into the public
`RuntimeDiagnostics` the way it copies `interrupt_dismissals_ignored`. The consequence is that
nothing outside the process can read it -- not `/v1/devices/{id}`, not the companion -- which
is half of the "a declined tap and an event that never arrived look identical" problem its
sibling counter's doc comment describes.

### What changed in response, and what deliberately did not

Shipped the same day, all server-side, no wire and no firmware:

- **Per-transfer asset instrumentation** (`runtime_device.rs`). Every completed push now logs
  one line -- `chunks`, `bytes`, `slowest_chunk_ms`, `commit_ms`, `total_ms` -- and a chunk
  slower than 250 ms gets its own. A failed chunk reports how many chunks had already landed
  and how long the transfer had been open. `slowest_chunk` rather than a mean is the whole
  point: one chunk past the 2 s `REQUEST_TIMEOUT` fails the push, and a mean of 33 hides it.
  The accumulation lives in the server, not in `app-core`, because `app-core` has no `tracing`
  dependency and should not gain one.
- **`taps_dropped` now travels in `RuntimeDiagnostics`**, so `/v1/devices/{id}` and the
  companion can read it. Needed the browser contract fixture regenerated.

**Deliberately NOT changed, with the numbers that decided it:**

- **The `owner already live` refusal stays.** `PING_INTERVAL` is 3 s, `IDLE_TIMEOUT` is 30 s
  (`DEVICE_NETWORK_LINK_TIMEOUT` 45 s minus a 15 s margin), and the device retries about every
  2 s, so an unclean reboot costs one or two refusals and the link is back in about 5 s -- as
  observed, 13:08:50 first refusal to 13:08:55 established. Making a new authenticated link
  evict the old session would shave those 5 s off, at the cost of letting a flapping network
  thrash sessions, and it would be treating the symptom of a failure whose cause is still
  unmeasured. `runtime_device.rs` already has a test
  (`the_server_releases_a_dead_link_before_the_device_redials`) pinning the ordering that
  handles the *idle* case; the reboot case is simply faster than a ping.
- **The tap-push interlock is not built, and on inspection should not be.** The idea was to
  make the refresher await its `ImageSourceUpdated` before rendering again, so a burst of taps
  could not queue renders behind a notification that may sit for the full 25 s. Working out
  what it would actually buy killed it. Today a tap renders at once and the frame lands in the
  store, so the tap's intent is recorded promptly and whichever synchronize next succeeds
  carries the *newest* view to the panel. An interlock would delay that by up to 25 s -- it
  would make a tap during a stuck push do nothing at all for the length of the stall. What
  today's code wastes is a render (0.66 s of CPU) and one Open-Meteo call per redundant tap;
  what the interlock would cost is the responsiveness the whole track exists to add. The
  coalescing that already happens while the loop is parked in its `select!` covers the case
  that matters.

  Recorded because a first pass here claimed the interlock was merely **untestable**, which was
  simply false and would have misled the next reader: `RuntimeHandle::start_with_ports` takes a
  `Box<dyn RuntimeDevice>` and that trait is public, so the server's own unit tests can drive a
  runtime over a deliberately slow fake device. The seam exists. The reason not to use it here
  is that the change is not wanted, not that it cannot be checked.

Still owed, unchanged by this session: the same test at the other mounting, a tap-to-redraw
latency measured on a build whose push path is trusted, and Hacker News paging (the live
"Hacker News" card is an RSS face and takes no taps).

## 2026-09-26 -- a tap, filmed and correlated: it works, and the wire is not the problem

The owner filmed a single tap on the weather card (30 s, 720x1280, `dev-0005` at 270 degrees)
and the new instrumentation caught the same event server-side. This is the first
tap-to-redraw ever measured. **Nothing failed**: no link drop, no timeout, no reboot. It was
simply slow -- **13.3 seconds from finger to new picture**.

Video time was anchored to the panel's own clock: the minute flips to 15:12 between video
t=15.6 s and t=15.7 s, so **t=0 = 15:11:44.35 UTC**, +/- 0.1 s. (The panel reads UTC because
`preferences.timezone` is `UTC` in the live config -- correct behaviour, not a fault; the
value is seeded on first run and never rewritten.)

| video t | UTC | event | source |
|---|---|---|---|
| 5.0-5.5 | 15:11:49.4-49.9 | finger on the glass | video |
| 7.25 | 15:11:51.604 | `face-state.json` -> `view: days`; frame rendered and accepted | state file |
| ~8.0 | ~15:11:52.4 | asset transfer opens | derived from `total_ms` |
| 9.0-9.5 | 15:11:53.4-53.9 | **panel drops to the clock face** | video |
| 10.88 | 15:11:55.226 | transfer completed: 17 chunks, 32,224 B, slowest chunk **84 ms**, commit **1,540 ms**, total 2,826 ms | journal |
| 18.8 | 15:12:03.15 | **new picture drawn** (tomorrow, 23 degrees, Overcast) | video |
| 20.38 | 15:12:04.726 | `AssetRelease` acked: **9,359 ms** | journal |

### Where the 13.3 seconds actually goes

| phase | cost | share |
|---|---|---|
| tap event + face render (0.66 s of it is the face) | 1.75 s | 13% |
| chunk phase on the wire (17 chunks, ~76 ms each) | ~1.29 s | **10%** |
| `AssetCommit` (device writes the asset to flash) | 1.54 s | 12% |
| `AssetRelease` (device compacts the flash blob region) | ~7.9 s visible | **59%** |

**The network is under 10% of it.** The Cloudflare-tunnel hypothesis from 2026-09-24 is
refuted by its own instrument: the 33-chunk Hacker News frame in the same session delivered
all 62,752 bytes in 4,477 ms with a worst chunk of 569 ms, and typical chunks run 83-234 ms.
Whatever is wrong with this board's pushes, **it is not the wire**, and the earlier entry's
"strongest remaining hypothesis" should be read as closed.

### The clock face is not a lost link

The panel shows the standalone clock from 15:11:53.6 to 15:12:03.15 -- **9.55 s**, which
tracks the 9.36 s release almost exactly (offset by ~1.7 s, the release's own round trip).
This is what the owner described on 2026-09-24 as the device "switching into autonomous
mode", and it is not a disconnection at all: **the panel falls back to the clock for the
duration of the device's flash work on every asset change.** It looked like a dropped link
because it lasts ten seconds.

### The lever, and a field nobody reads

`asset_sync.rs`'s `reconcile` sends `AssetRelease` **unconditionally at the end of every
pass**, including passes where `already_present` short-circuited every transfer. Each one
costs 9-10 s of device-side compaction. For a tap the release is doing real work -- the old
weather digest really is being dropped -- so this is not simply a redundant call to delete;
taking it out of the interactive path means deferring the garbage collection, not skipping
it, and that needs a policy and a headroom number.

The headroom number may already be on the wire and ignored. `encode_status_payload` in
`firmware/main/core/protocol_message.c:2086` puts `asset_store_used_bytes`,
`asset_store_free_bytes` and `asset_count` in **key 31** of every `StatusResponse`. Nothing in
`companion/crates/` mentions any of the three, and `docs/protocol/v2.md` does not describe
key 31. Decoding it would be purely additive on the host -- the device is already sending it.
