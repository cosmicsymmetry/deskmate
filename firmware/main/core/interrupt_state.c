#include "interrupt_state.h"

#include <string.h>

static bool bounded_nonempty(const char *text, size_t capacity)
{
    const char *end = memchr(text, '\0', capacity);
    return end != NULL && end != text;
}

static bool bounded_text(const char *text, size_t capacity)
{
    return memchr(text, '\0', capacity) != NULL;
}

static bool same_trigger(const interrupt_slot_t *slot,
                         const protocol_trigger_interrupt_t *trigger)
{
    return slot->occupied && slot->token == trigger->token &&
           strcmp(slot->widget_id, trigger->widget_id) == 0 &&
           strcmp(slot->reason, trigger->reason) == 0;
}

static void fill_slot(interrupt_slot_t *slot,
                      const protocol_trigger_interrupt_t *trigger)
{
    memset(slot, 0, sizeof(*slot));
    slot->occupied = true;
    strcpy(slot->widget_id, trigger->widget_id);
    slot->token = trigger->token;
    strcpy(slot->reason, trigger->reason);
}

void interrupt_state_init(interrupt_state_t *state)
{
    if (state != NULL) {
        memset(state, 0, sizeof(*state));
    }
}

void interrupt_state_clear(interrupt_state_t *state)
{
    if (state == NULL) {
        return;
    }
    memset(&state->active, 0, sizeof(state->active));
    memset(&state->pending, 0, sizeof(state->pending));
    memset(state->saved_screen_id, 0, sizeof(state->saved_screen_id));
}

uint32_t interrupt_state_latest_token(const interrupt_state_t *state)
{
    return state == NULL ? 0U : state->latest_token;
}

bool interrupt_state_set_saved_screen(interrupt_state_t *state,
                                      const char *screen_id)
{
    if (state == NULL || !state->active.occupied || screen_id == NULL ||
        !bounded_nonempty(screen_id,
                          PROTOCOL_MAX_SCREEN_ID_LENGTH + 1U)) {
        return false;
    }
    strcpy(state->saved_screen_id, screen_id);
    return true;
}

interrupt_trigger_result_t interrupt_state_trigger(
    interrupt_state_t *state,
    const protocol_trigger_interrupt_t *trigger,
    const char *active_carousel_screen_id)
{
    if (state == NULL || trigger == NULL ||
        !bounded_nonempty(trigger->widget_id, sizeof(trigger->widget_id)) ||
        trigger->token == 0U ||
        !bounded_text(trigger->reason, sizeof(trigger->reason))) {
        return INTERRUPT_TRIGGER_INVALID_ARGUMENT;
    }
    if ((state->active.occupied &&
         state->active.token == trigger->token) ||
        (state->pending.occupied &&
         state->pending.token == trigger->token)) {
        return same_trigger(&state->active, trigger) ||
                       same_trigger(&state->pending, trigger)
                   ? INTERRUPT_TRIGGER_REPLAYED
                   : INTERRUPT_TRIGGER_STALE_TOKEN;
    }
    if (trigger->token <= state->latest_token) {
        return INTERRUPT_TRIGGER_STALE_TOKEN;
    }
    if (state->pending.occupied) {
        return INTERRUPT_TRIGGER_BUSY;
    }
    if (!state->active.occupied) {
        if (active_carousel_screen_id == NULL ||
            !bounded_nonempty(active_carousel_screen_id,
                              PROTOCOL_MAX_SCREEN_ID_LENGTH + 1U)) {
            return INTERRUPT_TRIGGER_INVALID_ARGUMENT;
        }
        fill_slot(&state->active, trigger);
        strcpy(state->saved_screen_id, active_carousel_screen_id);
        state->latest_token = trigger->token;
        return INTERRUPT_TRIGGER_ACTIVATED;
    }
    fill_slot(&state->pending, trigger);
    state->latest_token = trigger->token;
    return INTERRUPT_TRIGGER_QUEUED;
}

bool interrupt_state_dismiss(interrupt_state_t *state,
                             interrupt_dismissal_t *dismissal)
{
    if (state == NULL || dismissal == NULL || !state->active.occupied) {
        return false;
    }
    memset(dismissal, 0, sizeof(*dismissal));
    strcpy(dismissal->widget_id, state->active.widget_id);
    dismissal->token = state->active.token;
    strcpy(dismissal->saved_screen_id, state->saved_screen_id);
    if (state->pending.occupied) {
        state->active = state->pending;
        memset(&state->pending, 0, sizeof(state->pending));
        dismissal->promoted_pending = true;
    } else {
        memset(&state->active, 0, sizeof(state->active));
        memset(state->saved_screen_id, 0, sizeof(state->saved_screen_id));
        dismissal->restore_saved_screen = true;
    }
    return true;
}

const interrupt_slot_t *interrupt_state_active(
    const interrupt_state_t *state)
{
    return state != NULL && state->active.occupied ? &state->active : NULL;
}
