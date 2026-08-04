#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "protocol_message.h"

typedef struct {
    bool online;
    bool time_valid;
    bool has_push_data;
    uint64_t last_valid_request_ms;
    int16_t utc_offset_minutes;
    uint32_t latest_revision;
    protocol_push_data_t push_data;
} link_state_t;

typedef enum {
    LINK_STATE_PUSH_ACCEPTED = 0,
    LINK_STATE_PUSH_STALE,
    LINK_STATE_PUSH_INVALID,
} link_state_push_result_t;

void link_state_init(link_state_t *state);

/** Record a valid request and return true only on standalone -> online. */
bool link_state_note_valid_request(link_state_t *state, uint64_t now_ms);

/** Apply the liveness deadline and return true only on online -> standalone. */
bool link_state_poll(link_state_t *state, uint64_t now_ms);

/** Validate and retain a time-sync offset without mutating state on error. */
bool link_state_apply_time_sync(link_state_t *state,
                                const protocol_time_sync_t *sync);

/** Convert UTC seconds to the retained fixed-offset wall-clock seconds. */
bool link_state_local_seconds(const link_state_t *state,
                              int64_t utc_seconds,
                              int64_t *local_seconds);

/**
 * Validate and retain a strictly newer pushed sample. The revision domain is
 * global across widget IDs for the powered session and survives host timeout.
 */
link_state_push_result_t link_state_accept_push(
    link_state_t *state,
    const protocol_push_data_t *push);
