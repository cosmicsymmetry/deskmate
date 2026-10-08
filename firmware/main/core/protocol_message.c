#include "protocol_message.h"

#include <string.h>
#include <stdlib.h>

#include "cbor.h"
#include "core/apply_config_validation.h"
#include "core/scene_decode.h"

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

/* The capability bit docs/protocol/v1.md gates `type` on, or 0 for a request
 * every conforming device answers. One place, so a new gated message cannot
 * be added to the dispatch table without deciding which bit it rides on. */
static uint64_t request_capability(protocol_message_type_t type)
{
    switch (type) {
    case PROTOCOL_TYPE_NETWORK_CONFIG:
    case PROTOCOL_TYPE_FACTORY_RESET:
        return PROTOCOL_CAPABILITY_NETWORKING;
    case PROTOCOL_TYPE_ASSET_BEGIN:
    case PROTOCOL_TYPE_ASSET_CHUNK:
    case PROTOCOL_TYPE_ASSET_COMMIT:
    case PROTOCOL_TYPE_ASSET_RELEASE:
        return PROTOCOL_CAPABILITY_ASSET_TRANSFER;
    case PROTOCOL_TYPE_PUSH_SCENE:
        return PROTOCOL_CAPABILITY_SCENE_RENDER;
    default:
        return 0U;
    }
}

protocol_request_gate_t protocol_message_request_gate(
    protocol_message_type_t type,
    uint64_t capabilities)
{
    switch (type) {
    case PROTOCOL_TYPE_STATUS_REQUEST:
    case PROTOCOL_TYPE_TIME_SYNC:
    case PROTOCOL_TYPE_PUSH_TIMER:
    case PROTOCOL_TYPE_HEARTBEAT:
    case PROTOCOL_TYPE_APPLY_CONFIG:
    case PROTOCOL_TYPE_ACTIVATE_CARD:
    case PROTOCOL_TYPE_TRIGGER_INTERRUPT:
    case PROTOCOL_TYPE_NETWORK_CONFIG:
    case PROTOCOL_TYPE_FACTORY_RESET:
    case PROTOCOL_TYPE_ASSET_BEGIN:
    case PROTOCOL_TYPE_ASSET_CHUNK:
    case PROTOCOL_TYPE_ASSET_COMMIT:
    case PROTOCOL_TYPE_ASSET_RELEASE:
    case PROTOCOL_TYPE_PUSH_SCENE:
        break;
    default:
        return PROTOCOL_REQUEST_NOT_A_REQUEST;
    }
    uint64_t required = request_capability(type);
    if (required != 0U && (capabilities & required) == 0U) {
        return PROTOCOL_REQUEST_MISSING_CAPABILITY;
    }
    return PROTOCOL_REQUEST_DISPATCHABLE;
}

protocol_request_gate_t protocol_asset_begin_request_gate(
    const protocol_asset_begin_t *begin,
    uint64_t capabilities)
{
    if (begin != NULL && begin->volatile_tier &&
        (capabilities & PROTOCOL_CAPABILITY_VOLATILE_ASSETS) == 0U) {
        return PROTOCOL_REQUEST_MISSING_CAPABILITY;
    }
    return PROTOCOL_REQUEST_DISPATCHABLE;
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

static protocol_message_result_t decode_push_timer(
    const protocol_frame_t *frame,
    protocol_push_timer_t *push)
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
            result = read_text(&contents, push->card_id,
                               sizeof(push->card_id), 1U,
                               PROTOCOL_MAX_CARD_ID_LENGTH);
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
        } else if (key == 2U || key == 3U) {
            uint64_t milliseconds = 0U;
            result = read_unsigned(&contents, &milliseconds);
            if (result == PROTOCOL_MESSAGE_OK && milliseconds > UINT32_MAX) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                if (key == 2U) {
                    push->total_ms = (uint32_t)milliseconds;
                    present |= REQUIRED_BIT(2);
                } else {
                    push->remaining_ms = (uint32_t)milliseconds;
                    present |= REQUIRED_BIT(3);
                }
            }
        } else if (key == 4U) {
            result = read_boolean(&contents, &push->running);
            present |= REQUIRED_BIT(4);
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    uint32_t required = REQUIRED_BIT(0) | REQUIRED_BIT(1) | REQUIRED_BIT(2) |
                        REQUIRED_BIT(3) | REQUIRED_BIT(4);
    if ((present & required) != required) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    /* A timer past its own total is not a state the host can mean, and it
     * would drive a `timer.permille` binding out of range. */
    if (push->remaining_ms > push->total_ms) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    return PROTOCOL_MESSAGE_OK;
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
    switch (apply_config_validate(config)) {
    case APPLY_CONFIG_VALID:
        return PROTOCOL_MESSAGE_OK;
    case APPLY_CONFIG_TOO_LARGE:
        return PROTOCOL_MESSAGE_ERR_CONFIG_TOO_LARGE;
    case APPLY_CONFIG_DUPLICATE_ID:
        return PROTOCOL_MESSAGE_ERR_DUPLICATE_KEY;
    case APPLY_CONFIG_INVALID_ARGUMENT:
    case APPLY_CONFIG_INVALID_VALUE:
    default:
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
}

/* AssetBegin payload keys (additive protocol-v1 table):
 *   0 digest             required 32-byte bstr
 *   1 kind               required asset kind
 *   2 total_length       required WIRE length
 *   3 volatile           optional on decode; false when absent, but always
 *                        emitted (including false) for deployed-v1 compatibility
 *   4 encoding           optional; raw (0) when absent, emitted only non-raw
 *   5 decoded_length     optional; required iff encoding is non-raw
 */
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
        } else if (key == 4U) {
            uint64_t raw = 0U;
            result = read_unsigned(&contents, &raw);
            if (result == PROTOCOL_MESSAGE_OK &&
                raw != PROTOCOL_ASSET_ENCODING_RAW &&
                raw != PROTOCOL_ASSET_ENCODING_RLE565) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                begin->encoding = (uint8_t)raw;
            }
        } else if (key == 5U) {
            uint64_t raw = 0U;
            result = read_unsigned(&contents, &raw);
            if (result == PROTOCOL_MESSAGE_OK &&
                (raw == 0U || raw > (uint64_t)ASSET_MAX_BYTES)) {
                result = PROTOCOL_MESSAGE_ERR_TOO_LARGE;
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                begin->decoded_length = (uint32_t)raw;
                begin->has_decoded_length = true;
            }
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
        if (key <= 5U) {
            present |= REQUIRED_BIT((uint32_t)key);
        }
    }
    uint32_t required =
        REQUIRED_BIT(0) | REQUIRED_BIT(1) | REQUIRED_BIT(2);
    if ((present & required) != required) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    if (begin->encoding == PROTOCOL_ASSET_ENCODING_RAW) {
        if (begin->has_decoded_length) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        return PROTOCOL_MESSAGE_OK;
    }
    if (!begin->has_decoded_length) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    /* The tier is no longer part of this rule: bit 10 means the durable tier
     * decodes too. A VOLATILE image is still pinned to the full canvas, because
     * it lands in a fixed-size PSRAM slot; a durable image is an ordinary
     * stored asset of any bounded size. */
    if (begin->volatile_tier && begin->kind == ASSET_KIND_IMAGE &&
        begin->decoded_length != PROTOCOL_VOLATILE_IMAGE_DECODED_LENGTH) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    if (begin->total_length >= begin->decoded_length) {
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
    return PROTOCOL_MESSAGE_OK;
}

/* Every structural scene failure is a payload the device refuses; the only
 * distinction worth carrying into the protocol's own vocabulary is "too
 * many nodes", which is a capacity answer a host can act on by splitting
 * the scene rather than by fixing its encoder. scene_model_result_t
 * deliberately has no CBOR-specific code (see scene_model.h), so the rest
 * cannot be told apart here either. */
static protocol_message_result_t scene_result(scene_model_result_t status)
{
    switch (status) {
    case SCENE_MODEL_OK:
        return PROTOCOL_MESSAGE_OK;
    case SCENE_MODEL_ERR_NODE_COUNT:
        return PROTOCOL_MESSAGE_ERR_TOO_LARGE;
    default:
        return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    }
}

static protocol_message_result_t decode_push_scene(
    const protocol_frame_t *frame,
    protocol_push_scene_t *push)
{
    CborParser parser;
    CborValue contents;
    size_t count = 0U;
    /* open_payload_map() validates the WHOLE payload at the root --
     * canonical form, unique keys, UTF-8, complete data, no undefined, no
     * tags -- so the scene nested under key 2 inherits all of it. That is
     * what lets scene_decode_map() read a sub-map without re-validating;
     * see core/scene_decode.h. */
    protocol_message_result_t result = open_payload_map(
        frame->payload, frame->payload_length, &parser, &contents, &count);
    if (result != PROTOCOL_MESSAGE_OK) {
        return result;
    }
    CborValue primary;
    push->tap_wrap = true;
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
            result = read_text(&contents, push->card_id,
                               sizeof(push->card_id), 1U,
                               PROTOCOL_MAX_CARD_ID_LENGTH);
        } else if (key == 1U) {
            uint64_t raw = 0U;
            result = read_unsigned(&contents, &raw);
            if (result == PROTOCOL_MESSAGE_OK &&
                (raw == 0U || raw > UINT32_MAX)) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                push->revision = (uint32_t)raw;
            }
        } else if (key == 2U) {
            primary = contents;
            result = skip_value(&contents);
        } else if (key == 3U) {
            size_t views = 0U;
            CborValue array;
            if (!cbor_value_is_array(&contents) ||
                cbor_value_get_array_length(&contents, &views) != CborNoError ||
                views == 0U || views > PROTOCOL_MAX_TAP_VIEWS ||
                cbor_value_enter_container(&contents, &array) != CborNoError) {
                return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            push->tap_view_count = (uint8_t)views;
            for (size_t j = 0U; j < views; ++j) {
                push->tap_views[j] = cbor_value_get_next_byte(&array);
                result = scene_result(scene_decode_map(&array, &push->scene));
                if (result != PROTOCOL_MESSAGE_OK) return result;
                push->tap_view_lengths[j] = (size_t)(cbor_value_get_next_byte(&array) - push->tap_views[j]);
            }
            if (cbor_value_leave_container(&contents, &array) != CborNoError) {
                return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
        } else if (key == 4U) {
            if (push->tap_view_count == 0U) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            result = read_boolean(&contents, &push->tap_wrap);
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
    return scene_result(scene_decode_map(&primary, &push->scene));
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

static protocol_message_result_t decode_card(CborValue *value,
                                             protocol_card_config_t *card)
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
            result = read_text(&fields, card->card_id,
                               sizeof(card->card_id), 1U,
                               PROTOCOL_MAX_CARD_ID_LENGTH);
            present |= REQUIRED_BIT(0);
        } else if (key == 1U) {
            uint64_t raw = 0U;
            result = read_unsigned(&fields, &raw);
            if (result == PROTOCOL_MESSAGE_OK) {
                if (raw > PROTOCOL_TAP_RESET) {
                    result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
                } else {
                    card->tap_action = (protocol_tap_action_t)raw;
                    present |= REQUIRED_BIT(1);
                }
            }
        } else {
            result = skip_value(&fields);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    error = cbor_value_leave_container(value, &fields);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    uint32_t required = REQUIRED_BIT(0) | REQUIRED_BIT(1);
    return (present & required) == required ? PROTOCOL_MESSAGE_OK
                                            : PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
}

static protocol_message_result_t decode_cards(CborValue *value,
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
    if (count > PROTOCOL_MAX_CONFIG_CARDS) {
        return PROTOCOL_MESSAGE_ERR_CONFIG_TOO_LARGE;
    }
    CborValue items;
    error = cbor_value_enter_container(value, &items);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    for (size_t i = 0U; i < count; ++i) {
        protocol_message_result_t result = decode_card(&items, &config->cards[i]);
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    error = cbor_value_leave_container(value, &items);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    config->card_count = count;
    return PROTOCOL_MESSAGE_OK;
}

static protocol_message_result_t decode_apply_config(
    const protocol_frame_t *frame,
    protocol_apply_config_t *config)
{
    config->rotation = 90U;
    config->has_brightness = false;
    config->brightness = 0U;
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
            result = decode_cards(&contents, config);
            present |= REQUIRED_BIT(1);
        } else if (key == 3U) {
            uint64_t rotation = 0U;
            result = read_unsigned(&contents, &rotation);
            if (result == PROTOCOL_MESSAGE_OK &&
                rotation != 90U && rotation != 270U) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            config->rotation = (uint16_t)rotation;
        } else if (key == 4U) {
            uint64_t brightness = 0U;
            result = read_unsigned(&contents, &brightness);
            if (result == PROTOCOL_MESSAGE_OK && brightness > UINT8_MAX) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            config->brightness = (uint8_t)brightness;
            config->has_brightness = true;
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    if ((present & UINT32_C(0x03)) != UINT32_C(0x03)) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    return validate_apply_config(config);
}

static protocol_message_result_t decode_activate_card(
    const protocol_frame_t *frame,
    protocol_activate_card_t *activate)
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
            result = read_text(&contents, activate->card_id,
                               sizeof(activate->card_id), 1U,
                               PROTOCOL_MAX_CARD_ID_LENGTH);
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
            result = read_text(&contents, interrupt->card_id,
                               sizeof(interrupt->card_id), 1U,
                               PROTOCOL_MAX_CARD_ID_LENGTH);
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
    if ((event->has_view_index && event->kind != PROTOCOL_EVENT_TAP) ||
        event->sequence == 0U ||
        !bounded_length(event->card_id, sizeof(event->card_id), &length) ||
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
        } else if (key == 1U || key == 4U || key == 5U || key == 6U) {
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
            } else if (result == PROTOCOL_MESSAGE_OK && key == 6U) {
                if (raw > UINT8_MAX) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
                event->has_view_index = true;
                event->view_index = (uint8_t)raw;
            }
            present |= REQUIRED_BIT((uint32_t)key);
        } else if (key == 2U) {
            result = read_text(&contents, event->card_id,
                               sizeof(event->card_id), 1U,
                               PROTOCOL_MAX_CARD_ID_LENGTH);
            present |= REQUIRED_BIT(2);
        } else {
            result = skip_value(&contents);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    /* Keys 0, 1, 2 and 4 are required; key 3 was retired with v1's second
     * identifier and key 5 is the optional interrupt token. */
    if ((present & UINT32_C(0x17)) != UINT32_C(0x17)) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    return validate_device_event(event);
}

static protocol_message_result_t validate_ack_payload(
    const protocol_ack_t *ack)
{
    bool acknowledged_type_valid =
        ack->acknowledged_type == PROTOCOL_TYPE_TIME_SYNC ||
        ack->acknowledged_type == PROTOCOL_TYPE_PUSH_TIMER ||
        ack->acknowledged_type == PROTOCOL_TYPE_APPLY_CONFIG ||
        ack->acknowledged_type == PROTOCOL_TYPE_ACTIVATE_CARD ||
        ack->acknowledged_type == PROTOCOL_TYPE_TRIGGER_INTERRUPT ||
        ack->acknowledged_type == PROTOCOL_TYPE_NETWORK_CONFIG ||
        ack->acknowledged_type == PROTOCOL_TYPE_FACTORY_RESET ||
        ack->acknowledged_type == PROTOCOL_TYPE_ASSET_BEGIN ||
        ack->acknowledged_type == PROTOCOL_TYPE_ASSET_CHUNK ||
        ack->acknowledged_type == PROTOCOL_TYPE_ASSET_COMMIT ||
        ack->acknowledged_type == PROTOCOL_TYPE_ASSET_RELEASE ||
        ack->acknowledged_type == PROTOCOL_TYPE_PUSH_SCENE;
    bool revision_required =
        ack->acknowledged_type == PROTOCOL_TYPE_PUSH_TIMER ||
        ack->acknowledged_type == PROTOCOL_TYPE_APPLY_CONFIG ||
        ack->acknowledged_type == PROTOCOL_TYPE_PUSH_SCENE;
    bool already_present_required =
        ack->acknowledged_type == PROTOCOL_TYPE_ASSET_BEGIN;
    return acknowledged_type_valid &&
                   revision_required == ack->has_revision &&
                   (!ack->has_revision || ack->revision != 0U) &&
                   already_present_required == ack->has_already_present
               ? PROTOCOL_MESSAGE_OK
               : PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
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
    return validate_ack_payload(ack);
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

static protocol_message_result_t decode_volatile_asset_stats(
    CborValue *value,
    protocol_status_response_t *status)
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
        if (key <= 4U) {
            uint64_t raw = 0U;
            result = read_unsigned(&fields, &raw);
            if (result == PROTOCOL_MESSAGE_OK && raw > UINT32_MAX) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                switch (key) {
                case 0U:
                    status->volatile_committed_count = (uint32_t)raw;
                    break;
                case 1U:
                    status->volatile_slot_capacity = (uint32_t)raw;
                    break;
                case 2U:
                    status->volatile_used_bytes = (uint32_t)raw;
                    break;
                case 3U:
                    status->psram_free_bytes = (uint32_t)raw;
                    break;
                default:
                    status->psram_low_water_bytes = (uint32_t)raw;
                    break;
                }
            }
            present |= REQUIRED_BIT((uint32_t)key);
        } else {
            result = skip_value(&fields);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    error = cbor_value_leave_container(value, &fields);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    uint32_t required = REQUIRED_BIT(0) | REQUIRED_BIT(1) | REQUIRED_BIT(2) |
                       REQUIRED_BIT(3) | REQUIRED_BIT(4);
    if ((present & required) != required) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    status->has_volatile_asset_stats = true;
    return PROTOCOL_MESSAGE_OK;
}

static protocol_message_result_t decode_asset_store_stats(
    CborValue *value,
    protocol_status_response_t *status)
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
        if (key <= 2U) {
            uint64_t raw = 0U;
            result = read_unsigned(&fields, &raw);
            if (result == PROTOCOL_MESSAGE_OK && raw > UINT32_MAX) {
                result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                if (key == 0U) {
                    status->asset_store_used_bytes = (uint32_t)raw;
                } else if (key == 1U) {
                    status->asset_store_free_bytes = (uint32_t)raw;
                } else {
                    status->asset_count = (uint32_t)raw;
                }
            }
            present |= REQUIRED_BIT((uint32_t)key);
        } else {
            result = skip_value(&fields);
        }
        if (result != PROTOCOL_MESSAGE_OK) {
            return result;
        }
    }
    error = cbor_value_leave_container(value, &fields);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    uint32_t required = REQUIRED_BIT(0) | REQUIRED_BIT(1) | REQUIRED_BIT(2);
    if ((present & required) != required) {
        return PROTOCOL_MESSAGE_ERR_MISSING_FIELD;
    }
    status->has_asset_store_stats = true;
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
    status->has_asset_store_stats = false;
    status->asset_store_used_bytes = 0U;
    status->asset_store_free_bytes = 0U;
    status->asset_count = 0U;
    status->has_volatile_asset_stats = false;
    status->volatile_committed_count = 0U;
    status->volatile_slot_capacity = 0U;
    status->volatile_used_bytes = 0U;
    status->psram_free_bytes = 0U;
    status->psram_low_water_bytes = 0U;
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
        } else if (key == 31U) {
            result = decode_asset_store_stats(&contents, status);
            present |= REQUIRED_BIT(31);
        } else if (key == 32U) {
            /* No `present` bit, and key 33 will not get one either: `present`
             * is a uint32_t and REQUIRED_BIT(32) shifts off the end of it.
             * Nothing needs one here -- the required mask is the low 16 bits,
             * and this key's own presence is carried by
             * status->has_volatile_asset_stats. */
            result = decode_volatile_asset_stats(&contents, status);
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
        /* A v2 peer always states its capabilities. */
        status->capabilities = 0U;
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
    case PROTOCOL_TYPE_PUSH_TIMER:
        return decode_push_timer(frame, &message->value.push_timer);
    case PROTOCOL_TYPE_HEARTBEAT_ACK:
        return decode_heartbeat_ack(frame, &message->value.heartbeat_ack);
    case PROTOCOL_TYPE_ERROR:
        return decode_error(frame, &message->value.error);
    case PROTOCOL_TYPE_APPLY_CONFIG:
        return decode_apply_config(frame, &message->value.apply_config);
    case PROTOCOL_TYPE_ACTIVATE_CARD:
        return decode_activate_card(frame, &message->value.activate_card);
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
    case PROTOCOL_TYPE_PUSH_SCENE:
        return decode_push_scene(frame, &message->value.push_scene);
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
    case PROTOCOL_TYPE_ACK:
        return validate_ack_payload(&message->value.ack);
    case PROTOCOL_TYPE_PUSH_TIMER: {
        const protocol_push_timer_t *push = &message->value.push_timer;
        if (!bounded_length(push->card_id, sizeof(push->card_id), &length) ||
            length == 0U || push->revision == 0U ||
            push->remaining_ms > push->total_ms) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        return PROTOCOL_MESSAGE_OK;
    }
    case PROTOCOL_TYPE_APPLY_CONFIG:
        return validate_apply_config(&message->value.apply_config);
    case PROTOCOL_TYPE_ACTIVATE_CARD:
        if (!bounded_length(message->value.activate_card.card_id,
                            sizeof(message->value.activate_card.card_id),
                            &length) ||
            length == 0U) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        return PROTOCOL_MESSAGE_OK;
    case PROTOCOL_TYPE_TRIGGER_INTERRUPT:
        if (!bounded_length(message->value.trigger_interrupt.card_id,
                            sizeof(message->value.trigger_interrupt.card_id),
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
    case PROTOCOL_TYPE_ASSET_BEGIN: {
        const protocol_asset_begin_t *begin = &message->value.asset_begin;
        if (begin->kind != ASSET_KIND_FONT &&
            begin->kind != ASSET_KIND_ICON_FONT &&
            begin->kind != ASSET_KIND_IMAGE) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        if (begin->total_length == 0U ||
            begin->total_length > (uint32_t)ASSET_MAX_BYTES) {
            return PROTOCOL_MESSAGE_ERR_TOO_LARGE;
        }
        if (begin->encoding != PROTOCOL_ASSET_ENCODING_RAW &&
            begin->encoding != PROTOCOL_ASSET_ENCODING_RLE565) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        if (begin->encoding == PROTOCOL_ASSET_ENCODING_RAW) {
            return begin->has_decoded_length
                       ? PROTOCOL_MESSAGE_ERR_INVALID_VALUE
                       : PROTOCOL_MESSAGE_OK;
        }
        /* Encoding was volatile-only until bit 10; the durable tier decodes
         * now, so the tier is no longer part of this rule. */
        if (!begin->has_decoded_length || begin->decoded_length == 0U ||
            begin->decoded_length > (uint32_t)ASSET_MAX_BYTES ||
            begin->total_length >= begin->decoded_length) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        /* A VOLATILE image is a full-canvas frame by construction: it lands in
         * a fixed-size PSRAM slot, so its decoded length is pinned. A durable
         * image is an ordinary stored asset of any bounded size, and pinning it
         * would reject every plugin image that is not a whole screen. */
        if (begin->volatile_tier && begin->kind == ASSET_KIND_IMAGE &&
            begin->decoded_length != PROTOCOL_VOLATILE_IMAGE_DECODED_LENGTH) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        return PROTOCOL_MESSAGE_OK;
    }
    case PROTOCOL_TYPE_ASSET_CHUNK:
        if (message->value.asset_chunk.data_length >
            PROTOCOL_MAX_ASSET_CHUNK_BYTES) {
            return PROTOCOL_MESSAGE_ERR_TOO_LARGE;
        }
        return PROTOCOL_MESSAGE_OK;
    case PROTOCOL_TYPE_ASSET_COMMIT:
        return PROTOCOL_MESSAGE_OK;
    case PROTOCOL_TYPE_ASSET_RELEASE:
        if (message->value.asset_release.digest_count >
            PROTOCOL_MAX_ASSET_DIGESTS) {
            return PROTOCOL_MESSAGE_ERR_TOO_LARGE;
        }
        return PROTOCOL_MESSAGE_OK;
    case PROTOCOL_TYPE_PUSH_SCENE: {
        const protocol_push_scene_t *push = &message->value.push_scene;
        if (!bounded_length(push->card_id, sizeof(push->card_id), &length) ||
            length == 0U || push->revision == 0U) {
            return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
        }
        /* The scene's own invariants belong to scene_model_validate(), and
         * are checked here rather than restated. Note it does NOT cover
         * bindings -- scene_binding_parse() is applied on the DECODE path
         * only, because a binding is untrusted input rather than a bound
         * on this struct, and pulling that parser in here would put it in
         * the encode path of every message. */
        return scene_result(scene_model_validate(&push->scene));
    }
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

static protocol_message_result_t encode_bytes(CborEncoder *encoder,
                                               const uint8_t *data,
                                               size_t length)
{
    return cbor_result(cbor_encode_byte_string(encoder, data, length));
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

static protocol_message_result_t encode_apply_config_payload(
    CborEncoder *root,
    const protocol_apply_config_t *config)
{
    CborEncoder map;
    CborEncoder cards;
    protocol_message_result_t result = begin_map(root, &map, config->has_brightness ? 4U : 3U);
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_pair_uint(&map, 0U, config->revision);
    }
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_uint(&map, 1U);
    }
    if (result == PROTOCOL_MESSAGE_OK) {
        result = begin_array(&map, &cards, config->card_count);
    }
    for (size_t i = 0U;
         result == PROTOCOL_MESSAGE_OK && i < config->card_count; ++i) {
        const protocol_card_config_t *card = &config->cards[i];
        CborEncoder item;
        result = begin_map(&cards, &item, 2U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&item, 0U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&item, card->card_id);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&item, 1U, card->tap_action);
        if (result == PROTOCOL_MESSAGE_OK) result = end_map(&cards, &item);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(&map, &cards);
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_pair_uint(&map, 3U, config->rotation);
    }
    if (result == PROTOCOL_MESSAGE_OK && config->has_brightness) {
        result = encode_pair_uint(&map, 4U, config->brightness);
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
                         (status->has_last_ota_error ? 1U : 0U) +
                         (status->has_asset_store_stats ? 1U : 0U) +
                         (status->has_volatile_asset_stats ? 1U : 0U);
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
    if (result == PROTOCOL_MESSAGE_OK && status->has_asset_store_stats) {
        result = encode_uint(&map, 31U);
        if (result == PROTOCOL_MESSAGE_OK) {
            CborEncoder stats_map;
            result = begin_map(&map, &stats_map, 3U);
            if (result == PROTOCOL_MESSAGE_OK) {
                result = encode_pair_uint(&stats_map, 0U,
                                          status->asset_store_used_bytes);
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                result = encode_pair_uint(&stats_map, 1U,
                                          status->asset_store_free_bytes);
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                result = encode_pair_uint(&stats_map, 2U, status->asset_count);
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                result = end_map(&map, &stats_map);
            }
        }
    }
    if (result == PROTOCOL_MESSAGE_OK && status->has_volatile_asset_stats) {
        result = encode_uint(&map, 32U);
        if (result == PROTOCOL_MESSAGE_OK) {
            CborEncoder pool_map;
            result = begin_map(&map, &pool_map, 5U);
            if (result == PROTOCOL_MESSAGE_OK) {
                result = encode_pair_uint(&pool_map, 0U,
                                          status->volatile_committed_count);
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                result = encode_pair_uint(&pool_map, 1U,
                                          status->volatile_slot_capacity);
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                result = encode_pair_uint(&pool_map, 2U,
                                          status->volatile_used_bytes);
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                result = encode_pair_uint(&pool_map, 3U,
                                          status->psram_free_bytes);
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                result = encode_pair_uint(&pool_map, 4U,
                                          status->psram_low_water_bytes);
            }
            if (result == PROTOCOL_MESSAGE_OK) {
                result = end_map(&map, &pool_map);
            }
        }
    }
    if (result != PROTOCOL_MESSAGE_OK) return result;
    return end_map(root, &map);
}

static protocol_message_result_t encode_asset_begin_payload(
    CborEncoder *root,
    const protocol_asset_begin_t *begin)
{
    CborEncoder map;
    /* Key 3 always emitted (see the Rust encoder's comment): deployed
     * decoders require it, so only keys 4/5 follow the optional rule. */
    size_t pair_count = 4U +
                        (begin->encoding != PROTOCOL_ASSET_ENCODING_RAW ? 1U
                                                                        : 0U) +
                        (begin->has_decoded_length ? 1U : 0U);
    protocol_message_result_t result = begin_map(root, &map, pair_count);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_bytes(&map, begin->digest, ASSET_DIGEST_BYTES);
    }
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_pair_uint(&map, 1U, (uint64_t)begin->kind);
    }
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_pair_uint(&map, 2U, begin->total_length);
    }
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_uint(&map, 3U);
        if (result == PROTOCOL_MESSAGE_OK) {
            result = encode_bool(&map, begin->volatile_tier);
        }
    }
    if (result == PROTOCOL_MESSAGE_OK &&
        begin->encoding != PROTOCOL_ASSET_ENCODING_RAW) {
        result = encode_pair_uint(&map, 4U, begin->encoding);
    }
    if (result == PROTOCOL_MESSAGE_OK && begin->has_decoded_length) {
        result = encode_pair_uint(&map, 5U, begin->decoded_length);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(root, &map);
    return result;
}

static protocol_message_result_t encode_asset_chunk_payload(
    CborEncoder *root,
    const protocol_asset_chunk_t *chunk)
{
    CborEncoder map;
    protocol_message_result_t result = begin_map(root, &map, 3U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_bytes(&map, chunk->digest, ASSET_DIGEST_BYTES);
    }
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_pair_uint(&map, 1U, chunk->offset);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_bytes(&map, chunk->data, chunk->data_length);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(root, &map);
    return result;
}

static protocol_message_result_t encode_asset_commit_payload(
    CborEncoder *root,
    const protocol_asset_commit_t *commit)
{
    CborEncoder map;
    protocol_message_result_t result = begin_map(root, &map, 1U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_bytes(&map, commit->digest, ASSET_DIGEST_BYTES);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(root, &map);
    return result;
}

static protocol_message_result_t encode_asset_release_payload(
    CborEncoder *root,
    const protocol_asset_release_t *release)
{
    CborEncoder map;
    CborEncoder digests;
    protocol_message_result_t result = begin_map(root, &map, 1U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
    if (result == PROTOCOL_MESSAGE_OK) {
        result = begin_array(&map, &digests, release->digest_count);
    }
    for (size_t i = 0U;
         result == PROTOCOL_MESSAGE_OK && i < release->digest_count; ++i) {
        result = encode_bytes(&digests, release->digests[i], ASSET_DIGEST_BYTES);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(&map, &digests);
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(root, &map);
    return result;
}

/* ------------------------------------------------------------------
 * The scene encoder.
 *
 * The device never SENDS a PushScene. This exists so the cross-language
 * fixture corpus can prove the two implementations agree byte for byte:
 * firmware/host_tests/test_protocol.c decodes protocol/fixtures/v1/
 * push_scene.bin, re-encodes it here, and memcmp's the result against the
 * file the Rust encoder wrote. Without an encoder on this side that check
 * would not exist for the largest and most nested message on the wire.
 *
 * CANONICAL EMISSION RULE, and both languages must follow it or the
 * round-trip above fails: a required key is always emitted; an OPTIONAL key
 * is emitted only when its value differs from the default the decoder would
 * have supplied. That is not tidiness -- it is what keeps a 24-node scene
 * inside the 2034-byte payload (see the wire-shape comment at the top of
 * core/scene_decode.c). The defaults are: RECT radius 0, fill 0, opacity
 * 255, clip absent; ARC color 0, rounded false, end_binding "", opacity 255,
 * running_color absent; LINE color 0; TEXT align LEFT, color 0,
 * ellipsize false, running_color absent; IMAGE recolor false, color 0; GLYPH color
 * 0; SCALE major_tick_color 0; LABEL colours/style fields 0,
 * hide_when_empty false, horizontal_anchor LEFT; ROT_RECT style/transform
 * fields 0, rotation_binding "", and clip absent; the value map's literal
 * and binding "".
 * ------------------------------------------------------------------ */

static size_t scene_rect_entries(const scene_rect_t *rect)
{
    return 4U + (rect->radius != 0 ? 1U : 0U) + (rect->fill != 0U ? 1U : 0U) +
           (rect->opacity != UINT8_MAX ? 1U : 0U) +
           (rect->has_clip ? 1U : 0U);
}

static protocol_message_result_t encode_scene_clip_rect(
    CborEncoder *parent,
    const scene_clip_rect_t *clip)
{
    CborEncoder map;
    protocol_message_result_t result = begin_map(parent, &map, 4U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, clip->x);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 1U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, clip->y);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, clip->w);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 3U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, clip->h);
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(parent, &map);
    return result;
}

static protocol_message_result_t encode_scene_rect(CborEncoder *parent,
                                                    const scene_rect_t *rect)
{
    CborEncoder map;
    protocol_message_result_t result =
        begin_map(parent, &map, scene_rect_entries(rect));
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, rect->x);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 1U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, rect->y);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, rect->w);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 3U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, rect->h);
    if (result == PROTOCOL_MESSAGE_OK && rect->radius != 0) {
        result = encode_uint(&map, 4U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, rect->radius);
    }
    if (result == PROTOCOL_MESSAGE_OK && rect->fill != 0U) {
        result = encode_pair_uint(&map, 5U, rect->fill);
    }
    if (result == PROTOCOL_MESSAGE_OK && rect->opacity != UINT8_MAX) {
        result = encode_pair_uint(&map, 6U, rect->opacity);
    }
    if (result == PROTOCOL_MESSAGE_OK && rect->has_clip) {
        result = encode_uint(&map, 7U);
        if (result == PROTOCOL_MESSAGE_OK) {
            result = encode_scene_clip_rect(&map, &rect->clip);
        }
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(parent, &map);
    return result;
}

static protocol_message_result_t encode_scene_arc(CborEncoder *parent,
                                                   const scene_arc_t *arc)
{
    CborEncoder map;
    size_t entries = 6U + (arc->color != 0U ? 1U : 0U) +
                     (arc->rounded ? 1U : 0U) +
                     (arc->end_binding[0] != '\0' ? 1U : 0U) +
                     (arc->opacity != UINT8_MAX ? 1U : 0U) +
                     (arc->has_running_color ? 1U : 0U);
    protocol_message_result_t result = begin_map(parent, &map, entries);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, arc->cx);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 1U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, arc->cy);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, arc->r);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 3U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, arc->start_deg);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 4U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, arc->end_deg);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 5U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, arc->width);
    if (result == PROTOCOL_MESSAGE_OK && arc->color != 0U) {
        result = encode_pair_uint(&map, 6U, arc->color);
    }
    if (result == PROTOCOL_MESSAGE_OK && arc->rounded) {
        result = encode_uint(&map, 7U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_bool(&map, true);
    }
    if (result == PROTOCOL_MESSAGE_OK && arc->end_binding[0] != '\0') {
        result = encode_uint(&map, 8U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, arc->end_binding);
    }
    if (result == PROTOCOL_MESSAGE_OK && arc->opacity != UINT8_MAX) {
        result = encode_pair_uint(&map, 9U, arc->opacity);
    }
    if (result == PROTOCOL_MESSAGE_OK && arc->has_running_color) {
        result = encode_pair_uint(&map, 10U, arc->running_color);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(parent, &map);
    return result;
}

static protocol_message_result_t encode_scene_points(CborEncoder *map,
                                                      uint64_t key,
                                                      const int32_t *points,
                                                      uint32_t count)
{
    protocol_message_result_t result = encode_uint(map, key);
    CborEncoder array;
    if (result == PROTOCOL_MESSAGE_OK) result = begin_array(map, &array, count);
    for (uint32_t i = 0U; result == PROTOCOL_MESSAGE_OK && i < count; ++i) {
        result = encode_int(&array, points[i]);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(map, &array);
    return result;
}

static protocol_message_result_t encode_scene_line(CborEncoder *parent,
                                                    const scene_line_t *line)
{
    CborEncoder map;
    bool bound = line->angle_binding[0] != '\0';
    protocol_message_result_t result =
        begin_map(parent, &map,
                  (bound ? 5U : 3U) + (line->color != 0U ? 1U : 0U));
    if (result == PROTOCOL_MESSAGE_OK && !bound) {
        result = encode_scene_points(&map, 0U, line->xs, line->point_count);
    }
    if (result == PROTOCOL_MESSAGE_OK && !bound) {
        result = encode_scene_points(&map, 1U, line->ys, line->point_count);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, line->width);
    if (result == PROTOCOL_MESSAGE_OK && line->color != 0U) {
        result = encode_pair_uint(&map, 3U, line->color);
    }
    if (result == PROTOCOL_MESSAGE_OK && bound) {
        result = encode_uint(&map, 4U);
    }
    if (result == PROTOCOL_MESSAGE_OK && bound) {
        result = encode_int(&map, line->pivot_x);
    }
    if (result == PROTOCOL_MESSAGE_OK && bound) {
        result = encode_uint(&map, 5U);
    }
    if (result == PROTOCOL_MESSAGE_OK && bound) {
        result = encode_int(&map, line->pivot_y);
    }
    if (result == PROTOCOL_MESSAGE_OK && bound) {
        result = encode_uint(&map, 6U);
    }
    if (result == PROTOCOL_MESSAGE_OK && bound) {
        result = encode_int(&map, line->length);
    }
    if (result == PROTOCOL_MESSAGE_OK && bound) {
        result = encode_uint(&map, 7U);
    }
    if (result == PROTOCOL_MESSAGE_OK && bound) {
        result = encode_text(&map, line->angle_binding);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(parent, &map);
    return result;
}

/* A BAKED font carries its tier and no digest; an ASSET font carries its
 * digest and pixel size and no tier. Emitting the other half either way
 * would spend 34 bytes a node on a field the decoder ignores. */
static protocol_message_result_t encode_scene_font(CborEncoder *parent,
                                                    const scene_font_ref_t *font)
{
    CborEncoder map;
    bool asset = font->kind == SCENE_FONT_ASSET;
    protocol_message_result_t result = begin_map(parent, &map, asset ? 3U : 2U);
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_pair_uint(&map, 0U, (uint64_t)font->kind);
    }
    if (result == PROTOCOL_MESSAGE_OK && !asset) {
        result = encode_pair_uint(&map, 1U, (uint64_t)font->baked);
    }
    if (result == PROTOCOL_MESSAGE_OK && asset) {
        result = encode_uint(&map, 2U);
        if (result == PROTOCOL_MESSAGE_OK) {
            result = encode_bytes(&map, font->digest, ASSET_DIGEST_BYTES);
        }
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 3U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, font->pixel_size);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(parent, &map);
    return result;
}

static protocol_message_result_t encode_scene_value(CborEncoder *parent,
                                                     const scene_value_t *value)
{
    CborEncoder map;
    size_t entries = 1U + (value->literal[0] != '\0' ? 1U : 0U) +
                     (value->binding[0] != '\0' ? 1U : 0U);
    protocol_message_result_t result = begin_map(parent, &map, entries);
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_pair_uint(&map, 0U, (uint64_t)value->kind);
    }
    if (result == PROTOCOL_MESSAGE_OK && value->literal[0] != '\0') {
        result = encode_uint(&map, 1U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, value->literal);
    }
    if (result == PROTOCOL_MESSAGE_OK && value->binding[0] != '\0') {
        result = encode_uint(&map, 2U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, value->binding);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(parent, &map);
    return result;
}

static protocol_message_result_t encode_scene_text(CborEncoder *parent,
                                                    const scene_text_t *text)
{
    CborEncoder map;
    size_t entries = 5U + (text->align != SCENE_ALIGN_LEFT ? 1U : 0U) +
                     (text->color != 0U ? 1U : 0U) +
                     (text->ellipsize ? 1U : 0U) +
                     (text->has_running_color ? 1U : 0U);
    protocol_message_result_t result = begin_map(parent, &map, entries);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, text->x);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 1U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, text->baseline_y);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, text->w);
    if (result == PROTOCOL_MESSAGE_OK && text->align != SCENE_ALIGN_LEFT) {
        result = encode_pair_uint(&map, 3U, (uint64_t)text->align);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 4U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_scene_font(&map, &text->font);
    if (result == PROTOCOL_MESSAGE_OK && text->color != 0U) {
        result = encode_pair_uint(&map, 5U, text->color);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 6U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_scene_value(&map, &text->value);
    if (result == PROTOCOL_MESSAGE_OK && text->ellipsize) {
        result = encode_uint(&map, 7U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_bool(&map, true);
    }
    if (result == PROTOCOL_MESSAGE_OK && text->has_running_color) {
        result = encode_pair_uint(&map, 8U, text->running_color);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(parent, &map);
    return result;
}

static protocol_message_result_t encode_scene_image(CborEncoder *parent,
                                                     const scene_image_t *image)
{
    CborEncoder map;
    size_t entries = 5U + (image->recolor ? 1U : 0U) +
                     (image->color != 0U ? 1U : 0U);
    protocol_message_result_t result = begin_map(parent, &map, entries);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, image->x);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 1U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, image->y);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, image->w);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 3U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, image->h);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 4U);
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_bytes(&map, image->digest, ASSET_DIGEST_BYTES);
    }
    if (result == PROTOCOL_MESSAGE_OK && image->recolor) {
        result = encode_uint(&map, 5U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_bool(&map, true);
    }
    if (result == PROTOCOL_MESSAGE_OK && image->color != 0U) {
        result = encode_pair_uint(&map, 6U, image->color);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(parent, &map);
    return result;
}

static protocol_message_result_t encode_scene_glyph(CborEncoder *parent,
                                                     const scene_glyph_t *glyph)
{
    CborEncoder map;
    protocol_message_result_t result =
        begin_map(parent, &map, 5U + (glyph->color != 0U ? 1U : 0U));
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, glyph->x);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 1U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, glyph->baseline_y);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, glyph->size);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 3U);
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_bytes(&map, glyph->digest, ASSET_DIGEST_BYTES);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 4U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, glyph->name);
    if (result == PROTOCOL_MESSAGE_OK && glyph->color != 0U) {
        result = encode_pair_uint(&map, 5U, glyph->color);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(parent, &map);
    return result;
}

static protocol_message_result_t encode_scene_scale(CborEncoder *parent,
                                                     const scene_scale_t *scale)
{
    CborEncoder map;
    protocol_message_result_t result =
        begin_map(parent, &map, 5U + (scale->major_tick_color != 0U ? 1U : 0U));
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, scale->x);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 1U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, scale->y);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, scale->box);
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_pair_uint(&map, 3U, scale->total_tick_count);
    }
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_pair_uint(&map, 4U, scale->major_tick_every);
    }
    if (result == PROTOCOL_MESSAGE_OK && scale->major_tick_color != 0U) {
        result = encode_pair_uint(&map, 5U, scale->major_tick_color);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(parent, &map);
    return result;
}

static protocol_message_result_t encode_scene_label(CborEncoder *parent,
                                                     const scene_label_t *label)
{
    CborEncoder map;
    size_t entries = 4U + (label->ink != 0U ? 1U : 0U) +
                     (label->fill != 0U ? 1U : 0U) +
                     (label->fill_opacity != 0U ? 1U : 0U) +
                     (label->radius != 0 ? 1U : 0U) +
                     (label->pad_hor != 0 ? 1U : 0U) +
                     (label->pad_ver != 0 ? 1U : 0U) +
                     (label->letter_space != 0 ? 1U : 0U) +
                     (label->hide_when_empty ? 1U : 0U) +
                     (label->horizontal_anchor != SCENE_LABEL_ANCHOR_LEFT ? 1U : 0U);
    protocol_message_result_t result = begin_map(parent, &map, entries);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, label->x);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 1U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, label->y);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_scene_font(&map, &label->font);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 3U);
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_scene_value(&map, &label->value);
    }
    if (result == PROTOCOL_MESSAGE_OK && label->ink != 0U) {
        result = encode_pair_uint(&map, 4U, label->ink);
    }
    if (result == PROTOCOL_MESSAGE_OK && label->fill != 0U) {
        result = encode_pair_uint(&map, 5U, label->fill);
    }
    if (result == PROTOCOL_MESSAGE_OK && label->fill_opacity != 0U) {
        result = encode_pair_uint(&map, 6U, label->fill_opacity);
    }
    if (result == PROTOCOL_MESSAGE_OK && label->radius != 0) {
        result = encode_uint(&map, 7U);
        if (result == PROTOCOL_MESSAGE_OK) {
            result = encode_int(&map, label->radius);
        }
    }
    if (result == PROTOCOL_MESSAGE_OK && label->pad_hor != 0) {
        result = encode_uint(&map, 8U);
        if (result == PROTOCOL_MESSAGE_OK) {
            result = encode_int(&map, label->pad_hor);
        }
    }
    if (result == PROTOCOL_MESSAGE_OK && label->pad_ver != 0) {
        result = encode_uint(&map, 9U);
        if (result == PROTOCOL_MESSAGE_OK) {
            result = encode_int(&map, label->pad_ver);
        }
    }
    if (result == PROTOCOL_MESSAGE_OK && label->letter_space != 0) {
        result = encode_uint(&map, 10U);
        if (result == PROTOCOL_MESSAGE_OK) {
            result = encode_int(&map, label->letter_space);
        }
    }
    if (result == PROTOCOL_MESSAGE_OK && label->hide_when_empty) {
        result = encode_uint(&map, 11U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_bool(&map, true);
    }
    if (result == PROTOCOL_MESSAGE_OK &&
        label->horizontal_anchor != SCENE_LABEL_ANCHOR_LEFT) {
        result = encode_pair_uint(&map, 12U,
                                  (uint64_t)label->horizontal_anchor);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(parent, &map);
    return result;
}

static protocol_message_result_t encode_scene_rot_rect(
    CborEncoder *parent,
    const scene_rot_rect_t *rect)
{
    CborEncoder map;
    size_t entries = 4U + (rect->radius != 0 ? 1U : 0U) +
                     (rect->fill != 0U ? 1U : 0U) +
                     (rect->pivot_x != 0 ? 1U : 0U) +
                     (rect->pivot_y != 0 ? 1U : 0U) +
                     (rect->rotation != 0 ? 1U : 0U) +
                     (rect->rotation_binding[0] != '\0' ? 1U : 0U) +
                     (rect->has_clip ? 1U : 0U);
    protocol_message_result_t result = begin_map(parent, &map, entries);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, rect->x);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 1U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, rect->y);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, rect->w);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 3U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_int(&map, rect->h);
    if (result == PROTOCOL_MESSAGE_OK && rect->radius != 0) {
        result = encode_uint(&map, 4U);
        if (result == PROTOCOL_MESSAGE_OK) {
            result = encode_int(&map, rect->radius);
        }
    }
    if (result == PROTOCOL_MESSAGE_OK && rect->fill != 0U) {
        result = encode_pair_uint(&map, 5U, rect->fill);
    }
    if (result == PROTOCOL_MESSAGE_OK && rect->pivot_x != 0) {
        result = encode_uint(&map, 6U);
        if (result == PROTOCOL_MESSAGE_OK) {
            result = encode_int(&map, rect->pivot_x);
        }
    }
    if (result == PROTOCOL_MESSAGE_OK && rect->pivot_y != 0) {
        result = encode_uint(&map, 7U);
        if (result == PROTOCOL_MESSAGE_OK) {
            result = encode_int(&map, rect->pivot_y);
        }
    }
    if (result == PROTOCOL_MESSAGE_OK && rect->rotation != 0) {
        result = encode_uint(&map, 8U);
        if (result == PROTOCOL_MESSAGE_OK) {
            result = encode_int(&map, rect->rotation);
        }
    }
    if (result == PROTOCOL_MESSAGE_OK &&
        rect->rotation_binding[0] != '\0') {
        result = encode_uint(&map, 9U);
        if (result == PROTOCOL_MESSAGE_OK) {
            result = encode_text(&map, rect->rotation_binding);
        }
    }
    if (result == PROTOCOL_MESSAGE_OK && rect->has_clip) {
        result = encode_uint(&map, 10U);
        if (result == PROTOCOL_MESSAGE_OK) {
            result = encode_scene_clip_rect(&map, &rect->clip);
        }
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(parent, &map);
    return result;
}

static protocol_message_result_t encode_scene_node(CborEncoder *parent,
                                                    const scene_node_t *node)
{
    CborEncoder map;
    protocol_message_result_t result = begin_map(parent, &map, 2U);
    if (result == PROTOCOL_MESSAGE_OK) {
        result = encode_pair_uint(&map, 0U, (uint64_t)node->kind);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 1U);
    if (result == PROTOCOL_MESSAGE_OK) {
        switch (node->kind) {
        case SCENE_NODE_RECT:
            result = encode_scene_rect(&map, &node->value.rect);
            break;
        case SCENE_NODE_ARC:
            result = encode_scene_arc(&map, &node->value.arc);
            break;
        case SCENE_NODE_LINE:
            result = encode_scene_line(&map, &node->value.line);
            break;
        case SCENE_NODE_TEXT:
            result = encode_scene_text(&map, &node->value.text);
            break;
        case SCENE_NODE_IMAGE:
            result = encode_scene_image(&map, &node->value.image);
            break;
        case SCENE_NODE_GLYPH:
            result = encode_scene_glyph(&map, &node->value.glyph);
            break;
        case SCENE_NODE_SCALE:
            result = encode_scene_scale(&map, &node->value.scale);
            break;
        case SCENE_NODE_LABEL:
            result = encode_scene_label(&map, &node->value.label);
            break;
        case SCENE_NODE_ROT_RECT:
            result = encode_scene_rot_rect(&map, &node->value.rot_rect);
            break;
        default:
            /* Unreachable: validate_message() ran scene_model_validate()
             * before encode_payload() was called, and it refuses any kind
             * outside this switch. */
            result = PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
            break;
        }
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(parent, &map);
    return result;
}

static protocol_message_result_t encode_scene_map(CborEncoder *parent, const scene_t *value)
{
    CborEncoder scene, nodes;
    protocol_message_result_t result = begin_map(parent, &scene, 3U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&scene, 0U, value->revision);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&scene, 1U, value->background);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&scene, 2U);
    if (result == PROTOCOL_MESSAGE_OK) result = begin_array(&scene, &nodes, value->node_count);
    for (uint32_t i = 0U; result == PROTOCOL_MESSAGE_OK && i < value->node_count; ++i) {
        result = encode_scene_node(&nodes, &value->nodes[i]);
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(&scene, &nodes);
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(parent, &scene);
    return result;
}

static protocol_message_result_t encode_push_scene_payload(
    CborEncoder *root, const protocol_push_scene_t *push)
{
    if (push->tap_view_count > PROTOCOL_MAX_TAP_VIEWS) return PROTOCOL_MESSAGE_ERR_INVALID_VALUE;
    CborEncoder map;
    size_t count = 3U + (push->tap_view_count ? 1U + (push->tap_wrap ? 0U : 1U) : 0U);
    protocol_message_result_t result = begin_map(root, &map, count);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, push->card_id);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 1U, push->revision);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
    if (result == PROTOCOL_MESSAGE_OK) result = encode_scene_map(&map, &push->scene);
    if (result == PROTOCOL_MESSAGE_OK && push->tap_view_count) {
        /* Encoder used by host fixtures; firmware only receives PushScene.
         * One scratch scene on the heap, never on the protocol task stack. */
        scene_t *scratch = malloc(sizeof(*scratch));
        if (scratch == NULL) return PROTOCOL_MESSAGE_ERR_TOO_LARGE;
        CborEncoder views;
        result = encode_uint(&map, 3U);
        if (result == PROTOCOL_MESSAGE_OK) result = begin_array(&map, &views, push->tap_view_count);
        for (size_t i = 0U; result == PROTOCOL_MESSAGE_OK && i < push->tap_view_count; ++i) {
            result = scene_result(scene_decode(push->tap_views[i], push->tap_view_lengths[i], scratch));
            if (result == PROTOCOL_MESSAGE_OK) result = encode_scene_map(&views, scratch);
        }
        free(scratch);
        if (result == PROTOCOL_MESSAGE_OK) result = end_map(&map, &views);
        if (result == PROTOCOL_MESSAGE_OK && !push->tap_wrap) {
            result = encode_uint(&map, 4U);
            if (result == PROTOCOL_MESSAGE_OK) result = encode_bool(&map, false);
        }
    }
    if (result == PROTOCOL_MESSAGE_OK) result = end_map(root, &map);
    return result;
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
    case PROTOCOL_TYPE_PUSH_TIMER:
        result = begin_map(&root, &map, 5U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, message->value.push_timer.card_id);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 1U, message->value.push_timer.revision);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 2U, message->value.push_timer.total_ms);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 3U, message->value.push_timer.remaining_ms);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 4U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_bool(&map, message->value.push_timer.running);
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
    case PROTOCOL_TYPE_ACTIVATE_CARD:
        result = begin_map(&root, &map, 1U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, message->value.activate_card.card_id);
        if (result == PROTOCOL_MESSAGE_OK) result = end_map(&root, &map);
        break;
    case PROTOCOL_TYPE_TRIGGER_INTERRUPT:
        result = begin_map(&root, &map, 3U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 0U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, message->value.trigger_interrupt.card_id);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 1U, message->value.trigger_interrupt.token);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, message->value.trigger_interrupt.reason);
        if (result == PROTOCOL_MESSAGE_OK) result = end_map(&root, &map);
        break;
    case PROTOCOL_TYPE_DEVICE_EVENT:
        result = begin_map(&root, &map,
                           4U + (message->value.device_event.has_interrupt_token ? 1U : 0U) +
                           (message->value.device_event.has_view_index ? 1U : 0U));
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 0U, message->value.device_event.sequence);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 1U, message->value.device_event.kind);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_uint(&map, 2U);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_text(&map, message->value.device_event.card_id);
        if (result == PROTOCOL_MESSAGE_OK) result = encode_pair_uint(&map, 4U, message->value.device_event.action);
        if (result == PROTOCOL_MESSAGE_OK && message->value.device_event.has_interrupt_token) {
            result = encode_pair_uint(&map, 5U, message->value.device_event.interrupt_token);
        }
        if (result == PROTOCOL_MESSAGE_OK && message->value.device_event.has_view_index) {
            result = encode_pair_uint(&map, 6U, message->value.device_event.view_index);
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
    case PROTOCOL_TYPE_ASSET_BEGIN:
        result = encode_asset_begin_payload(&root, &message->value.asset_begin);
        break;
    case PROTOCOL_TYPE_ASSET_CHUNK:
        result = encode_asset_chunk_payload(&root, &message->value.asset_chunk);
        break;
    case PROTOCOL_TYPE_ASSET_COMMIT:
        result = encode_asset_commit_payload(&root, &message->value.asset_commit);
        break;
    case PROTOCOL_TYPE_ASSET_RELEASE:
        result = encode_asset_release_payload(&root, &message->value.asset_release);
        break;
    case PROTOCOL_TYPE_PUSH_SCENE:
        result = encode_push_scene_payload(&root, &message->value.push_scene);
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
