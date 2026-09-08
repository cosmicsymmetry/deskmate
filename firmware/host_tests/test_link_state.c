#include <assert.h>
#include <stdio.h>

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
    assert(state.utc_offset_minutes == 240);

    protocol_time_sync_t invalid = valid;
    invalid.utc_offset_minutes = 841;
    assert(!link_state_apply_time_sync(&state, &invalid));
    assert(state.utc_offset_minutes == 240);
}

static void test_network_timeout_is_longer(void)
{
    link_state_t state;
    link_state_init_with_timeout(&state, 45000U);
    link_state_note_valid_request(&state, 0U);
    // Well past the USB deadline, and still online.
    assert(!link_state_poll(&state, 20000U));
    assert(!link_state_poll(&state, 44999U));
    assert(link_state_poll(&state, 45000U));
}

int main(void)
{
    test_liveness();
    test_time();
    test_network_timeout_is_longer();
    puts("test_link_state: OK");
    return 0;
}
