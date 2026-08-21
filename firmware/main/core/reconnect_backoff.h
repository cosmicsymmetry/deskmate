#pragma once

#include <stdint.h>

#define RECONNECT_BACKOFF_MIN_DELAY_MS 1000U
#define RECONNECT_BACKOFF_MAX_DELAY_MS 60000U

typedef uint32_t (*reconnect_backoff_random_fn)(void *context);

typedef struct {
    uint32_t base_delay_ms;
    reconnect_backoff_random_fn random;
    void *random_context;
} reconnect_backoff_t;

/** Initialise a 1 s, doubling, 60 s capped backoff with +/-20% jitter. */
void reconnect_backoff_init(reconnect_backoff_t *backoff,
                            reconnect_backoff_random_fn random,
                            void *random_context);

/** Return a jittered delay for the current base without advancing it. */
uint32_t reconnect_backoff_peek_delay_ms(reconnect_backoff_t *backoff);

/** Return a jittered delay for the current base, then advance the curve. */
uint32_t reconnect_backoff_next_delay_ms(reconnect_backoff_t *backoff);

/** Reset the next base delay to 1 s. */
void reconnect_backoff_reset(reconnect_backoff_t *backoff);

/** Return the unjittered base delay that will be used next. */
uint32_t reconnect_backoff_base_delay_ms(const reconnect_backoff_t *backoff);
