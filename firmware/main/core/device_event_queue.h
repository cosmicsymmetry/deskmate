#pragma once

#include <stdatomic.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "protocol_message.h"

#define DEVICE_EVENT_QUEUE_CAPACITY 8U

typedef struct {
    protocol_device_event_t events[DEVICE_EVENT_QUEUE_CAPACITY];
    size_t head;
    size_t count;
    atomic_flag lock;
    uint64_t latest_sequence;
    uint32_t dropped;
    uint32_t high_water;
} device_event_queue_t;

void device_event_queue_init(device_event_queue_t *queue);

/**
 * Assign the next sequence and copy the event. If full, the newest event is
 * dropped after consuming its sequence so the host observes a gap.
 */
bool device_event_queue_push(device_event_queue_t *queue,
                             const protocol_device_event_t *event);

bool device_event_queue_pop(device_event_queue_t *queue,
                            protocol_device_event_t *event);

uint32_t device_event_queue_dropped(device_event_queue_t *queue);
uint32_t device_event_queue_high_water(device_event_queue_t *queue);
