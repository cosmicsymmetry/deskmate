#include "net_ring.h"

#include <limits.h>
#include <string.h>

static void add_dropped_bytes(net_ring_t *ring, size_t count)
{
    if (count > UINT32_MAX - ring->dropped_bytes) {
        ring->dropped_bytes = UINT32_MAX;
    } else {
        ring->dropped_bytes += (uint32_t)count;
    }
}

void net_ring_init(net_ring_t *ring, uint8_t *storage, size_t capacity)
{
    if (ring == NULL) {
        return;
    }
    ring->storage = storage;
    ring->capacity = capacity;
    ring->head = 0U;
    ring->tail = 0U;
    ring->used = 0U;
    ring->dropped_bytes = 0U;
}

size_t net_ring_write(net_ring_t *ring, const uint8_t *data, size_t length)
{
    if (ring == NULL || ring->storage == NULL || ring->capacity == 0U ||
        data == NULL || length == 0U) {
        return 0U;
    }

    size_t available = ring->capacity - ring->used;
    size_t accepted = length < available ? length : available;
    size_t first = accepted;
    size_t until_wrap = ring->capacity - ring->head;
    if (first > until_wrap) {
        first = until_wrap;
    }
    if (first != 0U) {
        memcpy(ring->storage + ring->head, data, first);
    }
    size_t second = accepted - first;
    if (second != 0U) {
        memcpy(ring->storage, data + first, second);
    }
    ring->head = (ring->head + accepted) % ring->capacity;
    ring->used += accepted;
    add_dropped_bytes(ring, length - accepted);
    return accepted;
}

size_t net_ring_read(net_ring_t *ring, uint8_t *out, size_t capacity)
{
    if (ring == NULL || ring->storage == NULL || ring->capacity == 0U ||
        out == NULL || capacity == 0U) {
        return 0U;
    }

    size_t count = capacity < ring->used ? capacity : ring->used;
    size_t first = count;
    size_t until_wrap = ring->capacity - ring->tail;
    if (first > until_wrap) {
        first = until_wrap;
    }
    if (first != 0U) {
        memcpy(out, ring->storage + ring->tail, first);
    }
    size_t second = count - first;
    if (second != 0U) {
        memcpy(out + first, ring->storage, second);
    }
    ring->tail = (ring->tail + count) % ring->capacity;
    ring->used -= count;
    return count;
}

size_t net_ring_used(const net_ring_t *ring)
{
    return ring == NULL ? 0U : ring->used;
}

uint32_t net_ring_dropped_bytes(const net_ring_t *ring)
{
    return ring == NULL ? 0U : ring->dropped_bytes;
}
