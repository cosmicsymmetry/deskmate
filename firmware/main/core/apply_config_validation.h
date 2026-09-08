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
    APPLY_CONFIG_UNKNOWN_WIDGET,
    APPLY_CONFIG_UNSUPPORTED_TEMPLATE,
    APPLY_CONFIG_UNSUPPORTED_SIZE_CLASS,
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
 * unchanged. */
static inline apply_config_validation_result_t apply_config_validate(
    const protocol_apply_config_t *config)
{
    if (config == NULL) {
        return APPLY_CONFIG_INVALID_ARGUMENT;
    }
    if (config->revision == 0U || config->widget_count == 0U ||
        config->screen_count == 0U) {
        return APPLY_CONFIG_INVALID_VALUE;
    }
    if (config->widget_count > PROTOCOL_MAX_CONFIG_WIDGETS ||
        config->screen_count > PROTOCOL_MAX_CONFIG_SCREENS) {
        return APPLY_CONFIG_TOO_LARGE;
    }
    if (config->rotation != 90U && config->rotation != 270U) {
        return APPLY_CONFIG_INVALID_VALUE;
    }
    for (size_t i = 0U; i < config->widget_count; ++i) {
        const protocol_widget_config_t *widget = &config->widgets[i];
        if (!apply_config_text_nonempty(widget->widget_id,
                                        sizeof(widget->widget_id))) {
            return APPLY_CONFIG_INVALID_VALUE;
        }
        if (!protocol_template_kind_valid(widget->template_kind)) {
            return APPLY_CONFIG_UNSUPPORTED_TEMPLATE;
        }
        if (widget->size_class != PROTOCOL_SIZE_FULL &&
            widget->size_class != PROTOCOL_SIZE_STANDARD) {
            return APPLY_CONFIG_UNSUPPORTED_SIZE_CLASS;
        }
        if (widget->tap_action < PROTOCOL_TAP_NONE ||
            widget->tap_action > PROTOCOL_TAP_RESET ||
            widget->interrupt_policy < PROTOCOL_INTERRUPT_DISABLED ||
            widget->interrupt_policy > PROTOCOL_INTERRUPT_ENABLED ||
            (widget->template_kind != PROTOCOL_TEMPLATE_PROGRESS_RING &&
             widget->tap_action != PROTOCOL_TAP_NONE)) {
            return APPLY_CONFIG_INVALID_VALUE;
        }
        for (size_t j = 0U; j < i; ++j) {
            if (strcmp(widget->widget_id, config->widgets[j].widget_id) == 0) {
                return APPLY_CONFIG_DUPLICATE_ID;
            }
        }
    }
    for (size_t i = 0U; i < config->screen_count; ++i) {
        const protocol_screen_config_t *screen = &config->screens[i];
        if (!apply_config_text_nonempty(screen->screen_id,
                                        sizeof(screen->screen_id)) ||
            !apply_config_text_nonempty(screen->widget_id,
                                        sizeof(screen->widget_id))) {
            return APPLY_CONFIG_INVALID_VALUE;
        }
        for (size_t j = 0U; j < i; ++j) {
            if (strcmp(screen->screen_id, config->screens[j].screen_id) == 0) {
                return APPLY_CONFIG_DUPLICATE_ID;
            }
        }
        bool found = false;
        for (size_t j = 0U; j < config->widget_count; ++j) {
            if (strcmp(screen->widget_id, config->widgets[j].widget_id) == 0) {
                found = true;
                break;
            }
        }
        if (!found) {
            return APPLY_CONFIG_UNKNOWN_WIDGET;
        }
    }
    return APPLY_CONFIG_VALID;
}
