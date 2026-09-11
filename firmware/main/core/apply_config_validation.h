#pragma once

#include <stddef.h>
#include <string.h>

#include "core/protocol_message.h"

typedef enum {
    APPLY_CONFIG_VALID = 0,
    APPLY_CONFIG_INVALID_ARGUMENT,
    APPLY_CONFIG_INVALID_VALUE,
    APPLY_CONFIG_TOO_LARGE,
    APPLY_CONFIG_DUPLICATE_ID,
} apply_config_validation_result_t;

static inline bool apply_config_text_nonempty(const char *text,
                                              size_t capacity)
{
    const char *end = memchr(text, '\0', capacity);
    return end != NULL && end != text;
}

/* Canonical ApplyConfig semantic validation shared by wire and model
 * boundaries. Decode-time CBOR, required-key, narrowing, and scalar range
 * checks remain in protocol_message.c so malformed-wire error precedence is
 * unchanged.
 *
 * Protocol v2 left this with two rules. The template, size-class and
 * tap-action-composition checks all asked whether the device could RENDER a
 * card; it has not rendered one since stage 3a, and the host decides what a
 * face looks like. The screen rules went with the screens array, which since
 * config schema v10 only restated the card list. */
static inline apply_config_validation_result_t apply_config_validate(
    const protocol_apply_config_t *config)
{
    if (config == NULL) {
        return APPLY_CONFIG_INVALID_ARGUMENT;
    }
    if (config->revision == 0U || config->card_count == 0U) {
        return APPLY_CONFIG_INVALID_VALUE;
    }
    if (config->card_count > PROTOCOL_MAX_CONFIG_WIDGETS) {
        return APPLY_CONFIG_TOO_LARGE;
    }
    if (config->rotation != 90U && config->rotation != 270U) {
        return APPLY_CONFIG_INVALID_VALUE;
    }
    for (size_t i = 0U; i < config->card_count; ++i) {
        const protocol_card_config_t *card = &config->cards[i];
        if (!apply_config_text_nonempty(card->card_id,
                                        sizeof(card->card_id))) {
            return APPLY_CONFIG_INVALID_VALUE;
        }
        if (card->tap_action < PROTOCOL_TAP_NONE ||
            card->tap_action > PROTOCOL_TAP_RESET) {
            return APPLY_CONFIG_INVALID_VALUE;
        }
        for (size_t j = 0U; j < i; ++j) {
            if (strcmp(card->card_id, config->cards[j].card_id) == 0) {
                return APPLY_CONFIG_DUPLICATE_ID;
            }
        }
    }
    return APPLY_CONFIG_VALID;
}
