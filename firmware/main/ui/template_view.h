#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "core/protocol_message.h"
#include "core/template_fields.h"
#include "lvgl.h"

/** All functions run in LVGL context; callers outside it use ui_runtime. */
bool template_view_show(protocol_template_kind_t template_kind,
                        protocol_size_class_t size_class,
                        const template_field_state_t *fields);

bool template_view_patch(protocol_template_kind_t template_kind,
                         const template_field_state_t *fields,
                         uint16_t dirty_mask);

void template_view_set_data_state(bool stale, const char *error);
void template_view_set_utc_offset_minutes(int16_t offset_minutes);
void template_view_apply_local_action(protocol_event_action_t action);

bool template_view_activate(void);
void template_view_deactivate(void);
bool template_view_destroy(void);
bool template_view_active(void);
lv_obj_t *template_view_screen(void);

uint32_t template_view_generation(void);
uint32_t template_view_patch_count(void);
