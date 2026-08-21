#include "ota_screen.h"

#include <stdio.h>

#include "clock_screen.h"
#include "esp_lvgl_port.h"
#include "lvgl.h"
#include "templates/template_internal.h"

#define OTA_TITLE_Y (7 * DESKMATE_GRID)
#define OTA_MODULE_Y (17 * DESKMATE_GRID)
#define OTA_MODULE_W (50 * DESKMATE_GRID)
#define OTA_MODULE_H (18 * DESKMATE_GRID)

static lv_obj_t *s_screen;
static lv_obj_t *s_percentage;
static bool s_active;

static void screen_deleted_cb(lv_event_t *event)
{
    if (lv_event_get_target_obj(event) == s_screen) {
        s_screen = NULL;
        s_percentage = NULL;
        s_active = false;
    }
}

/* Every LVGL call in this helper runs under ota_screen_show_progress()'s
 * lvgl_port lock. */
static bool create_screen_in_lvgl(void)
{
    lv_obj_t *screen = lv_obj_create(NULL);
    if (screen == NULL) {
        return false;
    }
    lv_obj_remove_flag(screen, LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_style_bg_color(screen, DESKMATE_COLOR_CANVAS, 0);
    lv_obj_set_style_bg_opa(screen, LV_OPA_COVER, 0);

    lv_obj_t *title = deskmate_label_box(
        screen, DESKMATE_MARGIN, OTA_TITLE_Y, OTA_MODULE_W,
        LV_TEXT_ALIGN_LEFT, DESKMATE_COLOR_PRIMARY, DESKMATE_FONT_DISPLAY);
    lv_label_set_text(title, "Updating");

    lv_obj_t *module = deskmate_module(
        screen, DESKMATE_MARGIN, OTA_MODULE_Y, OTA_MODULE_W, OTA_MODULE_H);
    (void)deskmate_eyebrow(module, "FIRMWARE", 2 * DESKMATE_GRID,
                           2 * DESKMATE_GRID, DESKMATE_COLOR_SECONDARY);
    s_percentage = deskmate_label_box(
        module, 2 * DESKMATE_GRID, 7 * DESKMATE_GRID,
        OTA_MODULE_W - 4 * DESKMATE_GRID, LV_TEXT_ALIGN_RIGHT,
        DESKMATE_COLOR_PRIMARY, DESKMATE_FONT_HERO);

    lv_obj_add_event_cb(screen, screen_deleted_cb, LV_EVENT_DELETE, NULL);
    s_screen = screen;
    s_active = true;
    lv_screen_load_anim(screen, LV_SCREEN_LOAD_ANIM_NONE, 0U, 0U, true);
    return true;
}

bool ota_screen_show_progress(uint8_t percentage)
{
    if (percentage > 100U) {
        percentage = 100U;
    }
    if (!lvgl_port_lock(0U)) {
        return false;
    }
    bool ready = s_screen != NULL || create_screen_in_lvgl();
    if (ready && s_percentage != NULL) {
        char text[5];
        int length = snprintf(text, sizeof(text), "%u%%",
                              (unsigned)percentage);
        if (length > 0 && (size_t)length < sizeof(text)) {
            lv_label_set_text(s_percentage, text);
            lv_obj_invalidate(s_screen);
        } else {
            ready = false;
        }
    }
    lvgl_port_unlock();
    return ready;
}

void ota_screen_close(void)
{
    if (!lvgl_port_lock(0U)) {
        return;
    }
    if (s_screen != NULL) {
        /* Loads a fresh clock and auto-deletes this takeover screen. */
        clock_screen_show_in_lvgl();
    }
    lvgl_port_unlock();
}

bool ota_screen_active_in_lvgl(void)
{
    return s_active;
}
