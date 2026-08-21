#include <assert.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "../main/core/net_ring.h"

static void test_wraparound_split_copy_and_transitions(void)
{
    uint8_t storage[8];
    uint8_t output[8];
    const uint8_t first[] = {0U, 1U, 2U, 3U, 4U, 5U};
    const uint8_t second[] = {6U, 7U, 8U, 9U, 10U, 11U};
    const uint8_t expected[] = {4U, 5U, 6U, 7U, 8U, 9U, 10U, 11U};
    net_ring_t ring;

    net_ring_init(&ring, storage, sizeof(storage));
    assert(net_ring_used(&ring) == 0U);
    assert(net_ring_read(&ring, output, sizeof(output)) == 0U);

    assert(net_ring_write(&ring, first, sizeof(first)) == sizeof(first));
    assert(net_ring_read(&ring, output, 4U) == 4U);
    assert(memcmp(output, first, 4U) == 0);

    // head is at 6: this write must split two bytes at the physical end and
    // four at the beginning, reaching the full transition exactly.
    assert(net_ring_write(&ring, second, sizeof(second)) == sizeof(second));
    assert(net_ring_used(&ring) == sizeof(storage));
    assert(net_ring_write(&ring, second, 1U) == 0U);
    assert(net_ring_dropped_bytes(&ring) == 1U);

    // tail is at 4: reading all logical bytes must split at the boundary too.
    assert(net_ring_read(&ring, output, sizeof(output)) == sizeof(output));
    assert(memcmp(output, expected, sizeof(expected)) == 0);
    assert(net_ring_used(&ring) == 0U);
    assert(net_ring_read(&ring, output, sizeof(output)) == 0U);
}

static void test_dropped_byte_counter_saturates(void)
{
    uint8_t storage[1];
    uint8_t byte = 0xa5U;
    net_ring_t ring;

    net_ring_init(&ring, storage, sizeof(storage));
    assert(net_ring_write(&ring, &byte, 1U) == 1U);
    // The ring is full, so no source byte is dereferenced for these writes;
    // their lengths exercise the saturating counter without a giant buffer.
    assert(net_ring_write(&ring, &byte, (size_t)UINT32_MAX) == 0U);
    assert(net_ring_dropped_bytes(&ring) == UINT32_MAX);
    assert(net_ring_write(&ring, &byte, 1U) == 0U);
    assert(net_ring_dropped_bytes(&ring) == UINT32_MAX);
}

static void test_clear_discards_bytes_and_preserves_diagnostics(void)
{
    uint8_t storage[4];
    uint8_t output[4];
    const uint8_t stale[] = {1U, 2U, 3U, 4U, 5U};
    const uint8_t fresh[] = {6U, 7U};
    net_ring_t ring;

    net_ring_init(&ring, storage, sizeof(storage));
    assert(net_ring_write(&ring, stale, sizeof(stale)) == sizeof(storage));
    assert(net_ring_dropped_bytes(&ring) == 1U);

    net_ring_clear(&ring);
    assert(net_ring_used(&ring) == 0U);
    assert(net_ring_dropped_bytes(&ring) == 1U);
    assert(net_ring_read(&ring, output, sizeof(output)) == 0U);

    assert(net_ring_write(&ring, fresh, sizeof(fresh)) == sizeof(fresh));
    assert(net_ring_read(&ring, output, sizeof(output)) == sizeof(fresh));
    assert(memcmp(output, fresh, sizeof(fresh)) == 0);
}

int main(void)
{
    test_wraparound_split_copy_and_transitions();
    test_dropped_byte_counter_saturates();
    test_clear_discards_bytes_and_preserves_diagnostics();
    puts("test_net_ring: OK");
    return 0;
}
