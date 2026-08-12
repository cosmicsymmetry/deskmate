#include "template_internal.h"

#include <stdio.h>
#include <string.h>

enum {
    OBJ_ARC,
    OBJ_REMAINING,
    OBJ_LABEL,
    OBJ_PHASE,
    OBJ_STATE,
};

/* The ring fills the short axis between the canvas margins. */
#define RING_DIAMETER (368 - 2 * DESKMATE_MARGIN)
#define RING_WIDTH    12

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

/* One word under the countdown, and only when there is something to say. A
 * running timer says nothing: the shrinking arc already does. The words are
 * derived from the same `running`/`remaining`/`duration` fields the arc is
 * drawn from — nothing new comes off the wire — and they are set at CAPTION,
 * because the DISPLAY and HERO tiers carry digits only. */
static void refresh_phase(template_widget_view_t *view, int64_t remaining_ms)
{
    const char *word = "";
    if (remaining_ms <= 0) {
        word = "Done";
    } else if (view->progress_running) {
        word = "";
    } else if (view->progress_duration_seconds > 0 &&
               remaining_ms >=
                   view->progress_duration_seconds * INT64_C(1000)) {
        word = "Ready";
    } else {
        word = "Paused";
    }
    lv_label_set_text(view->objects[OBJ_PHASE], word);
}

static void set_running_color(template_widget_view_t *view, bool running)
{
    view->progress_running = running;
    /* ACCENT is reserved for the active element, so an idle ring drops to
     * TERTIARY — the same colour as the track, at full opacity against the
     * track's 40%, so a paused timer still reads its remaining fraction
     * without a full circle of SECONDARY shouting over the countdown. */
    lv_obj_set_style_arc_color(view->objects[OBJ_ARC],
                               running ? DESKMATE_COLOR_ACCENT
                                       : DESKMATE_COLOR_TERTIARY,
                               LV_PART_INDICATOR);
    refresh_phase(view, current_remaining_ms(view, lv_tick_get()));
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
    refresh_phase(view, remaining_ms);
    int64_t seconds = (remaining_ms + 999) / 1000;
    if (seconds == view->progress_displayed_seconds) {
        return;
    }
    view->progress_displayed_seconds = seconds;
    char text[24];
    snprintf(text, sizeof(text), "%02lld:%02lld",
             (long long)(seconds / 60), (long long)(seconds % 60));
    lv_label_set_text(view->objects[OBJ_REMAINING], text);
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
    view->progress_displayed_seconds = -1;
    view->root = lv_obj_create(parent);
    lv_obj_remove_style_all(view->root);
    lv_obj_remove_flag(view->root,
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->root, LV_PCT(100), LV_PCT(100));

    view->objects[OBJ_ARC] = lv_arc_create(view->root);
    lv_obj_set_size(view->objects[OBJ_ARC], RING_DIAMETER, RING_DIAMETER);
    lv_obj_center(view->objects[OBJ_ARC]);
    lv_arc_set_range(view->objects[OBJ_ARC], 0, 1000);
    lv_arc_set_rotation(view->objects[OBJ_ARC], 270);
    /* LVGL's arc defaults to a 270-degree gauge with a gap at the bottom.
     * A countdown is a whole circle that empties from twelve o'clock, and
     * the default gap is a large part of why this face read as a demo. */
    lv_arc_set_bg_angles(view->objects[OBJ_ARC], 0, 360);
    lv_obj_remove_style(view->objects[OBJ_ARC], NULL, LV_PART_KNOB);
    lv_obj_remove_flag(view->objects[OBJ_ARC], LV_OBJ_FLAG_CLICKABLE);
    /* The track is structure, not data: TERTIARY at full opacity puts as
     * much ink on the canvas as the countdown itself. Held back to 40% it
     * still closes the circle so the indicator reads as a fraction. */
    lv_obj_set_style_arc_color(view->objects[OBJ_ARC],
                               DESKMATE_COLOR_TERTIARY, LV_PART_MAIN);
    lv_obj_set_style_arc_opa(view->objects[OBJ_ARC], LV_OPA_40, LV_PART_MAIN);
    lv_obj_set_style_arc_color(view->objects[OBJ_ARC], DESKMATE_COLOR_ACCENT,
                               LV_PART_INDICATOR);
    lv_obj_set_style_arc_width(view->objects[OBJ_ARC], RING_WIDTH,
                               LV_PART_MAIN);
    lv_obj_set_style_arc_width(view->objects[OBJ_ARC], RING_WIDTH,
                               LV_PART_INDICATOR);

    /* The ring's interior is the whole composition: name, countdown, phase,
     * stacked on the canvas centre. The label cannot sit above the ring — at
     * 368 - 2 * MARGIN across, the ring's top edge *is* the top margin — and
     * naming what is counting down belongs next to the count anyway. */
    view->objects[OBJ_REMAINING] = lv_label_create(view->root);
    lv_obj_set_style_text_font(view->objects[OBJ_REMAINING],
                               DESKMATE_FONT_DISPLAY, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_REMAINING],
                                DESKMATE_COLOR_PRIMARY, 0);
    lv_label_set_text(view->objects[OBJ_REMAINING], "00:00");
    /* Nudged down a grid unit: the phase word below is empty while the
     * timer runs, and centring the boxes rather than the visible type left
     * the pair reading high inside the ring. */
    lv_obj_align(view->objects[OBJ_REMAINING], LV_ALIGN_CENTER, 0,
                 DESKMATE_GRID);

    view->objects[OBJ_LABEL] = lv_label_create(view->root);
    lv_obj_set_style_text_font(view->objects[OBJ_LABEL],
                               DESKMATE_FONT_CAPTION, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_LABEL],
                                DESKMATE_COLOR_SECONDARY, 0);
    lv_label_set_long_mode(view->objects[OBJ_LABEL], LV_LABEL_LONG_DOT);
    /* Widest chord that stays clear of the ring's inner edge. */
    lv_obj_set_width(view->objects[OBJ_LABEL], 30 * DESKMATE_GRID);
    lv_obj_set_height(view->objects[OBJ_LABEL],
                      lv_font_get_line_height(DESKMATE_FONT_CAPTION));
    lv_obj_set_style_text_align(view->objects[OBJ_LABEL],
                                LV_TEXT_ALIGN_CENTER, 0);
    lv_label_set_text(view->objects[OBJ_LABEL], "");
    lv_obj_align_to(view->objects[OBJ_LABEL], view->objects[OBJ_REMAINING],
                    LV_ALIGN_OUT_TOP_MID, 0, -2 * DESKMATE_GRID);

    view->objects[OBJ_PHASE] = lv_label_create(view->root);
    lv_obj_set_style_text_font(view->objects[OBJ_PHASE],
                               DESKMATE_FONT_CAPTION, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_PHASE],
                                DESKMATE_COLOR_SECONDARY, 0);
    /* Fixed box, centred text: `lv_obj_align_to` below resolves to plain
     * coordinates once, so a content-sized label would drift off the axis
     * every time the word changed length. */
    lv_obj_set_width(view->objects[OBJ_PHASE], 30 * DESKMATE_GRID);
    lv_obj_set_height(view->objects[OBJ_PHASE],
                      lv_font_get_line_height(DESKMATE_FONT_CAPTION));
    lv_obj_set_style_text_align(view->objects[OBJ_PHASE],
                                LV_TEXT_ALIGN_CENTER, 0);
    lv_label_set_text(view->objects[OBJ_PHASE], "");
    lv_obj_align_to(view->objects[OBJ_PHASE], view->objects[OBJ_REMAINING],
                    LV_ALIGN_OUT_BOTTOM_MID, 0, 2 * DESKMATE_GRID);

    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_MID, 0,
                 -DESKMATE_MARGIN / 2);
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
