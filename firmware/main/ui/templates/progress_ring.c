#include "template_internal.h"

#include <stdio.h>
#include <string.h>

enum {
    OBJ_ARC,
    OBJ_REMAINING,
    OBJ_LABEL,
    OBJ_STATE,
};

static void set_running_color(template_widget_view_t *view, bool running)
{
    view->progress_running = running;
    lv_obj_set_style_arc_color(
        view->objects[OBJ_ARC],
        running ? lv_color_hex(0x65d1ff) : lv_color_hex(0x8f93a8),
        LV_PART_INDICATOR);
}

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

static void render_remaining(template_widget_view_t *view,
                             int64_t remaining_ms)
{
    if (view->progress_duration_seconds <= 0) {
        return;
    }
    int64_t duration_ms = view->progress_duration_seconds * INT64_C(1000);
    int64_t value = remaining_ms * 1000 / duration_ms;
    lv_arc_set_value(view->objects[OBJ_ARC], (int32_t)value);
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
    int32_t diameter = size == PROTOCOL_SIZE_FULL ? 250 : 220;
    lv_obj_set_size(view->objects[OBJ_ARC], diameter, diameter);
    lv_obj_center(view->objects[OBJ_ARC]);
    lv_arc_set_range(view->objects[OBJ_ARC], 0, 1000);
    lv_arc_set_rotation(view->objects[OBJ_ARC], 270);
    lv_obj_remove_style(view->objects[OBJ_ARC], NULL, LV_PART_KNOB);
    lv_obj_remove_flag(view->objects[OBJ_ARC], LV_OBJ_FLAG_CLICKABLE);
    lv_obj_set_style_arc_color(view->objects[OBJ_ARC],
                               lv_color_hex(0x2f3142), LV_PART_MAIN);
    lv_obj_set_style_arc_color(view->objects[OBJ_ARC],
                               lv_color_hex(0x65d1ff), LV_PART_INDICATOR);
    lv_obj_set_style_arc_width(view->objects[OBJ_ARC], 14, LV_PART_MAIN);
    lv_obj_set_style_arc_width(view->objects[OBJ_ARC], 14,
                               LV_PART_INDICATOR);

    view->objects[OBJ_REMAINING] = lv_label_create(view->root);
    lv_obj_set_style_text_font(view->objects[OBJ_REMAINING],
                               &lv_font_montserrat_48, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_REMAINING],
                                lv_color_hex(0xffffff), 0);
    lv_obj_align(view->objects[OBJ_REMAINING], LV_ALIGN_CENTER, 0, -12);

    view->objects[OBJ_LABEL] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_LABEL],
                                lv_color_hex(0xaeb2c4), 0);
    lv_obj_align(view->objects[OBJ_LABEL], LV_ALIGN_CENTER, 0, 42);

    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_MID, 0, -8);
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
