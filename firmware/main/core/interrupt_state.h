#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "protocol_message.h"

typedef struct {
    bool occupied;
    char card_id[PROTOCOL_MAX_CARD_ID_LENGTH + 1U];
    uint32_t token;
    char reason[PROTOCOL_MAX_INTERRUPT_REASON_LENGTH + 1U];
} interrupt_slot_t;

typedef struct {
    interrupt_slot_t active;
    interrupt_slot_t pending;
    char saved_card_id[PROTOCOL_MAX_CARD_ID_LENGTH + 1U];
    uint32_t latest_token;
} interrupt_state_t;

typedef enum {
    INTERRUPT_TRIGGER_ACTIVATED = 0,
    INTERRUPT_TRIGGER_QUEUED,
    INTERRUPT_TRIGGER_REPLAYED,
    INTERRUPT_TRIGGER_INVALID_ARGUMENT,
    INTERRUPT_TRIGGER_STALE_TOKEN,
    INTERRUPT_TRIGGER_BUSY,
} interrupt_trigger_result_t;

typedef struct {
    char card_id[PROTOCOL_MAX_CARD_ID_LENGTH + 1U];
    uint32_t token;
    char saved_card_id[PROTOCOL_MAX_CARD_ID_LENGTH + 1U];
    bool promoted_pending;
    bool restore_saved_card;
} interrupt_dismissal_t;

void interrupt_state_init(interrupt_state_t *state);

/** Clear active/pending slots without emitting dismissal or resetting token order. */
void interrupt_state_clear(interrupt_state_t *state);

/** Highest accepted token, retained when active/pending slots are cleared. */
uint32_t interrupt_state_latest_token(const interrupt_state_t *state);

bool interrupt_state_set_saved_card(interrupt_state_t *state,
                                      const char *card_id);

interrupt_trigger_result_t interrupt_state_trigger(
    interrupt_state_t *state,
    const protocol_trigger_interrupt_t *trigger,
    const char *active_carousel_card_id);

bool interrupt_state_dismiss(interrupt_state_t *state,
                             interrupt_dismissal_t *dismissal);

const interrupt_slot_t *interrupt_state_active(
    const interrupt_state_t *state);
