#include "link_state.h"

#include <limits.h>
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
    state->time_valid = true;
    return true;
}

bool link_state_local_seconds(const link_state_t *state,
                              int64_t utc_seconds,
                              int64_t *local_seconds)
{
    if (state == NULL || local_seconds == NULL) {
        return false;
    }
    int64_t delta = (int64_t)state->utc_offset_minutes * INT64_C(60);
    if ((delta > 0 && utc_seconds > INT64_MAX - delta) ||
        (delta < 0 && utc_seconds < INT64_MIN - delta)) {
        return false;
    }
    *local_seconds = utc_seconds + delta;
    return true;
}

static bool bounded_text(const char *text, size_t capacity, size_t minimum)
{
    const char *end = memchr(text, '\0', capacity);
    return end != NULL && (size_t)(end - text) >= minimum;
}

static bool push_is_valid(const protocol_push_data_t *push)
{
    if (push == NULL || push->revision == 0U ||
        push->field_count > PROTOCOL_MAX_FIELD_COUNT ||
        !bounded_text(push->widget_id, sizeof(push->widget_id), 1U)) {
        return false;
    }
    for (size_t i = 0; i < push->field_count; ++i) {
        const protocol_field_t *field = &push->fields[i];
        if (!bounded_text(field->key, sizeof(field->key), 1U)) {
            return false;
        }
        for (size_t j = 0; j < i; ++j) {
            if (strcmp(field->key, push->fields[j].key) == 0) {
                return false;
            }
        }
        if (field->type == PROTOCOL_FIELD_TEXT) {
            if (!bounded_text(field->value.text, sizeof(field->value.text),
                              0U)) {
                return false;
            }
        } else if (field->type != PROTOCOL_FIELD_INTEGER &&
                   field->type != PROTOCOL_FIELD_BOOLEAN) {
            return false;
        }
    }
    return true;
}

link_state_push_result_t link_state_accept_push(
    link_state_t *state,
    const protocol_push_data_t *push)
{
    if (state == NULL || !push_is_valid(push)) {
        return LINK_STATE_PUSH_INVALID;
    }
    if (push->revision <= state->latest_revision) {
        return LINK_STATE_PUSH_STALE;
    }
    state->push_data = *push;
    state->latest_revision = push->revision;
    state->has_push_data = true;
    return LINK_STATE_PUSH_ACCEPTED;
}
