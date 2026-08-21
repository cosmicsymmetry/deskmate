#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "core/protocol_message.h"
#include "core/template_fields.h"
#include "core/device_event_queue.h"
#include "esp_err.h"
#include "ui_command_queue.h"

/**
 * Start the fixed UI command consumer after LVGL/display initialization.
 * All post functions are safe from non-LVGL tasks and never allocate.
 */
esp_err_t ui_runtime_init(void);
bool ui_runtime_is_initialized(void);
void ui_runtime_set_event_queue(device_event_queue_t *event_queue);

bool ui_runtime_show_view(const char *widget_id,
                          protocol_template_kind_t template_kind,
                          protocol_size_class_t size_class,
                          const template_field_state_t *fields,
                          const ui_view_context_t *view_context);

bool ui_runtime_patch_view(const char *widget_id,
                           protocol_template_kind_t template_kind,
                           const template_field_state_t *fields,
                           uint16_t dirty_mask);

bool ui_runtime_show_standalone(void);
bool ui_runtime_set_online(bool online);
bool ui_runtime_set_utc_offset_minutes(int16_t offset_minutes);

uint32_t ui_runtime_dropped_commands(void);
uint32_t ui_runtime_coalesced_commands(void);
uint32_t ui_runtime_queue_high_water(void);
