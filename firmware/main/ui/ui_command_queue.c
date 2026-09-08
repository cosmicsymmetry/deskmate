#include "ui_command_queue.h"

#include <limits.h>
#include <string.h>

static void lock_queue(ui_command_queue_t *queue)
{
    while (atomic_flag_test_and_set_explicit(&queue->lock,
                                              memory_order_acquire)) {
    }
}

static void unlock_queue(ui_command_queue_t *queue)
{
    atomic_flag_clear_explicit(&queue->lock, memory_order_release);
}

static void increment(uint32_t *value)
{
    if (*value != UINT32_MAX) {
        ++*value;
    }
}

static size_t slot_index(const ui_command_queue_t *queue, size_t offset)
{
    return (queue->head + offset) % UI_COMMAND_QUEUE_CAPACITY;
}

static bool same_coalescing_key(const ui_command_t *pending,
                                const ui_command_t *incoming)
{
    return incoming->type == pending->type &&
           (incoming->type == UI_COMMAND_SHOW_CARD_FALLBACK ||
            incoming->type == UI_COMMAND_LINK_STATE ||
            incoming->type == UI_COMMAND_TIME_OFFSET);
}

static bool is_view_work(ui_command_type_t type)
{
    return type == UI_COMMAND_SHOW_CARD_FALLBACK;
}

static void discard_superseded_view_work(ui_command_queue_t *queue)
{
    size_t original_count = queue->count;
    size_t kept = 0U;
    for (size_t i = 0U; i < original_count; ++i) {
        size_t read_index = slot_index(queue, i);
        if (is_view_work(queue->commands[read_index].type)) {
            increment(&queue->coalesced);
            continue;
        }
        size_t write_index = slot_index(queue, kept);
        if (write_index != read_index) {
            queue->commands[write_index] = queue->commands[read_index];
        }
        ++kept;
    }
    queue->count = kept;
}

static bool discard_card_fallbacks_locked(ui_command_queue_t *queue)
{
    size_t original_count = queue->count;
    size_t kept = 0U;
    bool discarded = false;
    for (size_t i = 0U; i < original_count; ++i) {
        size_t read_index = slot_index(queue, i);
        if (queue->commands[read_index].type ==
            UI_COMMAND_SHOW_CARD_FALLBACK) {
            increment(&queue->coalesced);
            discarded = true;
            continue;
        }
        size_t write_index = slot_index(queue, kept);
        if (write_index != read_index) {
            queue->commands[write_index] = queue->commands[read_index];
        }
        ++kept;
    }
    queue->count = kept;
    return discarded;
}

void ui_command_queue_init(ui_command_queue_t *queue)
{
    if (queue != NULL) {
        memset(queue, 0, sizeof(*queue));
        atomic_flag_clear(&queue->lock);
    }
}

bool ui_command_queue_push(ui_command_queue_t *queue,
                           const ui_command_t *command)
{
    if (queue == NULL || command == NULL) {
        return false;
    }
    lock_queue(queue);
    if (command->type == UI_COMMAND_LINK_STATE && !command->online) {
        /* Screen replacement makes older view work obsolete, but scalar
         * time/link state still has to reach the fallback/new screen in order. */
        discard_superseded_view_work(queue);
    } else if (command->type == UI_COMMAND_SHOW_CARD_FALLBACK) {
        (void)discard_card_fallbacks_locked(queue);
    } else {
        for (size_t i = 0U; i < queue->count; ++i) {
            ui_command_t *pending = &queue->commands[slot_index(queue, i)];
            if (same_coalescing_key(pending, command)) {
                *pending = *command;
                increment(&queue->coalesced);
                unlock_queue(queue);
                return true;
            }
        }
    }
    if (queue->count == UI_COMMAND_QUEUE_CAPACITY) {
        increment(&queue->dropped);
        unlock_queue(queue);
        return false;
    }
    queue->commands[slot_index(queue, queue->count)] = *command;
    ++queue->count;
    if (queue->count > queue->high_water) {
        queue->high_water = (uint32_t)queue->count;
    }
    unlock_queue(queue);
    return true;
}

bool ui_command_queue_discard_card_fallbacks(ui_command_queue_t *queue)
{
    if (queue == NULL) {
        return false;
    }
    lock_queue(queue);
    bool discarded = discard_card_fallbacks_locked(queue);
    unlock_queue(queue);
    return discarded;
}

bool ui_command_queue_pop(ui_command_queue_t *queue,
                          ui_command_t *command)
{
    if (queue == NULL || command == NULL) {
        return false;
    }
    lock_queue(queue);
    if (queue->count == 0U) {
        unlock_queue(queue);
        return false;
    }
    *command = queue->commands[queue->head];
    queue->head = (queue->head + 1U) % UI_COMMAND_QUEUE_CAPACITY;
    --queue->count;
    unlock_queue(queue);
    return true;
}

uint32_t ui_command_queue_dropped(ui_command_queue_t *queue)
{
    if (queue == NULL) {
        return 0U;
    }
    lock_queue(queue);
    uint32_t value = queue->dropped;
    unlock_queue(queue);
    return value;
}

uint32_t ui_command_queue_coalesced(ui_command_queue_t *queue)
{
    if (queue == NULL) {
        return 0U;
    }
    lock_queue(queue);
    uint32_t value = queue->coalesced;
    unlock_queue(queue);
    return value;
}

uint32_t ui_command_queue_high_water(ui_command_queue_t *queue)
{
    if (queue == NULL) {
        return 0U;
    }
    lock_queue(queue);
    uint32_t value = queue->high_water;
    unlock_queue(queue);
    return value;
}
