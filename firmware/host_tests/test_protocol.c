#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "../main/core/protocol_frame.h"
#include "../main/core/protocol_message.h"

#define FIXTURE_DIRECTORY "../../protocol/fixtures/v1/"

typedef struct {
    size_t ok;
    size_t errors;
    size_t overflow;
    uint8_t types[8];
} callback_state_t;

static uint8_t *read_fixture(const char *name, size_t *length)
{
    char path[256];
    int written = snprintf(path, sizeof(path), "%s%s", FIXTURE_DIRECTORY,
                           name);
    assert(written > 0 && (size_t)written < sizeof(path));
    FILE *file = fopen(path, "rb");
    assert(file != NULL);
    assert(fseek(file, 0, SEEK_END) == 0);
    long file_length = ftell(file);
    assert(file_length >= 0);
    assert(fseek(file, 0, SEEK_SET) == 0);
    uint8_t *bytes = malloc((size_t)file_length == 0U ? 1U
                                                       : (size_t)file_length);
    assert(bytes != NULL);
    assert(fread(bytes, 1U, (size_t)file_length, file) ==
           (size_t)file_length);
    assert(fclose(file) == 0);
    *length = (size_t)file_length;
    return bytes;
}

static void decoder_callback(protocol_frame_result_t result,
                             const protocol_frame_t *frame,
                             void *context)
{
    callback_state_t *state = context;
    if (result == PROTOCOL_FRAME_OK) {
        assert(frame != NULL);
        state->types[state->ok++] = frame->message_type;
    } else {
        assert(frame == NULL);
        ++state->errors;
        if (result == PROTOCOL_FRAME_ERR_OVERLONG) {
            ++state->overflow;
        }
    }
}

static void test_crc(void)
{
    static const uint8_t input[] = "123456789";
    assert(protocol_crc32c(input, sizeof(input) - 1U) == UINT32_C(0xe3069283));
    assert(protocol_crc32c(NULL, 0U) == 0U);
}

static void test_status_request_accepts_reserved_ota_trigger_id(void)
{
    protocol_message_t request = {.type = PROTOCOL_TYPE_STATUS_REQUEST};
    uint8_t encoded[PROTOCOL_MAX_WIRE_FRAME];
    size_t encoded_length = 0U;
    assert(protocol_message_encode(UINT32_MAX, &request, encoded,
                                   sizeof(encoded), &encoded_length) ==
           PROTOCOL_MESSAGE_OK);

    protocol_frame_t frame;
    assert(protocol_frame_decode(encoded, encoded_length, &frame) ==
           PROTOCOL_FRAME_OK);
    assert(frame.request_id == UINT32_MAX);
    protocol_message_t decoded;
    assert(protocol_message_decode(&frame, &decoded) == PROTOCOL_MESSAGE_OK);
    assert(decoded.type == PROTOCOL_TYPE_STATUS_REQUEST);
}

static void assert_valid_fixture(const char *name,
                                 uint8_t expected_type,
                                 uint32_t expected_request_id)
{
    size_t length = 0U;
    uint8_t *fixture = read_fixture(name, &length);
    protocol_frame_t frame;
    assert(protocol_frame_decode(fixture, length, &frame) ==
           PROTOCOL_FRAME_OK);
    assert(frame.version == PROTOCOL_VERSION);
    assert(frame.message_type == expected_type);
    assert(frame.request_id == expected_request_id);

    protocol_message_t message;
    assert(protocol_message_decode(&frame, &message) == PROTOCOL_MESSAGE_OK);
    assert(message.type == expected_type);

    uint8_t encoded[PROTOCOL_MAX_WIRE_FRAME];
    size_t encoded_length = 0U;
    assert(protocol_message_encode(expected_request_id, &message, encoded,
                                   sizeof(encoded), &encoded_length) ==
           PROTOCOL_MESSAGE_OK);
    assert(encoded_length == length);
    assert(memcmp(encoded, fixture, length) == 0);
    free(fixture);
}

static void test_valid_fixtures(void)
{
    assert_valid_fixture("status_request.bin", PROTOCOL_TYPE_STATUS_REQUEST, 1U);
    assert_valid_fixture("status_response.bin", PROTOCOL_TYPE_STATUS_RESPONSE, 1U);
    assert_valid_fixture("time_sync.bin", PROTOCOL_TYPE_TIME_SYNC, 2U);
    assert_valid_fixture("ack_time.bin", PROTOCOL_TYPE_ACK, 2U);
    assert_valid_fixture("push_data.bin", PROTOCOL_TYPE_PUSH_DATA, 3U);
    assert_valid_fixture("ack_push.bin", PROTOCOL_TYPE_ACK, 3U);
    assert_valid_fixture("heartbeat.bin", PROTOCOL_TYPE_HEARTBEAT, 4U);
    assert_valid_fixture("heartbeat_ack.bin", PROTOCOL_TYPE_HEARTBEAT_ACK, 4U);
    assert_valid_fixture("error.bin", PROTOCOL_TYPE_ERROR, 5U);
    assert_valid_fixture("apply_config_min.bin", PROTOCOL_TYPE_APPLY_CONFIG,
                         10U);
    assert_valid_fixture("ack_config.bin", PROTOCOL_TYPE_ACK, 10U);
    assert_valid_fixture("apply_config_max.bin", PROTOCOL_TYPE_APPLY_CONFIG,
                         11U);
    assert_valid_fixture("activate_screen.bin",
                         PROTOCOL_TYPE_ACTIVATE_SCREEN, 12U);
    assert_valid_fixture("ack_activate.bin", PROTOCOL_TYPE_ACK, 12U);
    assert_valid_fixture("trigger_interrupt.bin",
                         PROTOCOL_TYPE_TRIGGER_INTERRUPT, 13U);
    assert_valid_fixture("ack_interrupt.bin", PROTOCOL_TYPE_ACK, 13U);
    assert_valid_fixture("device_event_tap.bin", PROTOCOL_TYPE_DEVICE_EVENT,
                         0U);
    assert_valid_fixture("device_event_previous.bin",
                         PROTOCOL_TYPE_DEVICE_EVENT, 0U);
    assert_valid_fixture("device_event_next.bin", PROTOCOL_TYPE_DEVICE_EVENT,
                         0U);
    assert_valid_fixture("device_event_dismissed.bin",
                         PROTOCOL_TYPE_DEVICE_EVENT, 0U);
    assert_valid_fixture("error_unknown_widget.bin", PROTOCOL_TYPE_ERROR,
                         14U);
    assert_valid_fixture("error_unknown_screen.bin", PROTOCOL_TYPE_ERROR,
                         15U);
    assert_valid_fixture("error_unsupported_template.bin",
                         PROTOCOL_TYPE_ERROR, 16U);
    assert_valid_fixture("error_unsupported_size.bin", PROTOCOL_TYPE_ERROR,
                         17U);
    assert_valid_fixture("error_config_too_large.bin", PROTOCOL_TYPE_ERROR,
                         18U);
    assert_valid_fixture("push_unknown_field.bin", PROTOCOL_TYPE_PUSH_DATA,
                         19U);
    assert_valid_fixture("network_config.bin", PROTOCOL_TYPE_NETWORK_CONFIG,
                         7U);
    assert_valid_fixture("factory_reset.bin", PROTOCOL_TYPE_FACTORY_RESET,
                         9U);
    assert_valid_fixture("status_response_networked.bin",
                         PROTOCOL_TYPE_STATUS_RESPONSE, 3U);
    assert_valid_fixture("status_response_ota_failed.bin",
                         PROTOCOL_TYPE_STATUS_RESPONSE, 32U);
    assert_valid_fixture("asset_begin.bin", PROTOCOL_TYPE_ASSET_BEGIN, 20U);
    assert_valid_fixture("ack_asset_begin.bin", PROTOCOL_TYPE_ACK, 20U);
    assert_valid_fixture("asset_chunk.bin", PROTOCOL_TYPE_ASSET_CHUNK, 21U);
    assert_valid_fixture("asset_commit.bin", PROTOCOL_TYPE_ASSET_COMMIT, 22U);
    assert_valid_fixture("asset_release.bin", PROTOCOL_TYPE_ASSET_RELEASE,
                         23U);
    /* The largest and most nested message on the wire, and the only proof
     * the two scene encoders agree byte for byte. */
    assert_valid_fixture("push_scene.bin", PROTOCOL_TYPE_PUSH_SCENE, 24U);
    assert_valid_fixture("push_scene_min.bin", PROTOCOL_TYPE_PUSH_SCENE, 25U);
    assert_valid_fixture("ack_scene.bin", PROTOCOL_TYPE_ACK, 24U);
}

static void test_status_capability_handshake_and_legacy_defaults(void)
{
    size_t length = 0U;
    uint8_t *fixture = read_fixture("status_response.bin", &length);
    protocol_frame_t frame;
    assert(protocol_frame_decode(fixture, length, &frame) ==
           PROTOCOL_FRAME_OK);
    free(fixture);

    protocol_message_t message;
    assert(protocol_message_decode(&frame, &message) == PROTOCOL_MESSAGE_OK);
    assert(message.value.status.max_protocol_version == PROTOCOL_MAX_VERSION);
    /* status_response.bin is wire data generated by the companion protocol
     * crate; its capabilities field is regenerated whenever
     * CURRENT_CAPABILITIES changes (most recently by scene rendering, bit 8,
     * taking it from 235 to 491). test_current_capabilities_is_491() pins
     * the live PROTOCOL_CURRENT_CAPABILITIES value; this fixture only
     * exercises decode of a capabilities field and the legacy-default path
     * below. */
    assert(message.value.status.capabilities == UINT64_C(491));

    /* The additive networking fields (Task 1) keep the entry count at or
     * above 24, so the map header stays in its two-byte extended form
     * regardless of the exact count. Keys 22 (max_protocol_version) and 23
     * (capabilities) are still encoded contiguously and in this order
     * because keys are canonical; locate and drop them regardless of what
     * now follows them on the wire. The pattern tail is
     * PROTOCOL_CURRENT_CAPABILITIES; adding bit 8 took it from 235 to 491,
     * which moved it out of CBOR's one-byte form (0x18 0xeb) into its
     * two-byte one (0x19 0x01 0xeb). It moves again whenever a capability
     * bit is added. */
    assert(frame.payload_length >= 6U);
    assert(frame.payload[0] == 0xb8U);
    uint8_t original_count = frame.payload[1];
    static const uint8_t pattern[] = {
        0x16U, PROTOCOL_MAX_VERSION, 0x17U, 0x19U, 0x01U, 0xebU};
    size_t pattern_offset = 0U;
    bool found = false;
    for (size_t i = 2U; i + sizeof(pattern) <= frame.payload_length; ++i) {
        if (memcmp(frame.payload + i, pattern, sizeof(pattern)) == 0) {
            pattern_offset = i;
            found = true;
            break;
        }
    }
    assert(found);
    memmove(frame.payload + pattern_offset,
            frame.payload + pattern_offset + sizeof(pattern),
            frame.payload_length - pattern_offset - sizeof(pattern));
    frame.payload_length -= (uint16_t)sizeof(pattern);
    frame.payload[1] = (uint8_t)(original_count - 2U);

    assert(protocol_message_decode(&frame, &message) == PROTOCOL_MESSAGE_OK);
    assert(message.value.status.max_protocol_version == PROTOCOL_VERSION);
    assert(message.value.status.capabilities == PROTOCOL_LEGACY_CAPABILITIES);
}

static void test_status_last_ota_error_round_trip_and_legacy_default(void)
{
    size_t length = 0U;
    uint8_t *fixture = read_fixture("status_response_networked.bin", &length);
    protocol_frame_t frame;
    assert(protocol_frame_decode(fixture, length, &frame) ==
           PROTOCOL_FRAME_OK);
    free(fixture);

    protocol_message_t message;
    assert(protocol_message_decode(&frame, &message) == PROTOCOL_MESSAGE_OK);
    /* This fixture predates key 30, so its absence must remain decodable. */
    assert(!message.value.status.has_last_ota_error);
    assert(message.value.status.last_ota_error[0] == '\0');

    uint8_t encoded[PROTOCOL_MAX_WIRE_FRAME];
    size_t encoded_length = 0U;
    assert(protocol_message_encode(frame.request_id, &message, encoded,
                                   sizeof(encoded), &encoded_length) ==
           PROTOCOL_MESSAGE_OK);
    protocol_frame_t round_trip_frame;
    assert(protocol_frame_decode(encoded, encoded_length,
                                 &round_trip_frame) == PROTOCOL_FRAME_OK);
    protocol_message_t decoded;
    assert(protocol_message_decode(&round_trip_frame, &decoded) ==
           PROTOCOL_MESSAGE_OK);
    assert(!decoded.value.status.has_last_ota_error);

    message.value.status.has_last_ota_error = true;
    strcpy(message.value.status.last_ota_error,
           "download: ESP_ERR_NO_MEM");
    assert(protocol_message_encode(frame.request_id, &message, encoded,
                                   sizeof(encoded), &encoded_length) ==
           PROTOCOL_MESSAGE_OK);
    assert(protocol_frame_decode(encoded, encoded_length,
                                 &round_trip_frame) == PROTOCOL_FRAME_OK);
    assert(protocol_message_decode(&round_trip_frame, &decoded) ==
           PROTOCOL_MESSAGE_OK);
    assert(decoded.value.status.has_last_ota_error);
    assert(strcmp(decoded.value.status.last_ota_error,
                  "download: ESP_ERR_NO_MEM") == 0);

    memset(message.value.status.last_ota_error, 'x',
           PROTOCOL_MAX_DIAGNOSTIC_LENGTH);
    message.value.status
        .last_ota_error[PROTOCOL_MAX_DIAGNOSTIC_LENGTH] = '\0';
    assert(protocol_message_encode(frame.request_id, &message, encoded,
                                   sizeof(encoded), &encoded_length) ==
           PROTOCOL_MESSAGE_OK);
    assert(protocol_frame_decode(encoded, encoded_length,
                                 &round_trip_frame) == PROTOCOL_FRAME_OK);
    assert(protocol_message_decode(&round_trip_frame, &decoded) ==
           PROTOCOL_MESSAGE_OK);
    assert(decoded.value.status.has_last_ota_error);
    assert(strlen(decoded.value.status.last_ota_error) ==
           PROTOCOL_MAX_DIAGNOSTIC_LENGTH);
}

static void test_status_asset_store_stats_round_trip_and_legacy_default(void)
{
    size_t length = 0U;
    uint8_t *fixture = read_fixture("status_response_networked.bin", &length);
    protocol_frame_t frame;
    assert(protocol_frame_decode(fixture, length, &frame) ==
           PROTOCOL_FRAME_OK);
    free(fixture);

    protocol_message_t message;
    assert(protocol_message_decode(&frame, &message) == PROTOCOL_MESSAGE_OK);
    /* This fixture predates key 31, so its absence must remain decodable. */
    assert(!message.value.status.has_asset_store_stats);
    assert(message.value.status.asset_store_used_bytes == 0U);
    assert(message.value.status.asset_store_free_bytes == 0U);
    assert(message.value.status.asset_count == 0U);

    uint8_t encoded[PROTOCOL_MAX_WIRE_FRAME];
    size_t encoded_length = 0U;
    assert(protocol_message_encode(frame.request_id, &message, encoded,
                                   sizeof(encoded), &encoded_length) ==
           PROTOCOL_MESSAGE_OK);
    protocol_frame_t round_trip_frame;
    assert(protocol_frame_decode(encoded, encoded_length,
                                 &round_trip_frame) == PROTOCOL_FRAME_OK);
    protocol_message_t decoded;
    assert(protocol_message_decode(&round_trip_frame, &decoded) ==
           PROTOCOL_MESSAGE_OK);
    assert(!decoded.value.status.has_asset_store_stats);

    message.value.status.has_asset_store_stats = true;
    message.value.status.asset_store_used_bytes = 4096U;
    message.value.status.asset_store_free_bytes = 6291456U;
    message.value.status.asset_count = 3U;
    assert(protocol_message_encode(frame.request_id, &message, encoded,
                                   sizeof(encoded), &encoded_length) ==
           PROTOCOL_MESSAGE_OK);
    assert(protocol_frame_decode(encoded, encoded_length,
                                 &round_trip_frame) == PROTOCOL_FRAME_OK);
    assert(protocol_message_decode(&round_trip_frame, &decoded) ==
           PROTOCOL_MESSAGE_OK);
    assert(decoded.value.status.has_asset_store_stats);
    assert(decoded.value.status.asset_store_used_bytes == 4096U);
    assert(decoded.value.status.asset_store_free_bytes == 6291456U);
    assert(decoded.value.status.asset_count == 3U);
}

static void assert_invalid_message_fixture(
    const char *name,
    protocol_message_result_t expected)
{
    size_t length = 0U;
    uint8_t *fixture = read_fixture(name, &length);
    protocol_frame_t frame;
    assert(protocol_frame_decode(fixture, length, &frame) ==
           PROTOCOL_FRAME_OK);
    protocol_message_t message;
    assert(protocol_message_decode(&frame, &message) == expected);
    free(fixture);
}

static void test_incremental_decoder(void)
{
    size_t first_length = 0U;
    size_t second_length = 0U;
    uint8_t *first = read_fixture("status_request.bin", &first_length);
    uint8_t *second = read_fixture("heartbeat.bin", &second_length);
    uint8_t *combined = malloc(first_length + second_length);
    assert(combined != NULL);
    memcpy(combined, first, first_length);
    memcpy(combined + first_length, second, second_length);

    protocol_decoder_t decoder;
    callback_state_t state = {0};
    protocol_decoder_init(&decoder);
    for (size_t i = 0; i < first_length; ++i) {
        protocol_decoder_feed(&decoder, first + i, 1U, decoder_callback,
                              &state);
    }
    assert(state.ok == 1U);
    protocol_decoder_feed(&decoder, combined + first_length, second_length,
                          decoder_callback, &state);
    assert(state.ok == 2U);
    assert(state.types[0] == PROTOCOL_TYPE_STATUS_REQUEST);
    assert(state.types[1] == PROTOCOL_TYPE_HEARTBEAT);
    assert(state.errors == 0U);

    free(combined);
    free(second);
    free(first);
}

static void test_overflow_resynchronizes(void)
{
    size_t valid_length = 0U;
    uint8_t *valid = read_fixture("status_request.bin", &valid_length);
    size_t overlong_length = PROTOCOL_MAX_ENCODED_FRAME + 2U;
    uint8_t *input = malloc(overlong_length + valid_length);
    assert(input != NULL);
    memset(input, 0x55, overlong_length - 1U);
    input[overlong_length - 1U] = 0U;
    memcpy(input + overlong_length, valid, valid_length);

    protocol_decoder_t decoder;
    callback_state_t state = {0};
    protocol_decoder_init(&decoder);
    protocol_decoder_feed(&decoder, input, overlong_length + valid_length,
                          decoder_callback, &state);
    assert(state.overflow == 1U);
    assert(state.errors == 1U);
    assert(state.ok == 1U);
    assert(state.types[0] == PROTOCOL_TYPE_STATUS_REQUEST);
    free(input);
    free(valid);
}

static void test_invalid_fixtures(void)
{
    size_t length = 0U;
    uint8_t *fixture = read_fixture("bad_crc.bin", &length);
    protocol_frame_t frame;
    assert(protocol_frame_decode(fixture, length, &frame) ==
           PROTOCOL_FRAME_ERR_CHECKSUM);
    free(fixture);

    fixture = read_fixture("unsupported_version.bin", &length);
    assert(protocol_frame_decode(fixture, length, &frame) ==
           PROTOCOL_FRAME_OK);
    protocol_message_t message;
    assert(protocol_message_decode(&frame, &message) ==
           PROTOCOL_MESSAGE_ERR_VERSION);
    free(fixture);

    fixture = read_fixture("unsupported_type.bin", &length);
    assert(protocol_frame_decode(fixture, length, &frame) ==
           PROTOCOL_FRAME_OK);
    assert(protocol_message_decode(&frame, &message) ==
           PROTOCOL_MESSAGE_ERR_UNSUPPORTED_TYPE);
    free(fixture);

    fixture = read_fixture("invalid_cbor.bin", &length);
    assert(protocol_frame_decode(fixture, length, &frame) ==
           PROTOCOL_FRAME_OK);
    assert(protocol_message_decode(&frame, &message) ==
           PROTOCOL_MESSAGE_ERR_CBOR);
    free(fixture);

    fixture = read_fixture("duplicate_keys.bin", &length);
    assert(protocol_frame_decode(fixture, length, &frame) ==
           PROTOCOL_FRAME_OK);
    assert(protocol_message_decode(&frame, &message) ==
           PROTOCOL_MESSAGE_ERR_DUPLICATE_KEY);
    free(fixture);

    assert_invalid_message_fixture("duplicate_widget_ids.bin",
                                   PROTOCOL_MESSAGE_ERR_DUPLICATE_KEY);
    assert_invalid_message_fixture("duplicate_screen_ids.bin",
                                   PROTOCOL_MESSAGE_ERR_DUPLICATE_KEY);
    assert_invalid_message_fixture("missing_widget_reference.bin",
                                   PROTOCOL_MESSAGE_ERR_UNKNOWN_WIDGET);
    assert_invalid_message_fixture("unsupported_template_config.bin",
                                   PROTOCOL_MESSAGE_ERR_UNSUPPORTED_TEMPLATE);
    assert_invalid_message_fixture(
        "unsupported_size_config.bin",
        PROTOCOL_MESSAGE_ERR_UNSUPPORTED_SIZE_CLASS);
    assert_invalid_message_fixture("config_too_many_widgets.bin",
                                   PROTOCOL_MESSAGE_ERR_CONFIG_TOO_LARGE);
    assert_invalid_message_fixture("config_too_many_screens.bin",
                                   PROTOCOL_MESSAGE_ERR_CONFIG_TOO_LARGE);
    assert_invalid_message_fixture("invalid_config_utf8.bin",
                                   PROTOCOL_MESSAGE_ERR_CBOR);
    assert_invalid_message_fixture("duplicate_field_names.bin",
                                   PROTOCOL_MESSAGE_ERR_DUPLICATE_KEY);
    assert_invalid_message_fixture("zero_request_config.bin",
                                   PROTOCOL_MESSAGE_ERR_INVALID_REQUEST_ID);
    assert_invalid_message_fixture("nonzero_request_event.bin",
                                   PROTOCOL_MESSAGE_ERR_INVALID_REQUEST_ID);

    fixture = read_fixture("garbage.bin", &length);
    assert(protocol_frame_decode(fixture, length, &frame) !=
           PROTOCOL_FRAME_OK);
    free(fixture);
}

static void test_maximum_config_payload(void)
{
    size_t length = 0U;
    uint8_t *fixture = read_fixture("apply_config_max.bin", &length);
    protocol_frame_t frame;
    assert(protocol_frame_decode(fixture, length, &frame) ==
           PROTOCOL_FRAME_OK);
    assert(frame.payload_length == 935U);
    assert(frame.payload_length <= PROTOCOL_MAX_PAYLOAD_SIZE);
    free(fixture);
}

static void test_maximum_frames(void)
{
    protocol_frame_t frame = {
        .version = PROTOCOL_VERSION,
        .message_type = PROTOCOL_TYPE_PUSH_DATA,
        .request_id = 9U,
        .payload_length = PROTOCOL_MAX_PAYLOAD_SIZE,
    };
    memset(frame.payload, 0U, sizeof(frame.payload));
    uint8_t wire[PROTOCOL_MAX_WIRE_FRAME];
    size_t wire_length = 0U;
    assert(protocol_frame_encode(&frame, wire, sizeof(wire), &wire_length) ==
           PROTOCOL_FRAME_OK);
    assert(wire_length <= sizeof(wire));
    protocol_frame_t decoded;
    assert(protocol_frame_decode(wire, wire_length, &decoded) ==
           PROTOCOL_FRAME_OK);
    assert(decoded.payload_length == PROTOCOL_MAX_PAYLOAD_SIZE);
    assert(memcmp(decoded.payload, frame.payload, sizeof(frame.payload)) == 0);

    memset(frame.payload, 0xa5, sizeof(frame.payload));
    assert(protocol_frame_encode(&frame, wire, sizeof(wire), &wire_length) ==
           PROTOCOL_FRAME_OK);
    assert(protocol_frame_decode(wire, wire_length, &decoded) ==
           PROTOCOL_FRAME_OK);
    assert(memcmp(decoded.payload, frame.payload, sizeof(frame.payload)) == 0);
}

static void test_excessive_cbor_nesting_is_rejected(void)
{
    protocol_frame_t frame = {
        .version = PROTOCOL_VERSION,
        .message_type = PROTOCOL_TYPE_TIME_SYNC,
        .request_id = 10U,
    };
    // Canonical {0: [[[[... 0 ...]]]]}; TinyCBOR must reject before the
    // schema decoder recurses or an embedded task stack can be exhausted.
    frame.payload[0] = 0xa1;
    frame.payload[1] = 0x00;
    for (size_t i = 0; i < 12U; ++i) {
        frame.payload[2U + i] = 0x81;
    }
    frame.payload[14] = 0x00;
    frame.payload_length = 15U;
    protocol_message_t message;
    assert(protocol_message_decode(&frame, &message) ==
           PROTOCOL_MESSAGE_ERR_CBOR);
}

static void test_invalid_time_has_specific_error(void)
{
    // {0: 1577836799, 1: 0}: canonical CBOR, one second before the v1 floor.
    const uint8_t payload[] = {
        0xa2U, 0x00U, 0x1aU, 0x5eU, 0x0bU, 0xe0U, 0xffU, 0x01U, 0x00U,
    };
    protocol_frame_t frame = {
        .version = PROTOCOL_VERSION,
        .message_type = PROTOCOL_TYPE_TIME_SYNC,
        .request_id = 1U,
        .payload_length = sizeof(payload),
    };
    memcpy(frame.payload, payload, sizeof(payload));
    protocol_message_t message;
    assert(protocol_message_decode(&frame, &message) ==
           PROTOCOL_MESSAGE_ERR_INVALID_TIME);
}

static void test_network_config_rejects_oversized_ssid(void)
{
    // {1: "x"*33, 2: "p", 3: "s", 4: "d", 5: "t", 6: 0, 7: 0}: a canonical
    // seven-entry network config map whose ssid exceeds the 32-byte bound.
    uint8_t payload[64];
    size_t offset = 0U;
    payload[offset++] = 0xa7U;
    payload[offset++] = 0x01U;
    payload[offset++] = 0x78U;
    payload[offset++] = 0x21U; /* text length 33 */
    memset(payload + offset, 'x', 33U);
    offset += 33U;
    payload[offset++] = 0x02U;
    payload[offset++] = 0x61U;
    payload[offset++] = 'p';
    payload[offset++] = 0x03U;
    payload[offset++] = 0x61U;
    payload[offset++] = 's';
    payload[offset++] = 0x04U;
    payload[offset++] = 0x61U;
    payload[offset++] = 'd';
    payload[offset++] = 0x05U;
    payload[offset++] = 0x61U;
    payload[offset++] = 't';
    payload[offset++] = 0x06U;
    payload[offset++] = 0x00U;
    payload[offset++] = 0x07U;
    payload[offset++] = 0x00U;

    protocol_frame_t frame = {
        .version = PROTOCOL_VERSION,
        .message_type = PROTOCOL_TYPE_NETWORK_CONFIG,
        .request_id = 30U,
        .payload_length = (uint16_t)offset,
    };
    memcpy(frame.payload, payload, offset);
    protocol_message_t message;
    assert(protocol_message_decode(&frame, &message) ==
           PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
}

static void test_network_config_rejects_unknown_key(void)
{
    // {1: "s", 2: "p", 3: "u", 4: "d", 5: "t", 6: 0, 7: 0, 8: 0}: an
    // otherwise-valid network config map with one unrecognized key (8).
    const uint8_t payload[] = {
        0xa8U, 0x01U, 0x61U, 's', 0x02U, 0x61U, 'p', 0x03U, 0x61U,
        'u',   0x04U, 0x61U, 'd', 0x05U, 0x61U, 't', 0x06U, 0x00U,
        0x07U, 0x00U, 0x08U, 0x00U,
    };
    protocol_frame_t frame = {
        .version = PROTOCOL_VERSION,
        .message_type = PROTOCOL_TYPE_NETWORK_CONFIG,
        .request_id = 31U,
        .payload_length = sizeof(payload),
    };
    memcpy(frame.payload, payload, sizeof(payload));
    protocol_message_t message;
    assert(protocol_message_decode(&frame, &message) ==
           PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
}

static void test_cardinal_display_rotations(void)
{
    size_t length = 0U;
    uint8_t *fixture = read_fixture("status_response.bin", &length);
    protocol_frame_t frame;
    assert(protocol_frame_decode(fixture, length, &frame) ==
           PROTOCOL_FRAME_OK);
    protocol_message_t message;
    assert(protocol_message_decode(&frame, &message) == PROTOCOL_MESSAGE_OK);
    free(fixture);

    const uint16_t valid[] = {0U, 90U, 180U, 270U};
    uint8_t encoded[PROTOCOL_MAX_WIRE_FRAME];
    for (size_t i = 0U; i < sizeof(valid) / sizeof(valid[0]); ++i) {
        message.value.status.rotation = valid[i];
        size_t encoded_length = 0U;
        assert(protocol_message_encode(1U, &message, encoded,
                                       sizeof(encoded), &encoded_length) ==
               PROTOCOL_MESSAGE_OK);
        assert(encoded_length > 0U);
    }

    message.value.status.rotation = 45U;
    size_t encoded_length = 0U;
    assert(protocol_message_encode(1U, &message, encoded, sizeof(encoded),
                                   &encoded_length) ==
           PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
}

static void test_config_landscape_rotations(void)
{
    size_t length = 0U;
    uint8_t *fixture = read_fixture("apply_config_min.bin", &length);
    protocol_frame_t frame;
    assert(protocol_frame_decode(fixture, length, &frame) ==
           PROTOCOL_FRAME_OK);
    protocol_message_t message;
    assert(protocol_message_decode(&frame, &message) == PROTOCOL_MESSAGE_OK);
    free(fixture);
    assert(message.value.apply_config.rotation == 90U);

    uint8_t encoded[PROTOCOL_MAX_WIRE_FRAME];
    size_t encoded_length = 0U;
    message.value.apply_config.rotation = 270U;
    assert(protocol_message_encode(1U, &message, encoded, sizeof(encoded),
                                   &encoded_length) == PROTOCOL_MESSAGE_OK);
    assert(encoded_length > 0U);

    message.value.apply_config.rotation = 180U;
    assert(protocol_message_encode(1U, &message, encoded, sizeof(encoded),
                                   &encoded_length) ==
           PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
}

static void test_extended_template_kinds(void)
{
    assert(protocol_template_kind_valid(PROTOCOL_TEMPLATE_ANALOG_CLOCK));
    assert(protocol_template_kind_valid(PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL));
    assert(protocol_template_kind_valid(PROTOCOL_TEMPLATE_ICON_BADGE_TEXT));
    assert((int)PROTOCOL_TEMPLATE_ANALOG_CLOCK == 4);
    assert((int)PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL == 5);
    assert((int)PROTOCOL_TEMPLATE_ICON_BADGE_TEXT == 6);
    /* Unknown kinds must still be refused, not clamped. */
    assert(!protocol_template_kind_valid((protocol_template_kind_t)7));
    assert(!protocol_template_kind_valid((protocol_template_kind_t)0));
}

/* Minimal canonical-CBOR primitive writers for hand-building asset transfer
 * fixtures: a 32-byte digest and up to 33 digests are impractical as literal
 * hex arrays, unlike the small fixed maps other tests in this file spell out
 * byte-by-byte. */
static size_t append_uint_cbor(uint8_t *out, size_t offset, uint64_t value)
{
    if (value < 24U) {
        out[offset++] = (uint8_t)value;
    } else if (value <= UINT8_MAX) {
        out[offset++] = 0x18U;
        out[offset++] = (uint8_t)value;
    } else if (value <= UINT16_MAX) {
        out[offset++] = 0x19U;
        out[offset++] = (uint8_t)(value >> 8);
        out[offset++] = (uint8_t)value;
    } else {
        out[offset++] = 0x1aU;
        out[offset++] = (uint8_t)(value >> 24);
        out[offset++] = (uint8_t)(value >> 16);
        out[offset++] = (uint8_t)(value >> 8);
        out[offset++] = (uint8_t)value;
    }
    return offset;
}

static size_t append_bstr_cbor(uint8_t *out, size_t offset,
                               const uint8_t *data, size_t length)
{
    assert(length <= UINT16_MAX);
    if (length < 24U) {
        out[offset++] = (uint8_t)(0x40U | length);
    } else if (length <= UINT8_MAX) {
        out[offset++] = 0x58U;
        out[offset++] = (uint8_t)length;
    } else {
        out[offset++] = 0x59U;
        out[offset++] = (uint8_t)(length >> 8);
        out[offset++] = (uint8_t)length;
    }
    memcpy(out + offset, data, length);
    return offset + length;
}

static size_t append_array_header_cbor(uint8_t *out, size_t offset,
                                       size_t count)
{
    assert(count <= UINT8_MAX);
    if (count < 24U) {
        out[offset++] = (uint8_t)(0x80U | count);
    } else {
        out[offset++] = 0x98U;
        out[offset++] = (uint8_t)count;
    }
    return offset;
}

static size_t append_bool_cbor(uint8_t *out, size_t offset, bool value)
{
    out[offset++] = value ? 0xf5U : 0xf4U;
    return offset;
}

static protocol_message_result_t decode_asset_begin_fixture(
    const uint8_t *digest, asset_kind_t kind, uint32_t total_length,
    bool volatile_tier, protocol_message_t *message)
{
    uint8_t payload[64];
    size_t offset = 0U;
    payload[offset++] = 0xa4U; /* map(4) */
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_bstr_cbor(payload, offset, digest, ASSET_DIGEST_BYTES);
    offset = append_uint_cbor(payload, offset, 1U);
    offset = append_uint_cbor(payload, offset, (uint64_t)kind);
    offset = append_uint_cbor(payload, offset, 2U);
    offset = append_uint_cbor(payload, offset, total_length);
    offset = append_uint_cbor(payload, offset, 3U);
    offset = append_bool_cbor(payload, offset, volatile_tier);

    protocol_frame_t frame = {
        .version = PROTOCOL_VERSION,
        .message_type = PROTOCOL_TYPE_ASSET_BEGIN,
        .request_id = 40U,
        .payload_length = (uint16_t)offset,
    };
    memcpy(frame.payload, payload, offset);
    return protocol_message_decode(&frame, message);
}

static protocol_message_result_t decode_asset_begin_short_digest(
    protocol_message_t *message)
{
    uint8_t digest[31];
    memset(digest, 0x11, sizeof digest);
    uint8_t payload[64];
    size_t offset = 0U;
    payload[offset++] = 0xa4U; /* map(4) */
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_bstr_cbor(payload, offset, digest, sizeof digest);
    offset = append_uint_cbor(payload, offset, 1U);
    offset = append_uint_cbor(payload, offset, (uint64_t)ASSET_KIND_FONT);
    offset = append_uint_cbor(payload, offset, 2U);
    offset = append_uint_cbor(payload, offset, 4096U);
    offset = append_uint_cbor(payload, offset, 3U);
    offset = append_bool_cbor(payload, offset, false);

    protocol_frame_t frame = {
        .version = PROTOCOL_VERSION,
        .message_type = PROTOCOL_TYPE_ASSET_BEGIN,
        .request_id = 41U,
        .payload_length = (uint16_t)offset,
    };
    memcpy(frame.payload, payload, offset);
    return protocol_message_decode(&frame, message);
}

static protocol_message_result_t decode_asset_chunk_with_data_length(
    size_t data_length, protocol_message_t *message)
{
    uint8_t digest[ASSET_DIGEST_BYTES];
    memset(digest, 0x22, sizeof digest);
    uint8_t data[PROTOCOL_MAX_ASSET_CHUNK_BYTES + 64U];
    assert(data_length <= sizeof data);
    memset(data, 0x33, data_length);

    uint8_t payload[PROTOCOL_MAX_PAYLOAD_SIZE];
    size_t offset = 0U;
    payload[offset++] = 0xa3U; /* map(3) */
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_bstr_cbor(payload, offset, digest, ASSET_DIGEST_BYTES);
    offset = append_uint_cbor(payload, offset, 1U);
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_uint_cbor(payload, offset, 2U);
    offset = append_bstr_cbor(payload, offset, data, data_length);
    assert(offset <= sizeof payload);

    protocol_frame_t frame = {
        .version = PROTOCOL_VERSION,
        .message_type = PROTOCOL_TYPE_ASSET_CHUNK,
        .request_id = 42U,
        .payload_length = (uint16_t)offset,
    };
    memcpy(frame.payload, payload, offset);
    return protocol_message_decode(&frame, message);
}

static protocol_message_result_t decode_asset_release_with_count(
    size_t count, protocol_message_t *message)
{
    uint8_t payload[PROTOCOL_MAX_PAYLOAD_SIZE];
    size_t offset = 0U;
    payload[offset++] = 0xa1U; /* map(1) */
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_array_header_cbor(payload, offset, count);
    for (size_t i = 0U; i < count; ++i) {
        uint8_t digest[ASSET_DIGEST_BYTES];
        memset(digest, (uint8_t)(i + 1U), sizeof digest);
        offset = append_bstr_cbor(payload, offset, digest, ASSET_DIGEST_BYTES);
    }
    assert(offset <= sizeof payload);

    protocol_frame_t frame = {
        .version = PROTOCOL_VERSION,
        .message_type = PROTOCOL_TYPE_ASSET_RELEASE,
        .request_id = 43U,
        .payload_length = (uint16_t)offset,
    };
    memcpy(frame.payload, payload, offset);
    return protocol_message_decode(&frame, message);
}

static protocol_message_result_t decode_ack_with_already_present(
    uint8_t acknowledged_type, bool already_present, protocol_ack_t *ack)
{
    uint8_t payload[16];
    size_t offset = 0U;
    payload[offset++] = 0xa2U; /* map(2) */
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_uint_cbor(payload, offset, acknowledged_type);
    offset = append_uint_cbor(payload, offset, 2U);
    offset = append_bool_cbor(payload, offset, already_present);

    protocol_frame_t frame = {
        .version = PROTOCOL_VERSION,
        .message_type = PROTOCOL_TYPE_ACK,
        .request_id = 44U,
        .payload_length = (uint16_t)offset,
    };
    memcpy(frame.payload, payload, offset);
    protocol_message_t message;
    protocol_message_result_t result = protocol_message_decode(&frame, &message);
    if (result == PROTOCOL_MESSAGE_OK) {
        *ack = message.value.ack;
    }
    return result;
}

static void test_asset_begin_roundtrips(void)
{
    uint8_t digest[ASSET_DIGEST_BYTES];
    memset(digest, 0x5A, sizeof digest);
    protocol_message_t message;

    assert(decode_asset_begin_fixture(digest, ASSET_KIND_FONT, 4096U, false,
                                      &message) == PROTOCOL_MESSAGE_OK);
    assert(message.type == PROTOCOL_TYPE_ASSET_BEGIN);
    assert(message.value.asset_begin.kind == ASSET_KIND_FONT);
    assert(message.value.asset_begin.total_length == 4096U);
    assert(message.value.asset_begin.volatile_tier == false);
    assert(memcmp(message.value.asset_begin.digest, digest, ASSET_DIGEST_BYTES) == 0);
}

static void test_asset_begin_rejects_short_digest(void)
{
    protocol_message_t message;
    /* A 31-byte digest must be rejected outright: a truncated digest would
     * silently address a different asset. */
    assert(decode_asset_begin_short_digest(&message)
           == PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
}

static void test_asset_chunk_rejects_oversize_data(void)
{
    protocol_message_t message;
    assert(decode_asset_chunk_with_data_length(PROTOCOL_MAX_ASSET_CHUNK_BYTES + 1U,
                                               &message)
           == PROTOCOL_MESSAGE_ERR_TOO_LARGE);
}

static void test_asset_release_rejects_too_many_digests(void)
{
    protocol_message_t message;
    assert(decode_asset_release_with_count(PROTOCOL_MAX_ASSET_DIGESTS + 1U, &message)
           == PROTOCOL_MESSAGE_ERR_TOO_LARGE);
}

static void test_ack_carries_already_present_only_for_asset_begin(void)
{
    protocol_ack_t ack;
    assert(decode_ack_with_already_present(PROTOCOL_TYPE_ASSET_BEGIN, true, &ack)
           == PROTOCOL_MESSAGE_OK);
    assert(ack.has_already_present && ack.already_present);

    /* Key 2 on any other acknowledged type is a contract violation. */
    assert(decode_ack_with_already_present(PROTOCOL_TYPE_ASSET_COMMIT, true, &ack)
           == PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
}

static void test_current_capabilities_is_491(void)
{
    /* Bit 7 sat defined-but-dark for most of V2 and a conforming host could
     * not provision the device. Pin the number, not the expression. */
    assert(PROTOCOL_CURRENT_CAPABILITIES == UINT64_C(491));
    /* And pin the newest bit on its own, so a future edit that drops it from
     * the union fails here rather than at a host that gates on it. */
    assert((PROTOCOL_CURRENT_CAPABILITIES &
            PROTOCOL_CAPABILITY_SCENE_RENDER) != 0U);
    assert(PROTOCOL_CAPABILITY_SCENE_RENDER == UINT64_C(256));
}

/* ------------------------------------------------------------------
 * PushScene (type 19).
 *
 * The payload is {0: card_id, 1: revision, 2: <scene map>}; the scene map's
 * own shape is documented at the top of core/scene_decode.c. These helpers
 * hand-build it, because a scene is far too nested to spell out as a hex
 * literal the way the small fixed maps above are.
 * ------------------------------------------------------------------ */

static size_t append_map_header_cbor(uint8_t *out, size_t offset, size_t count)
{
    assert(count < 24U);
    out[offset++] = (uint8_t)(0xa0U | count);
    return offset;
}

static size_t append_text_cbor(uint8_t *out, size_t offset, const char *text)
{
    size_t length = strlen(text);
    assert(length < 24U);
    out[offset++] = (uint8_t)(0x60U | length);
    memcpy(out + offset, text, length);
    return offset + length;
}

/* The smallest legal RECT node: {0: kind, 1: {0: x, 1: y, 2: w, 3: h}}.
 * Every one of these is INSIDE the canvas and otherwise valid, which is what
 * makes the node-cap test below able to fail for the count and nothing
 * else. */
static size_t append_minimal_rect_node_cbor(uint8_t *out, size_t offset)
{
    offset = append_map_header_cbor(out, offset, 2U);
    offset = append_uint_cbor(out, offset, 0U);
    offset = append_uint_cbor(out, offset, 1U); /* SCENE_NODE_RECT */
    offset = append_uint_cbor(out, offset, 1U);
    offset = append_map_header_cbor(out, offset, 4U);
    offset = append_uint_cbor(out, offset, 0U);
    offset = append_uint_cbor(out, offset, 4U);
    offset = append_uint_cbor(out, offset, 1U);
    offset = append_uint_cbor(out, offset, 5U);
    offset = append_uint_cbor(out, offset, 2U);
    offset = append_uint_cbor(out, offset, 10U);
    offset = append_uint_cbor(out, offset, 3U);
    offset = append_uint_cbor(out, offset, 11U);
    return offset;
}

static protocol_message_result_t decode_push_scene_frame(const uint8_t *payload,
                                                         size_t payload_length,
                                                         protocol_message_t *message)
{
    protocol_frame_t frame = {
        .version = PROTOCOL_VERSION,
        .message_type = PROTOCOL_TYPE_PUSH_SCENE,
        .request_id = 45U,
        .payload_length = (uint16_t)payload_length,
    };
    assert(payload_length <= sizeof frame.payload);
    memcpy(frame.payload, payload, payload_length);
    return protocol_message_decode(&frame, message);
}

/* Writes {0: card_id, 1: revision, 2: {0: 9, 1: 0, 2: [rect x node_count]}}. */
static size_t build_push_scene_payload(uint8_t *out, const char *card_id,
                                       uint32_t revision, size_t node_count)
{
    size_t offset = 0U;
    offset = append_map_header_cbor(out, offset, 3U);
    offset = append_uint_cbor(out, offset, 0U);
    offset = append_text_cbor(out, offset, card_id);
    offset = append_uint_cbor(out, offset, 1U);
    offset = append_uint_cbor(out, offset, revision);
    offset = append_uint_cbor(out, offset, 2U);
    offset = append_map_header_cbor(out, offset, 3U);
    offset = append_uint_cbor(out, offset, 0U);
    offset = append_uint_cbor(out, offset, 9U); /* scene revision */
    offset = append_uint_cbor(out, offset, 1U);
    offset = append_uint_cbor(out, offset, 0U); /* background */
    offset = append_uint_cbor(out, offset, 2U);
    offset = append_array_header_cbor(out, offset, node_count);
    for (size_t i = 0U; i < node_count; ++i) {
        offset = append_minimal_rect_node_cbor(out, offset);
    }
    return offset;
}

static void test_push_scene_roundtrips(void)
{
    /* A RECT, and a TEXT whose value is a binding: the text node drags in
     * the nested font and value maps, which nothing else on this message
     * exercises. */
    uint8_t payload[PROTOCOL_MAX_PAYLOAD_SIZE];
    size_t offset = 0U;
    offset = append_map_header_cbor(payload, offset, 3U);
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_text_cbor(payload, offset, "clock");
    offset = append_uint_cbor(payload, offset, 1U);
    offset = append_uint_cbor(payload, offset, 7U);
    offset = append_uint_cbor(payload, offset, 2U);
    offset = append_map_header_cbor(payload, offset, 3U);
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_uint_cbor(payload, offset, 9U);
    offset = append_uint_cbor(payload, offset, 1U);
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_uint_cbor(payload, offset, 2U);
    offset = append_array_header_cbor(payload, offset, 2U);
    offset = append_minimal_rect_node_cbor(payload, offset);
    /* TEXT: {0: kind, 1: {0: x, 1: baseline_y, 2: w, 4: font, 6: value}} */
    offset = append_map_header_cbor(payload, offset, 2U);
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_uint_cbor(payload, offset, 4U); /* SCENE_NODE_TEXT */
    offset = append_uint_cbor(payload, offset, 1U);
    offset = append_map_header_cbor(payload, offset, 5U);
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_uint_cbor(payload, offset, 16U);
    offset = append_uint_cbor(payload, offset, 1U);
    offset = append_uint_cbor(payload, offset, 200U);
    offset = append_uint_cbor(payload, offset, 2U);
    offset = append_uint_cbor(payload, offset, 400U);
    offset = append_uint_cbor(payload, offset, 4U);
    offset = append_map_header_cbor(payload, offset, 2U);
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_uint_cbor(payload, offset, 1U); /* SCENE_FONT_BAKED */
    offset = append_uint_cbor(payload, offset, 1U);
    offset = append_uint_cbor(payload, offset, 2U); /* SCENE_FONT_BODY */
    offset = append_uint_cbor(payload, offset, 6U);
    offset = append_map_header_cbor(payload, offset, 2U);
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_uint_cbor(payload, offset, 2U); /* SCENE_VALUE_BINDING */
    offset = append_uint_cbor(payload, offset, 2U);
    offset = append_text_cbor(payload, offset, "time:HH:mm");

    protocol_message_t *message = malloc(sizeof *message);
    assert(message != NULL);
    assert(decode_push_scene_frame(payload, offset, message) ==
           PROTOCOL_MESSAGE_OK);
    assert(message->type == PROTOCOL_TYPE_PUSH_SCENE);
    assert(strcmp(message->value.push_scene.card_id, "clock") == 0);
    assert(message->value.push_scene.revision == 7U);
    assert(message->value.push_scene.scene.revision == 9U);
    assert(message->value.push_scene.scene.node_count == 2U);
    assert(message->value.push_scene.scene.nodes[0].kind == SCENE_NODE_RECT);
    assert(message->value.push_scene.scene.nodes[0].value.rect.w == 10);
    assert(message->value.push_scene.scene.nodes[0].value.rect.opacity == 255U);
    assert(message->value.push_scene.scene.nodes[1].kind == SCENE_NODE_TEXT);
    assert(message->value.push_scene.scene.nodes[1].value.text.align ==
           SCENE_ALIGN_LEFT);
    assert(strcmp(message->value.push_scene.scene.nodes[1].value.text.value.binding,
                  "time:HH:mm") == 0);

    /* Re-encoded, the payload must come back byte-identical: the omitted
     * optional keys stay omitted, which is the rule the cross-language
     * fixture depends on. */
    uint8_t wire[PROTOCOL_MAX_WIRE_FRAME];
    size_t wire_length = 0U;
    assert(protocol_message_encode(45U, message, wire, sizeof wire,
                                   &wire_length) == PROTOCOL_MESSAGE_OK);
    protocol_frame_t frame;
    assert(protocol_frame_decode(wire, wire_length, &frame) ==
           PROTOCOL_FRAME_OK);
    assert(frame.payload_length == offset);
    assert(memcmp(frame.payload, payload, offset) == 0);
    free(message);
}

/* The trap this test is built around: scene_model_validate() rejects a
 * node_count over the cap with the SAME code the decoder's own bound
 * produces, so an assertion on the result alone cannot tell a working
 * decoder from one that has already written past nodes[23].
 *
 * What it DOES prove, unconditionally, is that this message type routes its
 * nested map into the scene decoder and propagates the refusal: a PushScene
 * handler that skipped key 2, or swallowed the scene decoder's verdict,
 * returns OK here. Every other part of the envelope is deliberately
 * acceptable to the message layer -- a legal card_id, a nonzero revision,
 * sorted keys, and 25 individually valid RECT nodes -- so the count is the
 * only thing left that can refuse it.
 *
 * MASKED WITHOUT ASan: the decoder's own bound is proved only under
 * `make -C firmware/host_tests sanitize`, where the overflowing write into
 * this heap-allocated message is a heap-buffer-overflow. Same reasoning as
 * test_scene_decode.c's node-cap case; do not delete either as duplication.
 */
static void test_push_scene_rejects_a_scene_over_the_node_cap(void)
{
    uint8_t payload[PROTOCOL_MAX_PAYLOAD_SIZE];
    size_t offset =
        build_push_scene_payload(payload, "clock", 3U, SCENE_MAX_NODES + 1U);

    protocol_message_t *message = malloc(sizeof *message);
    assert(message != NULL);
    assert(decode_push_scene_frame(payload, offset, message) ==
           PROTOCOL_MESSAGE_ERR_TOO_LARGE);
    /* Refused whole, not clipped to the first 24. */
    assert(message->value.push_scene.scene.node_count == 0U);

    /* The same payload one node smaller is accepted, so the cap is the
     * reason and nothing else about these nodes is. */
    offset = build_push_scene_payload(payload, "clock", 3U, SCENE_MAX_NODES);
    assert(decode_push_scene_frame(payload, offset, message) ==
           PROTOCOL_MESSAGE_OK);
    assert(message->value.push_scene.scene.node_count == SCENE_MAX_NODES);
    free(message);
}

/* A distinct result class from the node cap, so the assertion above is
 * about the count rather than about "some scene problem". */
static void test_push_scene_rejects_an_off_canvas_node(void)
{
    uint8_t payload[PROTOCOL_MAX_PAYLOAD_SIZE];
    size_t offset = 0U;
    offset = append_map_header_cbor(payload, offset, 3U);
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_text_cbor(payload, offset, "clock");
    offset = append_uint_cbor(payload, offset, 1U);
    offset = append_uint_cbor(payload, offset, 3U);
    offset = append_uint_cbor(payload, offset, 2U);
    offset = append_map_header_cbor(payload, offset, 3U);
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_uint_cbor(payload, offset, 9U);
    offset = append_uint_cbor(payload, offset, 1U);
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_uint_cbor(payload, offset, 2U);
    offset = append_array_header_cbor(payload, offset, 1U);
    offset = append_map_header_cbor(payload, offset, 2U);
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_uint_cbor(payload, offset, 1U); /* SCENE_NODE_RECT */
    offset = append_uint_cbor(payload, offset, 1U);
    offset = append_map_header_cbor(payload, offset, 4U);
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_uint_cbor(payload, offset, 1U);
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_uint_cbor(payload, offset, 2U);
    offset = append_uint_cbor(payload, offset, 4096U); /* wider than 448 */
    offset = append_uint_cbor(payload, offset, 3U);
    offset = append_uint_cbor(payload, offset, 10U);

    protocol_message_t *message = malloc(sizeof *message);
    assert(message != NULL);
    assert(decode_push_scene_frame(payload, offset, message) ==
           PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
    assert(message->value.push_scene.scene.node_count == 0U);
    free(message);
}

static void test_push_scene_rejects_a_zero_revision(void)
{
    uint8_t payload[PROTOCOL_MAX_PAYLOAD_SIZE];
    size_t offset = build_push_scene_payload(payload, "clock", 0U, 1U);
    protocol_message_t *message = malloc(sizeof *message);
    assert(message != NULL);
    assert(decode_push_scene_frame(payload, offset, message) ==
           PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
    free(message);
}

static protocol_message_result_t decode_ack_for(uint8_t acknowledged_type,
                                                bool with_revision)
{
    uint8_t payload[16];
    size_t offset = 0U;
    offset = append_map_header_cbor(payload, offset, with_revision ? 2U : 1U);
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_uint_cbor(payload, offset, acknowledged_type);
    if (with_revision) {
        offset = append_uint_cbor(payload, offset, 1U);
        offset = append_uint_cbor(payload, offset, 5U);
    }

    protocol_frame_t frame = {
        .version = PROTOCOL_VERSION,
        .message_type = PROTOCOL_TYPE_ACK,
        .request_id = 46U,
        .payload_length = (uint16_t)offset,
    };
    memcpy(frame.payload, payload, offset);
    protocol_message_t message;
    return protocol_message_decode(&frame, &message);
}

static void test_ack_for_push_scene_requires_a_revision(void)
{
    /* Like PushData and ApplyConfig, and unlike every other acknowledged
     * type: a scene the host cannot pin a revision to is a scene it cannot
     * tell apart from the one already on the panel. */
    assert(decode_ack_for(PROTOCOL_TYPE_PUSH_SCENE, true) ==
           PROTOCOL_MESSAGE_OK);
    assert(decode_ack_for(PROTOCOL_TYPE_PUSH_SCENE, false) ==
           PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
    /* The rule is per-type, not global: ActivateScreen must still refuse
     * one. */
    assert(decode_ack_for(PROTOCOL_TYPE_ACTIVATE_SCREEN, true) ==
           PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
}

int main(void)
{
    test_crc();
    test_status_request_accepts_reserved_ota_trigger_id();
    test_valid_fixtures();
    test_status_capability_handshake_and_legacy_defaults();
    test_status_last_ota_error_round_trip_and_legacy_default();
    test_status_asset_store_stats_round_trip_and_legacy_default();
    test_incremental_decoder();
    test_overflow_resynchronizes();
    test_invalid_fixtures();
    test_maximum_config_payload();
    test_maximum_frames();
    test_excessive_cbor_nesting_is_rejected();
    test_invalid_time_has_specific_error();
    test_network_config_rejects_oversized_ssid();
    test_network_config_rejects_unknown_key();
    test_cardinal_display_rotations();
    test_config_landscape_rotations();
    test_extended_template_kinds();
    test_asset_begin_roundtrips();
    test_asset_begin_rejects_short_digest();
    test_asset_chunk_rejects_oversize_data();
    test_asset_release_rejects_too_many_digests();
    test_ack_carries_already_present_only_for_asset_begin();
    test_current_capabilities_is_491();
    test_push_scene_roundtrips();
    test_push_scene_rejects_a_scene_over_the_node_cap();
    test_push_scene_rejects_an_off_canvas_node();
    test_push_scene_rejects_a_zero_revision();
    test_ack_for_push_scene_requires_a_revision();
    puts("test_protocol: OK");
    return 0;
}
