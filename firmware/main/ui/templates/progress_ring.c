#include "template_internal.h"

#include <stdio.h>
#include <string.h>

enum {
    OBJ_ARC,
    OBJ_REMAINING,
    OBJ_LABEL,
    OBJ_TOTAL,
    OBJ_ELAPSED,
    OBJ_STATUS,
    OBJ_STATE,
};

/* The ring sits left of centre so a column of modules can carry the numbers
 * the arc can only imply. Everything on the face is derived from the same
 * three fields the arc is drawn from — no new wire data. */
#define RING_DIAMETER (30 * DESKMATE_GRID)
#define RING_X        (2 * DESKMATE_GRID)
#define RING_Y        (8 * DESKMATE_GRID)
#define RING_CENTER_X (RING_X + RING_DIAMETER / 2)
#define RING_CENTER_Y (RING_Y + RING_DIAMETER / 2)
/* Fat enough to read as a gauge rather than a hairline, with rounded caps
 * so a partly-run timer has ends rather than cuts. */
#define RING_WIDTH    26

#define CHIP_X      (34 * DESKMATE_GRID)
#define CHIP_W      (19 * DESKMATE_GRID)
#define CHIP_H      (10 * DESKMATE_GRID)
#define CHIP_GAP    (2 * DESKMATE_GRID)
#define CHIP_FIRST_Y (6 * DESKMATE_GRID)
#define CHIP_PAD    (2 * DESKMATE_GRID)

/* Widest chord that stays clear of the ring's inner edge. */
#define RING_TEXT_W (25 * DESKMATE_GRID)
#define RING_TEXT_X (RING_CENTER_X - RING_TEXT_W / 2)

static int64_t current_remaining_ms(const template_widget_view_t *view,
                                    uint32_t now_ms)
{
    if (!view->progress_running) {
        return view->progress_remaining_ms;
    }
    uint32_t elapsed = now_ms - view->progress_anchor_ms;
    return (int64_t)elapsed >= view->progress_remaining_ms
               ? 0
               : view->progress_remaining_ms - elapsed;
}

/* The registry bounds duration_seconds and remaining_seconds to 86400, but
 * this face must not depend on that: the preview harness writes field state
 * straight in without going through template_fields_resolve, and the wire is
 * untrusted by policy. Clamping to the registry's own ceiling keeps the
 * result inside MMMM:SS — and lets the compiler prove the caller's buffer is
 * big enough, which an unclamped int64 minute count does not. */
#define CLOCK_SECONDS_MAX 86400

static void format_clock(char *out, size_t size, int64_t seconds)
{
    if (seconds < 0) {
        seconds = 0;
    } else if (seconds > CLOCK_SECONDS_MAX) {
        seconds = CLOCK_SECONDS_MAX;
    }
    snprintf(out, size, "%02u:%02u", (unsigned)(seconds / 60),
             (unsigned)(seconds % 60));
}

/* The one word the timer's state is worth saying, derived from the same
 * `running`/`remaining`/`duration` the ring is drawn from. A running timer
 * used to say nothing, on the theory that the shrinking arc says it; with a
 * status module on the face there is a slot that should never read empty. */
static const char *status_word(const template_widget_view_t *view,
                               int64_t remaining_seconds)
{
    if (remaining_seconds <= 0) {
        return "Done";
    }
    if (view->progress_running) {
        return "Running";
    }
    if (view->progress_duration_seconds > 0 &&
        remaining_seconds >= view->progress_duration_seconds) {
        return "Ready";
    }
    return "Paused";
}

/* Fills the module column from the countdown the ring already shows. */
static void refresh_modules(template_widget_view_t *view,
                            int64_t remaining_ms)
{
    if (view->progress_duration_seconds <= 0) {
        return;
    }
    int64_t remaining = (remaining_ms + 999) / 1000;
    if (remaining < 0) {
        remaining = 0;
    }
    int64_t elapsed = view->progress_duration_seconds - remaining;
    char text[16];
    format_clock(text, sizeof(text), view->progress_duration_seconds);
    lv_label_set_text(view->objects[OBJ_TOTAL], text);
    format_clock(text, sizeof(text), elapsed);
    lv_label_set_text(view->objects[OBJ_ELAPSED], text);
    lv_label_set_text(view->objects[OBJ_STATUS],
                      status_word(view, remaining));
}

static void set_running_color(template_widget_view_t *view, bool running)
{
    view->progress_running = running;
    const deskmate_palette_t palette =
        deskmate_palette(PROTOCOL_TEMPLATE_PROGRESS_RING);
    /* An idle ring drops to the neutral ramp: the hue is reserved for the
     * element that is actually doing something, which is what makes a
     * running timer readable across the room. TERTIARY rather than
     * SECONDARY, because a full circle of SECONDARY on a never-started
     * timer out-shouts the countdown inside it. */
    lv_obj_set_style_arc_color(view->objects[OBJ_ARC],
                               running ? palette.hue
                                       : DESKMATE_COLOR_TERTIARY,
                               LV_PART_INDICATOR);
    lv_obj_set_style_text_color(view->objects[OBJ_STATUS],
                                running ? palette.hue
                                        : DESKMATE_COLOR_PRIMARY, 0);
    refresh_modules(view, current_remaining_ms(view, lv_tick_get()));
}

static void render_remaining(template_widget_view_t *view,
                             int64_t remaining_ms)
{
    if (view->progress_duration_seconds <= 0) {
        return;
    }
    int64_t duration_ms = view->progress_duration_seconds * INT64_C(1000);
    int64_t value = remaining_ms * 1000 / duration_ms;
    lv_arc_set_value(view->objects[OBJ_ARC], (int32_t)value);
    refresh_modules(view, remaining_ms);
    int64_t seconds = (remaining_ms + 999) / 1000;
    if (seconds == view->progress_displayed_seconds) {
        return;
    }
    view->progress_displayed_seconds = seconds;
    char text[24];
    format_clock(text, sizeof(text), seconds);
    lv_label_set_text(view->objects[OBJ_REMAINING], text);
}

/* One module: an eyebrow over a value, the pair centred in the surface. */
static lv_obj_t *build_chip(lv_obj_t *parent, int32_t y, const char *caption,
                            lv_color_t caption_color, lv_color_t value_color)
{
    lv_obj_t *module = deskmate_module(parent, CHIP_X, y, CHIP_W, CHIP_H);
    const int32_t eyebrow_line =
        lv_font_get_line_height(DESKMATE_FONT_CAPTION);
    const int32_t value_line = lv_font_get_line_height(DESKMATE_FONT_BODY);
    const int32_t stack_top =
        (CHIP_H - eyebrow_line - DESKMATE_GRID - value_line) / 2;
    deskmate_eyebrow(module, caption, CHIP_PAD, stack_top, caption_color);
    return deskmate_label_box(module, CHIP_PAD,
                              stack_top + eyebrow_line + DESKMATE_GRID,
                              CHIP_W - 2 * CHIP_PAD, LV_TEXT_ALIGN_LEFT,
                              value_color, DESKMATE_FONT_BODY);
}

bool progress_ring_create(template_widget_view_t *view,
                          lv_obj_t *parent,
                          protocol_size_class_t size)
{
    if (view == NULL || parent == NULL ||
        (size != PROTOCOL_SIZE_FULL && size != PROTOCOL_SIZE_STANDARD)) {
        return false;
    }
    memset(view, 0, sizeof(*view));
    const deskmate_palette_t palette =
        deskmate_palette(PROTOCOL_TEMPLATE_PROGRESS_RING);
    view->progress_displayed_seconds = -1;
    view->root = lv_obj_create(parent);
    lv_obj_remove_style_all(view->root);
    lv_obj_remove_flag(view->root,
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->root, LV_PCT(100), LV_PCT(100));

    view->objects[OBJ_ARC] = lv_arc_create(view->root);
    lv_obj_set_size(view->objects[OBJ_ARC], RING_DIAMETER, RING_DIAMETER);
    lv_obj_set_pos(view->objects[OBJ_ARC], RING_X, RING_Y);
    lv_arc_set_range(view->objects[OBJ_ARC], 0, 1000);
    lv_arc_set_rotation(view->objects[OBJ_ARC], 270);
    /* LVGL's arc defaults to a 270-degree gauge with a gap at the bottom. A
     * countdown is a whole circle that empties from twelve o'clock. */
    lv_arc_set_bg_angles(view->objects[OBJ_ARC], 0, 360);
    lv_obj_remove_style(view->objects[OBJ_ARC], NULL, LV_PART_KNOB);
    lv_obj_remove_flag(view->objects[OBJ_ARC], LV_OBJ_FLAG_CLICKABLE);
    /* The track is the same hue held far back, not a neutral grey, so the
     * ring reads as one object with a spent part rather than two rings. */
    lv_obj_set_style_arc_color(view->objects[OBJ_ARC], palette.hue,
                               LV_PART_MAIN);
    lv_obj_set_style_arc_opa(view->objects[OBJ_ARC], LV_OPA_20, LV_PART_MAIN);
    lv_obj_set_style_arc_color(view->objects[OBJ_ARC], palette.hue,
                               LV_PART_INDICATOR);
    lv_obj_set_style_arc_width(view->objects[OBJ_ARC], RING_WIDTH,
                               LV_PART_MAIN);
    lv_obj_set_style_arc_width(view->objects[OBJ_ARC], RING_WIDTH,
                               LV_PART_INDICATOR);
    lv_obj_set_style_arc_rounded(view->objects[OBJ_ARC], true,
                                 LV_PART_INDICATOR);

    /* The ring's interior carries the name and the countdown as one centred
     * pair; the state word lives in its own module rather than under the
     * countdown, so the slot is never empty. */
    const int32_t label_line = lv_font_get_line_height(DESKMATE_FONT_CAPTION);
    const int32_t time_line = lv_font_get_line_height(DESKMATE_FONT_DISPLAY);
    const int32_t pair_top =
        RING_CENTER_Y - (label_line + DESKMATE_GRID + time_line) / 2;
    view->objects[OBJ_LABEL] = deskmate_label_box(
        view->root, RING_TEXT_X, pair_top, RING_TEXT_W,
        LV_TEXT_ALIGN_CENTER, palette.tint, DESKMATE_FONT_CAPTION);
    view->objects[OBJ_REMAINING] = deskmate_label_box(
        view->root, RING_TEXT_X, pair_top + label_line + DESKMATE_GRID,
        RING_TEXT_W, LV_TEXT_ALIGN_CENTER, DESKMATE_COLOR_PRIMARY,
        DESKMATE_FONT_DISPLAY);
    lv_label_set_text(view->objects[OBJ_REMAINING], "00:00");

    view->objects[OBJ_TOTAL] =
        build_chip(view->root, CHIP_FIRST_Y, "TOTAL",
                   DESKMATE_COLOR_SECONDARY, DESKMATE_COLOR_PRIMARY);
    view->objects[OBJ_ELAPSED] =
        build_chip(view->root, CHIP_FIRST_Y + CHIP_H + CHIP_GAP, "ELAPSED",
                   DESKMATE_COLOR_SECONDARY, palette.tint);
    view->objects[OBJ_STATUS] =
        build_chip(view->root, CHIP_FIRST_Y + 2 * (CHIP_H + CHIP_GAP),
                   "STATUS", DESKMATE_COLOR_SECONDARY, palette.hue);

    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_MID, 0,
                 -2 * DESKMATE_GRID);
    view->state_label = view->objects[OBJ_STATE];
    return true;
}

void progress_ring_patch(template_widget_view_t *view,
                         const template_field_state_t *fields,
                         uint16_t dirty_mask)
{
    (void)dirty_mask;
    if (view == NULL || view->root == NULL || fields == NULL) {
        return;
    }
    const template_field_value_t *duration = template_fields_get(
        fields, "duration_seconds");
    const template_field_value_t *remaining = template_fields_get(
        fields, "remaining_seconds");
    const template_field_value_t *label = template_fields_get(fields,
                                                               "label");
    const template_field_value_t *running = template_fields_get(fields,
                                                                 "running");
    if (duration != NULL && remaining != NULL &&
        duration->value.integer > 0) {
        view->progress_duration_seconds = duration->value.integer;
        view->progress_remaining_ms =
            remaining->value.integer * INT64_C(1000);
        view->progress_anchor_ms = lv_tick_get();
        view->progress_displayed_seconds = -1;
        render_remaining(view, view->progress_remaining_ms);
    }
    if (label != NULL) {
        lv_label_set_text(view->objects[OBJ_LABEL], label->value.text);
    }
    if (running != NULL) {
        view->progress_anchor_ms = lv_tick_get();
        set_running_color(view, running->value.boolean);
    }
}

void progress_ring_local_action(template_widget_view_t *view,
                                protocol_event_action_t action)
{
    if (view == NULL || view->root == NULL) {
        return;
    }
    if (action == PROTOCOL_EVENT_ACTION_START_PAUSE) {
        uint32_t now = lv_tick_get();
        if (view->progress_running) {
            view->progress_remaining_ms = current_remaining_ms(view, now);
        } else {
            view->progress_anchor_ms = now;
        }
        set_running_color(view, !view->progress_running);
    } else if (action == PROTOCOL_EVENT_ACTION_RESET &&
               view->progress_duration_seconds > 0) {
        view->progress_remaining_ms =
            view->progress_duration_seconds * INT64_C(1000);
        view->progress_displayed_seconds = -1;
        view->progress_anchor_ms = lv_tick_get();
        render_remaining(view, view->progress_remaining_ms);
        set_running_color(view, false);
    }
}

void progress_ring_tick(template_widget_view_t *view)
{
    if (view == NULL || view->root == NULL || !view->progress_running) {
        return;
    }
    render_remaining(view, current_remaining_ms(view, lv_tick_get()));
}
