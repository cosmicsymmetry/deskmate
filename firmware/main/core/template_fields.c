#include "template_fields.h"

#include <limits.h>
#include <string.h>

#define TEXT_FIELD(name_, required_, maximum_, default_) \
    {                                                     \
        .name = (name_),                                  \
        .type = PROTOCOL_FIELD_TEXT,                      \
        .required = (required_),                          \
        .maximum_text_length = (maximum_),                \
        .default_value.text = (default_),                 \
    }

#define BOOL_FIELD(name_, required_, default_) \
    {                                              \
        .name = (name_),                           \
        .type = PROTOCOL_FIELD_BOOLEAN,            \
        .required = (required_),                   \
        .default_value.boolean = (default_),       \
    }

#define INT_FIELD(name_, required_, minimum_, maximum_, default_) \
    {                                                            \
        .name = (name_),                                         \
        .type = PROTOCOL_FIELD_INTEGER,                           \
        .required = (required_),                                  \
        .minimum_integer = (minimum_),                            \
        .maximum_integer = (maximum_),                            \
        .default_value.integer = (default_),                      \
    }

static const template_field_descriptor_t s_digital_clock_fields[] = {
    TEXT_FIELD("title", false, 64U, ""),
    BOOL_FIELD("show_seconds", false, true),
    BOOL_FIELD("stale", false, false),
    TEXT_FIELD("error", false, 96U, ""),
};

static const template_field_descriptor_t s_progress_ring_fields[] = {
    TEXT_FIELD("label", false, 64U, "Pomodoro"),
    INT_FIELD("duration_seconds", true, 1, 86400, 0),
    INT_FIELD("remaining_seconds", true, 0, 86400, 0),
    BOOL_FIELD("running", true, false),
    BOOL_FIELD("stale", false, false),
    TEXT_FIELD("error", false, 96U, ""),
};

static const template_field_descriptor_t s_row_list_fields[] = {
    TEXT_FIELD("title", false, 64U, "Calendar"),
    TEXT_FIELD("row0_title", false, 96U, ""),
    TEXT_FIELD("row0_time", false, 32U, ""),
    TEXT_FIELD("row1_title", false, 96U, ""),
    TEXT_FIELD("row1_time", false, 32U, ""),
    TEXT_FIELD("row2_title", false, 96U, ""),
    TEXT_FIELD("row2_time", false, 32U, ""),
    TEXT_FIELD("row3_title", false, 96U, ""),
    TEXT_FIELD("row3_time", false, 32U, ""),
    TEXT_FIELD("row4_title", false, 96U, ""),
    TEXT_FIELD("row4_time", false, 32U, ""),
    BOOL_FIELD("stale", false, false),
    TEXT_FIELD("error", false, 96U, ""),
};

static size_t array_length(size_t bytes, size_t element_size)
{
    return bytes / element_size;
}

const template_field_descriptor_t *template_fields_registry(
    protocol_template_kind_t template_kind,
    size_t *field_count)
{
    const template_field_descriptor_t *fields = NULL;
    size_t count = 0U;
    if (template_kind == PROTOCOL_TEMPLATE_DIGITAL_CLOCK) {
        fields = s_digital_clock_fields;
        count = array_length(sizeof(s_digital_clock_fields),
                             sizeof(s_digital_clock_fields[0]));
    } else if (template_kind == PROTOCOL_TEMPLATE_PROGRESS_RING) {
        fields = s_progress_ring_fields;
        count = array_length(sizeof(s_progress_ring_fields),
                             sizeof(s_progress_ring_fields[0]));
    } else if (template_kind == PROTOCOL_TEMPLATE_ROW_LIST) {
        fields = s_row_list_fields;
        count = array_length(sizeof(s_row_list_fields),
                             sizeof(s_row_list_fields[0]));
    }
    if (field_count != NULL) {
        *field_count = count;
    }
    return fields;
}

bool template_fields_init(protocol_template_kind_t template_kind,
                          template_field_state_t *state)
{
    if (state == NULL) {
        return false;
    }
    size_t count = 0U;
    const template_field_descriptor_t *fields = template_fields_registry(
        template_kind, &count);
    if (fields == NULL || count > PROTOCOL_MAX_FIELD_COUNT) {
        return false;
    }
    memset(state, 0, sizeof(*state));
    state->template_kind = template_kind;
    state->field_count = count;
    for (size_t i = 0U; i < count; ++i) {
        state->values[i].type = fields[i].type;
        if (fields[i].type == PROTOCOL_FIELD_TEXT) {
            strcpy(state->values[i].value.text, fields[i].default_value.text);
        } else if (fields[i].type == PROTOCOL_FIELD_INTEGER) {
            state->values[i].value.integer = fields[i].default_value.integer;
        } else {
            state->values[i].value.boolean = fields[i].default_value.boolean;
        }
    }
    return true;
}

static bool bounded_text_length(const char *text,
                                size_t capacity,
                                size_t *length)
{
    const char *end = memchr(text, '\0', capacity);
    if (end == NULL) {
        return false;
    }
    *length = (size_t)(end - text);
    return true;
}

static bool values_equal(const template_field_value_t *left,
                         const template_field_value_t *right)
{
    if (left->type != right->type) {
        return false;
    }
    if (left->type == PROTOCOL_FIELD_TEXT) {
        return strcmp(left->value.text, right->value.text) == 0;
    }
    if (left->type == PROTOCOL_FIELD_INTEGER) {
        return left->value.integer == right->value.integer;
    }
    return left->value.boolean == right->value.boolean;
}

static size_t descriptor_index(
    const template_field_descriptor_t *fields,
    size_t count,
    const char *name)
{
    for (size_t i = 0U; i < count; ++i) {
        if (strcmp(fields[i].name, name) == 0) {
            return i;
        }
    }
    return count;
}

template_fields_result_t template_fields_resolve(
    template_field_state_t *state,
    template_field_state_t *staging,
    const protocol_push_data_t *push,
    template_field_patch_t *patch)
{
    if (state == NULL || staging == NULL || push == NULL || patch == NULL ||
        state == staging || push->field_count > PROTOCOL_MAX_FIELD_COUNT) {
        return TEMPLATE_FIELDS_INVALID_ARGUMENT;
    }
    memset(patch, 0, sizeof(*patch));
    size_t count = 0U;
    const template_field_descriptor_t *fields = template_fields_registry(
        state->template_kind, &count);
    if (fields == NULL || count != state->field_count ||
        !template_fields_init(state->template_kind, staging)) {
        return TEMPLATE_FIELDS_INVALID_ARGUMENT;
    }

    uint16_t seen = 0U;
    for (size_t i = 0U; i < push->field_count; ++i) {
        const protocol_field_t *incoming = &push->fields[i];
        size_t key_length = 0U;
        if (!bounded_text_length(incoming->key, sizeof(incoming->key),
                                 &key_length) ||
            key_length == 0U) {
            return TEMPLATE_FIELDS_OUT_OF_RANGE;
        }
        size_t index = descriptor_index(fields, count, incoming->key);
        if (index == count) {
            ++patch->unknown_fields;
            continue;
        }
        uint16_t bit = (uint16_t)(UINT16_C(1) << index);
        if ((seen & bit) != 0U) {
            return TEMPLATE_FIELDS_DUPLICATE_FIELD;
        }
        seen |= bit;
        if (incoming->type != fields[index].type) {
            return TEMPLATE_FIELDS_WRONG_TYPE;
        }
        if (incoming->type == PROTOCOL_FIELD_TEXT) {
            size_t length = 0U;
            if (!bounded_text_length(incoming->value.text,
                                     sizeof(incoming->value.text), &length) ||
                length > fields[index].maximum_text_length) {
                return TEMPLATE_FIELDS_OUT_OF_RANGE;
            }
            strcpy(staging->values[index].value.text,
                   incoming->value.text);
        } else if (incoming->type == PROTOCOL_FIELD_INTEGER) {
            if (incoming->value.integer < fields[index].minimum_integer ||
                incoming->value.integer > fields[index].maximum_integer) {
                return TEMPLATE_FIELDS_OUT_OF_RANGE;
            }
            staging->values[index].value.integer = incoming->value.integer;
        } else {
            staging->values[index].value.boolean = incoming->value.boolean;
        }
    }

    for (size_t i = 0U; i < count; ++i) {
        if (fields[i].required &&
            (seen & (uint16_t)(UINT16_C(1) << i)) == 0U) {
            return TEMPLATE_FIELDS_MISSING_REQUIRED;
        }
    }
    if (state->template_kind == PROTOCOL_TEMPLATE_PROGRESS_RING) {
        const template_field_value_t *duration = template_fields_get(
            staging, "duration_seconds");
        const template_field_value_t *remaining = template_fields_get(
            staging, "remaining_seconds");
        if (duration == NULL || remaining == NULL ||
            remaining->value.integer > duration->value.integer) {
            return TEMPLATE_FIELDS_OUT_OF_RANGE;
        }
    }

    for (size_t i = 0U; i < count; ++i) {
        if (!values_equal(&state->values[i], &staging->values[i])) {
            patch->dirty_mask |= (uint16_t)(UINT16_C(1) << i);
        }
    }
    *state = *staging;
    return TEMPLATE_FIELDS_OK;
}

const template_field_value_t *template_fields_get(
    const template_field_state_t *state,
    const char *name)
{
    if (state == NULL || name == NULL) {
        return NULL;
    }
    size_t count = 0U;
    const template_field_descriptor_t *fields = template_fields_registry(
        state->template_kind, &count);
    if (fields == NULL || count != state->field_count) {
        return NULL;
    }
    size_t index = descriptor_index(fields, count, name);
    return index < count ? &state->values[index] : NULL;
}
