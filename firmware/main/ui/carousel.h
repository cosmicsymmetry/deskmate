#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "core/device_event_queue.h"
#include "core/protocol_message.h"
#include "lvgl.h"

/* Protocol v1 carried a widget id beside every screen id here, and both were
 * always the same card. v2 has one identifier space. */
typedef struct {
    char card_id[PROTOCOL_MAX_CARD_ID_LENGTH + 1U];
    char previous_card_id[PROTOCOL_MAX_CARD_ID_LENGTH + 1U];
    char next_card_id[PROTOCOL_MAX_CARD_ID_LENGTH + 1U];
    protocol_tap_action_t tap_action;
    bool interrupt;
    uint32_t interrupt_token;
} carousel_binding_t;

void carousel_init(device_event_queue_t *event_queue);

/** Bind the currently active card to a fixed metadata snapshot. */
bool carousel_bind(lv_obj_t *screen, const carousel_binding_t *binding);

void carousel_unbind(void);
