#include "template_internal.h"

#include <stdio.h>
#include <string.h>
#include <time.h>

#include "core/timefmt.h"

enum {
    OBJ_TITLE,
    OBJ_TIME,
    OBJ_DATE,
    OBJ_STATE,
};

static bool s_show_seconds = true;

bool digital_clock_create(template_widget_view_t *view,
                          lv_obj_t *parent,
                          protocol_size_class_t size)
{
    if (view == NULL || parent == NULL ||
        (size != PROTOCOL_SIZE_FULL && size != PROTOCOL_SIZE_STANDARD)) {
        return false;
    }
    memset(view, 0, sizeof(*view));
    view->root = lv_obj_create(parent);
    lv_obj_remove_style_all(view->root);
    lv_obj_set_size(view->root, LV_PCT(100), LV_PCT(100));

    view->objects[OBJ_TITLE] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_TITLE],
                                lv_color_hex(0x8f93a8), 0);
    lv_obj_align(view->objects[OBJ_TITLE], LV_ALIGN_TOP_MID, 0,
                 size == PROTOCOL_SIZE_FULL ? 36 : 18);

    view->objects[OBJ_TIME] = lv_label_create(view->root);
    lv_obj_set_style_text_font(view->objects[OBJ_TIME],
                               &lv_font_montserrat_48, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_TIME],
                                lv_color_hex(0xffffff), 0);
    lv_obj_align(view->objects[OBJ_TIME], LV_ALIGN_CENTER, 0, -22);

    view->objects[OBJ_DATE] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_DATE],
                                lv_color_hex(0xb3b6c7), 0);
    lv_obj_align_to(view->objects[OBJ_DATE], view->objects[OBJ_TIME],
                    LV_ALIGN_OUT_BOTTOM_MID, 0, 12);

    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_MID, 0, -14);
    view->state_label = view->objects[OBJ_STATE];
    return true;
}

void digital_clock_patch(template_widget_view_t *view,
                         const template_field_state_t *fields,
                         uint16_t dirty_mask)
{
    (void)dirty_mask;
    if (view == NULL || view->root == NULL || fields == NULL) {
        return;
    }
    const template_field_value_t *title = template_fields_get(fields,
                                                               "title");
    const template_field_value_t *show = template_fields_get(
        fields, "show_seconds");
    if (title != NULL) {
        lv_label_set_text(view->objects[OBJ_TITLE], title->value.text);
    }
    if (show != NULL) {
        s_show_seconds = show->value.boolean;
    }
}

void digital_clock_tick(template_widget_view_t *view,
                        int16_t utc_offset_minutes)
{
    if (view == NULL || view->root == NULL) {
        return;
    }
    time_t local = time(NULL) + (time_t)utc_offset_minutes * 60;
    struct tm now;
    if (gmtime_r(&local, &now) == NULL) {
        return;
    }
    char time_text[9];
    if (s_show_seconds) {
        snprintf(time_text, sizeof(time_text), "%02d:%02d:%02d",
                 now.tm_hour, now.tm_min, now.tm_sec);
    } else {
        timefmt_hhmm(time_text, now.tm_hour, now.tm_min);
    }
    lv_label_set_text(view->objects[OBJ_TIME], time_text);
    char date[32];
    timefmt_date(date, now.tm_year + 1900, now.tm_mon + 1, now.tm_mday,
                 (now.tm_wday + 6) % 7);
    lv_label_set_text(view->objects[OBJ_DATE], date);
}
