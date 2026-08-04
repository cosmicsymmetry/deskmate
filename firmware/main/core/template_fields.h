#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "protocol_message.h"

typedef union {
    const char *text;
    int64_t integer;
    bool boolean;
} template_field_default_t;

typedef struct {
    const char *name;
    protocol_field_type_t type;
    bool required;
    size_t maximum_text_length;
    int64_t minimum_integer;
    int64_t maximum_integer;
    template_field_default_t default_value;
} template_field_descriptor_t;

typedef struct {
    protocol_field_type_t type;
    union {
        char text[PROTOCOL_MAX_FIELD_TEXT_LENGTH + 1U];
        int64_t integer;
        bool boolean;
    } value;
} template_field_value_t;

typedef struct {
    protocol_template_kind_t template_kind;
    size_t field_count;
    template_field_value_t values[PROTOCOL_MAX_FIELD_COUNT];
} template_field_state_t;

typedef struct {
    uint16_t dirty_mask;
    size_t unknown_fields;
} template_field_patch_t;

typedef enum {
    TEMPLATE_FIELDS_OK = 0,
    TEMPLATE_FIELDS_INVALID_ARGUMENT,
    TEMPLATE_FIELDS_MISSING_REQUIRED,
    TEMPLATE_FIELDS_WRONG_TYPE,
    TEMPLATE_FIELDS_OUT_OF_RANGE,
    TEMPLATE_FIELDS_DUPLICATE_FIELD,
} template_fields_result_t;

const template_field_descriptor_t *template_fields_registry(
    protocol_template_kind_t template_kind,
    size_t *field_count);

bool template_fields_init(protocol_template_kind_t template_kind,
                          template_field_state_t *state);

/**
 * Resolve one complete PushData snapshot into caller-owned staging storage.
 * The live state is copied from staging only after every known field and
 * required-field relationship validates. Unknown fields are counted and
 * ignored. Neither state nor revision ownership lives in this layer.
 */
template_fields_result_t template_fields_resolve(
    template_field_state_t *state,
    template_field_state_t *staging,
    const protocol_push_data_t *push,
    template_field_patch_t *patch);

const template_field_value_t *template_fields_get(
    const template_field_state_t *state,
    const char *name);
