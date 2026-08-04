#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "core/device_event_queue.h"
#include "core/protocol_message.h"
#include "lvgl.h"

typedef struct {
    char screen_id[PROTOCOL_MAX_SCREEN_ID_LENGTH + 1U];
    char widget_id[PROTOCOL_MAX_WIDGET_ID_LENGTH + 1U];
    char previous_screen_id[PROTOCOL_MAX_SCREEN_ID_LENGTH + 1U];
    char previous_widget_id[PROTOCOL_MAX_WIDGET_ID_LENGTH + 1U];
    char next_screen_id[PROTOCOL_MAX_SCREEN_ID_LENGTH + 1U];
    char next_widget_id[PROTOCOL_MAX_WIDGET_ID_LENGTH + 1U];
    protocol_tap_action_t tap_action;
    bool interrupt;
    uint32_t interrupt_token;
} carousel_binding_t;

void carousel_init(device_event_queue_t *event_queue);

/** Bind the currently active template screen to a fixed metadata snapshot. */
bool carousel_bind(lv_obj_t *screen, const carousel_binding_t *binding);

void carousel_unbind(void);
