#include <stdio.h>
#include "esp_log.h"
#include "esp_timer.h"
#include "board/board.h"
#include "board/display.h"
#include "board/touch.h"
#include "esp_lvgl_port.h"
#include "lvgl.h"

static const char *TAG = "deskmate";

// Touch coordinate log is tagged "touch" (not "deskmate") so it's greppable
// on its own -- the controller uses this to human-verify finger tracking
// (green dot follows finger, correct axes) after this task.
static const char *TOUCH_TAG = "touch";

static lv_obj_t *s_dot;

// Throttle the coordinate log to ~5/sec max: LVGL fires LV_EVENT_PRESSING on
// every input read tick while a finger is down (much faster than 5 Hz), and
// logging every one of those would flood the serial console.
#define TOUCH_LOG_MIN_INTERVAL_US (200 * 1000)
static int64_t s_last_touch_log_us;

// Proves touch coordinates AND axis orientation agree with the panel: the
// dot jumps to wherever LVGL thinks the press is. Registered on the screen
// object so it fires for presses anywhere on the display.
static void screen_pressed_cb(lv_event_t *e)
{
    (void)e;
    lv_indev_t *indev = lv_indev_active();
    if (indev == NULL) {
        return;
    }
    lv_point_t p;
    lv_indev_get_point(indev, &p);
    lv_obj_set_pos(s_dot, p.x - 10, p.y - 10);

    int64_t now_us = esp_timer_get_time();
    if (now_us - s_last_touch_log_us >= TOUCH_LOG_MIN_INTERVAL_US) {
        s_last_touch_log_us = now_us;
        ESP_LOGI(TOUCH_TAG, "press at x=%d y=%d", (int)p.x, (int)p.y);
    }
}

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

    s_dot = lv_obj_create(scr);
    lv_obj_set_size(s_dot, 20, 20);
    lv_obj_set_style_radius(s_dot, LV_RADIUS_CIRCLE, 0);
    lv_obj_set_style_bg_color(s_dot, lv_color_hex(0x00c853), 0);
    lv_obj_add_event_cb(scr, screen_pressed_cb, LV_EVENT_PRESSING, NULL);
    lvgl_port_unlock();

    ESP_LOGI(TAG, "test screen drawn, LVGL task running");

    // Touch registers itself against the lv_display_t LVGL already created
    // inside board_display_init() (via lvgl_port_add_disp()); display.h
    // deliberately doesn't expose that handle, so pull it back out through
    // LVGL's own default-display accessor instead of changing display.h.
    lv_display_t *disp = lv_display_get_default();
    ESP_ERROR_CHECK(board_touch_init(disp));
    ESP_LOGI(TAG, "board_touch_init OK");
}
