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
| `BOARD_I2C_PIN_SDA` | `GPIO_NUM_15` | `ESP32_SDA` | Schematic (GPIO15, shared bus net `ESP32_SDA`) = BSP `BSP_I2C_SDA` |
| `BOARD_I2C_PIN_SCL` | `GPIO_NUM_14` | `ESP32_SCL` | Schematic (GPIO14, shared bus net `ESP32_SCL`) = BSP `BSP_I2C_SCL` |
| `BOARD_TOUCH_PIN_INT` | `GPIO_NUM_21` | `TP_INT` | Schematic (GPIO21 net `NLTP0INT`) = BSP `BSP_LCD_TOUCH_INT` |
| `BOARD_TOUCH_PIN_RST` | `GPIO_NUM_NC` | `TP_RESET` | Schematic shows `TP_RESET` tied to `EXIO2` (TCA9554 IO-expander port P2), **not** a direct ESP32 GPIO. BSP agrees: `BSP_LCD_TOUCH_RST = GPIO_NUM_NC`. |

Every LCD/touch/I2C pin above was cross-checked two ways — schematic net trace *and*
Waveshare's own shipping BSP source — and both agreed in every case. No pin-level
discrepancy was found between the schematic and the demo code for this board revision.

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
pipeline is functioning end-to-end; **actual on-screen visual confirmation (dark-blue
background, centered white "deskmate M0" text, no tearing/garbage) still requires a
human looking at the physical panel** and was not and cannot be verified by this agent.
