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
static lv_timer_t *s_clock_timer;
static int s_last_minute = -1;

// Fires when the active screen is deleted (M1's carousel will do this when
// switching screens). Releases the periodic timer and NULLs the label
// statics so they can never be dereferenced after the objects they point to
// are freed, and so a later clock_screen_show() call knows it must rebuild
// from scratch (see the idempotence guard in clock_screen_show() below).
static void clock_screen_on_delete(lv_event_t *e)
{
    (void)e;

    if (s_clock_timer) {
        lv_timer_delete(s_clock_timer);
        s_clock_timer = NULL;
    }
    s_time_label = NULL;
    s_date_label = NULL;

    ESP_LOGI(TAG, "clock_screen: screen deleted, timer + labels released");
}

// Runs in LVGL task context (esp_lvgl_port's own timer handler already holds
// lvgl_port's lock while it services registered lv_timers) -- do NOT take
// lvgl_port_lock() in here, only at the external entry point below.
static void clock_timer_cb(lv_timer_t *timer)
{
    (void)timer;

    // Defensive: the screen may have been deleted (clearing these statics
    // via clock_screen_on_delete() above) in the window between this timer
    // firing and lv_timer_delete() actually taking effect.
    if (s_time_label == NULL || s_date_label == NULL) {
        return;
    }

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

    if (s_time_label != NULL) {
        // Idempotence guard: the clock UI is already built and live on the
        // still-active screen (this can only be true here -- a deleted
        // screen's LV_EVENT_DELETE handler above NULLs s_time_label before
        // this function could be called again). Early-return instead of
        // tearing down and rebuilding: simpler, and there is no state a
        // second call would need to refresh since the 1s timer already
        // keeps the labels current. If a caller (e.g. M1's carousel) wants
        // a genuine rebuild, it should lv_obj_del() the current screen
        // first -- that fires clock_screen_on_delete(), which clears these
        // statics so the next clock_screen_show() call rebuilds from
        // scratch.
        lvgl_port_unlock();
        ESP_LOGI(TAG, "clock_screen_show: already shown, skipping");
        return;
    }

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

    // Tears down the timer and NULLs the label statics if this screen is
    // ever deleted (M1's carousel) -- without this, the timer would keep
    // firing against freed lv_obj_t labels (use-after-free).
    lv_obj_add_event_cb(scr, clock_screen_on_delete, LV_EVENT_DELETE, NULL);

    s_last_minute = -1;
    s_clock_timer = lv_timer_create(clock_timer_cb, 1000, NULL);
    // Populate real data immediately rather than waiting up to 1s for the
    // first tick (we're already holding the lock, so this is safe to call
    // directly here -- see the "do NOT lock inside the callback" note above,
    // which is about the periodic path, not this one-time initial call).
    clock_timer_cb(NULL);

    lvgl_port_unlock();

    ESP_LOGI(TAG, "clock_screen_show: screen ready");
}
