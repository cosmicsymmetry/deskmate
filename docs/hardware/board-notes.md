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
