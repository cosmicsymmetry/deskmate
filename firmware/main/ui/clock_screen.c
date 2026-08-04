#include <stdatomic.h>
#include <time.h>
#include "esp_log.h"
#include "esp_system.h"
#include "esp_lvgl_port.h"
#include "lvgl.h"
#include "board/display.h"
#include "core/timefmt.h"
#include "clock_screen.h"

static const char *TAG = "clock";

static lv_obj_t *s_time_label;
static lv_obj_t *s_date_label;
static lv_obj_t *s_hint_label;
static lv_obj_t *s_clock_screen;
static lv_timer_t *s_clock_timer;
static int s_last_minute = -1;
static atomic_int s_utc_offset_minutes;
static atomic_bool s_online;
static atomic_bool s_refresh_requested;

static const uint8_t BRIGHTNESS_LEVELS[] = { 64U, 128U, 255U };

static void apply_online_state(void *unused);

static void invalidate_entire_screen(void)
{
    if (s_time_label != NULL) {
        lv_obj_invalidate(lv_screen_active());
    }
}

// Fires when the active screen is deleted (M1's carousel will do this when
// switching screens). Releases the periodic timer and NULLs the label
// statics so they can never be dereferenced after the objects they point to
// are freed, and so a later clock_screen_show() call knows it must rebuild
// from scratch (see the idempotence guard in clock_screen_show() below).
static void clock_screen_on_delete(lv_event_t *e)
{
    if (lv_event_get_target_obj(e) != s_clock_screen) {
        return;
    }

    if (s_clock_timer) {
        lv_timer_delete(s_clock_timer);
        s_clock_timer = NULL;
    }
    s_time_label = NULL;
    s_date_label = NULL;
    s_hint_label = NULL;
    s_clock_screen = NULL;

    ESP_LOGI(TAG, "clock_screen: screen deleted, timer + labels released");
}

static uint8_t next_brightness_level(uint8_t current)
{
    for (size_t i = 0U; i < sizeof(BRIGHTNESS_LEVELS); ++i) {
        if (BRIGHTNESS_LEVELS[i] > current) {
            return BRIGHTNESS_LEVELS[i];
        }
    }
    return BRIGHTNESS_LEVELS[0];
}

// M1 has no persistent settings/config command yet, but its exit gate must
// still exercise the M0 display controls on the real device. A release in
// the visual top half cycles 25% -> 50% -> 100% brightness; a release in the
// visual bottom half flips the landscape UI by 180 degrees. LVGL presents
// touch coordinates in the current logical space, so compare against its
// current logical height instead of the panel's portrait dimensions. The
// callback runs in LVGL context, and both board setters are safe there. M2
// replaces these diagnostic zones with carousel gestures and config-driven
// display settings.
static void clock_screen_released_cb(lv_event_t *event)
{
    (void)event;
    lv_indev_t *indev = lv_indev_active();
    if (indev == NULL) {
        return;
    }

    lv_point_t point;
    lv_indev_get_point(indev, &point);
    lv_display_t *display = lv_display_get_default();
    if (display == NULL) {
        return;
    }
    int32_t logical_height = lv_display_get_vertical_resolution(display);
    if (point.y < logical_height / 2) {
        uint8_t level = next_brightness_level(board_display_brightness());
        esp_err_t result = board_display_set_brightness(level);
        ESP_LOGI(TAG, "top-half release x=%d y=%d: brightness=%u (%s)",
                 (int)point.x, (int)point.y, (unsigned)level,
                 esp_err_to_name(result));
        return;
    }

    bool rotate = board_display_rotation_degrees() != 270U;
    esp_err_t result = board_display_set_rotation_180(rotate);
    if (result == ESP_OK) {
        invalidate_entire_screen();
    }
    ESP_LOGI(TAG, "bottom-half release x=%d y=%d: rotation=%u (%s)",
             (int)point.x, (int)point.y, rotate ? 180U : 0U,
             esp_err_to_name(result));
}

// Runs in LVGL task context (esp_lvgl_port's own timer handler already holds
// lvgl_port's lock while it services registered lv_timers) -- do NOT take
// lvgl_port_lock() in here, only at the external entry point below.
static void clock_timer_cb(lv_timer_t *timer)
{
    (void)timer;

    // External tasks publish into fixed atomics; the LVGL timer is the sole
    // consumer that mutates objects, so host traffic never allocates an
    // lv_async_call timer or blocks on the LVGL mutex.
    apply_online_state(NULL);
    if (atomic_exchange(&s_refresh_requested, false)) {
        s_last_minute = -1;
    }

    // Defensive: the screen may have been deleted (clearing these statics
    // via clock_screen_on_delete() above) in the window between this timer
    // firing and lv_timer_delete() actually taking effect.
    if (s_time_label == NULL || s_date_label == NULL) {
        return;
    }

    time_t now = time(NULL);
    time_t local_seconds = now +
        (time_t)(atomic_load(&s_utc_offset_minutes) * 60);
    struct tm tm_now;
    if (gmtime_r(&local_seconds, &tm_now) == NULL) {
        return;
    }

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

    // The CO5300 retains GRAM across soft resets and M1 uses partial render
    // buffers. Repaint the complete logical canvas on each minute change so
    // a text update cannot leave stale rotated pixels outside its new bounds.
    invalidate_entire_screen();

    // Once-per-minute heap log for Task 7's soak test.
    ESP_LOGI(TAG, "free heap: %u bytes", (unsigned)esp_get_free_heap_size());
}

void clock_screen_show_in_lvgl(void)
{
    if (s_clock_screen != NULL && lv_screen_active() == s_clock_screen) {
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
        ESP_LOGI(TAG, "clock_screen_show: already shown, skipping");
        return;
    }

    lv_obj_t *scr = lv_obj_create(NULL);
    if (scr == NULL) {
        ESP_LOGE(TAG, "clock_screen_show: screen allocation failed");
        return;
    }
    // Fresh screens default to LV_OBJ_FLAG_SCROLLABLE; a finger swipe would
    // otherwise drag this screen's labels around (human-caught Task 5 bug,
    // see board-notes.md). This screen has nothing to scroll to.
    lv_obj_remove_flag(scr, LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_style_bg_color(scr, lv_color_hex(0x101020), 0);
    lv_obj_set_style_bg_opa(scr, LV_OPA_COVER, 0);

    s_time_label = lv_label_create(scr);
    lv_obj_set_style_text_font(s_time_label, &lv_font_montserrat_48, 0);
    lv_obj_set_style_text_color(s_time_label, lv_color_hex(0xffffff), 0);
    lv_label_set_text(s_time_label, "00:00");
    lv_obj_align(s_time_label, LV_ALIGN_CENTER, 0, -40);

    s_date_label = lv_label_create(scr);
    lv_obj_set_style_text_color(s_date_label, lv_color_hex(0xa0a0b0), 0);
    lv_label_set_text(s_date_label, "");
    lv_obj_align_to(s_date_label, s_time_label, LV_ALIGN_OUT_BOTTOM_MID, 0, 12);

    s_hint_label = lv_label_create(scr);
    lv_obj_set_style_text_color(s_hint_label, lv_color_hex(0x505060), 0);
    lv_label_set_text(s_hint_label, "Connect deskmate app");
    lv_obj_align(s_hint_label, LV_ALIGN_BOTTOM_MID, 0, -16);
    if (atomic_load(&s_online)) {
        lv_obj_add_flag(s_hint_label, LV_OBJ_FLAG_HIDDEN);
    }

    lv_obj_add_event_cb(scr, clock_screen_released_cb, LV_EVENT_RELEASED, NULL);

    // Tears down the timer and NULLs the label statics if this screen is
    // ever deleted (M1's carousel) -- without this, the timer would keep
    // firing against freed lv_obj_t labels (use-after-free).
    lv_obj_add_event_cb(scr, clock_screen_on_delete, LV_EVENT_DELETE, NULL);
    s_clock_screen = scr;

    s_last_minute = -1;
    s_clock_timer = lv_timer_create(clock_timer_cb, 1000, NULL);
    // Populate real data immediately rather than waiting up to 1s for the
    // first tick (we're already holding the lock, so this is safe to call
    // directly here -- see the "do NOT lock inside the callback" note above,
    // which is about the periodic path, not this one-time initial call).
    clock_timer_cb(NULL);

    // A reset initializes the panel controller but does not promise to clear
    // its GRAM. Force one synchronous full-canvas render before returning so
    // corruption from a previous firmware/rotation cannot survive in regions
    // that the otherwise-static clock would never dirty.
    lv_obj_invalidate(scr);
    lv_screen_load_anim(scr, LV_SCREEN_LOAD_ANIM_NONE, 0U, 0U, true);
    lv_refr_now(lv_display_get_default());

    ESP_LOGI(TAG, "clock_screen_show: screen ready");
}

void clock_screen_show(void)
{
    lvgl_port_lock(0);
    clock_screen_show_in_lvgl();
    lvgl_port_unlock();
}

static void apply_online_state(void *unused)
{
    (void)unused;
    if (s_hint_label != NULL) {
        if (atomic_load(&s_online)) {
            lv_obj_add_flag(s_hint_label, LV_OBJ_FLAG_HIDDEN);
        } else {
            lv_obj_remove_flag(s_hint_label, LV_OBJ_FLAG_HIDDEN);
        }
        invalidate_entire_screen();
    }
}

void clock_screen_set_online(bool online)
{
    atomic_store(&s_online, online);
}

void clock_screen_set_utc_offset_minutes(int16_t offset_minutes)
{
    atomic_store(&s_utc_offset_minutes, offset_minutes);
    atomic_store(&s_refresh_requested, true);
}
