#include "device_event_queue.h"

#include <limits.h>
#include <string.h>

_Static_assert(sizeof(device_event_queue_t) <= 4U * 1024U,
               "device event queue exceeded its M2 RAM budget");

static void lock_queue(device_event_queue_t *queue)
{
    while (atomic_flag_test_and_set_explicit(&queue->lock,
                                              memory_order_acquire)) {
    }
}

static void unlock_queue(device_event_queue_t *queue)
{
    atomic_flag_clear_explicit(&queue->lock, memory_order_release);
}

static void increment(uint32_t *counter)
{
    if (*counter != UINT32_MAX) {
        ++*counter;
    }
}

void device_event_queue_init(device_event_queue_t *queue)
{
    if (queue != NULL) {
        memset(queue, 0, sizeof(*queue));
        atomic_flag_clear(&queue->lock);
    }
}

bool device_event_queue_push(device_event_queue_t *queue,
                             const protocol_device_event_t *event)
{
    if (queue == NULL || event == NULL) {
        return false;
    }
    lock_queue(queue);
    if (queue->latest_sequence == UINT64_MAX) {
        increment(&queue->dropped);
        unlock_queue(queue);
        return false;
    }
    uint64_t sequence = ++queue->latest_sequence;
    if (queue->count == DEVICE_EVENT_QUEUE_CAPACITY) {
        increment(&queue->dropped);
        unlock_queue(queue);
        return false;
    }
    size_t tail = (queue->head + queue->count) %
                  DEVICE_EVENT_QUEUE_CAPACITY;
    queue->events[tail] = *event;
    queue->events[tail].sequence = sequence;
    ++queue->count;
    if (queue->count > queue->high_water) {
        queue->high_water = (uint32_t)queue->count;
    }
    unlock_queue(queue);
    return true;
}

bool device_event_queue_pop(device_event_queue_t *queue,
                            protocol_device_event_t *event)
{
    if (queue == NULL || event == NULL) {
        return false;
    }
    lock_queue(queue);
    if (queue->count == 0U) {
        unlock_queue(queue);
        return false;
    }
    *event = queue->events[queue->head];
    queue->head = (queue->head + 1U) % DEVICE_EVENT_QUEUE_CAPACITY;
    --queue->count;
    unlock_queue(queue);
    return true;
}

uint32_t device_event_queue_dropped(device_event_queue_t *queue)
{
    if (queue == NULL) {
        return 0U;
    }
    lock_queue(queue);
    uint32_t value = queue->dropped;
    unlock_queue(queue);
    return value;
}
uint32_t device_event_queue_high_water(device_event_queue_t *queue)
{
    if (queue == NULL) {
        return 0U;
    }
    lock_queue(queue);
    uint32_t value = queue->high_water;
    unlock_queue(queue);
    return value;
}
