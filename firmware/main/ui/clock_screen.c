#include <time.h>
#include "esp_log.h"
#include "esp_system.h"
#include "esp_lvgl_port.h"
#include "lvgl.h"
#include "core/timefmt.h"
#include "clock_screen.h"

static const char *TAG = "clock";

static lv_obj_t *s_time_label;
static lv_obj_t *s_date_label;
static int s_last_minute = -1;

// Runs in LVGL task context (esp_lvgl_port's own timer handler already holds
// lvgl_port's lock while it services registered lv_timers) -- do NOT take
// lvgl_port_lock() in here, only at the external entry point below.
static void clock_timer_cb(lv_timer_t *timer)
{
    (void)timer;

    time_t now = time(NULL);
    struct tm tm_now;
    localtime_r(&now, &tm_now);

    if (tm_now.tm_min == s_last_minute) {
        return;
    }
    s_last_minute = tm_now.tm_min;

    char hhmm[6];
    timefmt_hhmm(hhmm, tm_now.tm_hour, tm_now.tm_min);
    lv_label_set_text(s_time_label, hhmm);

    // struct tm's tm_wday is 0=Sunday..6=Saturday; timefmt_date's dow is
    // 0=Monday..6=Sunday.
    int dow = (tm_now.tm_wday + 6) % 7;
    char date_str[32];
    timefmt_date(date_str, tm_now.tm_year + 1900, tm_now.tm_mon + 1, tm_now.tm_mday, dow);
    lv_label_set_text(s_date_label, date_str);

    // Once-per-minute heap log for Task 7's soak test.
    ESP_LOGI(TAG, "free heap: %u bytes", (unsigned)esp_get_free_heap_size());
}

void clock_screen_show(void)
{
    lvgl_port_lock(0);

    lv_obj_t *scr = lv_screen_active();
    // Fresh screens default to LV_OBJ_FLAG_SCROLLABLE; a finger swipe would
    // otherwise drag this screen's labels around (human-caught Task 5 bug,
    // see board-notes.md). This screen has nothing to scroll to.
    lv_obj_remove_flag(scr, LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_style_bg_color(scr, lv_color_hex(0x101020), 0);

    s_time_label = lv_label_create(scr);
    lv_obj_set_style_text_font(s_time_label, &lv_font_montserrat_48, 0);
    lv_obj_set_style_text_color(s_time_label, lv_color_hex(0xffffff), 0);
    lv_label_set_text(s_time_label, "00:00");
    lv_obj_align(s_time_label, LV_ALIGN_CENTER, 0, -40);

    s_date_label = lv_label_create(scr);
    lv_obj_set_style_text_color(s_date_label, lv_color_hex(0xa0a0b0), 0);
    lv_label_set_text(s_date_label, "");
    lv_obj_align_to(s_date_label, s_time_label, LV_ALIGN_OUT_BOTTOM_MID, 0, 12);

    lv_obj_t *hint = lv_label_create(scr);
    lv_obj_set_style_text_color(hint, lv_color_hex(0x505060), 0);
    lv_label_set_text(hint, "Connect deskmate app");
    lv_obj_align(hint, LV_ALIGN_BOTTOM_MID, 0, -16);

    s_last_minute = -1;
    lv_timer_create(clock_timer_cb, 1000, NULL);
    // Populate real data immediately rather than waiting up to 1s for the
    // first tick (we're already holding the lock, so this is safe to call
    // directly here -- see the "do NOT lock inside the callback" note above,
    // which is about the periodic path, not this one-time initial call).
    clock_timer_cb(NULL);

    lvgl_port_unlock();

    ESP_LOGI(TAG, "clock_screen_show: screen ready");
}
