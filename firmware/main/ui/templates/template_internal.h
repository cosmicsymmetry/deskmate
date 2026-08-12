#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "core/protocol_message.h"
#include "core/template_fields.h"
#include "lvgl.h"
#include "ui/fonts/deskmate_fonts.h"

#define TEMPLATE_OBJECT_CAPACITY 14U

/* Design system constants (spec §6.2). Every per-face template and the
 * standalone clock inherit these; do not hardcode raw colors/fonts
 * elsewhere. */
#define DESKMATE_GRID              8      /* spacing unit, spec §6.2 */
#define DESKMATE_MARGIN            (3 * DESKMATE_GRID)  /* 24px canvas margin */
#define DESKMATE_COLOR_CANVAS      lv_color_hex(0x000000)
#define DESKMATE_COLOR_PRIMARY     lv_color_hex(0xf5f5f7)
#define DESKMATE_COLOR_SECONDARY   lv_color_hex(0x9a9aa5)
#define DESKMATE_COLOR_TERTIARY    lv_color_hex(0x5c5c66)
#define DESKMATE_COLOR_ACCENT      lv_color_hex(0x4f9dff)  /* candidate; tuned in loop */
#define DESKMATE_COLOR_STALE       lv_color_hex(0xf2c94c)  /* reserved, unchanged */
#define DESKMATE_COLOR_ERROR       lv_color_hex(0xff6b6b)  /* reserved, unchanged */
#define DESKMATE_FONT_CAPTION      (&deskmate_font_18)
#define DESKMATE_FONT_BODY         (&deskmate_font_28)
#define DESKMATE_FONT_DISPLAY      (&deskmate_font_56)
#define DESKMATE_FONT_HERO         (&deskmate_font_96)

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

bool big_number_label_create(template_widget_view_t *view,
                             lv_obj_t *parent,
                             protocol_size_class_t size);
void big_number_label_patch(template_widget_view_t *view,
                            const template_field_state_t *fields,
                            uint16_t dirty_mask);

bool icon_badge_text_create(template_widget_view_t *view,
                            lv_obj_t *parent,
                            protocol_size_class_t size);
void icon_badge_text_patch(template_widget_view_t *view,
                           const template_field_state_t *fields,
                           uint16_t dirty_mask);
