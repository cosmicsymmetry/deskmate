#pragma once

#include <stddef.h>
#include <stdint.h>

typedef struct {
    uint8_t *storage;
    size_t capacity;
    size_t head;
    size_t tail;
    size_t used;
    uint32_t dropped_bytes;
} net_ring_t;

void net_ring_init(net_ring_t *ring, uint8_t *storage, size_t capacity);
void net_ring_clear(net_ring_t *ring);
size_t net_ring_write(net_ring_t *ring, const uint8_t *data, size_t length);
size_t net_ring_read(net_ring_t *ring, uint8_t *out, size_t capacity);
size_t net_ring_used(const net_ring_t *ring);
uint32_t net_ring_dropped_bytes(const net_ring_t *ring);
