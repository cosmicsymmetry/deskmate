#include "template_internal.h"

#include <stdio.h>
#include <string.h>
#include <time.h>

#include "core/clock_source.h"
#include "core/timefmt.h"

enum {
    OBJ_TIME,
    OBJ_SECONDS,
    OBJ_DATE,
    OBJ_DIAL,
    OBJ_HAND_HOUR,
    OBJ_HAND_MINUTE,
    OBJ_STATE,
};

/* The reading is anchored to the left margin rather than centred: a hero
 * that starts on the same rail as the modules below it gives the face one
 * vertical edge to hang everything from, and it leaves the top-right free
 * for the seconds. Nothing labels the face — a clock names itself — so the
 * 248px hero-plus-modules stack centres on the canvas, leaving 64 above and
 * 56 below: very slightly top-heavy, which is what optical centring wants. */
#define TIME_Y      (8 * DESKMATE_GRID)
#define MODULE_Y    (22 * DESKMATE_GRID)
#define MODULE_H    (17 * DESKMATE_GRID)
#define DATE_W      (28 * DESKMATE_GRID)
#define DIAL_X      (33 * DESKMATE_GRID)
#define DIAL_W      (20 * DESKMATE_GRID)
#define DIAL_BOX    (14 * DESKMATE_GRID)

/* A 12-hour dial in minutes, so one scale positions both hands: the hour
 * hand at h*60+m and the minute hand at m*12. */
#define DIAL_RANGE      720
#define HAND_HOUR_LEN   28
#define HAND_MINUTE_LEN 42

/* Distance from a label box's top edge to the baseline of the type in it.
 * `base_line` is measured up from the bottom of the line box. */
static int32_t baseline_offset(const lv_font_t *font)
{
    return lv_font_get_line_height(font) - font->base_line;
}

bool digital_clock_create(template_widget_view_t *view,
                          lv_obj_t *parent,
                          protocol_size_class_t size)
{
    if (view == NULL || parent == NULL ||
        (size != PROTOCOL_SIZE_FULL && size != PROTOCOL_SIZE_STANDARD)) {
        return false;
    }
    memset(view, 0, sizeof(*view));
    const deskmate_palette_t palette =
        deskmate_palette(PROTOCOL_TEMPLATE_DIGITAL_CLOCK);
    /* Matches the schema default for `show_seconds`, so a view that ticks
     * before its first patch shows the same face the host asked for. */
    view->clock_show_seconds = true;
    view->root = lv_obj_create(parent);
    lv_obj_remove_style_all(view->root);
    lv_obj_remove_flag(view->root,
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->root, LV_PCT(100), LV_PCT(100));

    view->objects[OBJ_TIME] = lv_label_create(view->root);
    lv_obj_set_style_text_font(view->objects[OBJ_TIME], DESKMATE_FONT_HERO, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_TIME],
                                DESKMATE_COLOR_PRIMARY, 0);
    lv_label_set_text(view->objects[OBJ_TIME], "00:00");
    lv_obj_set_pos(view->objects[OBJ_TIME], DESKMATE_MARGIN, TIME_Y);

    /* The seconds are a superior figure in the face hue, sharing the hero's
     * baseline: the watch idiom for "this is live", and it keeps the
     * seconds out of the reading you actually want. Hiding it moves
     * nothing, because the hero is left-anchored rather than centred. */
    view->objects[OBJ_SECONDS] = lv_label_create(view->root);
    lv_obj_set_style_text_font(view->objects[OBJ_SECONDS],
                               DESKMATE_FONT_DISPLAY, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_SECONDS], palette.hue, 0);
    lv_label_set_text(view->objects[OBJ_SECONDS], "00");

    /* Two modules under the reading. The date is the one thing a wall clock
     * is always also asked for; the dial restates the same instant in the
     * notation the eye reads fastest at a glance. */
    lv_obj_t *date_module = deskmate_module(view->root, DESKMATE_MARGIN,
                                            MODULE_Y, DATE_W, MODULE_H);
    /* The date is the module's whole content — no eyebrow names it, since a
     * date needs no naming — so the value centres in the surface. */
    const int32_t value_line = lv_font_get_line_height(DESKMATE_FONT_BODY);
    const int32_t stack_top = (MODULE_H - value_line) / 2;
    view->objects[OBJ_DATE] = deskmate_label_box(
        date_module, DESKMATE_MARGIN, stack_top,
        DATE_W - 2 * DESKMATE_MARGIN, LV_TEXT_ALIGN_LEFT,
        DESKMATE_COLOR_PRIMARY, DESKMATE_FONT_BODY);

    lv_obj_t *dial_module =
        deskmate_module(view->root, DIAL_X, MODULE_Y, DIAL_W, MODULE_H);

    view->objects[OBJ_DIAL] = lv_scale_create(dial_module);
    lv_obj_set_size(view->objects[OBJ_DIAL], DIAL_BOX, DIAL_BOX);
    lv_obj_set_pos(view->objects[OBJ_DIAL], (DIAL_W - DIAL_BOX) / 2,
                   (MODULE_H - DIAL_BOX) / 2);
    lv_scale_set_mode(view->objects[OBJ_DIAL], LV_SCALE_MODE_ROUND_INNER);
    lv_scale_set_label_show(view->objects[OBJ_DIAL], false);
    /* Thirteen ticks over a full turn puts one at every hour, with the
     * thirteenth landing back on twelve. */
    lv_scale_set_total_tick_count(view->objects[OBJ_DIAL], 13);
    lv_scale_set_major_tick_every(view->objects[OBJ_DIAL], 3);
    lv_scale_set_range(view->objects[OBJ_DIAL], 0, DIAL_RANGE);
    lv_scale_set_angle_range(view->objects[OBJ_DIAL], 360);
    lv_scale_set_rotation(view->objects[OBJ_DIAL], 270);
    lv_obj_set_style_arc_width(view->objects[OBJ_DIAL], 0, LV_PART_MAIN);
    lv_obj_set_style_line_color(view->objects[OBJ_DIAL],
                                DESKMATE_COLOR_TERTIARY, LV_PART_ITEMS);
    lv_obj_set_style_line_width(view->objects[OBJ_DIAL], 2, LV_PART_ITEMS);
    lv_obj_set_style_length(view->objects[OBJ_DIAL], 6, LV_PART_ITEMS);
    lv_obj_set_style_line_color(view->objects[OBJ_DIAL], palette.hue,
                                LV_PART_INDICATOR);
    lv_obj_set_style_line_width(view->objects[OBJ_DIAL], 3,
                                LV_PART_INDICATOR);
    lv_obj_set_style_length(view->objects[OBJ_DIAL], 11, LV_PART_INDICATOR);
    lv_obj_set_style_line_opa(view->objects[OBJ_DIAL], LV_OPA_70,
                              LV_PART_INDICATOR);

    /* Needle lines must be children of the scale: lv_scale positions them
     * from its own box centre and aligns them to its top-left. LVGL owns
     * their point arrays and frees them with the object. */
    view->objects[OBJ_HAND_HOUR] = lv_line_create(view->objects[OBJ_DIAL]);
    lv_obj_set_style_line_color(view->objects[OBJ_HAND_HOUR],
                                DESKMATE_COLOR_PRIMARY, 0);
    lv_obj_set_style_line_width(view->objects[OBJ_HAND_HOUR], 6, 0);
    lv_obj_set_style_line_rounded(view->objects[OBJ_HAND_HOUR], true, 0);

    view->objects[OBJ_HAND_MINUTE] = lv_line_create(view->objects[OBJ_DIAL]);
    lv_obj_set_style_line_color(view->objects[OBJ_HAND_MINUTE], palette.hue,
                                0);
    lv_obj_set_style_line_width(view->objects[OBJ_HAND_MINUTE], 4, 0);
    lv_obj_set_style_line_rounded(view->objects[OBJ_HAND_MINUTE], true, 0);

    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_MID, 0,
                 -2 * DESKMATE_GRID);
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
    /* `title` is still a schema and wire field — it names the card in the
     * companion's library — but no clock face draws it, so nothing here
     * reads it. */
    const template_field_value_t *show = template_fields_get(
        fields, "show_seconds");
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

    /* The hero is content-sized, so its width is only final once LVGL has
     * run the layout. Re-anchoring the seconds after that keeps the two on
     * a shared baseline whatever the tier metrics are. */
    lv_obj_update_layout(view->root);
    lv_obj_align_to(view->objects[OBJ_SECONDS], view->objects[OBJ_TIME],
                    LV_ALIGN_OUT_RIGHT_TOP, 2 * DESKMATE_GRID,
                    baseline_offset(DESKMATE_FONT_HERO) -
                        baseline_offset(DESKMATE_FONT_DISPLAY));

    lv_scale_set_line_needle_value(view->objects[OBJ_DIAL],
                                   view->objects[OBJ_HAND_HOUR],
                                   HAND_HOUR_LEN,
                                   (now.tm_hour % 12) * 60 + now.tm_min);
    lv_scale_set_line_needle_value(view->objects[OBJ_DIAL],
                                   view->objects[OBJ_HAND_MINUTE],
                                   HAND_MINUTE_LEN, now.tm_min * 12);
}
