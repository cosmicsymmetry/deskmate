#include "board.h"
#include "board_i2c.h"
#include "display.h"
#include <stdatomic.h>

#include "esp_check.h"
#include "esp_lcd_co5300.h"
#include "esp_lcd_panel_ops.h"
#include "esp_log.h"
#include "esp_lvgl_port.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "lvgl.h"

static const char *TAG = "board_display";

// Initial brightness applied at the end of board_display_init(), through
// board_display_set_brightness() -- see that function's doc comment for why
// this is the single brightness code path (the co5300 driver's own default
// init cmd table also sends 0x51 0xFF internally inside esp_lcd_panel_init();
// this call is a deliberate, logged, single-source-of-truth follow-up so
// nothing outside board_display_set_brightness() is the last word on
// brightness).
#define BOARD_LCD_INIT_BRIGHTNESS 255

// QSPI command envelope for DCS commands on this panel. In QSPI mode the
// CO5300 doesn't accept a bare DCS command byte over esp_lcd_panel_io_tx_param()
// -- confirmed by reading esp_lcd_co5300_spi.c's own tx_param() helper (used
// for every command the driver itself sends, including its default init
// table's 0x51 brightness command) and cross-checked against Waveshare's own
// BSP (esp32_s3_touch_amoled_1_8.c: bsp_display_brightness_set(), fetched
// from waveshareteam/Waveshare-ESP32-components@main), which builds the
// exact same 32-bit word by hand for this exact board:
//   lcd_cmd = (LCD_OPCODE_WRITE_CMD << 24) | ((cmd & 0xff) << 8)
// with LCD_OPCODE_WRITE_CMD == 0x02. A bare esp_lcd_panel_io_tx_param(io,
// 0x51, ...) -- what an earlier version of board_display_set_brightness()
// sent -- is a different (invalid) 32-bit command word as far as the panel's
// QSPI decoder is concerned: the SPI transaction still completes (ESP_OK)
// but the panel silently ignores it, which is why brightness had no visible
// effect on hardware despite every call logging success. See board-notes.md
// "Task 5 fix round 2" for the full trace.
#define BOARD_LCD_QSPI_CMD(dcs_cmd) ((0x02UL << 24) | (((uint32_t)(dcs_cmd) & 0xff) << 8))

// TCA9554 IO-expander pin driving LCD_RESET (see docs/hardware/board-notes.md:
// LCD_RESET is wired to EXIO0, not a direct ESP32 GPIO, so BOARD_LCD_PIN_RST
// is GPIO_NUM_NC and esp_lcd_panel_reset() alone cannot toggle it).
#define BOARD_LCD_EXIO_RST_MASK IO_EXPANDER_PIN_NUM_0

static esp_lcd_panel_io_handle_t s_io;
static lv_display_t *s_disp;
static bool s_first_flush_logged = false;
static atomic_uchar s_brightness;
static atomic_ushort s_rotation_degrees;

// CO5300 column-address gap correction for this v2/CST820 board. Waveshare's
// own BSP (bsp/esp32_s3_touch_amoled_1_8.c, bsp_touch_new()) applies this
// exact 16px value via esp_lcd_panel_set_gap(panel, x_gap, 0) whenever it
// detects the CST816S-family touch controller (BSP_LCD_CST816S_X_GAP =
// 0x10) -- the BSP uses the touch-chip probe result as a proxy signal for
// which panel/touch hardware revision is fitted, since v1 (SH8601+FT3168)
// needs no gap (x_gap stays 0) while v2 (CO5300+CST820) does. Since this
// project targets v2 exclusively (confirmed on hardware -- see
// board-notes.md), the value is hardcoded here rather than probed. Found
// missing during Task 4 human hardware verification: a ~16px bright green
// vertical strip (uninitialized panel RAM) was visible along the right edge
// of the panel without this call.
#define BOARD_LCD_X_GAP 16

// Reads: Waveshare's own esp32_s3_touch_amoled_1_8 BSP (main branch,
// bsp/esp32_s3_touch_amoled_1_8/esp32_s3_touch_amoled_1_8.c) creates a TCA9554
// handle via bsp_io_expander_init() but never toggles EXIO0 from
// bsp_display_new(): BSP_LCD_RST is GPIO_NUM_NC, so it relies solely on
// esp_lcd_panel_reset()'s built-in software-reset path for the CO5300 driver
// (LCD_CMD_SWRESET sent over QSPI when reset_gpio_num < 0, see
// esp_lcd_co5300_spi.c:panel_co5300_reset()). No EXIO0 pulse exists anywhere
// in the BSP source actually read for this task.
//
// We still perform a real electrical reset pulse on EXIO0 here, because (a)
// the I2C bus + TCA9554 expander are required anyway for Task 4's touch
// reset (EXIO2) and are cheap to bring up now, and (b) a hardware reset
// ahead of the driver's own software reset can only help, never hurt, a
// panel that may be coming up from an unknown power state. Timing (10 ms
// low / 150 ms settle) is borrowed from the co5300 driver's own hardware
// GPIO-reset branch (same file, the `reset_gpio_num >= 0` case) since no
// board-specific EXIO0 timing fact exists in any source consulted.
//
// Uses the shared board_io_expander() handle (board_i2c.h/.c), NOT a
// locally-created one: esp_io_expander_new_i2c_tca9554() unconditionally
// resets the physical chip's DIR/OUTPUT registers on every call, so a second
// independent handle (e.g. Task 4's touch-reset code on EXIO2) would stomp
// whatever this function just set on EXIO0. See board_i2c.h for the full
// rationale.
static esp_err_t board_lcd_expander_reset(void)
{
    esp_io_expander_handle_t expander = board_io_expander();
    ESP_RETURN_ON_FALSE(expander != NULL, ESP_FAIL, TAG, "IO expander init failed");

    ESP_RETURN_ON_ERROR(
        esp_io_expander_set_dir(expander, BOARD_LCD_EXIO_RST_MASK, IO_EXPANDER_OUTPUT),
        TAG, "EXIO0 set_dir failed");
    // Active-low reset: assert low, then release high before touching the panel.
    ESP_RETURN_ON_ERROR(
        esp_io_expander_set_level(expander, BOARD_LCD_EXIO_RST_MASK, 0),
        TAG, "EXIO0 assert-low failed");
    vTaskDelay(pdMS_TO_TICKS(10));
    ESP_RETURN_ON_ERROR(
        esp_io_expander_set_level(expander, BOARD_LCD_EXIO_RST_MASK, 1),
        TAG, "EXIO0 release-high failed");
    vTaskDelay(pdMS_TO_TICKS(150));

    ESP_LOGI(TAG, "LCD panel reset via TCA9554 EXIO0");
    return ESP_OK;
}

static void board_lcd_flush_finish_cb(lv_event_t *e)
{
    (void)e;
    if (!s_first_flush_logged) {
        s_first_flush_logged = true;
        ESP_LOGI(TAG, "first LVGL flush completed");
    }
}

// CO5300 v2 panels require partial update windows to start and end on
// two-pixel boundaries. Waveshare added this exact rounding rule when it
// changed the ESP32-S3-Touch-AMOLED-1.8 BSP from the v1 SH8601 panel to the
// v2 CO5300 panel. It is especially important for 90/270-degree software
// rotation: our 29,440-pixel buffer otherwise divides a 448-pixel-wide
// landscape repaint into 65-row strips. Rotation turns each strip into an
// odd-width CO5300 column window, which leaves the controller's subsequent
// pixel stream/addressing visibly shifted (stale text and colored blocks).
//
// Both logical resolutions are even, so their final valid coordinates are
// odd (447/367). Expanding a clipped invalid area outward this way therefore
// remains inside the display while guaranteeing even x/y starts and even
// widths/heights.
static void board_lcd_rounder_cb(lv_area_t *area)
{
    area->x1 &= ~1;
    area->y1 &= ~1;
    area->x2 |= 1;
    area->y2 |= 1;
}

esp_err_t board_display_init(void)
{
    ESP_RETURN_ON_ERROR(board_lcd_expander_reset(), TAG, "panel expander reset failed");

    ESP_LOGI(TAG, "Initialize QSPI bus");
    const spi_bus_config_t buscfg = CO5300_PANEL_BUS_QSPI_CONFIG(
        BOARD_LCD_PIN_PCLK, BOARD_LCD_PIN_D0, BOARD_LCD_PIN_D1,
        BOARD_LCD_PIN_D2, BOARD_LCD_PIN_D3,
        BOARD_LCD_H_RES * 80 * sizeof(uint16_t));
    ESP_RETURN_ON_ERROR(spi_bus_initialize(BOARD_LCD_QSPI_HOST, &buscfg, SPI_DMA_CH_AUTO),
                         TAG, "spi_bus_initialize failed");

    ESP_LOGI(TAG, "Install panel IO");
    const esp_lcd_panel_io_spi_config_t io_config =
        CO5300_PANEL_IO_QSPI_CONFIG(BOARD_LCD_PIN_CS, NULL, NULL);
    ESP_RETURN_ON_ERROR(
        esp_lcd_new_panel_io_spi((esp_lcd_spi_bus_handle_t)BOARD_LCD_QSPI_HOST, &io_config, &s_io),
        TAG, "esp_lcd_new_panel_io_spi failed");

    ESP_LOGI(TAG, "Install CO5300 panel driver");
    co5300_vendor_config_t vendor_config = {
        // NULL init_cmds -> use the driver's default sequence, which already
        // includes sleep-out (0x11), max brightness (0x51 0xFF) and display-on
        // (0x29) -- see esp_lcd_co5300_spi.c:vendor_specific_init_default.
        // No extra manual brightness command is needed on top of this.
        .flags = { .use_qspi_interface = 1 },
    };
    const esp_lcd_panel_dev_config_t panel_config = {
        .reset_gpio_num = BOARD_LCD_PIN_RST, // GPIO_NUM_NC; real reset already done via EXIO0 above
        .rgb_ele_order = LCD_RGB_ELEMENT_ORDER_RGB,
        .bits_per_pixel = 16,
        .vendor_config = &vendor_config,
    };
    esp_lcd_panel_handle_t panel = NULL;
    ESP_RETURN_ON_ERROR(esp_lcd_new_panel_co5300(s_io, &panel_config, &panel),
                         TAG, "esp_lcd_new_panel_co5300 failed");
    // With reset_gpio_num == GPIO_NUM_NC this issues a QSPI software reset
    // (LCD_CMD_SWRESET) inside the driver -- a second, belt-and-suspenders
    // reset on top of the EXIO0 pulse above.
    ESP_RETURN_ON_ERROR(esp_lcd_panel_reset(panel), TAG, "esp_lcd_panel_reset failed");
    ESP_RETURN_ON_ERROR(esp_lcd_panel_init(panel), TAG, "esp_lcd_panel_init failed");
    // Correct the CO5300's column-address window so the driver writes to the
    // GRAM columns that actually map to the visible 368px panel area --
    // without this, the rightmost ~16px of the panel show uninitialized
    // GRAM as a bright green strip. See BOARD_LCD_X_GAP's comment above.
    ESP_RETURN_ON_ERROR(esp_lcd_panel_set_gap(panel, BOARD_LCD_X_GAP, 0),
                         TAG, "esp_lcd_panel_set_gap failed");
    ESP_RETURN_ON_ERROR(esp_lcd_panel_disp_on_off(panel, true), TAG, "disp_on_off failed");

    ESP_LOGI(TAG, "Initialize LVGL port");
    lvgl_port_cfg_t lvgl_cfg = ESP_LVGL_PORT_INIT_CONFIG();
    // Keep rendering on CPU0 so M1's parser/dispatcher can be pinned to CPU1
    // and hostile serial input cannot monopolize LVGL's execution core.
    lvgl_cfg.task_affinity = 0;
    ESP_RETURN_ON_ERROR(lvgl_port_init(&lvgl_cfg), TAG, "lvgl_port_init failed");

    const lvgl_port_display_cfg_t disp_cfg = {
        .io_handle = s_io,
        .panel_handle = panel,
        .buffer_size = BOARD_LCD_H_RES * 80,
        // Keep one draw buffer while the newly restored CO5300 even-window
        // rule is visually isolated on hardware. Switching from two buffers
        // to one did not change the corruption, so it is not treated as the
        // root cause or a durable board requirement.
        .double_buffer = false,
        .hres = BOARD_LCD_H_RES,
        .vres = BOARD_LCD_V_RES,
        .color_format = LV_COLOR_FORMAT_RGB565,
        .rounder_cb = board_lcd_rounder_cb,
        .flags = {
            .buff_dma = true,
            .swap_bytes = true,
            // Task 5: rotate in software (lv_draw_sw_rotate, an extra
            // internal-SRAM scratch buffer the same size as one flush
            // buffer) rather than letting esp_lvgl_port drive the CO5300's
            // MADCTL mirror bits on rotation change. See display.h's
            // board_display_set_rotation_180() doc comment and
            // board-notes.md for why: this keeps BOARD_LCD_X_GAP's
            // column-address correction untouched in both orientations.
            .sw_rotate = true,
        },
    };
    lv_display_t *disp = lvgl_port_add_disp(&disp_cfg);
    ESP_RETURN_ON_FALSE(disp != NULL, ESP_FAIL, TAG, "lvgl_port_add_disp failed");
    s_disp = disp;

    // The assembled desk device is used horizontally with its USB cable
    // exiting downward. The panel is physically 368x448, so a 90-degree
    // software rotation exposes a 448x368 logical landscape canvas. The
    // existing "rotation 180" control flips that landscape orientation to
    // 270 degrees rather than returning to portrait.
    ESP_RETURN_ON_ERROR(board_display_set_rotation_180(false), TAG,
                        "initial landscape rotation failed");

    lv_display_add_event_cb(disp, board_lcd_flush_finish_cb, LV_EVENT_FLUSH_FINISH, NULL);

    ESP_LOGI(TAG, "display init complete (%dx%d)", BOARD_LCD_H_RES, BOARD_LCD_V_RES);

    // Single brightness code path (see board_display_set_brightness()'s doc
    // comment in display.h): even though the co5300 driver's own default
    // init cmd table already sent DCS 0x51 0xFF internally inside
    // esp_lcd_panel_init() above, route the init-time brightness through our
    // own function too so there is exactly one call site any later config
    // code needs to know about, and so the level actually applied is logged.
    ESP_RETURN_ON_ERROR(board_display_set_brightness(BOARD_LCD_INIT_BRIGHTNESS),
                         TAG, "initial brightness set failed");

    return ESP_OK;
}

esp_lcd_panel_io_handle_t board_display_io(void)
{
    return s_io;
}

esp_err_t board_display_set_brightness(uint8_t level)
{
    ESP_RETURN_ON_FALSE(s_io != NULL, ESP_ERR_INVALID_STATE, TAG, "display not initialized");

    // DCS "Write Display Brightness" (0x51), one data byte, 0-255, wrapped in
    // the QSPI command envelope this panel requires -- see
    // BOARD_LCD_QSPI_CMD's comment above for why the wrap is mandatory (a
    // bare 0x51 silently no-ops on this panel). BCTRL (brightness control
    // block enable, DCS 0x53 bit 0x20) doesn't need re-sending here: the
    // co5300 driver's default init cmd table already sends 0x53 0x20 once
    // during board_display_init()'s esp_lcd_panel_init() call, before this
    // function is ever reachable.
    ESP_RETURN_ON_ERROR(
        esp_lcd_panel_io_tx_param(s_io, BOARD_LCD_QSPI_CMD(0x51), (uint8_t[]) { level }, 1),
        TAG, "brightness DCS 0x51 failed");
    atomic_store(&s_brightness, level);
    ESP_LOGI(TAG, "brightness set to %u/255", (unsigned)level);
    return ESP_OK;
}

esp_err_t board_display_set_rotation_180(bool on)
{
    ESP_RETURN_ON_FALSE(s_disp != NULL, ESP_ERR_INVALID_STATE, TAG, "display not initialized");

    lv_display_rotation_t rotation = on ? LV_DISPLAY_ROTATION_270
                                        : LV_DISPLAY_ROTATION_90;
    uint16_t degrees = on ? 270U : 90U;

    // lvgl_port_lock()'s mutex is recursive (esp_lvgl_port.c), so this is
    // safe to call both from app_main()-adjacent code and from inside an
    // LVGL event callback (main.c's tap-zone handler runs on the LVGL task,
    // already holding this lock).
    lvgl_port_lock(0);
    lv_display_set_rotation(s_disp, rotation);
    lvgl_port_unlock();
    atomic_store(&s_rotation_degrees, degrees);

    // Touch coordinates do NOT need a separate transform here. LVGL core
    // already remaps every pointer indev's coordinates for the display's
    // current rotation, unconditionally: lv_indev.c's indev_pointer_proc()
    // calls lv_display_rotate_point(i->disp, &data->point)
    // (managed_components/lvgl__lvgl/src/indev/lv_indev.c) on every read,
    // for every pointer indev, regardless of the sw_rotate flag (that flag
    // only controls esp_lvgl_port's OWN mirror/MADCTL vs. sw-rotate flush
    // path -- a different subsystem than LVGL core's indev processing).
    // lv_display_rotate_point() (lv_display.c) applies the appropriate
    // 90/270-degree mapping into the landscape logical canvas. An earlier
    // portrait implementation also called esp_lcd_touch_set_mirror_x/y(),
    // adding a redundant driver transform underneath LVGL's own. Removed
    // after review; see board-notes.md "Task 5" for the full trace.

    ESP_LOGI(TAG, "rotation set to %u degrees (landscape LVGL sw-rotate; "
             "touch remap handled by LVGL core's lv_display_rotate_point)",
             (unsigned)degrees);
    return ESP_OK;
}

uint8_t board_display_brightness(void)
{
    return atomic_load(&s_brightness);
}

uint16_t board_display_rotation_degrees(void)
{
    return atomic_load(&s_rotation_degrees);
}
