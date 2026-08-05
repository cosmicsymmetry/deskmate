#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "protocol_message.h"
#include "template_fields.h"

typedef enum {
    WIDGET_MODEL_CONFIG_APPLIED = 0,
    WIDGET_MODEL_CONFIG_REPLAYED,
    WIDGET_MODEL_CONFIG_INVALID_ARGUMENT,
    WIDGET_MODEL_CONFIG_STALE_REVISION,
    WIDGET_MODEL_CONFIG_TOO_LARGE,
    WIDGET_MODEL_CONFIG_DUPLICATE_ID,
    WIDGET_MODEL_CONFIG_UNKNOWN_WIDGET,
    WIDGET_MODEL_CONFIG_UNSUPPORTED_TEMPLATE,
    WIDGET_MODEL_CONFIG_UNSUPPORTED_SIZE_CLASS,
    WIDGET_MODEL_CONFIG_INVALID_VALUE,
} widget_model_config_result_t;

typedef enum {
    WIDGET_MODEL_PUSH_ACCEPTED = 0,
    WIDGET_MODEL_PUSH_INVALID_ARGUMENT,
    WIDGET_MODEL_PUSH_STALE_REVISION,
    WIDGET_MODEL_PUSH_UNKNOWN_WIDGET,
    WIDGET_MODEL_PUSH_INVALID_FIELDS,
} widget_model_push_result_t;

typedef enum {
    WIDGET_NAVIGATE_PREVIOUS = -1,
    WIDGET_NAVIGATE_NEXT = 1,
} widget_navigation_t;

typedef struct {
    size_t widget_index;
    uint16_t dirty_mask;
    size_t unknown_fields;
} widget_model_update_t;

typedef struct {
    /* Two fixed config buffers make validation + publication an index swap. */
    protocol_apply_config_t configs[2];
    uint8_t live_config_index;
    bool configured;
    size_t active_screen_index;

    template_field_state_t
        widget_fields[2][PROTOCOL_MAX_CONFIG_WIDGETS];
    bool widget_has_data[2][PROTOCOL_MAX_CONFIG_WIDGETS];
    /* Shared fixed scratch; never allocated on the protocol task stack. */
    template_field_state_t field_staging;

    uint32_t latest_data_revision;
    uint32_t unknown_field_count;
} widget_model_t;

void widget_model_init(widget_model_t *model);

widget_model_config_result_t widget_model_check_config(
    const widget_model_t *model,
    const protocol_apply_config_t *config);

widget_model_config_result_t widget_model_apply_config(
    widget_model_t *model,
    const protocol_apply_config_t *config);

const protocol_apply_config_t *widget_model_config(
    const widget_model_t *model);

uint32_t widget_model_config_revision(const widget_model_t *model);

const protocol_screen_config_t *widget_model_active_screen(
    const widget_model_t *model);

bool widget_model_activate_screen(widget_model_t *model,
                                  const char *screen_id);

bool widget_model_navigate(widget_model_t *model,
                           widget_navigation_t direction);

widget_model_push_result_t widget_model_apply_push(
    widget_model_t *model,
    const protocol_push_data_t *push,
    widget_model_update_t *update);

uint32_t widget_model_latest_data_revision(const widget_model_t *model);

uint32_t widget_model_unknown_field_count(const widget_model_t *model);

bool widget_model_widget_has_data(const widget_model_t *model,
                                  const char *widget_id);

const template_field_state_t *widget_model_widget_fields(
    const widget_model_t *model,
    const char *widget_id);
