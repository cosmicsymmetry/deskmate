#include "board.h"
#include "board_i2c.h"
#include "display.h"

#include "esp_check.h"
#include "esp_io_expander_tca9554.h"
#include "esp_lcd_co5300.h"
#include "esp_lcd_panel_ops.h"
#include "esp_log.h"
#include "esp_lvgl_port.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "lvgl.h"

static const char *TAG = "board_display";

// TCA9554 IO-expander pin driving LCD_RESET (see docs/hardware/board-notes.md:
// LCD_RESET is wired to EXIO0, not a direct ESP32 GPIO, so BOARD_LCD_PIN_RST
// is GPIO_NUM_NC and esp_lcd_panel_reset() alone cannot toggle it).
#define BOARD_LCD_EXIO_RST_MASK IO_EXPANDER_PIN_NUM_0

static esp_lcd_panel_io_handle_t s_io;
static bool s_first_flush_logged = false;

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
static esp_err_t board_lcd_expander_reset(void)
{
    i2c_master_bus_handle_t bus = board_i2c_bus();
    ESP_RETURN_ON_FALSE(bus != NULL, ESP_FAIL, TAG, "I2C bus init failed");

    esp_io_expander_handle_t expander = NULL;
    ESP_RETURN_ON_ERROR(
        esp_io_expander_new_i2c_tca9554(bus, ESP_IO_EXPANDER_I2C_TCA9554_ADDRESS_000, &expander),
        TAG, "TCA9554 init failed");

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
    ESP_RETURN_ON_ERROR(esp_lcd_panel_disp_on_off(panel, true), TAG, "disp_on_off failed");

    ESP_LOGI(TAG, "Initialize LVGL port");
    const lvgl_port_cfg_t lvgl_cfg = ESP_LVGL_PORT_INIT_CONFIG();
    ESP_RETURN_ON_ERROR(lvgl_port_init(&lvgl_cfg), TAG, "lvgl_port_init failed");

    const lvgl_port_display_cfg_t disp_cfg = {
        .io_handle = s_io,
        .panel_handle = panel,
        .buffer_size = BOARD_LCD_H_RES * 80,
        .double_buffer = true,
        .hres = BOARD_LCD_H_RES,
        .vres = BOARD_LCD_V_RES,
        .color_format = LV_COLOR_FORMAT_RGB565,
        .flags = {
            .buff_dma = true,
            .swap_bytes = true,
        },
    };
    lv_display_t *disp = lvgl_port_add_disp(&disp_cfg);
    ESP_RETURN_ON_FALSE(disp != NULL, ESP_FAIL, TAG, "lvgl_port_add_disp failed");

    lv_display_add_event_cb(disp, board_lcd_flush_finish_cb, LV_EVENT_FLUSH_FINISH, NULL);

    ESP_LOGI(TAG, "display init complete (%dx%d)", BOARD_LCD_H_RES, BOARD_LCD_V_RES);
    return ESP_OK;
}

esp_lcd_panel_io_handle_t board_display_io(void)
{
    return s_io;
}
