#include "link_state.h"

#include <string.h>

void link_state_init(link_state_t *state)
{
    link_state_init_with_timeout(state, PROTOCOL_LINK_TIMEOUT_MS);
}

void link_state_init_with_timeout(link_state_t *state, uint32_t timeout_ms)
{
    if (state != NULL) {
        memset(state, 0, sizeof(*state));
        state->timeout_ms = timeout_ms;
    }
}

bool link_state_note_valid_request(link_state_t *state, uint64_t now_ms)
{
    if (state == NULL) {
        return false;
    }
    bool changed = !state->online;
    state->online = true;
    state->last_valid_request_ms = now_ms;
    return changed;
}

bool link_state_poll(link_state_t *state, uint64_t now_ms)
{
    if (state == NULL || !state->online) {
        return false;
    }
    if (now_ms - state->last_valid_request_ms < state->timeout_ms) {
        return false;
    }
    state->online = false;
    return true;
}

bool link_state_apply_time_sync(link_state_t *state,
                                const protocol_time_sync_t *sync)
{
    if (state == NULL || sync == NULL ||
        sync->unix_seconds < PROTOCOL_MIN_UNIX_SECONDS ||
        sync->unix_seconds > PROTOCOL_MAX_UNIX_SECONDS ||
        sync->utc_offset_minutes < PROTOCOL_MIN_UTC_OFFSET_MINUTES ||
        sync->utc_offset_minutes > PROTOCOL_MAX_UTC_OFFSET_MINUTES) {
        return false;
    }
    state->utc_offset_minutes = sync->utc_offset_minutes;
    return true;
}
