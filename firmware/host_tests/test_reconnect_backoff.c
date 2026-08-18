#include <assert.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

#include "../main/core/reconnect_backoff.h"

typedef struct {
    const uint32_t *values;
    size_t count;
    size_t index;
} random_sequence_t;

static uint32_t next_random(void *context)
{
    random_sequence_t *sequence = context;
    assert(sequence->index < sequence->count);
    return sequence->values[sequence->index++];
}

static void test_curve_caps_and_reset(void)
{
    const uint32_t random_values[] = {20U, 20U, 20U, 20U, 20U,
                                      20U, 20U, 20U, 20U};
    const uint32_t expected[] = {1000U, 2000U, 4000U, 8000U, 16000U,
                                 32000U, 60000U, 60000U};
    random_sequence_t random = {
        .values = random_values,
        .count = sizeof(random_values) / sizeof(random_values[0]),
        .index = 0U,
    };
    reconnect_backoff_t backoff;
    reconnect_backoff_init(&backoff, next_random, &random);

    for (size_t index = 0U; index < sizeof(expected) / sizeof(expected[0]);
         ++index) {
        assert(reconnect_backoff_base_delay_ms(&backoff) == expected[index]);
        assert(reconnect_backoff_next_delay_ms(&backoff) == expected[index]);
    }
    assert(reconnect_backoff_base_delay_ms(&backoff) == 60000U);

    reconnect_backoff_reset(&backoff);
    assert(reconnect_backoff_base_delay_ms(&backoff) == 1000U);
    assert(reconnect_backoff_next_delay_ms(&backoff) == 1000U);
}

static void test_jitter_bounds(void)
{
    const uint32_t random_values[] = {0U, 40U};
    random_sequence_t random = {
        .values = random_values,
        .count = sizeof(random_values) / sizeof(random_values[0]),
        .index = 0U,
    };
    reconnect_backoff_t backoff;
    reconnect_backoff_init(&backoff, next_random, &random);

    assert(reconnect_backoff_peek_delay_ms(&backoff) == 800U);
    assert(reconnect_backoff_peek_delay_ms(&backoff) == 1200U);
    assert(reconnect_backoff_base_delay_ms(&backoff) == 1000U);
}

int main(void)
{
    test_curve_caps_and_reset();
    test_jitter_bounds();
    puts("test_reconnect_backoff: OK");
    return 0;
}
