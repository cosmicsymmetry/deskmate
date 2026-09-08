#pragma once

#include <stdatomic.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "core/ui_command_policy.h"

#define UI_COMMAND_QUEUE_CAPACITY 4U

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
 * Copy one caller-owned fixed command. An offline transition supersedes older
 * card-fallback work while retaining pending scalar state; card fallbacks and
 * scalar state commands replace their pending predecessor. A full
 * non-coalescible queue drops the newest command
 * and increments the pressure counter. Since template patches were removed,
 * normal traffic has only three coalescing categories for four slots; a drop
 * now signals an unexpected command mix rather than ordinary render pressure,
 * and the high-water metric is correspondingly a coarse compatibility
 * diagnostic rather than the signal it was for template-bearing traffic.
 */
bool ui_command_queue_push(ui_command_queue_t *queue,
                           const ui_command_t *command);

bool ui_command_queue_pop(ui_command_queue_t *queue,
                          ui_command_t *command);

/** Remove queued card-fallback work without disturbing an offline link
 * transition. Returns true when at least one fallback was removed. */
bool ui_command_queue_discard_card_fallbacks(ui_command_queue_t *queue);

uint32_t ui_command_queue_dropped(ui_command_queue_t *queue);
uint32_t ui_command_queue_coalesced(ui_command_queue_t *queue);
uint32_t ui_command_queue_high_water(ui_command_queue_t *queue);
