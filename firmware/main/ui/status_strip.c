#include "status_strip.h"

#include <string.h>
#include <time.h>

#include "core/timefmt.h"

static lv_obj_t *create_dot(lv_obj_t *parent, int32_t right_offset)
{
    lv_obj_t *dot = lv_obj_create(parent);
    lv_obj_remove_style_all(dot);
    lv_obj_set_size(dot, 7, 7);
    lv_obj_set_style_radius(dot, LV_RADIUS_CIRCLE, 0);
    lv_obj_align(dot, LV_ALIGN_RIGHT_MID, right_offset, 0);
    return dot;
}

bool status_strip_create(status_strip_t *strip, lv_obj_t *parent)
{
    if (strip == NULL || parent == NULL) {
        return false;
    }
    memset(strip, 0, sizeof(*strip));
    strip->root = lv_obj_create(parent);
    lv_obj_remove_style_all(strip->root);
    lv_obj_set_size(strip->root, LV_PCT(100), STATUS_STRIP_HEIGHT);
    lv_obj_align(strip->root, LV_ALIGN_TOP_MID, 0, 0);
    lv_obj_set_style_bg_color(strip->root, lv_color_hex(0x161725), 0);
    lv_obj_set_style_bg_opa(strip->root, LV_OPA_COVER, 0);

    strip->time_label = lv_label_create(strip->root);
    lv_obj_set_style_text_color(strip->time_label, lv_color_hex(0xaeb2c4), 0);
    lv_obj_align(strip->time_label, LV_ALIGN_LEFT_MID, 12, 0);

    strip->connection_dot = create_dot(strip->root, -12);
    strip->interrupt_dots[0] = create_dot(strip->root, -30);
    strip->interrupt_dots[1] = create_dot(strip->root, -43);
    status_strip_set_online(strip, false);
    status_strip_set_interrupts(strip, false, false);
    return true;
}

void status_strip_set_online(status_strip_t *strip, bool online)
{
    if (strip == NULL || strip->connection_dot == NULL) {
        return;
    }
    lv_obj_set_style_bg_color(strip->connection_dot,
                              online ? lv_color_hex(0x57d38c)
                                     : lv_color_hex(0x55586a), 0);
}

void status_strip_set_interrupts(status_strip_t *strip,
                                 bool active,
                                 bool pending)
{
    if (strip == NULL || strip->interrupt_dots[0] == NULL ||
        strip->interrupt_dots[1] == NULL) {
        return;
    }
    bool visible[2] = {active, pending};
    for (size_t i = 0U; i < 2U; ++i) {
        if (visible[i]) {
            lv_obj_remove_flag(strip->interrupt_dots[i], LV_OBJ_FLAG_HIDDEN);
            lv_obj_set_style_bg_color(strip->interrupt_dots[i],
                                      lv_color_hex(0xff7a90), 0);
        } else {
            lv_obj_add_flag(strip->interrupt_dots[i], LV_OBJ_FLAG_HIDDEN);
        }
    }
}

void status_strip_tick(status_strip_t *strip, int16_t utc_offset_minutes)
{
    if (strip == NULL || strip->time_label == NULL) {
        return;
    }
    time_t local = time(NULL) + (time_t)utc_offset_minutes * 60;
    struct tm now;
    if (gmtime_r(&local, &now) == NULL) {
        return;
    }
    char text[6];
    timefmt_hhmm(text, now.tm_hour, now.tm_min);
    lv_label_set_text(strip->time_label, text);
}

void status_strip_reset(status_strip_t *strip)
{
    if (strip != NULL) {
        memset(strip, 0, sizeof(*strip));
    }
}
