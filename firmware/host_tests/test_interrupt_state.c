#include <assert.h>
#include <stdio.h>
#include <string.h>

#include "../main/core/interrupt_state.h"

static interrupt_state_t s_state;

static protocol_trigger_interrupt_t trigger(const char *widget_id,
                                             uint32_t token,
                                             const char *reason)
{
    protocol_trigger_interrupt_t value = {.token = token};
    strcpy(value.widget_id, widget_id);
    strcpy(value.reason, reason);
    return value;
}

static void test_active_pending_busy_and_restore(void)
{
    interrupt_state_init(&s_state);
    protocol_trigger_interrupt_t first = trigger("timer", 10U, "done");
    protocol_trigger_interrupt_t second = trigger("calendar", 11U, "soon");
    protocol_trigger_interrupt_t third = trigger("timer", 12U, "again");
    assert(interrupt_state_trigger(&s_state, &first, "pomodoro") ==
           INTERRUPT_TRIGGER_ACTIVATED);
    assert(strcmp(s_state.saved_screen_id, "pomodoro") == 0);
    assert(interrupt_state_trigger(&s_state, &second, "ignored") ==
           INTERRUPT_TRIGGER_QUEUED);
    assert(interrupt_state_set_saved_screen(&s_state, "calendar"));
    assert(interrupt_state_trigger(&s_state, &third, "ignored") ==
           INTERRUPT_TRIGGER_BUSY);
    assert(s_state.latest_token == 11U);

    interrupt_dismissal_t dismissal;
    assert(interrupt_state_dismiss(&s_state, &dismissal));
    assert(dismissal.token == 10U && dismissal.promoted_pending);
    assert(!dismissal.restore_saved_screen);
    assert(interrupt_state_active(&s_state)->token == 11U);
    assert(interrupt_state_pending(&s_state) == NULL);

    /* Busy did not consume token 12, so the exact request is retryable. */
    assert(interrupt_state_trigger(&s_state, &third, "ignored") ==
           INTERRUPT_TRIGGER_QUEUED);
    assert(interrupt_state_dismiss(&s_state, &dismissal));
    assert(dismissal.token == 11U && dismissal.promoted_pending);
    assert(interrupt_state_dismiss(&s_state, &dismissal));
    assert(dismissal.token == 12U && dismissal.restore_saved_screen);
    assert(strcmp(dismissal.saved_screen_id, "calendar") == 0);
    assert(interrupt_state_active(&s_state) == NULL);
}

static void test_duplicate_stale_and_semantic_replay(void)
{
    interrupt_state_init(&s_state);
    protocol_trigger_interrupt_t value = trigger("timer", 4U, "done");
    assert(interrupt_state_trigger(&s_state, &value, "clock") ==
           INTERRUPT_TRIGGER_ACTIVATED);
    assert(interrupt_state_trigger(&s_state, &value, "clock") ==
           INTERRUPT_TRIGGER_REPLAYED);
    strcpy(value.reason, "different");
    assert(interrupt_state_trigger(&s_state, &value, "clock") ==
           INTERRUPT_TRIGGER_STALE_TOKEN);
    value = trigger("timer", 3U, "old");
    assert(interrupt_state_trigger(&s_state, &value, "clock") ==
           INTERRUPT_TRIGGER_STALE_TOKEN);
}

static void test_config_clear_retains_token_order(void)
{
    interrupt_state_init(&s_state);
    protocol_trigger_interrupt_t value = trigger("timer", 20U, "done");
    assert(interrupt_state_trigger(&s_state, &value, "clock") ==
           INTERRUPT_TRIGGER_ACTIVATED);
    interrupt_state_clear(&s_state);
    assert(interrupt_state_active(&s_state) == NULL);
    assert(s_state.latest_token == 20U);
    assert(interrupt_state_trigger(&s_state, &value, "clock") ==
           INTERRUPT_TRIGGER_STALE_TOKEN);
}

int main(void)
{
    test_active_pending_busy_and_restore();
    test_duplicate_stale_and_semantic_replay();
    test_config_clear_retains_token_order();
    puts("test_interrupt_state: OK");
    return 0;
}
