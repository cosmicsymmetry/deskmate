#include "esp_log.h"
#include "board/board.h"
#include "board/display.h"
#include "board/touch.h"
#include "link/usb_link.h"
#include "link/protocol_task.h"
#include "ui/clock_screen.h"
#include "ui/ui_runtime.h"
#include "lvgl.h"

static const char *TAG = "deskmate";

// M0's fallback brightness level (0-255 DCS "Write Display Brightness"
// value); later milestones will make this configurable.
#define BOARD_INIT_BRIGHTNESS 200

void app_main(void)
{
    ESP_LOGI(TAG, "deskmate M2 boot");
    ESP_LOGI(TAG, "board: %dx%d LCD, QSPI CS=%d PCLK=%d D0-D3=%d,%d,%d,%d",
             BOARD_LCD_H_RES, BOARD_LCD_V_RES,
             BOARD_LCD_PIN_CS, BOARD_LCD_PIN_PCLK,
             BOARD_LCD_PIN_D0, BOARD_LCD_PIN_D1, BOARD_LCD_PIN_D2, BOARD_LCD_PIN_D3);

    ESP_ERROR_CHECK(board_display_init());
    ESP_LOGI(TAG, "board_display_init OK, io=%p", (void *)board_display_io());

    // Touch registers itself against the lv_display_t LVGL already created
    // inside board_display_init() (via lvgl_port_add_disp()); display.h
    // deliberately doesn't expose that handle, so pull it back out through
    // LVGL's own default-display accessor instead of changing display.h.
    // The clock screen doesn't consume touch input yet, but the input
    // pipeline must keep running for later milestones (soak-tested in
    // Task 7).
    lv_display_t *disp = lv_display_get_default();
    ESP_ERROR_CHECK(board_touch_init(disp));
    ESP_LOGI(TAG, "board_touch_init OK");

    ESP_ERROR_CHECK(board_display_set_brightness(BOARD_INIT_BRIGHTNESS));

    clock_screen_show();
    ESP_LOGI(TAG, "clock screen shown, LVGL task running");
    ESP_ERROR_CHECK(ui_runtime_init());

    // The native USB Serial/JTAG driver allocates bounded RX/TX rings from
    // internal RAM. Install it only after LVGL has secured its DMA and
    // software-rotation buffers so the M0 display path remains deterministic.
    ESP_ERROR_CHECK(usb_link_init());
    ESP_LOGI(TAG, "native USB Serial/JTAG protocol link ready");
    ESP_ERROR_CHECK(protocol_task_start());
    ESP_LOGI(TAG, "protocol task running");
}
