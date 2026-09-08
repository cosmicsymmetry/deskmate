#include "reconnect_backoff.h"

#include <stddef.h>

static uint32_t jittered_delay_ms(reconnect_backoff_t *backoff)
{
    uint32_t random = 0U;
    if (backoff->random != NULL) {
        random = backoff->random(backoff->random_context);
    }
    uint32_t percent = 80U + (random % 41U);
    return (uint32_t)(((uint64_t)backoff->base_delay_ms * percent) / 100U);
}

void reconnect_backoff_init(reconnect_backoff_t *backoff,
                            reconnect_backoff_random_fn random,
                            void *random_context)
{
    if (backoff == NULL) {
        return;
    }
    backoff->random = random;
    backoff->random_context = random_context;
    reconnect_backoff_reset(backoff);
}

uint32_t reconnect_backoff_peek_delay_ms(reconnect_backoff_t *backoff)
{
    if (backoff == NULL) {
        return 0U;
    }
    return jittered_delay_ms(backoff);
}

uint32_t reconnect_backoff_next_delay_ms(reconnect_backoff_t *backoff)
{
    if (backoff == NULL) {
        return 0U;
    }
    uint32_t delay_ms = jittered_delay_ms(backoff);
    if (backoff->base_delay_ms < RECONNECT_BACKOFF_MAX_DELAY_MS) {
        uint32_t doubled = backoff->base_delay_ms * 2U;
        backoff->base_delay_ms =
            (doubled > RECONNECT_BACKOFF_MAX_DELAY_MS ||
             doubled < backoff->base_delay_ms)
            ? RECONNECT_BACKOFF_MAX_DELAY_MS
            : doubled;
    }
    return delay_ms;
}

void reconnect_backoff_reset(reconnect_backoff_t *backoff)
{
    if (backoff != NULL) {
        backoff->base_delay_ms = RECONNECT_BACKOFF_MIN_DELAY_MS;
    }
}
