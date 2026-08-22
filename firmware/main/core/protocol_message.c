#include "protocol_message.h"

#include <string.h>

#include "cbor.h"

#define REQUIRED_BIT(key) (UINT32_C(1) << (key))

static protocol_message_result_t cbor_result(CborError error)
{
    if (error == CborNoError) {
        return PROTOCOL_MESSAGE_OK;
    }
    if (error == CborErrorOutOfMemory || error == CborErrorDataTooLarge) {
        return PROTOCOL_MESSAGE_ERR_TOO_LARGE;
    }
    if (error == CborErrorMapKeysNotUnique || error == CborErrorMapNotSorted) {
        return PROTOCOL_MESSAGE_ERR_DUPLICATE_KEY;
    }
    return PROTOCOL_MESSAGE_ERR_CBOR;
}

static bool bounded_length(const char *text, size_t capacity, size_t *length)
{
    const char *end = memchr(text, '\0', capacity);
    if (end == NULL) {
        return false;
    }
    *length = (size_t)(end - text);
    return true;
}

bool protocol_template_kind_valid(protocol_template_kind_t kind)
{
    return kind >= PROTOCOL_TEMPLATE_DIGITAL_CLOCK &&
           kind <= PROTOCOL_TEMPLATE_ICON_BADGE_TEXT;
}

static protocol_message_result_t open_payload_map(
    const uint8_t *payload,
    size_t payload_length,
    CborParser *parser,
    CborValue *contents,
    size_t *map_length)
{
    CborValue root;
    CborError error = cbor_parser_init(payload, payload_length, 0, parser, &root);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    error = cbor_value_validate(
        &root,
        CborValidateCanonicalFormat | CborValidateMapKeysAreUnique |
            CborValidateUtf8 | CborValidateCompleteData |
            CborValidateNoUndefined | CborValidateNoTags);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    if (!cbor_value_is_map(&root)) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    error = cbor_value_get_map_length(&root, map_length);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    error = cbor_value_enter_container(&root, contents);
    return cbor_result(error);
}

static protocol_message_result_t read_unsigned(CborValue *value,
                                                uint64_t *result)
{
    if (!cbor_value_is_unsigned_integer(value)) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    CborError error = cbor_value_get_uint64(value, result);
    if (error == CborNoError) {
        error = cbor_value_advance_fixed(value);
    }
    return cbor_result(error);
}

static protocol_message_result_t read_signed(CborValue *value, int64_t *result)
{
    if (!cbor_value_is_integer(value)) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    CborError error = cbor_value_get_int64_checked(value, result);
    if (error == CborNoError) {
        error = cbor_value_advance_fixed(value);
    }
    return cbor_result(error);
}

static protocol_message_result_t read_boolean(CborValue *value, bool *result)
{
    if (!cbor_value_is_boolean(value)) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    CborError error = cbor_value_get_boolean(value, result);
    if (error == CborNoError) {
        error = cbor_value_advance_fixed(value);
    }
    return cbor_result(error);
}

static protocol_message_result_t read_text(CborValue *value,
                                            char *destination,
                                            size_t destination_capacity,
                                            size_t minimum_length,
                                            size_t maximum_length)
{
    if (!cbor_value_is_text_string(value)) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    size_t length = 0U;
    CborError error = cbor_value_calculate_string_length(value, &length);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    if (length < minimum_length || length > maximum_length ||
        destination_capacity <= length) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    size_t copied = destination_capacity - 1U;
    CborValue next;
    error = cbor_value_copy_text_string(value, destination, &copied, &next);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    destination[copied] = '\0';
    *value = next;
    return PROTOCOL_MESSAGE_OK;
}

static protocol_message_result_t read_bytes_exact(CborValue *value,
                                                   uint8_t *destination,
                                                   size_t exact_length)
{
    if (!cbor_value_is_byte_string(value)) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    size_t length = 0U;
    CborError error = cbor_value_calculate_string_length(value, &length);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    if (length != exact_length) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    size_t copied = exact_length;
    CborValue next;
    error = cbor_value_copy_byte_string(value, destination, &copied, &next);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    *value = next;
    return PROTOCOL_MESSAGE_OK;
}

static protocol_message_result_t read_bytes_bounded(CborValue *value,
                                                     uint8_t *destination,
                                                     size_t max_length,
                                                     size_t *out_length)
{
    if (!cbor_value_is_byte_string(value)) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    size_t length = 0U;
    CborError error = cbor_value_calculate_string_length(value, &length);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    if (length > max_length) {
        return PROTOCOL_MESSAGE_ERR_TOO_LARGE;
    }
    size_t copied = max_length;
    CborValue next;
    error = cbor_value_copy_byte_string(value, destination, &copied, &next);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    *value = next;
    *out_length = copied;
    return PROTOCOL_MESSAGE_OK;
}

static protocol_message_result_t read_key(CborValue *contents,
                                           uint64_t *key,
                                           uint64_t *previous,
                                           bool *has_previous)
{
    protocol_message_result_t result = read_unsigned(contents, key);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    if (*has_previous && *key <= *previous) {
        return PROTOCOL_MESSAGE_ERR_DUPLICATE_KEY;
    }
    *previous = *key;
    *has_previous = true;
    return PROTOCOL_MESSAGE_OK;
}

static protocol_message_result_t skip_value(CborValue *value)
{
    return cbor_result(cbor_value_advance(value));
}

static protocol_message_result_t require_empty_map(const protocol_frame_t *frame)
{
    CborParser parser;
    CborValue contents;
    size_t count = 0U;
    protocol_message_result_t result = open_payload_map(
        frame->payload, frame->payload_length, &parser, &contents, &count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    return count == 0U ? PROTOCOL_MESSAGE_OK
                       : PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
}

static protocol_message_result_t decode_time_sync(
    const protocol_frame_t *frame,
    protocol_time_sync_t *sync)
{
    CborParser parser;
    CborValue contents;
    size_t count = 0U;
    protocol_message_result_t result = open_payload_map(
        frame->payload, frame->payload_length, &parser, &contents, &count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }

    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0; i < count; ++i) {
        uint64_t key = 0U;
        result = read_key(&contents, &key, &previous, &has_previous);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key == 0U) {
            result = read_signed(&contents, &sync->unix_seconds);
            present |= REQUIRED_BIT(0);
        } else if (key == 1U) {
            int64_t offset = 0;
            result = read_signed(&contents, &offset);
            if (result == PROTOCOL_MESSAGE_OK &&
                (offset < PROTOCOL_MIN_UTC_OFFSET_MINUTES ||
                 offset > PROTOCOL_MAX_UTC_OFFSET_MINUTES)) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_TIME;
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                sync->utc_offset_minutes = (int16_t)offset;
            }
            present |= REQUIRED_BIT(1);
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    if ((present & (REQUIRED_BIT(0) | REQUIRED_BIT(1))) !=
        (REQUIRED_BIT(0) | REQUIRED_BIT(1))) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    if (sync->unix_seconds < PROTOCOL_MIN_UNIX_SECONDS ||
        sync->unix_seconds > PROTOCOL_MAX_UNIX_SECONDS) {
        return PROTOCOL_MESSAGE_ERR_INVALID_TIME;
    }
    return PROTOCOL_MESSAGE_OK;
}

static protocol_message_result_t decode_fields(CborValue *value,
                                                protocol_push_data_t *push)
{
    if (!cbor_value_is_map(value)) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    size_t count = 0U;
    CborError error = cbor_value_get_map_length(value, &count);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    if (count > PROTOCOL_MAX_FIELD_COUNT) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    CborValue fields;
    error = cbor_value_enter_container(value, &fields);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    for (size_t i = 0; i < count; ++i) {
        protocol_field_t *field = &push->fields[i];
        protocol_message_result_t result = read_text(
            &fields, field->key, sizeof(field->key), 1U,
            PROTOCOL_MAX_FIELD_KEY_LENGTH);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (cbor_value_is_integer(&fields)) {
            field->type = PROTOCOL_FIELD_INTEGER;
            result = read_signed(&fields, &field->value.integer);
        } else if (cbor_value_is_text_string(&fields)) {
            field->type = PROTOCOL_FIELD_TEXT;
            result = read_text(&fields, field->value.text,
                               sizeof(field->value.text), 0U,
                               PROTOCOL_MAX_FIELD_TEXT_LENGTH);
        } else if (cbor_value_is_boolean(&fields)) {
            field->type = PROTOCOL_FIELD_BOOLEAN;
            result = read_boolean(&fields, &field->value.boolean);
        } else {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    push->field_count = count;
    error = cbor_value_leave_container(value, &fields);
    return cbor_result(error);
}

static protocol_message_result_t decode_push_data(
    const protocol_frame_t *frame,
    protocol_push_data_t *push)
{
    CborParser parser;
    CborValue contents;
    size_t count = 0U;
    protocol_message_result_t result = open_payload_map(
        frame->payload, frame->payload_length, &parser, &contents, &count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }

    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0; i < count; ++i) {
        uint64_t key = 0U;
        result = read_key(&contents, &key, &previous, &has_previous);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key == 0U) {
            result = read_text(&contents, push->widget_id,
                               sizeof(push->widget_id), 1U,
                               PROTOCOL_MAX_WIDGET_ID_LENGTH);
            present |= REQUIRED_BIT(0);
        } else if (key == 1U) {
            uint64_t revision = 0U;
            result = read_unsigned(&contents, &revision);
            if (result == PROTOCOL_MESSAGE_OK &&
                (revision == 0U || revision > UINT32_MAX)) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            push->revision = (uint32_t)revision;
            present |= REQUIRED_BIT(1);
        } else if (key == 2U) {
            result = decode_fields(&contents, push);
            present |= REQUIRED_BIT(2);
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    uint32_t required = REQUIRED_BIT(0) | REQUIRED_BIT(1) | REQUIRED_BIT(2);
    return (present & required) == required ? PROTOCOL_MESSAGE_OK
                                            : PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
}

static protocol_message_result_t validate_network_config(
    const protocol_network_config_t *config)
{
    size_t length = 0U;
    if (!bounded_length(config->ssid, sizeof(config->ssid), &length) ||
        !bounded_length(config->psk, sizeof(config->psk), &length) ||
        !bounded_length(config->server_url, sizeof(config->server_url),
                        &length) ||
        !bounded_length(config->device_id, sizeof(config->device_id),
                        &length) ||
        !bounded_length(config->token, sizeof(config->token), &length)) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    if (config->utc_offset_minutes < PROTOCOL_MIN_UTC_OFFSET_MINUTES ||
        config->utc_offset_minutes > PROTOCOL_MAX_UTC_OFFSET_MINUTES) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    if (config->tier != PROTOCOL_TIER_LOCAL &&
        config->tier != PROTOCOL_TIER_NETWORKED) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    return PROTOCOL_MESSAGE_OK;
}

static protocol_message_result_t decode_network_config(
    const protocol_frame_t *frame,
    protocol_network_config_t *config)
{
    CborParser parser;
    CborValue contents;
    size_t count = 0U;
    protocol_message_result_t result = open_payload_map(
        frame->payload, frame->payload_length, &parser, &contents, &count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        result = read_key(&contents, &key, &previous, &has_previous);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key == 1U) {
            result = read_text(&contents, config->ssid, sizeof(config->ssid),
                               0U, PROTOCOL_MAX_SSID_LENGTH);
        } else if (key == 2U) {
            result = read_text(&contents, config->psk, sizeof(config->psk),
                               0U, PROTOCOL_MAX_PSK_LENGTH);
        } else if (key == 3U) {
            result = read_text(&contents, config->server_url,
                               sizeof(config->server_url), 0U,
                               PROTOCOL_MAX_SERVER_URL_LENGTH);
        } else if (key == 4U) {
            result = read_text(&contents, config->device_id,
                               sizeof(config->device_id), 0U,
                               PROTOCOL_MAX_DEVICE_ID_LENGTH);
        } else if (key == 5U) {
            result = read_text(&contents, config->token,
                               sizeof(config->token), 0U,
                               PROTOCOL_MAX_DEVICE_TOKEN_LENGTH);
        } else if (key == 6U) {
            int64_t offset = 0;
            result = read_signed(&contents, &offset);
            if (result == PROTOCOL_MESSAGE_OK &&
                (offset < PROTOCOL_MIN_UTC_OFFSET_MINUTES ||
                 offset > PROTOCOL_MAX_UTC_OFFSET_MINUTES)) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                config->utc_offset_minutes = (int16_t)offset;
            }
        } else if (key == 7U) {
            uint64_t raw = 0U;
            result = read_unsigned(&contents, &raw);
            if (result == PROTOCOL_MESSAGE_OK &&
                raw != (uint64_t)PROTOCOL_TIER_LOCAL &&
                raw != (uint64_t)PROTOCOL_TIER_NETWORKED) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                config->tier = (protocol_tier_t)raw;
            }
        } else {
            /* Unlike the other additive message types, network config is a
             * closed, security-sensitive schema: an unrecognized key is
             * rejected rather than skipped. */
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        present |= REQUIRED_BIT((uint32_t)key);
    }
    uint32_t required = REQUIRED_BIT(1) | REQUIRED_BIT(2) | REQUIRED_BIT(3) |
                        REQUIRED_BIT(4) | REQUIRED_BIT(5) | REQUIRED_BIT(6) |
                        REQUIRED_BIT(7);
    if ((present & required) != required) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    return validate_network_config(config);
}

static protocol_message_result_t validate_apply_config(
    const protocol_apply_config_t *config)
{
    if (config->revision == 0U || config->widget_count == 0U ||
        config->screen_count == 0U) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    if (config->widget_count > PROTOCOL_MAX_CONFIG_WIDGETS ||
        config->screen_count > PROTOCOL_MAX_CONFIG_SCREENS) {
        return PROTOCOL_MESSAGE_ERR_CONFIG_TOO_LARGE;
    }
    if (config->rotation != 90U && config->rotation != 270U) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    for (size_t i = 0U; i < config->widget_count; ++i) {
        const protocol_widget_config_t *widget = &config->widgets[i];
        size_t length = 0U;
        if (!bounded_length(widget->widget_id, sizeof(widget->widget_id),
                            &length) ||
            length == 0U) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        if (!protocol_template_kind_valid(widget->template_kind)) {
            return PROTOCOL_MESSAGE_ERR_UNSUPPORTED_TEMPLATE;
        }
        if (widget->size_class == PROTOCOL_SIZE_TILE ||
            widget->size_class < PROTOCOL_SIZE_FULL ||
            widget->size_class > PROTOCOL_SIZE_TILE) {
            return PROTOCOL_MESSAGE_ERR_UNSUPPORTED_SIZE_CLASS;
        }
        if (widget->tap_action < PROTOCOL_TAP_NONE ||
            widget->tap_action > PROTOCOL_TAP_RESET ||
            widget->interrupt_policy < PROTOCOL_INTERRUPT_DISABLED ||
            widget->interrupt_policy > PROTOCOL_INTERRUPT_ENABLED ||
            (widget->template_kind != PROTOCOL_TEMPLATE_PROGRESS_RING &&
             widget->tap_action != PROTOCOL_TAP_NONE)) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        for (size_t j = 0U; j < i; ++j) {
            if (strcmp(widget->widget_id, config->widgets[j].widget_id) == 0) {
                return PROTOCOL_MESSAGE_ERR_DUPLICATE_KEY;
            }
        }
    }
    for (size_t i = 0U; i < config->screen_count; ++i) {
        const protocol_screen_config_t *screen = &config->screens[i];
        size_t length = 0U;
        if (!bounded_length(screen->screen_id, sizeof(screen->screen_id),
                            &length) ||
            length == 0U ||
            !bounded_length(screen->widget_id, sizeof(screen->widget_id),
                            &length) ||
            length == 0U) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        for (size_t j = 0U; j < i; ++j) {
            if (strcmp(screen->screen_id, config->screens[j].screen_id) == 0) {
                return PROTOCOL_MESSAGE_ERR_DUPLICATE_KEY;
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
            return PROTOCOL_MESSAGE_ERR_UNKNOWN_WIDGET;
        }
    }
    return PROTOCOL_MESSAGE_OK;
}

static protocol_message_result_t decode_asset_begin(
    const protocol_frame_t *frame,
    protocol_asset_begin_t *begin)
{
    CborParser parser;
    CborValue contents;
    size_t count = 0U;
    protocol_message_result_t result = open_payload_map(
        frame->payload, frame->payload_length, &parser, &contents, &count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        result = read_key(&contents, &key, &previous, &has_previous);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key == 0U) {
            result = read_bytes_exact(&contents, begin->digest,
                                      ASSET_DIGEST_BYTES);
        } else if (key == 1U) {
            uint64_t raw = 0U;
            result = read_unsigned(&contents, &raw);
            if (result == PROTOCOL_MESSAGE_OK) {
                if (raw != (uint64_t)ASSET_KIND_FONT &&
                    raw != (uint64_t)ASSET_KIND_ICON_FONT &&
                    raw != (uint64_t)ASSET_KIND_IMAGE) {
                    result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
                } else {
                    begin->kind = (asset_kind_t)raw;
                }
            }
        } else if (key == 2U) {
            uint64_t raw = 0U;
            result = read_unsigned(&contents, &raw);
            if (result == PROTOCOL_MESSAGE_OK &&
                (raw == 0U || raw > (uint64_t)ASSET_MAX_BYTES)) {
                result = PROTOCOL_MESSAGE_ERR_TOO_LARGE;
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                begin->total_length = (uint32_t)raw;
            }
        } else if (key == 3U) {
            result = read_boolean(&contents, &begin->volatile_tier);
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key <= 3U) {
            present |= REQUIRED_BIT((uint32_t)key);
        }
    }
    uint32_t required =
        REQUIRED_BIT(0) | REQUIRED_BIT(1) | REQUIRED_BIT(2) | REQUIRED_BIT(3);
    if ((present & required) != required) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    return PROTOCOL_MESSAGE_OK;
}

static protocol_message_result_t decode_asset_chunk(
    const protocol_frame_t *frame,
    protocol_asset_chunk_t *chunk)
{
    CborParser parser;
    CborValue contents;
    size_t count = 0U;
    protocol_message_result_t result = open_payload_map(
        frame->payload, frame->payload_length, &parser, &contents, &count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        result = read_key(&contents, &key, &previous, &has_previous);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key == 0U) {
            result = read_bytes_exact(&contents, chunk->digest,
                                      ASSET_DIGEST_BYTES);
        } else if (key == 1U) {
            uint64_t raw = 0U;
            result = read_unsigned(&contents, &raw);
            if (result == PROTOCOL_MESSAGE_OK && raw > UINT32_MAX) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                chunk->offset = (uint32_t)raw;
            }
        } else if (key == 2U) {
            result = read_bytes_bounded(&contents, chunk->data,
                                        PROTOCOL_MAX_ASSET_CHUNK_BYTES,
                                        &chunk->data_length);
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key <= 2U) {
            present |= REQUIRED_BIT((uint32_t)key);
        }
    }
    uint32_t required = REQUIRED_BIT(0) | REQUIRED_BIT(1) | REQUIRED_BIT(2);
    if ((present & required) != required) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    return PROTOCOL_MESSAGE_OK;
}

static protocol_message_result_t decode_asset_commit(
    const protocol_frame_t *frame,
    protocol_asset_commit_t *commit)
{
    CborParser parser;
    CborValue contents;
    size_t count = 0U;
    protocol_message_result_t result = open_payload_map(
        frame->payload, frame->payload_length, &parser, &contents, &count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        result = read_key(&contents, &key, &previous, &has_previous);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key == 0U) {
            result = read_bytes_exact(&contents, commit->digest,
                                      ASSET_DIGEST_BYTES);
            present |= REQUIRED_BIT(0);
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    if ((present & REQUIRED_BIT(0)) == 0U) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    return PROTOCOL_MESSAGE_OK;
}

static protocol_message_result_t decode_asset_digests(
    CborValue *value,
    protocol_asset_release_t *release)
{
    if (!cbor_value_is_array(value)) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    size_t count = 0U;
    CborError error = cbor_value_get_array_length(value, &count);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    if (count > PROTOCOL_MAX_ASSET_DIGESTS) {
        return PROTOCOL_MESSAGE_ERR_TOO_LARGE;
    }
    CborValue items;
    error = cbor_value_enter_container(value, &items);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    for (size_t i = 0U; i < count; ++i) {
        protocol_message_result_t result = read_bytes_exact(
            &items, release->digests[i], ASSET_DIGEST_BYTES);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    release->digest_count = count;
    return cbor_result(cbor_value_leave_container(value, &items));
}

static protocol_message_result_t decode_asset_release(
    const protocol_frame_t *frame,
    protocol_asset_release_t *release)
{
    CborParser parser;
    CborValue contents;
    size_t count = 0U;
    protocol_message_result_t result = open_payload_map(
        frame->payload, frame->payload_length, &parser, &contents, &count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        result = read_key(&contents, &key, &previous, &has_previous);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key == 0U) {
            result = decode_asset_digests(&contents, release);
            present |= REQUIRED_BIT(0);
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    if ((present & REQUIRED_BIT(0)) == 0U) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    return PROTOCOL_MESSAGE_OK;
}

static protocol_message_result_t decode_widget(CborValue *value,
                                                protocol_widget_config_t *widget)
{
    if (!cbor_value_is_map(value)) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    size_t count = 0U;
    CborError error = cbor_value_get_map_length(value, &count);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    CborValue fields;
    error = cbor_value_enter_container(value, &fields);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        protocol_message_result_t result =
            read_key(&fields, &key, &previous, &has_previous);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key == 0U) {
            result = read_text(&fields, widget->widget_id,
                               sizeof(widget->widget_id), 1U,
                               PROTOCOL_MAX_WIDGET_ID_LENGTH);
        } else if (key >= 1U && key <= 4U) {
            uint64_t raw = 0U;
            result = read_unsigned(&fields, &raw);
            if (result == PROTOCOL_MESSAGE_OK && raw > UINT8_MAX) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            if (result == PROTOCOL_MESSAGE_OK && key == 1U) {
                if (!protocol_template_kind_valid((protocol_template_kind_t)raw)) {
                    result = PROTOCOL_MESSAGE_ERR_UNSUPPORTED_TEMPLATE;
                } else {
                    widget->template_kind = (protocol_template_kind_t)raw;
                }
            } else if (result == PROTOCOL_MESSAGE_OK && key == 2U) {
                if (raw < PROTOCOL_SIZE_FULL || raw > PROTOCOL_SIZE_TILE ||
                    raw == PROTOCOL_SIZE_TILE) {
                    result = PROTOCOL_MESSAGE_ERR_UNSUPPORTED_SIZE_CLASS;
                } else {
                    widget->size_class = (protocol_size_class_t)raw;
                }
            } else if (result == PROTOCOL_MESSAGE_OK && key == 3U) {
                if (raw > PROTOCOL_TAP_RESET) {
                    result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
                } else {
                    widget->tap_action = (protocol_tap_action_t)raw;
                }
            } else if (result == PROTOCOL_MESSAGE_OK && key == 4U) {
                if (raw > PROTOCOL_INTERRUPT_ENABLED) {
                    result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
                } else {
                    widget->interrupt_policy =
                        (protocol_interrupt_policy_t)raw;
                }
            }
        } else {
            result = skip_value(&fields);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key <= 4U) {
            present |= REQUIRED_BIT((uint32_t)key);
        }
    }
    if ((present & UINT32_C(0x1f)) != UINT32_C(0x1f)) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    return cbor_result(cbor_value_leave_container(value, &fields));
}

static protocol_message_result_t decode_widgets(CborValue *value,
                                                 protocol_apply_config_t *config)
{
    if (!cbor_value_is_array(value)) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    size_t count = 0U;
    CborError error = cbor_value_get_array_length(value, &count);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    if (count > PROTOCOL_MAX_CONFIG_WIDGETS) {
        return PROTOCOL_MESSAGE_ERR_CONFIG_TOO_LARGE;
    }
    CborValue items;
    error = cbor_value_enter_container(value, &items);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    for (size_t i = 0U; i < count; ++i) {
        protocol_message_result_t result =
            decode_widget(&items, &config->widgets[i]);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    config->widget_count = count;
    return cbor_result(cbor_value_leave_container(value, &items));
}

static protocol_message_result_t decode_screen(CborValue *value,
                                                protocol_screen_config_t *screen)
{
    if (!cbor_value_is_map(value)) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    size_t count = 0U;
    CborError error = cbor_value_get_map_length(value, &count);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    CborValue fields;
    error = cbor_value_enter_container(value, &fields);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        protocol_message_result_t result =
            read_key(&fields, &key, &previous, &has_previous);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key == 0U) {
            result = read_text(&fields, screen->screen_id,
                               sizeof(screen->screen_id), 1U,
                               PROTOCOL_MAX_SCREEN_ID_LENGTH);
            present |= REQUIRED_BIT(0);
        } else if (key == 1U) {
            result = read_text(&fields, screen->widget_id,
                               sizeof(screen->widget_id), 1U,
                               PROTOCOL_MAX_WIDGET_ID_LENGTH);
            present |= REQUIRED_BIT(1);
        } else {
            result = skip_value(&fields);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    if ((present & UINT32_C(0x03)) != UINT32_C(0x03)) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    return cbor_result(cbor_value_leave_container(value, &fields));
}

static protocol_message_result_t decode_screens(CborValue *value,
                                                 protocol_apply_config_t *config)
{
    if (!cbor_value_is_array(value)) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    size_t count = 0U;
    CborError error = cbor_value_get_array_length(value, &count);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    if (count > PROTOCOL_MAX_CONFIG_SCREENS) {
        return PROTOCOL_MESSAGE_ERR_CONFIG_TOO_LARGE;
    }
    CborValue items;
    error = cbor_value_enter_container(value, &items);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    for (size_t i = 0U; i < count; ++i) {
        protocol_message_result_t result =
            decode_screen(&items, &config->screens[i]);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    config->screen_count = count;
    return cbor_result(cbor_value_leave_container(value, &items));
}

static protocol_message_result_t decode_apply_config(
    const protocol_frame_t *frame,
    protocol_apply_config_t *config)
{
    config->rotation = 90U;
    CborParser parser;
    CborValue contents;
    size_t count = 0U;
    protocol_message_result_t result = open_payload_map(
        frame->payload, frame->payload_length, &parser, &contents, &count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        result = read_key(&contents, &key, &previous, &has_previous);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key == 0U) {
            uint64_t revision = 0U;
            result = read_unsigned(&contents, &revision);
            if (result == PROTOCOL_MESSAGE_OK &&
                (revision == 0U || revision > UINT32_MAX)) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            config->revision = (uint32_t)revision;
            present |= REQUIRED_BIT(0);
        } else if (key == 1U) {
            result = decode_widgets(&contents, config);
            present |= REQUIRED_BIT(1);
        } else if (key == 2U) {
            result = decode_screens(&contents, config);
            present |= REQUIRED_BIT(2);
        } else if (key == 3U) {
            uint64_t rotation = 0U;
            result = read_unsigned(&contents, &rotation);
            if (result == PROTOCOL_MESSAGE_OK &&
                rotation != 90U && rotation != 270U) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            config->rotation = (uint16_t)rotation;
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    if ((present & UINT32_C(0x07)) != UINT32_C(0x07)) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    return validate_apply_config(config);
}

static protocol_message_result_t decode_activate_screen(
    const protocol_frame_t *frame,
    protocol_activate_screen_t *activate)
{
    CborParser parser;
    CborValue contents;
    size_t count = 0U;
    protocol_message_result_t result = open_payload_map(
        frame->payload, frame->payload_length, &parser, &contents, &count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    bool present = false;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        result = read_key(&contents, &key, &previous, &has_previous);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key == 0U) {
            result = read_text(&contents, activate->screen_id,
                               sizeof(activate->screen_id), 1U,
                               PROTOCOL_MAX_SCREEN_ID_LENGTH);
            present = true;
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    return present ? PROTOCOL_MESSAGE_OK : PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
}

static protocol_message_result_t decode_trigger_interrupt(
    const protocol_frame_t *frame,
    protocol_trigger_interrupt_t *interrupt)
{
    CborParser parser;
    CborValue contents;
    size_t count = 0U;
    protocol_message_result_t result = open_payload_map(
        frame->payload, frame->payload_length, &parser, &contents, &count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        result = read_key(&contents, &key, &previous, &has_previous);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key == 0U) {
            result = read_text(&contents, interrupt->widget_id,
                               sizeof(interrupt->widget_id), 1U,
                               PROTOCOL_MAX_WIDGET_ID_LENGTH);
            present |= REQUIRED_BIT(0);
        } else if (key == 1U) {
            uint64_t token = 0U;
            result = read_unsigned(&contents, &token);
            if (result == PROTOCOL_MESSAGE_OK &&
                (token == 0U || token > UINT32_MAX)) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            interrupt->token = (uint32_t)token;
            present |= REQUIRED_BIT(1);
        } else if (key == 2U) {
            result = read_text(&contents, interrupt->reason,
                               sizeof(interrupt->reason), 0U,
                               PROTOCOL_MAX_INTERRUPT_REASON_LENGTH);
            present |= REQUIRED_BIT(2);
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    return (present & UINT32_C(0x07)) == UINT32_C(0x07)
               ? PROTOCOL_MESSAGE_OK
               : PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
}

static protocol_message_result_t validate_device_event(
    const protocol_device_event_t *event)
{
    size_t length = 0U;
    if (event->sequence == 0U ||
        !bounded_length(event->widget_id, sizeof(event->widget_id), &length) ||
        length == 0U ||
        !bounded_length(event->screen_id, sizeof(event->screen_id), &length) ||
        length == 0U) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    bool valid = false;
    if (event->kind == PROTOCOL_EVENT_TAP) {
        valid = (event->action == PROTOCOL_EVENT_ACTION_START_PAUSE ||
                 event->action == PROTOCOL_EVENT_ACTION_RESET) &&
                !event->has_interrupt_token;
    } else if (event->kind == PROTOCOL_EVENT_NAVIGATION) {
        valid = (event->action == PROTOCOL_EVENT_ACTION_NAVIGATE_PREVIOUS ||
                 event->action == PROTOCOL_EVENT_ACTION_NAVIGATE_NEXT) &&
                !event->has_interrupt_token;
    } else if (event->kind == PROTOCOL_EVENT_INTERRUPT_DISMISSED) {
        valid = event->action == PROTOCOL_EVENT_ACTION_DISMISS_INTERRUPT &&
                event->has_interrupt_token && event->interrupt_token != 0U;
    }
    return valid ? PROTOCOL_MESSAGE_OK : PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
}

static protocol_message_result_t decode_device_event(
    const protocol_frame_t *frame,
    protocol_device_event_t *event)
{
    CborParser parser;
    CborValue contents;
    size_t count = 0U;
    protocol_message_result_t result = open_payload_map(
        frame->payload, frame->payload_length, &parser, &contents, &count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        result = read_key(&contents, &key, &previous, &has_previous);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key == 0U) {
            result = read_unsigned(&contents, &event->sequence);
            present |= REQUIRED_BIT(0);
        } else if (key == 1U || key == 4U || key == 5U) {
            uint64_t raw = 0U;
            result = read_unsigned(&contents, &raw);
            if (result == PROTOCOL_MESSAGE_OK && raw > UINT32_MAX) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            if (result == PROTOCOL_MESSAGE_OK && key == 1U) {
                if (raw < PROTOCOL_EVENT_TAP ||
                    raw > PROTOCOL_EVENT_INTERRUPT_DISMISSED) {
                    result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
                } else {
                    event->kind = (protocol_event_kind_t)raw;
                }
            } else if (result == PROTOCOL_MESSAGE_OK && key == 4U) {
                if (raw > PROTOCOL_EVENT_ACTION_DISMISS_INTERRUPT) {
                    result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
                } else {
                    event->action = (protocol_event_action_t)raw;
                }
            } else if (result == PROTOCOL_MESSAGE_OK && key == 5U) {
                event->has_interrupt_token = true;
                event->interrupt_token = (uint32_t)raw;
            }
            present |= REQUIRED_BIT((uint32_t)key);
        } else if (key == 2U) {
            result = read_text(&contents, event->widget_id,
                               sizeof(event->widget_id), 1U,
                               PROTOCOL_MAX_WIDGET_ID_LENGTH);
            present |= REQUIRED_BIT(2);
        } else if (key == 3U) {
            result = read_text(&contents, event->screen_id,
                               sizeof(event->screen_id), 1U,
                               PROTOCOL_MAX_SCREEN_ID_LENGTH);
            present |= REQUIRED_BIT(3);
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    if ((present & UINT32_C(0x1f)) != UINT32_C(0x1f)) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    return validate_device_event(event);
}

static protocol_message_result_t decode_ack(const protocol_frame_t *frame,
                                             protocol_ack_t *ack)
{
    CborParser parser;
    CborValue contents;
    size_t count = 0U;
    protocol_message_result_t result = open_payload_map(
        frame->payload, frame->payload_length, &parser, &contents, &count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0; i < count; ++i) {
        uint64_t key = 0U;
        result = read_key(&contents, &key, &previous, &has_previous);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        uint64_t value = 0U;
        if (key == 0U || key == 1U) {
            result = read_unsigned(&contents, &value);
            if (result == PROTOCOL_MESSAGE_OK && value > UINT32_MAX) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            if (key == 0U) {
                if (value > UINT8_MAX) {
                    result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
                }
                ack->acknowledged_type = (uint8_t)value;
                present |= REQUIRED_BIT(0);
            } else {
                ack->has_revision = true;
                ack->revision = (uint32_t)value;
            }
        } else if (key == 2U) {
            result = read_boolean(&contents, &ack->already_present);
            ack->has_already_present = true;
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    if ((present & REQUIRED_BIT(0)) == 0U) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    bool revision_required =
        ack->acknowledged_type == PROTOCOL_TYPE_PUSH_DATA ||
        ack->acknowledged_type == PROTOCOL_TYPE_APPLY_CONFIG;
    bool already_present_required =
        ack->acknowledged_type == PROTOCOL_TYPE_ASSET_BEGIN;
    if (ack->acknowledged_type != PROTOCOL_TYPE_TIME_SYNC &&
        ack->acknowledged_type != PROTOCOL_TYPE_PUSH_DATA &&
        ack->acknowledged_type != PROTOCOL_TYPE_APPLY_CONFIG &&
        ack->acknowledged_type != PROTOCOL_TYPE_ACTIVATE_SCREEN &&
        ack->acknowledged_type != PROTOCOL_TYPE_TRIGGER_INTERRUPT &&
        ack->acknowledged_type != PROTOCOL_TYPE_NETWORK_CONFIG &&
        ack->acknowledged_type != PROTOCOL_TYPE_FACTORY_RESET &&
        ack->acknowledged_type != PROTOCOL_TYPE_ASSET_BEGIN &&
        ack->acknowledged_type != PROTOCOL_TYPE_ASSET_CHUNK &&
        ack->acknowledged_type != PROTOCOL_TYPE_ASSET_COMMIT &&
        ack->acknowledged_type != PROTOCOL_TYPE_ASSET_RELEASE) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    if (revision_required != ack->has_revision) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    if (ack->has_revision && ack->revision == 0U) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    if (already_present_required != ack->has_already_present) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    return PROTOCOL_MESSAGE_OK;
}

static protocol_message_result_t decode_heartbeat_ack(
    const protocol_frame_t *frame,
    protocol_heartbeat_ack_t *ack)
{
    CborParser parser;
    CborValue contents;
    size_t count = 0U;
    protocol_message_result_t result = open_payload_map(
        frame->payload, frame->payload_length, &parser, &contents, &count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    bool present = false;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0; i < count; ++i) {
        uint64_t key = 0U;
        result = read_key(&contents, &key, &previous, &has_previous);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key == 0U) {
            result = read_unsigned(&contents, &ack->uptime_ms);
            present = true;
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    return present ? PROTOCOL_MESSAGE_OK : PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
}

static protocol_message_result_t decode_error(const protocol_frame_t *frame,
                                               protocol_error_response_t *reply)
{
    CborParser parser;
    CborValue contents;
    size_t count = 0U;
    protocol_message_result_t result = open_payload_map(
        frame->payload, frame->payload_length, &parser, &contents, &count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0; i < count; ++i) {
        uint64_t key = 0U;
        result = read_key(&contents, &key, &previous, &has_previous);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key == 0U) {
            uint64_t code = 0U;
            result = read_unsigned(&contents, &code);
            if (result == PROTOCOL_MESSAGE_OK &&
                (code < PROTOCOL_ERROR_MALFORMED_FRAME ||
                 code > PROTOCOL_ERROR_WRONG_TIER)) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            reply->code = (protocol_error_code_t)code;
            present |= REQUIRED_BIT(0);
        } else if (key == 1U) {
            result = read_text(&contents, reply->diagnostic,
                               sizeof(reply->diagnostic), 0U,
                               PROTOCOL_MAX_DIAGNOSTIC_LENGTH);
            present |= REQUIRED_BIT(1);
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    uint32_t required = REQUIRED_BIT(0) | REQUIRED_BIT(1);
    return (present & required) == required ? PROTOCOL_MESSAGE_OK
                                            : PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
}

static protocol_message_result_t assign_status_unsigned(
    protocol_status_response_t *status,
    uint64_t key,
    uint64_t value)
{
    switch (key) {
    case 0:
        if (value > UINT8_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->protocol_version = (uint8_t)value;
        break;
    case 2:
        status->uptime_ms = value;
        break;
    case 3:
        if (value > UINT32_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->free_heap = (uint32_t)value;
        break;
    case 4:
        if (value > UINT16_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->display_width = (uint16_t)value;
        break;
    case 5:
        if (value > UINT16_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->display_height = (uint16_t)value;
        break;
    case 6:
        if (value > UINT8_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->brightness = (uint8_t)value;
        break;
    case 7:
        if (value > UINT16_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->rotation = (uint16_t)value;
        break;
    case 8:
        if (value > 1U) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->online = value == 1U;
        break;
    case 9:
        if (value > UINT32_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->latest_revision = (uint32_t)value;
        break;
    case 10:
        if (value > UINT32_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->valid_frames = (uint32_t)value;
        break;
    case 11:
        if (value > UINT32_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->malformed_frames = (uint32_t)value;
        break;
    case 12:
        if (value > UINT32_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->crc_errors = (uint32_t)value;
        break;
    case 13:
        if (value > UINT32_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->overflow_frames = (uint32_t)value;
        break;
    case 14:
        if (value > UINT32_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->dropped_responses = (uint32_t)value;
        break;
    case 15:
        if (value > UINT32_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->rx_dropped_bytes = (uint32_t)value;
        break;
    case 16:
        if (value > UINT32_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->dropped_events = (uint32_t)value;
        break;
    case 17:
        if (value > UINT32_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->event_queue_high_water = (uint32_t)value;
        break;
    case 18:
        if (value > UINT32_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->dropped_ui_commands = (uint32_t)value;
        break;
    case 19:
        if (value > UINT32_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->ui_queue_high_water = (uint32_t)value;
        break;
    case 20:
        if (value > UINT32_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->config_revision = (uint32_t)value;
        break;
    case 21:
        if (value > UINT32_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->latest_interrupt_token = (uint32_t)value;
        break;
    case 22:
        if (value > UINT8_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        status->max_protocol_version = (uint8_t)value;
        break;
    case 23:
        status->capabilities = value;
        break;
    case 24:
        if (value > (uint64_t)PROTOCOL_TIER_NETWORKED) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        status->tier = (protocol_tier_t)value;
        break;
    case 25:
        if (value > (uint64_t)PROTOCOL_WIFI_FAILED) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        status->wifi_state = (protocol_wifi_state_t)value;
        break;
    case 28:
        if (value > (uint64_t)PROTOCOL_OTA_FAILED) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        status->ota_state = (protocol_ota_state_t)value;
        break;
    default:
        return PROTOCOL_MESSAGE_ERR_ARGUMENT;
    }
    return PROTOCOL_MESSAGE_OK;
}

static protocol_message_result_t decode_status(
    const protocol_frame_t *frame,
    protocol_status_response_t *status)
{
    status->tier = PROTOCOL_TIER_LOCAL;
    status->wifi_state = PROTOCOL_WIFI_DOWN;
    status->wifi_rssi = 0;
    status->ip[0] = '\0';
    status->ota_state = PROTOCOL_OTA_IDLE;
    status->has_last_network_error = false;
    status->last_network_error[0] = '\0';
    status->has_last_ota_error = false;
    status->last_ota_error[0] = '\0';
    CborParser parser;
    CborValue contents;
    size_t count = 0U;
    protocol_message_result_t result = open_payload_map(
        frame->payload, frame->payload_length, &parser, &contents, &count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0; i < count; ++i) {
        uint64_t key = 0U;
        result = read_key(&contents, &key, &previous, &has_previous);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key == 1U) {
            result = read_text(&contents, status->firmware_version,
                               sizeof(status->firmware_version), 1U,
                               PROTOCOL_MAX_FIRMWARE_VERSION_LENGTH);
            present |= REQUIRED_BIT(1);
        } else if (key == 26U) {
            int64_t rssi = 0;
            result = read_signed(&contents, &rssi);
            if (result == PROTOCOL_MESSAGE_OK &&
                (rssi < INT8_MIN || rssi > INT8_MAX)) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                status->wifi_rssi = (int8_t)rssi;
            }
            present |= REQUIRED_BIT(26);
        } else if (key == 27U) {
            result = read_text(&contents, status->ip, sizeof(status->ip), 0U,
                               PROTOCOL_MAX_IP_LENGTH);
            present |= REQUIRED_BIT(27);
        } else if (key == 29U) {
            result = read_text(&contents, status->last_network_error,
                               sizeof(status->last_network_error), 0U,
                               PROTOCOL_MAX_DIAGNOSTIC_LENGTH);
            if (result == PROTOCOL_MESSAGE_OK) {
                status->has_last_network_error = true;
            }
            present |= REQUIRED_BIT(29);
        } else if (key == 30U) {
            result = read_text(&contents, status->last_ota_error,
                               sizeof(status->last_ota_error), 0U,
                               PROTOCOL_MAX_DIAGNOSTIC_LENGTH);
            if (result == PROTOCOL_MESSAGE_OK) {
                status->has_last_ota_error = true;
            }
            present |= REQUIRED_BIT(30);
        } else if (key <= 25U || key == 28U) {
            uint64_t value = 0U;
            result = read_unsigned(&contents, &value);
            if (result == PROTOCOL_MESSAGE_OK) {
                result = assign_status_unsigned(status, key, value);
            }
            present |= REQUIRED_BIT((uint32_t)key);
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    if ((present & UINT32_C(0xffff)) != UINT32_C(0xffff)) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    if ((present & REQUIRED_BIT(22)) == 0U) {
        status->max_protocol_version = status->protocol_version;
    }
    if ((present & REQUIRED_BIT(23)) == 0U) {
        status->capabilities = PROTOCOL_LEGACY_CAPABILITIES;
    }
    if (status->protocol_version != PROTOCOL_VERSION ||
        status->max_protocol_version < status->protocol_version ||
        (status->rotation != 0U && status->rotation != 90U &&
         status->rotation != 180U && status->rotation != 270U)) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    return PROTOCOL_MESSAGE_OK;
}

protocol_message_result_t protocol_message_decode(
    const protocol_frame_t *frame,
    protocol_message_t *message)
{
    if (frame == NULL || message == NULL) {
        return PROTOCOL_MESSAGE_ERR_ARGUMENT;
    }
    memset(message, 0, sizeof(*message));
    if (frame->version != PROTOCOL_VERSION) {
        return PROTOCOL_MESSAGE_ERR_VERSION;
    }
    if (frame->flags != 0U) {
        return PROTOCOL_MESSAGE_ERR_FRAME;
    }
    if ((frame->request_id == 0U) !=
        (frame->message_type == PROTOCOL_TYPE_DEVICE_EVENT)) {
        return PROTOCOL_MESSAGE_ERR_INVALID_REQUEST_ID;
    }
    message->type = frame->message_type;
    switch (frame->message_type) {
    case PROTOCOL_TYPE_STATUS_REQUEST:
    case PROTOCOL_TYPE_HEARTBEAT:
        return require_empty_map(frame);
    case PROTOCOL_TYPE_STATUS_RESPONSE:
        return decode_status(frame, &message->value.status);
    case PROTOCOL_TYPE_TIME_SYNC:
        return decode_time_sync(frame, &message->value.time_sync);
    case PROTOCOL_TYPE_ACK:
        return decode_ack(frame, &message->value.ack);
    case PROTOCOL_TYPE_PUSH_DATA:
        return decode_push_data(frame, &message->value.push_data);
    case PROTOCOL_TYPE_HEARTBEAT_ACK:
        return decode_heartbeat_ack(frame, &message->value.heartbeat_ack);
    case PROTOCOL_TYPE_ERROR:
        return decode_error(frame, &message->value.error);
    case PROTOCOL_TYPE_APPLY_CONFIG:
        return decode_apply_config(frame, &message->value.apply_config);
    case PROTOCOL_TYPE_ACTIVATE_SCREEN:
        return decode_activate_screen(frame, &message->value.activate_screen);
    case PROTOCOL_TYPE_TRIGGER_INTERRUPT:
        return decode_trigger_interrupt(frame,
                                        &message->value.trigger_interrupt);
    case PROTOCOL_TYPE_DEVICE_EVENT:
        return decode_device_event(frame, &message->value.device_event);
    case PROTOCOL_TYPE_NETWORK_CONFIG:
        return decode_network_config(frame, &message->value.network_config);
    case PROTOCOL_TYPE_FACTORY_RESET:
        return require_empty_map(frame);
    case PROTOCOL_TYPE_ASSET_BEGIN:
        return decode_asset_begin(frame, &message->value.asset_begin);
    case PROTOCOL_TYPE_ASSET_CHUNK:
        return decode_asset_chunk(frame, &message->value.asset_chunk);
    case PROTOCOL_TYPE_ASSET_COMMIT:
        return decode_asset_commit(frame, &message->value.asset_commit);
    case PROTOCOL_TYPE_ASSET_RELEASE:
        return decode_asset_release(frame, &message->value.asset_release);
    default:
        return PROTOCOL_MESSAGE_ERR_UNSUPPORTED_TYPE;
    }
}

static protocol_message_result_t validate_message(
    const protocol_message_t *message)
{
    size_t length = 0U;
    switch (message->type) {
    case PROTOCOL_TYPE_STATUS_REQUEST:
    case PROTOCOL_TYPE_HEARTBEAT:
    case PROTOCOL_TYPE_HEARTBEAT_ACK:
        return PROTOCOL_MESSAGE_OK;
    case PROTOCOL_TYPE_STATUS_RESPONSE:
        if (!bounded_length(message->value.status.firmware_version,
                            sizeof(message->value.status.firmware_version),
                            &length) ||
            length == 0U ||
            message->value.status.protocol_version != PROTOCOL_VERSION ||
            message->value.status.max_protocol_version <
                message->value.status.protocol_version ||
            (message->value.status.rotation != 0U &&
             message->value.status.rotation != 90U &&
             message->value.status.rotation != 180U &&
             message->value.status.rotation != 270U)) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        if (!bounded_length(message->value.status.ip,
                            sizeof(message->value.status.ip), &length) ||
            length > PROTOCOL_MAX_IP_LENGTH) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        if (message->value.status.has_last_network_error &&
            !bounded_length(message->value.status.last_network_error,
                            sizeof(message->value.status.last_network_error),
                            &length)) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        if (message->value.status.has_last_ota_error &&
            !bounded_length(message->value.status.last_ota_error,
                            sizeof(message->value.status.last_ota_error),
                            &length)) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        if (message->value.status.tier != PROTOCOL_TIER_LOCAL &&
            message->value.status.tier != PROTOCOL_TIER_NETWORKED) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        if (message->value.status.wifi_state < PROTOCOL_WIFI_DOWN ||
            message->value.status.wifi_state > PROTOCOL_WIFI_FAILED) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        if (message->value.status.ota_state < PROTOCOL_OTA_IDLE ||
            message->value.status.ota_state > PROTOCOL_OTA_FAILED) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        return PROTOCOL_MESSAGE_OK;
    case PROTOCOL_TYPE_TIME_SYNC:
        if (message->value.time_sync.unix_seconds <
                PROTOCOL_MIN_UNIX_SECONDS ||
            message->value.time_sync.unix_seconds >
                PROTOCOL_MAX_UNIX_SECONDS ||
            message->value.time_sync.utc_offset_minutes <
                PROTOCOL_MIN_UTC_OFFSET_MINUTES ||
            message->value.time_sync.utc_offset_minutes >
                PROTOCOL_MAX_UTC_OFFSET_MINUTES) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        return PROTOCOL_MESSAGE_OK;
    case PROTOCOL_TYPE_ACK: {
        const protocol_ack_t *ack = &message->value.ack;
        bool acknowledged_type_valid =
            ack->acknowledged_type == PROTOCOL_TYPE_TIME_SYNC ||
            ack->acknowledged_type == PROTOCOL_TYPE_PUSH_DATA ||
            ack->acknowledged_type == PROTOCOL_TYPE_APPLY_CONFIG ||
            ack->acknowledged_type == PROTOCOL_TYPE_ACTIVATE_SCREEN ||
            ack->acknowledged_type == PROTOCOL_TYPE_TRIGGER_INTERRUPT ||
            ack->acknowledged_type == PROTOCOL_TYPE_NETWORK_CONFIG ||
            ack->acknowledged_type == PROTOCOL_TYPE_FACTORY_RESET ||
            ack->acknowledged_type == PROTOCOL_TYPE_ASSET_BEGIN ||
            ack->acknowledged_type == PROTOCOL_TYPE_ASSET_CHUNK ||
            ack->acknowledged_type == PROTOCOL_TYPE_ASSET_COMMIT ||
            ack->acknowledged_type == PROTOCOL_TYPE_ASSET_RELEASE;
        bool revision_required =
            ack->acknowledged_type == PROTOCOL_TYPE_PUSH_DATA ||
            ack->acknowledged_type == PROTOCOL_TYPE_APPLY_CONFIG;
        bool already_present_required =
            ack->acknowledged_type == PROTOCOL_TYPE_ASSET_BEGIN;
        if (!acknowledged_type_valid ||
            revision_required != ack->has_revision ||
            (ack->has_revision && ack->revision == 0U) ||
            already_present_required != ack->has_already_present) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        return PROTOCOL_MESSAGE_OK;
    }
    case PROTOCOL_TYPE_PUSH_DATA: {
        const protocol_push_data_t *push = &message->value.push_data;
        if (!bounded_length(push->widget_id, sizeof(push->widget_id),
                            &length) ||
            length == 0U || push->revision == 0U ||
            push->field_count > PROTOCOL_MAX_FIELD_COUNT) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        for (size_t i = 0; i < push->field_count; ++i) {
            const protocol_field_t *field = &push->fields[i];
            if (!bounded_length(field->key, sizeof(field->key), &length) ||
                length == 0U) {
                return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            for (size_t j = 0; j < i; ++j) {
                if (strcmp(field->key, push->fields[j].key) == 0) {
                    return PROTOCOL_MESSAGE_ERR_DUPLICATE_KEY;
                }
            }
            if (field->type == PROTOCOL_FIELD_TEXT) {
                if (!bounded_length(field->value.text,
                                    sizeof(field->value.text), &length)) {
                    return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
                }
            } else if (field->type != PROTOCOL_FIELD_INTEGER &&
                       field->type != PROTOCOL_FIELD_BOOLEAN) {
                return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
        }
        return PROTOCOL_MESSAGE_OK;
    }
    case PROTOCOL_TYPE_APPLY_CONFIG:
        return validate_apply_config(&message->value.apply_config);
    case PROTOCOL_TYPE_ACTIVATE_SCREEN:
        if (!bounded_length(message->value.activate_screen.screen_id,
                            sizeof(message->value.activate_screen.screen_id),
                            &length) ||
            length == 0U) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        return PROTOCOL_MESSAGE_OK;
    case PROTOCOL_TYPE_TRIGGER_INTERRUPT:
        if (!bounded_length(message->value.trigger_interrupt.widget_id,
                            sizeof(message->value.trigger_interrupt.widget_id),
                            &length) ||
            length == 0U || message->value.trigger_interrupt.token == 0U ||
            !bounded_length(message->value.trigger_interrupt.reason,
                            sizeof(message->value.trigger_interrupt.reason),
                            &length)) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        return PROTOCOL_MESSAGE_OK;
    case PROTOCOL_TYPE_DEVICE_EVENT:
        return validate_device_event(&message->value.device_event);
    case PROTOCOL_TYPE_ERROR:
        if (message->value.error.code < PROTOCOL_ERROR_MALFORMED_FRAME ||
            message->value.error.code > PROTOCOL_ERROR_WRONG_TIER ||
            !bounded_length(message->value.error.diagnostic,
                            sizeof(message->value.error.diagnostic),
                            &length)) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        return PROTOCOL_MESSAGE_OK;
    case PROTOCOL_TYPE_NETWORK_CONFIG:
        return validate_network_config(&message->value.network_config);
    case PROTOCOL_TYPE_FACTORY_RESET:
        return PROTOCOL_MESSAGE_OK;
    default:
        return PROTOCOL_MESSAGE_ERR_UNSUPPORTED_TYPE;
    }
}

static protocol_message_result_t encode_uint(CborEncoder *encoder,
                                              uint64_t value)
{
    return cbor_result(cbor_encode_uint(encoder, value));
}

static protocol_message_result_t encode_int(CborEncoder *encoder, int64_t value)
{
    return cbor_result(cbor_encode_int(encoder, value));
}

static protocol_message_result_t encode_text(CborEncoder *encoder,
                                              const char *value)
{
    return cbor_result(cbor_encode_text_stringz(encoder, value));
}

static protocol_message_result_t encode_bool(CborEncoder *encoder, bool value)
{
    return cbor_result(cbor_encode_boolean(encoder, value));
}

static protocol_message_result_t begin_map(CborEncoder *parent,
                                           CborEncoder *map,
                                           size_t count)
{
    return cbor_result(cbor_encoder_create_map(parent, map, count));
}

static protocol_message_result_t begin_array(CborEncoder *parent,
                                             CborEncoder *array,
                                             size_t count)
{
    return cbor_result(cbor_encoder_create_array(parent, array, count));
}

static protocol_message_result_t end_map(CborEncoder *parent,
                                         CborEncoder *map)
{
    return cbor_result(cbor_encoder_close_container_checked(parent, map));
}

static protocol_message_result_t encode_pair_uint(CborEncoder *map,
                                                   uint64_t key,
                                                   uint64_t value)
{
    protocol_message_result_t result = encode_uint(map, key);
    return result == PROTOCOL_MESSAGE_OK ? encode_uint(map, value) : result;
}

static size_t encoded_text_length(const char *text)
{
    size_t length = strlen(text);
    return length + (length <= 23U ? 1U : 2U);
}

static bool field_before(const protocol_field_t *left,
                         const protocol_field_t *right)
{
    size_t left_encoded = encoded_text_length(left->key);
    size_t right_encoded = encoded_text_length(right->key);
    if (left_encoded != right_encoded) {
        return left_encoded < right_encoded;
    }
    return strcmp(left->key, right->key) < 0;
}

static protocol_message_result_t encode_push_fields(
    CborEncoder *parent,
    const protocol_push_data_t *push)
{
    CborEncoder fields;
    protocol_message_result_t result =
        begin_map(parent, &fields, push->field_count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    size_t order[PROTOCOL_MAX_FIELD_COUNT];
    for (size_t i = 0; i < push->field_count; ++i) {
        order[i] = i;
    }
    for (size_t i = 1; i < push->field_count; ++i) {
        size_t current = order[i];
        size_t j = i;
        while (j > 0U && field_before(&push->fields[current],
                                     &push->fields[order[j - 1U]])) {
            order[j] = order[j - 1U];
            --j;
        }
        order[j] = current;
    }
    for (size_t i = 0; i < push->field_count; ++i) {
        const protocol_field_t *field = &push->fields[order[i]];
        result = encode_text(&fields, field->key);
        if (result == PROTOCOL_MESSAGE_OK) {
            if (field->type == PROTOCOL_FIELD_TEXT) {
                result = encode_text(&fields, field->value.text);
            } else if (field->type == PROTOCOL_FIELD_INTEGER) {
                result = encode_int(&fields, field->value.integer);
            } else {
                result = encode_bool(&fields, field->value.boolean);
            }
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    return end_map(parent, &fields);
}

static protocol_message_result_t encode_apply_config_payload(
    CborEncoder *root,
    const protocol_apply_config_t *config)
{
    CborEncoder map;
    CborEncoder widgets;
    protocol_message_result_t result = begin_map(root, &map, 4U);
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_pair_uint(&map, 0U, config->revision);
    }
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_uint(&map, 1U);
    }
    if (result == PROTOCOL_MESSAGE_OK) {
        result = begin_array(&map, &widgets, config->widget_count);
    }
    for (size_t i = 0U;
         result == PROTOCOL_MESSAGE_OK && i < config->widget_count; ++i) {
        const protocol_widget_config_t *widget = &config->widgets[i];
        CborEncoder item;
        result = begin_map(&widgets, &item, 5U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&item, 0U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&item, widget->widget_id);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&item, 1U, widget->template_kind);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&item, 2U, widget->size_class);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&item, 3U, widget->tap_action);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&item, 4U, widget->interrupt_policy);
        if (result == PROTOCOL_MESSAGE_OK) result = end_map(&widgets, &item);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(&map, &widgets);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
    CborEncoder screens;
    if (result == PROTOCOL_MESSAGE_OK) {
        result = begin_array(&map, &screens, config->screen_count);
    }
    for (size_t i = 0U;
         result == PROTOCOL_MESSAGE_OK && i < config->screen_count; ++i) {
        const protocol_screen_config_t *screen = &config->screens[i];
        CborEncoder item;
        result = begin_map(&screens, &item, 2U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&item, 0U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&item, screen->screen_id);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&item, 1U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&item, screen->widget_id);
        if (result == PROTOCOL_MESSAGE_OK) result = end_map(&screens, &item);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(&map, &screens);
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_pair_uint(&map, 3U, config->rotation);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(root, &map);
    return result;
}

static protocol_message_result_t encode_network_config_payload(
    CborEncoder *root,
    const protocol_network_config_t *config)
{
    CborEncoder map;
    protocol_message_result_t result = begin_map(root, &map, 7U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 1U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, config->ssid);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, config->psk);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 3U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, config->server_url);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 4U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, config->device_id);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 5U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, config->token);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 6U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, config->utc_offset_minutes);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 7U, config->tier);
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(root, &map);
    return result;
}

static protocol_message_result_t encode_status_payload(
    CborEncoder *root,
    const protocol_status_response_t *status)
{
    CborEncoder map;
    size_t entry_count = 24U + 5U +
                         (status->has_last_network_error ? 1U : 0U) +
                         (status->has_last_ota_error ? 1U : 0U);
    protocol_message_result_t result = begin_map(root, &map, entry_count);
    if (result != PROTOCOL_MESSAGE_OK) return result;
    result = encode_pair_uint(&map, 0U, status->protocol_version);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 1U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, status->firmware_version);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 2U, status->uptime_ms);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 3U, status->free_heap);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 4U, status->display_width);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 5U, status->display_height);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 6U, status->brightness);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 7U, status->rotation);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 8U, status->online ? 1U : 0U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 9U, status->latest_revision);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 10U, status->valid_frames);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 11U, status->malformed_frames);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 12U, status->crc_errors);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 13U, status->overflow_frames);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 14U, status->dropped_responses);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 15U, status->rx_dropped_bytes);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 16U, status->dropped_events);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 17U, status->event_queue_high_water);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 18U, status->dropped_ui_commands);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 19U, status->ui_queue_high_water);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 20U, status->config_revision);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 21U, status->latest_interrupt_token);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 22U, status->max_protocol_version);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 23U, status->capabilities);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 24U, status->tier);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 25U, status->wifi_state);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 26U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, status->wifi_rssi);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 27U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, status->ip);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 28U, status->ota_state);
    if (result == PROTOCOL_MESSAGE_OK && status->has_last_network_error) {
        result = encode_uint(&map, 29U);
        if (result == PROTOCOL_MESSAGE_OK) {
            result = encode_text(&map, status->last_network_error);
        }
    }
    if (result == PROTOCOL_MESSAGE_OK && status->has_last_ota_error) {
        result = encode_uint(&map, 30U);
        if (result == PROTOCOL_MESSAGE_OK) {
            result = encode_text(&map, status->last_ota_error);
        }
    }
    if (result != PROTOCOL_MESSAGE_OK) return result;
    return end_map(root, &map);
}

static protocol_message_result_t encode_payload(
    const protocol_message_t *message,
    uint8_t *payload,
    size_t *payload_length)
{
    CborEncoder root;
    CborEncoder map;
    cbor_encoder_init(&root, payload, PROTOCOL_MAX_PAYLOAD_SIZE, 0);
    protocol_message_result_t result = PROTOCOL_MESSAGE_OK;
    switch (message->type) {
    case PROTOCOL_TYPE_STATUS_REQUEST:
    case PROTOCOL_TYPE_HEARTBEAT:
    case PROTOCOL_TYPE_FACTORY_RESET:
        result = begin_map(&root, &map, 0U);
        if (result == PROTOCOL_MESSAGE_OK) result = end_map(&root, &map);
        break;
    case PROTOCOL_TYPE_STATUS_RESPONSE:
        result = encode_status_payload(&root, &message->value.status);
        break;
    case PROTOCOL_TYPE_NETWORK_CONFIG:
        result = encode_network_config_payload(&root,
                                               &message->value.network_config);
        break;
    case PROTOCOL_TYPE_TIME_SYNC:
        result = begin_map(&root, &map, 2U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, message->value.time_sync.unix_seconds);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 1U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, message->value.time_sync.utc_offset_minutes);
        if (result == PROTOCOL_MESSAGE_OK) result = end_map(&root, &map);
        break;
    case PROTOCOL_TYPE_ACK: {
        size_t ack_field_count = 1U;
        if (message->value.ack.has_revision) ++ack_field_count;
        if (message->value.ack.has_already_present) ++ack_field_count;
        result = begin_map(&root, &map, ack_field_count);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 0U, message->value.ack.acknowledged_type);
        if (result == PROTOCOL_MESSAGE_OK && message->value.ack.has_revision) result = encode_pair_uint(&map, 1U, message->value.ack.revision);
        if (result == PROTOCOL_MESSAGE_OK && message->value.ack.has_already_present) {
            result = encode_uint(&map, 2U);
            if (result == PROTOCOL_MESSAGE_OK) result = encode_bool(&map, message->value.ack.already_present);
        }
        if (result == PROTOCOL_MESSAGE_OK) result = end_map(&root, &map);
        break;
    }
    case PROTOCOL_TYPE_PUSH_DATA:
        result = begin_map(&root, &map, 3U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, message->value.push_data.widget_id);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 1U, message->value.push_data.revision);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_push_fields(&map, &message->value.push_data);
        if (result == PROTOCOL_MESSAGE_OK) result = end_map(&root, &map);
        break;
    case PROTOCOL_TYPE_HEARTBEAT_ACK:
        result = begin_map(&root, &map, 1U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 0U, message->value.heartbeat_ack.uptime_ms);
        if (result == PROTOCOL_MESSAGE_OK) result = end_map(&root, &map);
        break;
    case PROTOCOL_TYPE_APPLY_CONFIG:
        result = encode_apply_config_payload(&root,
                                             &message->value.apply_config);
        break;
    case PROTOCOL_TYPE_ACTIVATE_SCREEN:
        result = begin_map(&root, &map, 1U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, message->value.activate_screen.screen_id);
        if (result == PROTOCOL_MESSAGE_OK) result = end_map(&root, &map);
        break;
    case PROTOCOL_TYPE_TRIGGER_INTERRUPT:
        result = begin_map(&root, &map, 3U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, message->value.trigger_interrupt.widget_id);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 1U, message->value.trigger_interrupt.token);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, message->value.trigger_interrupt.reason);
        if (result == PROTOCOL_MESSAGE_OK) result = end_map(&root, &map);
        break;
    case PROTOCOL_TYPE_DEVICE_EVENT:
        result = begin_map(&root, &map,
                           message->value.device_event.has_interrupt_token
                               ? 6U
                               : 5U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 0U, message->value.device_event.sequence);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 1U, message->value.device_event.kind);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, message->value.device_event.widget_id);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 3U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, message->value.device_event.screen_id);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 4U, message->value.device_event.action);
        if (result == PROTOCOL_MESSAGE_OK && message->value.device_event.has_interrupt_token) {
            result = encode_pair_uint(&map, 5U, message->value.device_event.interrupt_token);
        }
        if (result == PROTOCOL_MESSAGE_OK) result = end_map(&root, &map);
        break;
    case PROTOCOL_TYPE_ERROR:
        result = begin_map(&root, &map, 2U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 0U, message->value.error.code);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 1U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, message->value.error.diagnostic);
        if (result == PROTOCOL_MESSAGE_OK) result = end_map(&root, &map);
        break;
    default:
        return PROTOCOL_MESSAGE_ERR_UNSUPPORTED_TYPE;
    }
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    if (cbor_encoder_get_extra_bytes_needed(&root) != 0U) {
        return PROTOCOL_MESSAGE_ERR_TOO_LARGE;
    }
    *payload_length = cbor_encoder_get_buffer_size(&root, payload);
    return PROTOCOL_MESSAGE_OK;
}

protocol_message_result_t protocol_message_encode(
    uint32_t request_id,
    const protocol_message_t *message,
    uint8_t *wire,
    size_t wire_capacity,
    size_t *wire_length)
{
    if (message == NULL || wire == NULL || wire_length == NULL) {
        return PROTOCOL_MESSAGE_ERR_ARGUMENT;
    }
    if ((request_id == 0U) !=
        (message->type == PROTOCOL_TYPE_DEVICE_EVENT)) {
        return PROTOCOL_MESSAGE_ERR_INVALID_REQUEST_ID;
    }
    protocol_message_result_t result = validate_message(message);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    protocol_frame_t frame = {
        .version = PROTOCOL_VERSION,
        .message_type = message->type,
        .flags = 0U,
        .request_id = request_id,
    };
    size_t payload_length = 0U;
    result = encode_payload(message, frame.payload, &payload_length);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    frame.payload_length = (uint16_t)payload_length;
    protocol_frame_result_t frame_result = protocol_frame_encode(
        &frame, wire, wire_capacity, wire_length);
    return frame_result == PROTOCOL_FRAME_OK ? PROTOCOL_MESSAGE_OK
                                             : PROTOCOL_MESSAGE_ERR_FRAME;
}
