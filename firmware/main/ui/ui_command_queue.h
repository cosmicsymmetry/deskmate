#pragma once

#include <stdatomic.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "core/protocol_message.h"
#include "core/template_fields.h"

#define UI_COMMAND_QUEUE_CAPACITY 4U

typedef enum {
    UI_COMMAND_SHOW_STANDALONE = 0,
    UI_COMMAND_SHOW_VIEW,
    UI_COMMAND_PATCH_VIEW,
    UI_COMMAND_LINK_STATE,
    UI_COMMAND_TIME_OFFSET,
} ui_command_type_t;

typedef struct {
    char screen_id[PROTOCOL_MAX_SCREEN_ID_LENGTH + 1U];
    char previous_screen_id[PROTOCOL_MAX_SCREEN_ID_LENGTH + 1U];
    char previous_widget_id[PROTOCOL_MAX_WIDGET_ID_LENGTH + 1U];
    char next_screen_id[PROTOCOL_MAX_SCREEN_ID_LENGTH + 1U];
    char next_widget_id[PROTOCOL_MAX_WIDGET_ID_LENGTH + 1U];
    protocol_tap_action_t tap_action;
    bool interrupt;
    uint32_t interrupt_token;
} ui_view_context_t;

typedef struct {
    ui_command_type_t type;
    char widget_id[PROTOCOL_MAX_WIDGET_ID_LENGTH + 1U];
    ui_view_context_t view_context;
    protocol_template_kind_t template_kind;
    protocol_size_class_t size_class;
    template_field_state_t fields;
    uint16_t dirty_mask;
    bool online;
    int16_t utc_offset_minutes;
} ui_command_t;

typedef struct {
    ui_command_t commands[UI_COMMAND_QUEUE_CAPACITY];
    size_t head;
    size_t count;
    atomic_flag lock;
    uint32_t dropped;
    uint32_t coalesced;
    uint32_t high_water;
} ui_command_queue_t;

void ui_command_queue_init(ui_command_queue_t *queue);

/**
 * Copy one caller-owned fixed command. View/standalone commands and an
 * offline transition supersede older view work while retaining pending scalar
 * state; patches for the same widget and scalar state commands replace their
 * pending predecessor. A full non-coalescible queue drops the newest command
 * and increments the pressure counter.
 */
bool ui_command_queue_push(ui_command_queue_t *queue,
                           const ui_command_t *command);

bool ui_command_queue_pop(ui_command_queue_t *queue,
                          ui_command_t *command);

uint32_t ui_command_queue_dropped(ui_command_queue_t *queue);
uint32_t ui_command_queue_coalesced(ui_command_queue_t *queue);
uint32_t ui_command_queue_high_water(ui_command_queue_t *queue);
