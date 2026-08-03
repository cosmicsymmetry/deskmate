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

// Task 5 temporary test: tag "m0test" so serial shows exactly what action
// fired, separate from the "touch"-tagged coordinate log and the
// "deskmate"-tagged boot log. Removed in Task 6.
static const char *M0TEST_TAG = "m0test";

static lv_obj_t *s_dot;

// Throttle the coordinate log to ~5/sec max: LVGL fires LV_EVENT_PRESSING on
// every input read tick while a finger is down (much faster than 5 Hz), and
// logging every one of those would flood the serial console.
#define TOUCH_LOG_MIN_INTERVAL_US (200 * 1000)
static int64_t s_last_touch_log_us;

// Brightness cycle for the top-half tap zone: 25% -> 50% -> 100% -> repeat.
// board_display_set_brightness() takes a raw 0-255 DCS level, not a percent.
static const uint8_t BRIGHTNESS_LEVELS[] = { 64, 128, 255 };
#define BRIGHTNESS_LEVELS_COUNT (sizeof(BRIGHTNESS_LEVELS) / sizeof(BRIGHTNESS_LEVELS[0]))
static size_t s_brightness_idx = 0;
static bool s_rotated_180 = false;

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

// Task 5 temporary test (removed in Task 6): tapping the top half of the
// screen cycles brightness 25% -> 50% -> 100%, tapping the bottom half
// toggles 180-degree rotation. Registered on LV_EVENT_RELEASED (fires once
// per press-and-lift, not per input-read tick like PRESSING) so a drag
// across the boundary doesn't machine-gun the action -- only where the
// finger lifts decides the zone. Coexists with screen_pressed_cb: the dot
// and coordinate log above still fire on every PRESSING tick anywhere on
// screen, this handler additionally fires once on release.
static void screen_released_cb(lv_event_t *e)
{
    (void)e;
    lv_indev_t *indev = lv_indev_active();
    if (indev == NULL) {
        return;
    }
    lv_point_t p;
    lv_indev_get_point(indev, &p);

    if (p.y < BOARD_LCD_V_RES / 2) {
        s_brightness_idx = (s_brightness_idx + 1) % BRIGHTNESS_LEVELS_COUNT;
        uint8_t level = BRIGHTNESS_LEVELS[s_brightness_idx];
        esp_err_t err = board_display_set_brightness(level);
        ESP_LOGI(M0TEST_TAG, "top-half tap at x=%d y=%d -> brightness level=%u/255 (%s)",
                 (int)p.x, (int)p.y, (unsigned)level, esp_err_to_name(err));
    } else {
        s_rotated_180 = !s_rotated_180;
        esp_err_t err = board_display_set_rotation_180(s_rotated_180);
        ESP_LOGI(M0TEST_TAG, "bottom-half tap at x=%d y=%d -> rotation 180=%s (%s)",
                 (int)p.x, (int)p.y, s_rotated_180 ? "on" : "off", esp_err_to_name(err));
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
    // LVGL screens are scrollable by default (LV_OBJ_FLAG_SCROLLABLE); with
    // no scrollbar visible, a drag was instead observed dragging the whole
    // screen's content (the "deskmate M0" label visibly moved with the
    // finger) -- human-hardware-verification finding from Task 5's fix
    // round 2. This screen has nothing to scroll to, so disable it. Task 6's
    // clock screen must do the same.
    lv_obj_remove_flag(scr, LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_style_bg_color(scr, lv_color_hex(0x101020), 0);
    lv_obj_t *label = lv_label_create(scr);
    lv_label_set_text(label, "deskmate M0");
    lv_obj_set_style_text_color(label, lv_color_hex(0xffffff), 0);
    lv_obj_center(label);

    s_dot = lv_obj_create(scr);
    lv_obj_set_size(s_dot, 20, 20);
    lv_obj_set_style_radius(s_dot, LV_RADIUS_CIRCLE, 0);
    lv_obj_set_style_bg_color(s_dot, lv_color_hex(0x00c853), 0);
    // Without this, the dot (clickable by default) hit-tests touches once the
    // finger is over it, so screen_pressed_cb (registered on scr only, no
    // event-bubble flag) stops firing until the finger moves off the dot --
    // visible as tracking stutter/jump. Found in review + confirmed on
    // hardware.
    lv_obj_remove_flag(s_dot, LV_OBJ_FLAG_CLICKABLE);
    lv_obj_add_event_cb(scr, screen_pressed_cb, LV_EVENT_PRESSING, NULL);
    lv_obj_add_event_cb(scr, screen_released_cb, LV_EVENT_RELEASED, NULL);
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
