#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "protocol_message.h"

typedef struct {
    bool online;
    uint64_t last_valid_request_ms;
    int16_t utc_offset_minutes;
    uint32_t timeout_ms;
} link_state_t;

void link_state_init(link_state_t *state);

/**
 * Initialise with an explicit host-loss deadline. link_state_init() delegates
 * here with PROTOCOL_LINK_TIMEOUT_MS, so the USB path is bit-identical.
 */
void link_state_init_with_timeout(link_state_t *state, uint32_t timeout_ms);

/** Record a valid request and return true only on standalone -> online. */
bool link_state_note_valid_request(link_state_t *state, uint64_t now_ms);

/** Apply the liveness deadline and return true only on online -> standalone. */
bool link_state_poll(link_state_t *state, uint64_t now_ms);

/** Validate and retain a time-sync offset without mutating state on error. */
bool link_state_apply_time_sync(link_state_t *state,
                                const protocol_time_sync_t *sync);
