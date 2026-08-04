# Deskmate M0 — Hardware Bring-up + Walking Skeleton Implementation Plan

**Status:** Complete. Hardware-verified, documented, and tagged `m0` at commit `b77a894`;
the final review fixes landed in `54bc97b`.

**Historical correction:** The initial task text below names the v1 FT3168 touch
controller. Bring-up established that the target v2 board uses CST820 silicon through
the CST816S driver family at `0x15`. The completed code and `docs/hardware/board-notes.md`
are authoritative; do not reintroduce the FT3168 path for this board.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A from-scratch ESP-IDF firmware for the Waveshare ESP32-S3 Touch AMOLED 1.8" v2 that boots to a ticking standalone clock, with touch, brightness, and 180° rotation proven working.

**Architecture:** Clean board layer (`main/board/`) owning pins/panel/touch init, hardware-independent code in `main/core/` (host-testable), LVGL 9 via Espressif's `esp_lvgl_port` doing render loop plumbing. No code reuse from any prior project.

**Tech Stack:** ESP-IDF ≥5.3, LVGL 9.x, `esp_lvgl_port`, Espressif component registry drivers for CO5300/SH8601 panel + FT3168 touch.

## Global Constraints

- Firmware is **from scratch** — do not copy code from `~/dev/rsvpnano` or any other project. Reading the Waveshare wiki/demo for *facts* (pins, init sequences) is allowed and expected.
- Target board: Waveshare ESP32-S3 Touch AMOLED **1.8" v2** — CO5300 panel, 368×448, FT3168 touch, ESP32-S3R8 (8 MB octal PSRAM), 16 MB flash.
- Wiki (source of truth for pins): https://www.waveshare.com/wiki/ESP32-S3-Touch-AMOLED-1.8
- Hardware-independent logic lives in `main/core/` and must compile on host (plain C, no ESP-IDF includes).
- Every commit message uses conventional-commit prefixes (`feat:`, `chore:`, `test:`, `docs:`).
- Hardware bring-up tasks verify on-device (no meaningful unit tests exist for panel init); host-testable code (Task 6's time formatting) gets real TDD. This is the agreed testing posture for M0 only.

---

### Task 1: ESP-IDF project scaffold

**Files:**
- Create: `firmware/CMakeLists.txt`
- Create: `firmware/sdkconfig.defaults`
- Create: `firmware/main/CMakeLists.txt`
- Create: `firmware/main/idf_component.yml`
- Create: `firmware/main/main.c`
- Create: `.gitignore`
- Create: `README.md`

**Interfaces:**
- Consumes: nothing (first task).
- Produces: a building, flashable ESP-IDF project; `app_main()` in `main.c` that later tasks extend; `idf.py build|flash|monitor` workflow documented in README.

- [x] **Step 1: Verify ESP-IDF ≥5.3 is installed and exported**

Run: `idf.py --version`
Expected: `ESP-IDF v5.3` or newer. If missing, install per https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/get-started/ (git clone + `./install.sh esp32s3` + `. ./export.sh`) and record the chosen version in README.

- [x] **Step 2: Create project files**

`firmware/CMakeLists.txt`:
```cmake
cmake_minimum_required(VERSION 3.16)
include($ENV{IDF_PATH}/tools/cmake/project.cmake)
project(deskmate)
```

`firmware/main/CMakeLists.txt`:
```cmake
idf_component_register(
    SRCS "main.c"
    INCLUDE_DIRS "."
)
```

`firmware/main/idf_component.yml`:
```yaml
dependencies:
  idf: ">=5.3"
```

`firmware/main/main.c`:
```c
#include <stdio.h>
#include "esp_log.h"

static const char *TAG = "deskmate";

void app_main(void)
{
    ESP_LOGI(TAG, "deskmate M0 boot");
}
```

`firmware/sdkconfig.defaults`:
```
CONFIG_IDF_TARGET="esp32s3"
CONFIG_ESPTOOLPY_FLASHSIZE_16MB=y
CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ_240=y
CONFIG_SPIRAM=y
CONFIG_SPIRAM_MODE_OCT=y
CONFIG_SPIRAM_SPEED_80M=y
CONFIG_COMPILER_OPTIMIZATION_PERF=y
CONFIG_ESP_MAIN_TASK_STACK_SIZE=8192
```

`.gitignore` (repo root):
```
firmware/build/
firmware/sdkconfig
firmware/sdkconfig.old
firmware/managed_components/
.vscode/
.DS_Store
```

`README.md` (repo root):
```markdown
# deskmate

Monitor-clip AMOLED desk display. Spec: docs/superpowers/specs/2026-08-03-deskmate-design.md

## Firmware build

    cd firmware
    idf.py set-target esp32s3
    idf.py build
    idf.py -p /dev/cu.usbmodem* flash monitor

Requires ESP-IDF >= 5.3 exported in the shell.
```

- [x] **Step 3: Build and flash the blank app**

Run: `cd firmware && idf.py set-target esp32s3 && idf.py build && idf.py flash monitor`
Expected: build succeeds; monitor shows `deskmate M0 boot`. (Board connects via its USB-C port; it enumerates as `/dev/cu.usbmodem*`.)

- [x] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: scaffold ESP-IDF firmware project for ESP32-S3"
```

---

### Task 2: Board pin map from the Waveshare wiki

**Files:**
- Create: `firmware/main/board/board.h`
- Create: `docs/hardware/board-notes.md`
- Modify: `firmware/main/CMakeLists.txt` (add `board` include dir)

**Interfaces:**
- Consumes: Waveshare wiki + downloadable demo/schematic for ESP32-S3-Touch-AMOLED-1.8.
- Produces: `board.h` pin constants used by every later task: `BOARD_LCD_H_RES`, `BOARD_LCD_V_RES`, `BOARD_LCD_QSPI_HOST`, `BOARD_LCD_PIN_PCLK`, `BOARD_LCD_PIN_CS`, `BOARD_LCD_PIN_D0..D3`, `BOARD_LCD_PIN_RST`, `BOARD_I2C_PORT`, `BOARD_I2C_PIN_SDA`, `BOARD_I2C_PIN_SCL`, `BOARD_TOUCH_PIN_INT`, `BOARD_TOUCH_PIN_RST` (use `GPIO_NUM_NC` where the board ties a line to the IO expander or leaves it unconnected).

- [x] **Step 1: Extract facts from the wiki**

Fetch https://www.waveshare.com/wiki/ESP32-S3-Touch-AMOLED-1.8 (and its linked schematic PDF / ESP-IDF demo zip). Record in `docs/hardware/board-notes.md`: every LCD QSPI pin, reset pin, touch I²C pins + address (FT3168), any onboard IO expander or PMU chip and what hangs off it, and which panel init quirks the demo applies. Note the **v2 = CO5300** distinction explicitly.

- [x] **Step 2: Write `board.h` with the extracted values**

```c
#pragma once
#include "driver/gpio.h"
#include "driver/spi_master.h"

// Waveshare ESP32-S3 Touch AMOLED 1.8 v2 (CO5300 panel, FT3168 touch)
// Values sourced from the Waveshare wiki + schematic — see docs/hardware/board-notes.md
#define BOARD_LCD_H_RES        368
#define BOARD_LCD_V_RES        448
#define BOARD_LCD_QSPI_HOST    SPI2_HOST
#define BOARD_LCD_PIN_PCLK     /* from wiki */
#define BOARD_LCD_PIN_CS       /* from wiki */
#define BOARD_LCD_PIN_D0       /* from wiki */
#define BOARD_LCD_PIN_D1       /* from wiki */
#define BOARD_LCD_PIN_D2       /* from wiki */
#define BOARD_LCD_PIN_D3       /* from wiki */
#define BOARD_LCD_PIN_RST      /* from wiki */
#define BOARD_I2C_PORT         I2C_NUM_0
#define BOARD_I2C_PIN_SDA      /* from wiki */
#define BOARD_I2C_PIN_SCL      /* from wiki */
#define BOARD_TOUCH_PIN_INT    /* from wiki, or GPIO_NUM_NC */
#define BOARD_TOUCH_PIN_RST    /* from wiki, or GPIO_NUM_NC */
```
Every `/* from wiki */` must be replaced with a real number in this task — the file must not merge with comments in place of values.

- [x] **Step 3: Verify it compiles**

Add `"board"` to `INCLUDE_DIRS` in `firmware/main/CMakeLists.txt`, `#include "board/board.h"` from `main.c`, run `idf.py build`.
Expected: build succeeds.

- [x] **Step 4: Cross-check**

Diff your pin numbers against the pins used in Waveshare's own demo source (from the wiki zip). Any mismatch: schematic wins; note the discrepancy in `board-notes.md`.

- [x] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: board pin map for Waveshare AMOLED 1.8 v2 from wiki/schematic"
```

---

### Task 3: Display bring-up (CO5300 over QSPI + LVGL 9)

**Files:**
- Create: `firmware/main/board/display.c`
- Create: `firmware/main/board/display.h`
- Modify: `firmware/main/idf_component.yml` (panel + LVGL deps)
- Modify: `firmware/main/CMakeLists.txt` (add sources)
- Modify: `firmware/main/main.c`

**Interfaces:**
- Consumes: `board.h` pin constants (Task 2).
- Produces: `esp_err_t board_display_init(void)` — brings up QSPI bus, panel, and LVGL via `esp_lvgl_port`; after it returns, LVGL APIs are usable from the main task (guarded by `lvgl_port_lock()/unlock()`). Also exposes `esp_lcd_panel_io_handle_t board_display_io(void)` for Task 5's brightness command.

- [x] **Step 1: Choose and add the panel driver component**

Search the registry for a maintained driver, in this preference order:
```bash
idf.py add-dependency "espressif/esp_lcd_co5300"   # try first
# if not found:
idf.py add-dependency "espressif/esp_lcd_sh8601"   # CO5300 is SH8601-command-compatible
```
Also add the UI plumbing:
```bash
idf.py add-dependency "lvgl/lvgl^9"
idf.py add-dependency "espressif/esp_lvgl_port^2"
```
Record which panel component + version was chosen in `docs/hardware/board-notes.md`. Commit `dependencies.lock`.

- [x] **Step 2: Write `display.h`**

```c
#pragma once
#include "esp_err.h"
#include "esp_lcd_panel_io.h"

esp_err_t board_display_init(void);
esp_lcd_panel_io_handle_t board_display_io(void);
```

- [x] **Step 3: Write `display.c`**

Follow the chosen component's README example for the QSPI wiring, adapted to `board.h` names. Skeleton (macro/type names come from the chosen component — the SH8601 variants are shown):

```c
#include "board.h"
#include "display.h"
#include "esp_lcd_panel_ops.h"
#include "esp_lcd_sh8601.h"      // or esp_lcd_co5300.h
#include "esp_lvgl_port.h"

static esp_lcd_panel_io_handle_t s_io;

esp_err_t board_display_init(void)
{
    const spi_bus_config_t buscfg = SH8601_PANEL_BUS_QSPI_CONFIG(
        BOARD_LCD_PIN_PCLK, BOARD_LCD_PIN_D0, BOARD_LCD_PIN_D1,
        BOARD_LCD_PIN_D2, BOARD_LCD_PIN_D3,
        BOARD_LCD_H_RES * 80 * 2);
    ESP_ERROR_CHECK(spi_bus_initialize(BOARD_LCD_QSPI_HOST, &buscfg, SPI_DMA_CH_AUTO));

    const esp_lcd_panel_io_spi_config_t io_config =
        SH8601_PANEL_IO_QSPI_CONFIG(BOARD_LCD_PIN_CS, NULL, NULL);
    ESP_ERROR_CHECK(esp_lcd_new_panel_io_spi(
        (esp_lcd_spi_bus_handle_t)BOARD_LCD_QSPI_HOST, &io_config, &s_io));

    sh8601_vendor_config_t vendor_config = {
        .flags = { .use_qspi_interface = 1 },
    };
    const esp_lcd_panel_dev_config_t panel_config = {
        .reset_gpio_num = BOARD_LCD_PIN_RST,
        .rgb_ele_order = LCD_RGB_ELEMENT_ORDER_RGB,
        .bits_per_pixel = 16,
        .vendor_config = &vendor_config,
    };
    esp_lcd_panel_handle_t panel;
    ESP_ERROR_CHECK(esp_lcd_new_panel_sh8601(s_io, &panel_config, &panel));
    ESP_ERROR_CHECK(esp_lcd_panel_reset(panel));
    ESP_ERROR_CHECK(esp_lcd_panel_init(panel));
    ESP_ERROR_CHECK(esp_lcd_panel_disp_on_off(panel, true));

    const lvgl_port_cfg_t lvgl_cfg = ESP_LVGL_PORT_INIT_CONFIG();
    ESP_ERROR_CHECK(lvgl_port_init(&lvgl_cfg));

    const lvgl_port_display_cfg_t disp_cfg = {
        .io_handle = s_io,
        .panel_handle = panel,
        .buffer_size = BOARD_LCD_H_RES * 80,
        .double_buffer = true,
        .hres = BOARD_LCD_H_RES,
        .vres = BOARD_LCD_V_RES,
        .color_format = LV_COLOR_FORMAT_RGB565,
        .flags = { .buff_dma = true, .swap_bytes = true },
    };
    lvgl_port_add_disp(&disp_cfg);
    return ESP_OK;
}

esp_lcd_panel_io_handle_t board_display_io(void) { return s_io; }
```
If the panel shows garbage/offset: check the component README for this panel's known `x_gap`/color-order quirks and what the Waveshare demo sets — record the fix in `board-notes.md`.

- [x] **Step 4: Show a test screen from `main.c`**

```c
#include "board/display.h"
#include "esp_lvgl_port.h"
#include "lvgl.h"

void app_main(void)
{
    ESP_ERROR_CHECK(board_display_init());
    lvgl_port_lock(0);
    lv_obj_t *scr = lv_screen_active();
    lv_obj_set_style_bg_color(scr, lv_color_hex(0x101020), 0);
    lv_obj_t *label = lv_label_create(scr);
    lv_label_set_text(label, "deskmate M0");
    lv_obj_set_style_text_color(label, lv_color_hex(0xffffff), 0);
    lv_obj_center(label);
    lvgl_port_unlock();
}
```

- [x] **Step 5: Flash and verify on hardware**

Run: `idf.py build flash monitor`
Expected: dark-blue screen with centered white "deskmate M0", no tearing, no boot loop, clean log.

- [x] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: CO5300 QSPI display bring-up with LVGL 9 test screen"
```

---

### Task 4: Touch bring-up (FT3168 → LVGL input)

**Files:**
- Create: `firmware/main/board/touch.c`
- Create: `firmware/main/board/touch.h`
- Modify: `firmware/main/idf_component.yml`
- Modify: `firmware/main/CMakeLists.txt`
- Modify: `firmware/main/main.c`

**Interfaces:**
- Consumes: `board.h` I²C pins; LVGL display from Task 3.
- Produces: `esp_err_t board_touch_init(lv_display_t *disp)` — registers the touchscreen as an LVGL input device; LVGL widgets receive presses/gestures from here on.

- [x] **Step 1: Add the touch driver component**

FT3168 speaks the FocalTech FT5x06-family protocol:
```bash
idf.py add-dependency "espressif/esp_lcd_touch_ft5x06"
```
(If the wiki demo names a different component for FT3168, prefer what the demo uses; record the choice in `board-notes.md`.)

- [x] **Step 2: Write `touch.h` / `touch.c`**

```c
// touch.h
#pragma once
#include "esp_err.h"
#include "lvgl.h"
esp_err_t board_touch_init(lv_display_t *disp);
```

```c
// touch.c
#include "board.h"
#include "touch.h"
#include "driver/i2c_master.h"
#include "esp_lcd_touch_ft5x06.h"
#include "esp_lvgl_port.h"

esp_err_t board_touch_init(lv_display_t *disp)
{
    i2c_master_bus_handle_t bus;
    const i2c_master_bus_config_t bus_cfg = {
        .i2c_port = BOARD_I2C_PORT,
        .sda_io_num = BOARD_I2C_PIN_SDA,
        .scl_io_num = BOARD_I2C_PIN_SCL,
        .clk_source = I2C_CLK_SRC_DEFAULT,
        .flags.enable_internal_pullup = true,
    };
    ESP_ERROR_CHECK(i2c_new_master_bus(&bus_cfg, &bus));

    esp_lcd_panel_io_handle_t tp_io;
    esp_lcd_panel_io_i2c_config_t tp_io_cfg =
        ESP_LCD_TOUCH_IO_I2C_FT5x06_CONFIG();
    ESP_ERROR_CHECK(esp_lcd_new_panel_io_i2c(bus, &tp_io_cfg, &tp_io));

    const esp_lcd_touch_config_t tp_cfg = {
        .x_max = BOARD_LCD_H_RES,
        .y_max = BOARD_LCD_V_RES,
        .rst_gpio_num = BOARD_TOUCH_PIN_RST,
        .int_gpio_num = BOARD_TOUCH_PIN_INT,
    };
    esp_lcd_touch_handle_t tp;
    ESP_ERROR_CHECK(esp_lcd_touch_new_i2c_ft5x06(tp_io, &tp_cfg, &tp));

    const lvgl_port_touch_cfg_t touch_cfg = { .disp = disp, .handle = tp };
    lvgl_port_add_touch(&touch_cfg);
    return ESP_OK;
}
```

- [x] **Step 3: Add a touch-following dot to the test screen**

In `main.c`, after display init, create a small circle that jumps to the touch point (proves coordinates AND orientation agree with the panel):

```c
static lv_obj_t *s_dot;

static void screen_pressed_cb(lv_event_t *e)
{
    lv_indev_t *indev = lv_indev_active();
    lv_point_t p;
    lv_indev_get_point(indev, &p);
    lv_obj_set_pos(s_dot, p.x - 10, p.y - 10);
}

// in app_main, inside lvgl_port_lock:
s_dot = lv_obj_create(scr);
lv_obj_set_size(s_dot, 20, 20);
lv_obj_set_style_radius(s_dot, LV_RADIUS_CIRCLE, 0);
lv_obj_set_style_bg_color(s_dot, lv_color_hex(0x00c853), 0);
lv_obj_add_event_cb(scr, screen_pressed_cb, LV_EVENT_PRESSING, NULL);
```

- [x] **Step 4: Flash and verify on hardware**

Run: `idf.py build flash monitor`
Expected: green dot tracks the finger across the whole panel, correct axis directions (drag right → dot moves right; drag down → dot moves down). If axes are swapped/mirrored, fix with the touch config's `flags.swap_xy / mirror_x / mirror_y` and record in `board-notes.md`.

- [x] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: FT3168 touch bring-up wired into LVGL input"
```

---

### Task 5: Brightness control + 180° rotation

**Files:**
- Modify: `firmware/main/board/display.c`
- Modify: `firmware/main/board/display.h`
- Modify: `firmware/main/main.c`

**Interfaces:**
- Consumes: panel IO handle from Task 3; touch from Task 4.
- Produces: `esp_err_t board_display_set_brightness(uint8_t level)` (0–255, DCS 0x51 — AMOLED has no backlight GPIO); `esp_err_t board_display_set_rotation_180(bool on)` (for the spec's cable-exit-either-side enclosure requirement). Both used by config handling in later milestones.

- [x] **Step 1: Implement brightness**

```c
esp_err_t board_display_set_brightness(uint8_t level)
{
    // DCS "Write Display Brightness" — CO5300/SH8601 class panels
    return esp_lcd_panel_io_tx_param(s_io, 0x51, (uint8_t[]){ level }, 1);
}
```

- [x] **Step 2: Implement 180° rotation**

Keep the panel handle in a static (`s_panel`) and use mirror on both axes:
```c
esp_err_t board_display_set_rotation_180(bool on)
{
    ESP_ERROR_CHECK(esp_lcd_panel_mirror(s_panel, on, on));
    return ESP_OK;
}
```
If the panel ignores mirror commands (some AMOLED controllers do), fall back to LVGL software rotation: `lv_display_set_rotation(disp, on ? LV_DISPLAY_ROTATION_180 : LV_DISPLAY_ROTATION_0);` — and note which path worked in `board-notes.md`. Touch coordinates must be remapped consistently (retest the Task 4 dot after enabling rotation).

- [x] **Step 3: Add a temporary on-screen test**

In `main.c`: tapping the top half of the screen cycles brightness 25% → 50% → 100%; tapping the bottom half toggles 180° rotation. (Temporary code, removed in Task 6.)

- [x] **Step 4: Flash and verify on hardware**

Expected: three visibly distinct brightness levels; rotation flips the image AND the touch dot still lands under the finger in both orientations.

- [x] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: brightness (DCS 0x51) and 180-degree rotation controls"
```

---

### Task 6: Standalone clock screen (with host-tested core)

**Files:**
- Create: `firmware/main/core/timefmt.c`
- Create: `firmware/main/core/timefmt.h`
- Create: `firmware/host_tests/Makefile`
- Create: `firmware/host_tests/test_timefmt.c`
- Create: `firmware/main/ui/clock_screen.c`
- Create: `firmware/main/ui/clock_screen.h`
- Modify: `firmware/main/CMakeLists.txt`
- Modify: `firmware/main/main.c` (remove Task 5 temp test code)

**Interfaces:**
- Consumes: display/touch/brightness from Tasks 3–5.
- Produces: `void clock_screen_show(void)` — the standalone fallback screen (spec §3): big HH:MM, date line, "Connect deskmate app" hint. `timefmt.h`: `void timefmt_hhmm(char out[6], int hour, int minute)` and `void timefmt_date(char out[32], int year, int month, int day)` — pure C, host-testable, reused by M2's clock widget.

- [x] **Step 1: Write the failing host test**

`firmware/host_tests/test_timefmt.c`:
```c
#include <assert.h>
#include <string.h>
#include <stdio.h>
#include "../main/core/timefmt.h"

int main(void)
{
    char buf[32];

    timefmt_hhmm(buf, 9, 5);
    assert(strcmp(buf, "09:05") == 0);
    timefmt_hhmm(buf, 23, 59);
    assert(strcmp(buf, "23:59") == 0);
    timefmt_hhmm(buf, 0, 0);
    assert(strcmp(buf, "00:00") == 0);

    timefmt_date(buf, 2026, 8, 3, 0); // dow: 0 = Monday
    assert(strcmp(buf, "Mon, Aug 3") == 0);
    timefmt_date(buf, 2026, 12, 31, 3); // Thursday
    assert(strcmp(buf, "Thu, Dec 31") == 0);

    printf("test_timefmt: OK\n");
    return 0;
}
```

`firmware/host_tests/Makefile`:
```make
CC ?= cc
CFLAGS = -Wall -Wextra -Werror -std=c11

test: test_timefmt
	./test_timefmt

test_timefmt: test_timefmt.c ../main/core/timefmt.c
	$(CC) $(CFLAGS) -o $@ $^

clean:
	rm -f test_timefmt
.PHONY: test clean
```

- [x] **Step 2: Run to verify it fails**

Run: `make -C firmware/host_tests test`
Expected: FAIL — `timefmt.h: No such file or directory`.

- [x] **Step 3: Implement `timefmt`**

```c
// timefmt.h
#pragma once
void timefmt_hhmm(char out[6], int hour, int minute);
// dow: 0 = Monday ... 6 = Sunday
void timefmt_date(char out[32], int year, int month, int day, int dow);
```

```c
// timefmt.c
#include "timefmt.h"
#include <stdio.h>

static const char *DOW[] = { "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun" };
static const char *MON[] = { "Jan", "Feb", "Mar", "Apr", "May", "Jun",
                             "Jul", "Aug", "Sep", "Oct", "Nov", "Dec" };

void timefmt_hhmm(char out[6], int hour, int minute)
{
    snprintf(out, 6, "%02d:%02d", hour, minute);
}

void timefmt_date(char out[32], int year, int month, int day, int dow)
{
    (void)year;
    snprintf(out, 32, "%s, %s %d", DOW[dow], MON[month - 1], day);
}
```

- [x] **Step 4: Run tests to verify they pass**

Run: `make -C firmware/host_tests test`
Expected: `test_timefmt: OK`.

- [x] **Step 5: Build the clock screen**

`clock_screen.c`: LVGL screen with HH:MM in the largest built-in Montserrat font (enable `CONFIG_LV_FONT_MONTSERRAT_48=y` in `sdkconfig.defaults`), date below, dim "Connect deskmate app" hint at the bottom. A 1 Hz `lv_timer` reads `time()` / `localtime_r()` (system clock starts at epoch until M1's time sync — that's expected and fine for M0), formats via `timefmt_*`, updates labels only when the minute changes. `main.c` becomes: init display → init touch → set brightness 200 → `clock_screen_show()`.

- [x] **Step 6: Flash and verify on hardware**

Expected: ticking clock (starting from 00:00 epoch), date line, hint text; runs 5+ minutes with no watchdog resets and stable heap (log `esp_get_free_heap_size()` once a minute during this soak; it must plateau, not decline).

- [x] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: standalone clock screen with host-tested time formatting"
```

---

### Task 7: M0 exit — soak, docs, tag

**Files:**
- Modify: `README.md`
- Modify: `docs/hardware/board-notes.md`

**Interfaces:**
- Consumes: everything above.
- Produces: tagged `m0` baseline the M1 plan builds on.

- [x] **Step 1: 30-minute soak**

Leave the clock running 30 minutes. Expected: no reboot, no visual artifacts, heap log flat. Record the result + observed free-heap floor in `board-notes.md`.

- [x] **Step 2: Finalize docs**

README gains a "Status: M0 complete" line and a photo-free summary of what works (display, touch, brightness, rotation, standalone clock). `board-notes.md` must by now contain: final pin table, chosen driver components + versions, any orientation/gap/color quirks discovered.

- [x] **Step 3: Commit and tag**

```bash
git add -A
git commit -m "docs: M0 exit notes and status"
git tag m0
```

---

## Self-review checklist (run before handoff)

- Spec coverage for M0 scope: hardware target (§2) ✓ Tasks 2–5; from-scratch constraint (§3/§6) ✓ global constraints; standalone fallback (§3) ✓ Task 6; 180° rotation (§2) ✓ Task 5; host-testable core split (§9) ✓ Task 6.
- Not in M0 by design (deferred to M1+): USB protocol, time sync, template engine, carousel, status strip — see roadmap.
- Type consistency: `board_display_init/board_display_io/board_display_set_brightness/board_display_set_rotation_180`, `board_touch_init`, `clock_screen_show`, `timefmt_hhmm/timefmt_date` used consistently across tasks.
