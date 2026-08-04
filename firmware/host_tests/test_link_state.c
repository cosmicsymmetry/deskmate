#include <assert.h>
#include <limits.h>
#include <stdio.h>
#include <string.h>

#include "../main/core/link_state.h"

static void test_liveness(void)
{
    link_state_t state;
    link_state_init(&state);
    assert(!state.online);
    assert(link_state_note_valid_request(&state, 100U));
    assert(state.online);
    assert(!link_state_note_valid_request(&state, 500U));
    assert(!link_state_poll(&state, 10499U));
    assert(state.online);
    assert(link_state_poll(&state, 10500U));
    assert(!state.online);
    assert(!link_state_poll(&state, 20000U));
}

static void test_time(void)
{
    link_state_t state;
    link_state_init(&state);
    protocol_time_sync_t valid = {
        .unix_seconds = INT64_C(1775217600),
        .utc_offset_minutes = 240,
    };
    assert(link_state_apply_time_sync(&state, &valid));
    assert(state.time_valid);
    assert(state.utc_offset_minutes == 240);
    int64_t local = 0;
    assert(link_state_local_seconds(&state, valid.unix_seconds, &local));
    assert(local == valid.unix_seconds + INT64_C(14400));

    protocol_time_sync_t invalid = valid;
    invalid.utc_offset_minutes = 841;
    assert(!link_state_apply_time_sync(&state, &invalid));
    assert(state.utc_offset_minutes == 240);
    assert(!link_state_local_seconds(&state, INT64_MAX, &local));

    state.utc_offset_minutes = -840;
    assert(!link_state_local_seconds(&state, INT64_MIN, &local));
}

static void test_push_data(void)
{
    link_state_t state;
    link_state_init(&state);
    protocol_push_data_t push = {
        .widget_id = "weather",
        .revision = 7,
        .field_count = 2,
        .fields = {
            {.key = "summary", .type = PROTOCOL_FIELD_TEXT,
             .value.text = "Clear"},
            {.key = "temperature", .type = PROTOCOL_FIELD_INTEGER,
             .value.integer = 23},
        },
    };
    assert(link_state_accept_push(&state, &push) ==
           LINK_STATE_PUSH_ACCEPTED);
    assert(state.has_push_data);
    assert(state.latest_revision == 7U);
    assert(strcmp(state.push_data.fields[0].value.text, "Clear") == 0);

    push.revision = 7U;
    assert(link_state_accept_push(&state, &push) == LINK_STATE_PUSH_STALE);
    push.revision = 6U;
    assert(link_state_accept_push(&state, &push) == LINK_STATE_PUSH_STALE);
    push.revision = 8U;
    push.field_count = PROTOCOL_MAX_FIELD_COUNT + 1U;
    assert(link_state_accept_push(&state, &push) == LINK_STATE_PUSH_INVALID);
    assert(state.latest_revision == 7U);

    push.field_count = 0U;
    strcpy(push.widget_id, "timer");
    assert(link_state_accept_push(&state, &push) ==
           LINK_STATE_PUSH_ACCEPTED);
    assert(state.latest_revision == 8U);
    assert(strcmp(state.push_data.widget_id, "timer") == 0);
    assert(link_state_note_valid_request(&state, 100U));
    assert(link_state_poll(&state, 10100U));
    assert(!state.online);
    assert(state.latest_revision == 8U);
    assert(state.has_push_data);
}

int main(void)
{
    test_liveness();
    test_time();
    test_push_data();
    puts("test_link_state: OK");
    return 0;
}
