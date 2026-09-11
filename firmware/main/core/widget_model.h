#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "protocol_message.h"

/* The device's model of the loop.
 *
 * Protocol v2 removed templates, size classes and the parallel screens array
 * from the wire, so what the device keeps is the ordered card list, which card
 * is live, and the timer a pomodoro's `timer.*` scene bindings resolve against.
 * Everything that decides what a face LOOKS like lives on the host and arrives
 * as a scene; this file only decides which card is showing and what the timer
 * reads.
 *
 * The name is kept because the protocol task, its host tests and the status
 * report all use it, and renaming it would churn more than it explains. */

typedef enum {
    WIDGET_MODEL_CONFIG_APPLIED = 0,
    WIDGET_MODEL_CONFIG_REPLAYED,
    WIDGET_MODEL_CONFIG_INVALID_ARGUMENT,
    WIDGET_MODEL_CONFIG_STALE_REVISION,
    WIDGET_MODEL_CONFIG_TOO_LARGE,
    WIDGET_MODEL_CONFIG_DUPLICATE_ID,
    WIDGET_MODEL_CONFIG_UNKNOWN_WIDGET,
    WIDGET_MODEL_CONFIG_INVALID_VALUE,
} widget_model_config_result_t;

typedef enum {
    WIDGET_MODEL_PUSH_ACCEPTED = 0,
    WIDGET_MODEL_PUSH_INVALID_ARGUMENT,
    WIDGET_MODEL_PUSH_STALE_REVISION,
    WIDGET_MODEL_PUSH_UNKNOWN_WIDGET,
} widget_model_push_result_t;

/** The timer snapshot one card's `timer.*` bindings resolve against. */
typedef struct {
    char card_id[PROTOCOL_MAX_CARD_ID_LENGTH + 1U];
    bool present;
    uint32_t total_ms;
    uint32_t remaining_ms;
    bool running;
} widget_model_timer_t;

typedef struct {
    /* Two fixed config buffers make validation + publication an index swap. */
    protocol_apply_config_t configs[2];
    uint8_t live_config_index;
    bool configured;
    size_t active_card_index;

    widget_model_timer_t timer;
    uint32_t latest_data_revision;
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

/** The live card, or NULL when nothing is configured. */
const protocol_card_config_t *widget_model_active_card(
    const widget_model_t *model);

bool widget_model_activate_card(widget_model_t *model, const char *card_id);

widget_model_push_result_t widget_model_apply_timer(
    widget_model_t *model,
    const protocol_push_timer_t *push);

uint32_t widget_model_latest_data_revision(const widget_model_t *model);

/** True when the stored timer snapshot is running. */
bool widget_model_has_running_progress(const widget_model_t *model);

/** The timer for `card_id`, or NULL when none has been pushed for it. */
const widget_model_timer_t *widget_model_timer(const widget_model_t *model,
                                               const char *card_id);
