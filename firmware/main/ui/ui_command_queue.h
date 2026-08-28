#pragma once

#include <stdatomic.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define UI_COMMAND_QUEUE_CAPACITY 4U

typedef enum {
    UI_COMMAND_SHOW_STANDALONE = 0,
    UI_COMMAND_SHOW_CARD_FALLBACK,
    UI_COMMAND_LINK_STATE,
    UI_COMMAND_TIME_OFFSET,
} ui_command_type_t;

typedef struct {
    ui_command_type_t type;
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

/** Remove queued card-fallback work without disturbing a forced standalone
 * transition. Returns true when at least one fallback was removed. */
bool ui_command_queue_discard_card_fallbacks(ui_command_queue_t *queue);

uint32_t ui_command_queue_dropped(ui_command_queue_t *queue);
uint32_t ui_command_queue_coalesced(ui_command_queue_t *queue);
uint32_t ui_command_queue_high_water(ui_command_queue_t *queue);
