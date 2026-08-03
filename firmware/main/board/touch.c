#include "board.h"
#include "board_i2c.h"
#include "touch.h"

#include "esp_check.h"
#include "esp_lcd_touch_cst816s.h"
#include "esp_lvgl_port.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static const char *TAG = "touch";

// Set once board_touch_init() succeeds; exposed via board_touch_handle() so
// display.c's rotation handling can mirror touch coordinates without a
// second CST816S handle (mirrors the board_io_expander()/board_i2c_bus()
// singleton-accessor pattern already used across this codebase).
static esp_lcd_touch_handle_t s_tp;

// TCA9554 IO-expander pin driving TP_RESET (see docs/hardware/board-notes.md:
// TP_RESET is wired to EXIO2, not a direct ESP32 GPIO, so BOARD_TOUCH_PIN_RST
// is GPIO_NUM_NC and the touch driver's own reset_gpio_num path is a no-op —
// esp_lcd_touch_cst816s.c's touch_cst816s_reset() only does anything when
// rst_gpio_num != GPIO_NUM_NC). We must pulse TP_RESET ourselves before
// probing the controller, mirroring display.c's board_lcd_expander_reset()
// pattern for EXIO0/LCD_RESET.
#define BOARD_TOUCH_EXIO_RST_MASK IO_EXPANDER_PIN_NUM_2

// Pulse TP_RESET (EXIO2) low then high via the shared TCA9554 expander
// handle (board_io_expander() — NEVER esp_io_expander_new_i2c_tca9554()
// directly, see board_i2c.h/board-notes.md "Post-review fix" section: a
// second constructor call would reset EXIO0/LCD_RESET too).
//
// Timing: 10 ms assert-low (same as display.c's EXIO0 pulse — no
// board-specific timing fact exists for TP_RESET either), then a 200 ms
// settle delay after release. CST816-family chips are known to need longer
// than typical LCD reset lines before they ACK on I2C (task brief's
// "CST816-family gotcha"), so this settle delay is longer than display.c's
// 150 ms EXIO0 delay.
static esp_err_t board_touch_expander_reset(void)
{
    esp_io_expander_handle_t expander = board_io_expander();
    ESP_RETURN_ON_FALSE(expander != NULL, ESP_FAIL, TAG, "IO expander init failed");

    ESP_RETURN_ON_ERROR(
        esp_io_expander_set_dir(expander, BOARD_TOUCH_EXIO_RST_MASK, IO_EXPANDER_OUTPUT),
        TAG, "EXIO2 set_dir failed");
    // Active-low reset, matching the EXIO0/LCD_RESET convention.
    ESP_RETURN_ON_ERROR(
        esp_io_expander_set_level(expander, BOARD_TOUCH_EXIO_RST_MASK, 0),
        TAG, "EXIO2 assert-low failed");
    vTaskDelay(pdMS_TO_TICKS(10));
    ESP_RETURN_ON_ERROR(
        esp_io_expander_set_level(expander, BOARD_TOUCH_EXIO_RST_MASK, 1),
        TAG, "EXIO2 release-high failed");
    vTaskDelay(pdMS_TO_TICKS(200));

    ESP_LOGI(TAG, "touch controller reset via TCA9554 EXIO2");
    return ESP_OK;
}

// Attempt to construct the CST816S driver handle against an already-created
// panel IO. Broken out so board_touch_init() can retry it once after an
// extra reset pulse if the first probe (which reads the CHIP_ID register
// over I2C, see esp_lcd_touch_cst816s.c:touch_cst816s_read_id()) fails.
static esp_err_t board_touch_probe(esp_lcd_panel_io_handle_t tp_io, esp_lcd_touch_handle_t *tp_out)
{
    const esp_lcd_touch_config_t tp_cfg = {
        .x_max = BOARD_LCD_H_RES,
        .y_max = BOARD_LCD_V_RES,
        // TP_RESET lives on EXIO2 (see above), not a GPIO the driver can
        // drive itself -- always GPIO_NUM_NC here regardless of
        // BOARD_TOUCH_PIN_RST's value.
        .rst_gpio_num = GPIO_NUM_NC,
        .int_gpio_num = BOARD_TOUCH_PIN_INT,
    };
    return esp_lcd_touch_new_i2c_cst816s(tp_io, &tp_cfg, tp_out);
}

esp_err_t board_touch_init(lv_display_t *disp)
{
    ESP_RETURN_ON_FALSE(disp != NULL, ESP_ERR_INVALID_ARG, TAG, "disp is NULL");

    // Shared bus (Task 3's board_i2c.c) -- never a second i2c_new_master_bus().
    i2c_master_bus_handle_t bus = board_i2c_bus();
    ESP_RETURN_ON_FALSE(bus != NULL, ESP_FAIL, TAG, "I2C bus init failed");

    ESP_RETURN_ON_ERROR(board_touch_expander_reset(), TAG, "touch reset failed");

    esp_lcd_panel_io_handle_t tp_io = NULL;
    const esp_lcd_panel_io_i2c_config_t tp_io_cfg = ESP_LCD_TOUCH_IO_I2C_CST816S_CONFIG();
    ESP_RETURN_ON_ERROR(esp_lcd_new_panel_io_i2c(bus, &tp_io_cfg, &tp_io),
                         TAG, "esp_lcd_new_panel_io_i2c failed");

    esp_lcd_touch_handle_t tp = NULL;
    esp_err_t err = board_touch_probe(tp_io, &tp);
    if (err != ESP_OK) {
        // CST816-family chips can stay silent on I2C until freshly reset (or
        // even until first touched) -- per the task brief's known gotcha,
        // retry once after another reset pulse before giving up.
        ESP_LOGW(TAG, "CST816S probe failed (%s); retrying after reset pulse",
                 esp_err_to_name(err));
        ESP_RETURN_ON_ERROR(board_touch_expander_reset(), TAG, "touch reset retry failed");
        err = board_touch_probe(tp_io, &tp);
        ESP_RETURN_ON_ERROR(err, TAG, "CST816S probe failed after reset retry");
        ESP_LOGI(TAG, "CST816S probe succeeded on retry");
    }

    ESP_LOGI(TAG, "CST816S touch controller initialized, handle=%p", (void *)tp);
    s_tp = tp;

    const lvgl_port_touch_cfg_t touch_cfg = {
        .disp = disp,
        .handle = tp,
    };
    lv_indev_t *indev = lvgl_port_add_touch(&touch_cfg);
    ESP_RETURN_ON_FALSE(indev != NULL, ESP_FAIL, TAG, "lvgl_port_add_touch failed");

    ESP_LOGI(TAG, "touch registered as LVGL input device, indev=%p", (void *)indev);
    return ESP_OK;
}

esp_lcd_touch_handle_t board_touch_handle(void)
{
    return s_tp;
}
