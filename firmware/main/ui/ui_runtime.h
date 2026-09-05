#pragma once

#include <stdbool.h>
#include <stdint.h>

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

bool ui_runtime_show_card_fallback(void);
bool ui_runtime_discard_card_fallbacks(void);
bool ui_runtime_set_online(bool online);
bool ui_runtime_set_utc_offset_minutes(int16_t offset_minutes);

uint32_t ui_runtime_dropped_commands(void);
uint32_t ui_runtime_queue_high_water(void);
