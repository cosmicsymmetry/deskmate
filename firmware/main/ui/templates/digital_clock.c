#include "template_internal.h"

#include <stdio.h>
#include <string.h>
#include <time.h>

#include "core/clock_source.h"
#include "core/timefmt.h"

enum {
    OBJ_TITLE,
    OBJ_TIME_ROW,
    OBJ_TIME,
    OBJ_SECONDS,
    OBJ_DATE,
    OBJ_STATE,
};

bool digital_clock_create(template_widget_view_t *view,
                          lv_obj_t *parent,
                          protocol_size_class_t size)
{
    if (view == NULL || parent == NULL ||
        (size != PROTOCOL_SIZE_FULL && size != PROTOCOL_SIZE_STANDARD)) {
        return false;
    }
    memset(view, 0, sizeof(*view));
    /* Matches the schema default for `show_seconds`, so a view that ticks
     * before its first patch shows the same face the host asked for. */
    view->clock_show_seconds = true;
    view->root = lv_obj_create(parent);
    lv_obj_remove_style_all(view->root);
    lv_obj_remove_flag(view->root,
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->root, LV_PCT(100), LV_PCT(100));

    view->objects[OBJ_TITLE] = lv_label_create(view->root);
    lv_obj_set_style_text_font(view->objects[OBJ_TITLE],
                               DESKMATE_FONT_CAPTION, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_TITLE],
                                DESKMATE_COLOR_SECONDARY, 0);
    lv_label_set_long_mode(view->objects[OBJ_TITLE], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_TITLE], 448 - 2 * DESKMATE_MARGIN);
    lv_obj_set_height(view->objects[OBJ_TITLE],
                      lv_font_get_line_height(DESKMATE_FONT_CAPTION));
    lv_obj_set_style_text_align(view->objects[OBJ_TITLE],
                                LV_TEXT_ALIGN_CENTER, 0);
    lv_obj_align(view->objects[OBJ_TITLE], LV_ALIGN_TOP_MID, 0,
                 DESKMATE_MARGIN);
    lv_label_set_text(view->objects[OBJ_TITLE], "");

    /* HH:MM and SS are separate labels inside a content-sized flex row.
     *
     * A single "HH:MM:SS" run at DESKMATE_FONT_HERO measures 434px — the
     * tabular digit advance is 62.1px and the colon 30.6px — which overruns
     * the 400px content width between the canvas margins. Splitting the
     * seconds onto their own DISPLAY-tier label brings the group to 368px,
     * and buys the hierarchy a wall clock wants anyway: the hour and minute
     * are the reading, the seconds are the proof it is live.
     *
     * The row is LV_SIZE_CONTENT and centre-aligned, so hiding the seconds
     * label re-centres HH:MM by itself with no per-state coordinates. Cross
     * alignment is END (bottom): both numeric tiers carry base_line = 1, so
     * a shared box bottom is a shared baseline. */
    view->objects[OBJ_TIME_ROW] = lv_obj_create(view->root);
    lv_obj_remove_style_all(view->objects[OBJ_TIME_ROW]);
    lv_obj_remove_flag(view->objects[OBJ_TIME_ROW],
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->objects[OBJ_TIME_ROW], LV_SIZE_CONTENT,
                    LV_SIZE_CONTENT);
    lv_obj_set_flex_flow(view->objects[OBJ_TIME_ROW], LV_FLEX_FLOW_ROW);
    lv_obj_set_flex_align(view->objects[OBJ_TIME_ROW], LV_FLEX_ALIGN_CENTER,
                          LV_FLEX_ALIGN_END, LV_FLEX_ALIGN_CENTER);
    lv_obj_set_style_pad_column(view->objects[OBJ_TIME_ROW],
                                2 * DESKMATE_GRID, 0);
    lv_obj_align(view->objects[OBJ_TIME_ROW], LV_ALIGN_CENTER, 0,
                 -2 * DESKMATE_GRID);

    view->objects[OBJ_TIME] = lv_label_create(view->objects[OBJ_TIME_ROW]);
    lv_obj_set_style_text_font(view->objects[OBJ_TIME], DESKMATE_FONT_HERO, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_TIME],
                                DESKMATE_COLOR_PRIMARY, 0);
    lv_label_set_text(view->objects[OBJ_TIME], "00:00");

    view->objects[OBJ_SECONDS] = lv_label_create(view->objects[OBJ_TIME_ROW]);
    lv_obj_set_style_text_font(view->objects[OBJ_SECONDS],
                               DESKMATE_FONT_DISPLAY, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_SECONDS],
                                DESKMATE_COLOR_SECONDARY, 0);
    lv_label_set_text(view->objects[OBJ_SECONDS], "00");

    view->objects[OBJ_DATE] = lv_label_create(view->root);
    lv_obj_set_style_text_font(view->objects[OBJ_DATE],
                               DESKMATE_FONT_BODY, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_DATE],
                                DESKMATE_COLOR_TERTIARY, 0);
    lv_label_set_text(view->objects[OBJ_DATE], "");

    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_MID, 0,
                 -DESKMATE_MARGIN / 2);
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
        view->clock_show_seconds = show->value.boolean;
        if (view->clock_show_seconds) {
            lv_obj_remove_flag(view->objects[OBJ_SECONDS],
                               LV_OBJ_FLAG_HIDDEN);
        } else {
            lv_obj_add_flag(view->objects[OBJ_SECONDS], LV_OBJ_FLAG_HIDDEN);
        }
    }
}

void digital_clock_tick(template_widget_view_t *view,
                        int16_t utc_offset_minutes)
{
    if (view == NULL || view->root == NULL) {
        return;
    }
    time_t local = (time_t)clock_source_now() +
                   (time_t)utc_offset_minutes * 60;
    struct tm now;
    if (gmtime_r(&local, &now) == NULL) {
        return;
    }
    char time_text[6];
    timefmt_hhmm(time_text, now.tm_hour, now.tm_min);
    lv_label_set_text(view->objects[OBJ_TIME], time_text);
    char seconds_text[3];
    snprintf(seconds_text, sizeof(seconds_text), "%02d", now.tm_sec);
    lv_label_set_text(view->objects[OBJ_SECONDS], seconds_text);
    char date[32];
    timefmt_date(date, now.tm_year + 1900, now.tm_mon + 1, now.tm_mday,
                 (now.tm_wday + 6) % 7);
    lv_label_set_text(view->objects[OBJ_DATE], date);
    /* The flex row is content-sized: its width changes when the seconds
     * label is hidden, and its height is only final once LVGL has run the
     * layout. Re-anchor the date after that so the gap under the time stays
     * exactly 2 * DESKMATE_GRID in both states. */
    lv_obj_update_layout(view->root);
    lv_obj_align_to(view->objects[OBJ_DATE], view->objects[OBJ_TIME_ROW],
                    LV_ALIGN_OUT_BOTTOM_MID, 0, 3 * DESKMATE_GRID);
}
