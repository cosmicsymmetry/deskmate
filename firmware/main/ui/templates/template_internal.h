#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "core/protocol_message.h"
#include "core/template_fields.h"
#include "lvgl.h"

#define TEMPLATE_OBJECT_CAPACITY 14U

typedef struct {
    lv_obj_t *root;
    lv_obj_t *state_label;
    lv_obj_t *objects[TEMPLATE_OBJECT_CAPACITY];
    bool progress_running;
    int64_t progress_duration_seconds;
    int64_t progress_remaining_ms;
    int64_t progress_displayed_seconds;
    uint32_t progress_anchor_ms;
} template_widget_view_t;

bool digital_clock_create(template_widget_view_t *view,
                          lv_obj_t *parent,
                          protocol_size_class_t size);
void digital_clock_patch(template_widget_view_t *view,
                         const template_field_state_t *fields,
                         uint16_t dirty_mask);
void digital_clock_tick(template_widget_view_t *view,
                        int16_t utc_offset_minutes);

bool progress_ring_create(template_widget_view_t *view,
                          lv_obj_t *parent,
                          protocol_size_class_t size);
void progress_ring_patch(template_widget_view_t *view,
                         const template_field_state_t *fields,
                         uint16_t dirty_mask);
void progress_ring_local_action(template_widget_view_t *view,
                                protocol_event_action_t action);
void progress_ring_tick(template_widget_view_t *view);

bool row_list_create(template_widget_view_t *view,
                     lv_obj_t *parent,
                     protocol_size_class_t size);
void row_list_patch(template_widget_view_t *view,
                    const template_field_state_t *fields,
                    uint16_t dirty_mask);

bool analog_clock_create(template_widget_view_t *view,
                         lv_obj_t *parent,
                         protocol_size_class_t size);
void analog_clock_patch(template_widget_view_t *view,
                        const template_field_state_t *fields,
                        uint16_t dirty_mask);
void analog_clock_tick(template_widget_view_t *view,
                       int16_t utc_offset_minutes);
