#include <stdio.h>
#include "esp_log.h"
#include "board/board.h"
#include "board/display.h"
#include "esp_lvgl_port.h"
#include "lvgl.h"

static const char *TAG = "deskmate";

void app_main(void)
{
    ESP_LOGI(TAG, "deskmate M0 boot");
    ESP_LOGI(TAG, "board: %dx%d LCD, QSPI CS=%d PCLK=%d D0-D3=%d,%d,%d,%d",
             BOARD_LCD_H_RES, BOARD_LCD_V_RES,
             BOARD_LCD_PIN_CS, BOARD_LCD_PIN_PCLK,
             BOARD_LCD_PIN_D0, BOARD_LCD_PIN_D1, BOARD_LCD_PIN_D2, BOARD_LCD_PIN_D3);

    ESP_ERROR_CHECK(board_display_init());
    ESP_LOGI(TAG, "board_display_init OK, io=%p", (void *)board_display_io());

    lvgl_port_lock(0);
    lv_obj_t *scr = lv_screen_active();
    lv_obj_set_style_bg_color(scr, lv_color_hex(0x101020), 0);
    lv_obj_t *label = lv_label_create(scr);
    lv_label_set_text(label, "deskmate M0");
    lv_obj_set_style_text_color(label, lv_color_hex(0xffffff), 0);
    lv_obj_center(label);
    lvgl_port_unlock();

    ESP_LOGI(TAG, "test screen drawn, LVGL task running");
}
