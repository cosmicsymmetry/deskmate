#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "../main/core/protocol_frame.h"
#include "../main/core/protocol_message.h"
#include "../main/core/volatile_asset_store.h"

#define FIXTURE_DIRECTORY "../../protocol/fixtures/v2/"

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

static void test_status_request_accepts_the_maximum_request_id(void)
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
    assert_valid_fixture("push_timer.bin", PROTOCOL_TYPE_PUSH_TIMER, 3U);
    assert_valid_fixture("ack_push.bin", PROTOCOL_TYPE_ACK, 3U);
    assert_valid_fixture("heartbeat.bin", PROTOCOL_TYPE_HEARTBEAT, 4U);
    assert_valid_fixture("heartbeat_ack.bin", PROTOCOL_TYPE_HEARTBEAT_ACK, 4U);
    assert_valid_fixture("error.bin", PROTOCOL_TYPE_ERROR, 5U);
    assert_valid_fixture("apply_config_min.bin", PROTOCOL_TYPE_APPLY_CONFIG,
                         10U);
    assert_valid_fixture("ack_config.bin", PROTOCOL_TYPE_ACK, 10U);
    assert_valid_fixture("apply_config_max.bin", PROTOCOL_TYPE_APPLY_CONFIG,
                         11U);
    assert_valid_fixture("activate_card.bin",
                         PROTOCOL_TYPE_ACTIVATE_CARD, 12U);
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
    assert_valid_fixture("error_unknown_card.bin", PROTOCOL_TYPE_ERROR,
                         14U);
    assert_valid_fixture("error_config_too_large.bin", PROTOCOL_TYPE_ERROR,
                         18U);
    assert_valid_fixture("push_timer_paused.bin", PROTOCOL_TYPE_PUSH_TIMER,
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
    assert_valid_fixture("asset_begin_rle.bin", PROTOCOL_TYPE_ASSET_BEGIN,
                         33U);
    assert_valid_fixture("ack_asset_begin.bin", PROTOCOL_TYPE_ACK, 20U);
    assert_valid_fixture("asset_chunk.bin", PROTOCOL_TYPE_ASSET_CHUNK, 21U);
    assert_valid_fixture("asset_commit.bin", PROTOCOL_TYPE_ASSET_COMMIT, 22U);
    assert_valid_fixture("asset_release.bin", PROTOCOL_TYPE_ASSET_RELEASE,
                         23U);
    /* The largest and most nested message on the wire, and the only proof
     * the two scene encoders agree byte for byte. */
    assert_valid_fixture("push_scene.bin", PROTOCOL_TYPE_PUSH_SCENE, 24U);
    assert_valid_fixture("push_scene_min.bin", PROTOCOL_TYPE_PUSH_SCENE, 25U);
    assert_valid_fixture("push_scene_tap_views.bin", PROTOCOL_TYPE_PUSH_SCENE, 26U);
    assert_valid_fixture("push_scene_tap_stop.bin", PROTOCOL_TYPE_PUSH_SCENE, 27U);
    assert_valid_fixture("device_event_tap_view.bin", PROTOCOL_TYPE_DEVICE_EVENT, 0U);
    assert_valid_fixture("ack_scene.bin", PROTOCOL_TYPE_ACK, 24U);
}

static void test_scene_fixtures_pin_new_fields_and_omitted_defaults(void)
{
    size_t length = 0U;
    uint8_t *fixture = read_fixture("push_scene.bin", &length);
    protocol_frame_t frame;
    assert(protocol_frame_decode(fixture, length, &frame) ==
           PROTOCOL_FRAME_OK);
    free(fixture);

    protocol_message_t message;
    assert(protocol_message_decode(&frame, &message) == PROTOCOL_MESSAGE_OK);
    const scene_t *scene = &message.value.push_scene.scene;
    assert(scene->node_count == 12U);
    const scene_rect_t *rect = &scene->nodes[0].value.rect;
    const scene_arc_t *arc = &scene->nodes[1].value.arc;
    const scene_line_t *bound_line = &scene->nodes[3].value.line;
    const scene_text_t *running_text = &scene->nodes[4].value.text;
    const scene_text_t *date_text = &scene->nodes[5].value.text;
    const scene_rot_rect_t *rot_rect =
        &scene->nodes[11].value.rot_rect;
    assert(scene->nodes[0].kind == SCENE_NODE_RECT);
    assert(rect->opacity == 0xc8U);
    assert(rect->has_clip);
    assert(rect->clip.x == 16);
    assert(rect->clip.y == 16);
    assert(rect->clip.w == 416);
    assert(rect->clip.h == 336);
    assert(scene->nodes[1].kind == SCENE_NODE_ARC);
    assert(arc->opacity == 0x33U);
    assert(arc->has_running_color);
    assert(arc->running_color == UINT32_C(0x0064D2FF));
    assert(scene->nodes[3].kind == SCENE_NODE_LINE);
    assert(bound_line->point_count == 0U);
    assert(bound_line->pivot_x == 224);
    assert(bound_line->pivot_y == 184);
    assert(bound_line->length == 80);
    assert(strcmp(bound_line->angle_binding, "time:angle:hour") == 0);
    assert(scene->nodes[4].kind == SCENE_NODE_TEXT);
    assert(running_text->has_running_color);
    assert(running_text->running_color == UINT32_C(0x00FF9F0A));
    assert(strcmp(running_text->value.binding, "time:HH:mm") == 0);
    assert(scene->nodes[5].kind == SCENE_NODE_TEXT);
    assert(strcmp(date_text->value.binding, "date") == 0);
    assert(scene->nodes[11].kind == SCENE_NODE_ROT_RECT);
    assert(rot_rect->has_clip);
    assert(rot_rect->clip.x == 64);
    assert(rot_rect->clip.y == 24);
    assert(rot_rect->clip.w == 320);
    assert(rot_rect->clip.h == 320);

    fixture = read_fixture("push_scene_min.bin", &length);
    assert(protocol_frame_decode(fixture, length, &frame) ==
           PROTOCOL_FRAME_OK);
    free(fixture);
    assert(protocol_message_decode(&frame, &message) == PROTOCOL_MESSAGE_OK);
    assert(message.value.push_scene.scene.node_count == 1U);
    rect = &message.value.push_scene.scene.nodes[0].value.rect;
    assert(rect->opacity == UINT8_MAX);
    assert(!rect->has_clip);
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
     * CURRENT_CAPABILITIES changes. */
    assert(message.value.status.capabilities == UINT64_C(16352));

    /* The additive networking fields (Task 1) keep the entry count at or
     * above 24, so the map header stays in its two-byte extended form
     * regardless of the exact count. Keys 22 (max_protocol_version) and 23
     * (capabilities) are still encoded contiguously and in this order
     * because keys are canonical; locate and drop them regardless of what
     * now follows them on the wire. The pattern tail is the fixture's
     * capabilities value, 16352 (0x3fe0). */
    assert(frame.payload_length >= 6U);
    assert(frame.payload[0] == 0xb8U);
    uint8_t original_count = frame.payload[1];
    static const uint8_t pattern[] = {
        0x16U, PROTOCOL_MAX_VERSION, 0x17U, 0x19U, 0x3fU, 0xe0U};
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
    assert(message.value.status.capabilities == 0U);
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

static void test_status_volatile_pool_stats_round_trip_and_legacy_default(void)
{
    size_t length = 0U;
    uint8_t *fixture = read_fixture("status_response_networked.bin", &length);
    protocol_frame_t frame;
    assert(protocol_frame_decode(fixture, length, &frame) ==
           PROTOCOL_FRAME_OK);
    free(fixture);

    protocol_message_t message;
    assert(protocol_message_decode(&frame, &message) == PROTOCOL_MESSAGE_OK);
    /* This fixture predates key 32, so its absence must remain decodable, and
     * a reader must not mistake a silent zero for a pool with no room. */
    assert(!message.value.status.has_volatile_asset_stats);
    assert(message.value.status.volatile_committed_count == 0U);
    assert(message.value.status.volatile_slot_capacity == 0U);
    assert(message.value.status.volatile_used_bytes == 0U);
    assert(message.value.status.psram_free_bytes == 0U);
    assert(message.value.status.psram_low_water_bytes == 0U);

    uint8_t encoded[PROTOCOL_MAX_WIRE_FRAME];
    size_t encoded_length = 0U;
    protocol_frame_t round_trip_frame;
    protocol_message_t decoded;

    /* Keys 31 and 32 are independent: a device reports both, and each has to
     * survive the other being present. */
    message.value.status.has_asset_store_stats = true;
    message.value.status.asset_store_used_bytes = 141312U;
    message.value.status.asset_store_free_bytes = 6149120U;
    message.value.status.asset_count = 4U;
    message.value.status.has_volatile_asset_stats = true;
    message.value.status.volatile_committed_count = 4U;
    message.value.status.volatile_slot_capacity = VOLATILE_ASSET_SLOT_COUNT;
    message.value.status.volatile_used_bytes = 4U * VOLATILE_ASSET_FRAME_BYTES;
    message.value.status.psram_free_bytes = 7012352U;
    message.value.status.psram_low_water_bytes = 6803456U;
    assert(protocol_message_encode(frame.request_id, &message, encoded,
                                   sizeof(encoded), &encoded_length) ==
           PROTOCOL_MESSAGE_OK);
    assert(protocol_frame_decode(encoded, encoded_length,
                                 &round_trip_frame) == PROTOCOL_FRAME_OK);
    assert(protocol_message_decode(&round_trip_frame, &decoded) ==
           PROTOCOL_MESSAGE_OK);
    assert(decoded.value.status.has_asset_store_stats);
    assert(decoded.value.status.asset_store_used_bytes == 141312U);
    assert(decoded.value.status.has_volatile_asset_stats);
    assert(decoded.value.status.volatile_committed_count == 4U);
    assert(decoded.value.status.volatile_slot_capacity ==
           VOLATILE_ASSET_SLOT_COUNT);
    assert(decoded.value.status.volatile_used_bytes ==
           4U * VOLATILE_ASSET_FRAME_BYTES);
    assert(decoded.value.status.psram_free_bytes == 7012352U);
    assert(decoded.value.status.psram_low_water_bytes == 6803456U);
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

    assert_invalid_message_fixture("duplicate_card_ids.bin",
                                   PROTOCOL_MESSAGE_ERR_DUPLICATE_KEY);
    /* A card's tap action is the only enumerated value left on the wire, so
     * it is the only one that can be unsupported. */
    assert_invalid_message_fixture("unsupported_tap_action_config.bin",
                                   PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
    assert_invalid_message_fixture("config_too_many_cards.bin",
                                   PROTOCOL_MESSAGE_ERR_CONFIG_TOO_LARGE);
    assert_invalid_message_fixture("invalid_config_utf8.bin",
                                   PROTOCOL_MESSAGE_ERR_CBOR);
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
    assert(frame.payload_length == 317U);
    assert(frame.payload_length <= PROTOCOL_MAX_PAYLOAD_SIZE);
    free(fixture);
}

static void test_maximum_frames(void)
{
    protocol_frame_t frame = {
        .version = PROTOCOL_VERSION,
        .message_type = PROTOCOL_TYPE_PUSH_TIMER,
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

static void test_config_brightness(void)
{
    const char *valid[] = {"apply_config_min.bin", "apply_config_brightness_min.bin",
                          "apply_config_brightness_default.bin", "apply_config_brightness_max.bin"};
    const uint8_t levels[] = {0U, 26U, 200U, 255U};
    for (size_t i = 0; i < 4; ++i) {
        size_t length = 0;
        uint8_t *wire = read_fixture(valid[i], &length);
        protocol_frame_t frame;
        protocol_message_t message;
        assert(protocol_frame_decode(wire, length, &frame) == PROTOCOL_FRAME_OK);
        assert(protocol_message_decode(&frame, &message) == PROTOCOL_MESSAGE_OK);
        assert(message.value.apply_config.has_brightness == (i != 0));
        assert(message.value.apply_config.brightness == levels[i]);
        assert_valid_fixture(valid[i], PROTOCOL_TYPE_APPLY_CONFIG, i == 0 ? 10U : 40U);
        free(wire);
    }
    const char *invalid[] = {"brightness_zero.bin", "brightness_below_floor.bin",
                            "brightness_overflow.bin", "brightness_negative.bin",
                            "brightness_boolean.bin"};
    for (size_t i = 0; i < 5; ++i) {
        size_t length = 0;
        uint8_t *wire = read_fixture(invalid[i], &length);
        protocol_frame_t frame;
        protocol_message_t message;
        assert(protocol_frame_decode(wire, length, &frame) == PROTOCOL_FRAME_OK);
        assert(protocol_message_decode(&frame, &message) != PROTOCOL_MESSAGE_OK);
        free(wire);
    }
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
    /* Four pairs always: key 3 is emitted even when false, matching the wire. */
    payload[offset++] = 0xa4U;
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

static protocol_message_result_t decode_asset_begin_encoding_fixture(
    bool volatile_tier, uint32_t total_length, uint8_t encoding,
    bool has_decoded_length, uint32_t decoded_length,
    protocol_message_t *message)
{
    uint8_t digest[ASSET_DIGEST_BYTES];
    memset(digest, 0x6b, sizeof digest);
    uint8_t payload[80];
    /* Key 3 is ALWAYS emitted on the real wire, false included -- every
     * deployed decoder has REQUIRED_BIT(3). Omitting it when false made the
     * durable cases below decode as a missing field rather than as the durable
     * transfers they were meant to be, which hid the tier rule entirely. */
    size_t pair_count = 5U + (has_decoded_length ? 1U : 0U);
    size_t offset = 0U;
    payload[offset++] = (uint8_t)(0xa0U | pair_count);
    offset = append_uint_cbor(payload, offset, 0U);
    offset = append_bstr_cbor(payload, offset, digest, sizeof digest);
    offset = append_uint_cbor(payload, offset, 1U);
    offset = append_uint_cbor(payload, offset, ASSET_KIND_IMAGE);
    offset = append_uint_cbor(payload, offset, 2U);
    offset = append_uint_cbor(payload, offset, total_length);
    offset = append_uint_cbor(payload, offset, 3U);
    offset = append_bool_cbor(payload, offset, volatile_tier);
    offset = append_uint_cbor(payload, offset, 4U);
    offset = append_uint_cbor(payload, offset, encoding);
    if (has_decoded_length) {
        offset = append_uint_cbor(payload, offset, 5U);
        offset = append_uint_cbor(payload, offset, decoded_length);
    }
    protocol_frame_t frame = {
        .version = PROTOCOL_VERSION,
        .message_type = PROTOCOL_TYPE_ASSET_BEGIN,
        .request_id = 45U,
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
    assert(message.value.asset_begin.encoding == PROTOCOL_ASSET_ENCODING_RAW);
    assert(!message.value.asset_begin.has_decoded_length);
    assert(memcmp(message.value.asset_begin.digest, digest, ASSET_DIGEST_BYTES) == 0);
}

static void test_asset_begin_rle_relationships_are_validated(void)
{
    protocol_message_t message;
    assert(decode_asset_begin_encoding_fixture(
               true, 10032U, PROTOCOL_ASSET_ENCODING_RLE565, true,
               PROTOCOL_VOLATILE_IMAGE_DECODED_LENGTH, &message) ==
           PROTOCOL_MESSAGE_OK);
    assert(message.value.asset_begin.encoding ==
           PROTOCOL_ASSET_ENCODING_RLE565);
    assert(message.value.asset_begin.has_decoded_length);
    assert(message.value.asset_begin.decoded_length ==
           PROTOCOL_VOLATILE_IMAGE_DECODED_LENGTH);

    assert(decode_asset_begin_encoding_fixture(
               true, 10032U, 2U, true,
               PROTOCOL_VOLATILE_IMAGE_DECODED_LENGTH, &message) ==
           PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
    assert(decode_asset_begin_encoding_fixture(
               true, 10032U, PROTOCOL_ASSET_ENCODING_RAW, true,
               PROTOCOL_VOLATILE_IMAGE_DECODED_LENGTH, &message) ==
           PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
    assert(decode_asset_begin_encoding_fixture(
               true, 10032U, PROTOCOL_ASSET_ENCODING_RLE565, false, 0U,
               &message) == PROTOCOL_MESSAGE_ERR_MISSING_FIELD);
    /* Durable + encoded: rejected outright until bit 10, and rejecting it is
     * what made a picture card's frame cross as 172 raw chunks. */
    assert(decode_asset_begin_encoding_fixture(
               false, 10032U, PROTOCOL_ASSET_ENCODING_RLE565, true,
               PROTOCOL_VOLATILE_IMAGE_DECODED_LENGTH, &message) ==
           PROTOCOL_MESSAGE_OK);
    /* A durable image may be any bounded size; a volatile one may not. */
    assert(decode_asset_begin_encoding_fixture(
               false, 1000U, PROTOCOL_ASSET_ENCODING_RLE565, true, 4096U,
               &message) == PROTOCOL_MESSAGE_OK);
    assert(decode_asset_begin_encoding_fixture(
               true, 1000U, PROTOCOL_ASSET_ENCODING_RLE565, true, 4096U,
               &message) == PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
    assert(decode_asset_begin_encoding_fixture(
               true, 10032U, PROTOCOL_ASSET_ENCODING_RLE565, true, 0U,
               &message) == PROTOCOL_MESSAGE_ERR_TOO_LARGE);
    assert(decode_asset_begin_encoding_fixture(
               true, 10032U, PROTOCOL_ASSET_ENCODING_RLE565, true,
               ASSET_MAX_BYTES + 1U, &message) ==
           PROTOCOL_MESSAGE_ERR_TOO_LARGE);
    assert(decode_asset_begin_encoding_fixture(
               true, PROTOCOL_VOLATILE_IMAGE_DECODED_LENGTH,
               PROTOCOL_ASSET_ENCODING_RLE565, true,
               PROTOCOL_VOLATILE_IMAGE_DECODED_LENGTH, &message) ==
           PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
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

static void test_current_capabilities_is_16352(void)
{
    /* Bit 7 sat defined-but-dark for most of V2 and a conforming host could
     * not provision the device, so the NUMBER is pinned, not just the
     * expression. Protocol v2 re-based it: bits 0-4 described a device that
     * rendered templates itself and are retired, never re-issued, and the
     * current word includes the additive bit-12 chunk window. */
    assert(PROTOCOL_CURRENT_CAPABILITIES == UINT64_C(16352));
    /* And pin the newest bit on its own, so a future edit that drops it from
     * the union fails here rather than at a host that gates on it. */
    assert((PROTOCOL_CURRENT_CAPABILITIES &
            PROTOCOL_CAPABILITY_SCENE_RENDER) != 0U);
    assert(PROTOCOL_CAPABILITY_PIPELINED_ASSET_CHUNKS == UINT64_C(4096));
    assert(PROTOCOL_CAPABILITY_SCENE_RENDER == UINT64_C(256));
    assert(PROTOCOL_CAPABILITY_VOLATILE_ASSETS == UINT64_C(512));
    assert(PROTOCOL_CAPABILITY_DURABLE_ASSET_ENCODING == UINT64_C(1024));
    assert((PROTOCOL_CURRENT_CAPABILITIES &
            PROTOCOL_CAPABILITY_DURABLE_ASSET_ENCODING) != 0U);
    assert((PROTOCOL_CURRENT_CAPABILITIES &
            PROTOCOL_CAPABILITY_VOLATILE_ASSETS) != 0U);
}

static void test_volatile_asset_begin_is_gated_on_its_own_capability(void)
{
    protocol_asset_begin_t begin;
    memset(&begin, 0, sizeof begin);
    begin.volatile_tier = true;

    assert(protocol_asset_begin_request_gate(
               &begin, PROTOCOL_CURRENT_CAPABILITIES) ==
           PROTOCOL_REQUEST_DISPATCHABLE);
    assert(protocol_asset_begin_request_gate(
               &begin, PROTOCOL_CURRENT_CAPABILITIES &
                           ~PROTOCOL_CAPABILITY_VOLATILE_ASSETS) ==
           PROTOCOL_REQUEST_MISSING_CAPABILITY);

    begin.volatile_tier = false;
    assert(protocol_asset_begin_request_gate(
               &begin, PROTOCOL_CURRENT_CAPABILITIES &
                           ~PROTOCOL_CAPABILITY_VOLATILE_ASSETS) ==
           PROTOCOL_REQUEST_DISPATCHABLE);
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

/* Writes {0: card_id, 1: revision, 2: {0: 9, 1: 0, 2: [rect x node_count]}},
 * and with `trailing_unknown_key` a fourth entry {3: 42} AFTER the scene. */
static size_t build_push_scene_payload_with(uint8_t *out, const char *card_id,
                                            uint32_t revision,
                                            size_t node_count,
                                            bool trailing_unknown_key)
{
    size_t offset = 0U;
    offset = append_map_header_cbor(out, offset, trailing_unknown_key ? 4U : 3U);
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
    if (trailing_unknown_key) {
        offset = append_uint_cbor(out, offset, 5U);
        offset = append_uint_cbor(out, offset, 42U);
    }
    return offset;
}

static size_t build_push_scene_payload(uint8_t *out, const char *card_id,
                                       uint32_t revision, size_t node_count)
{
    return build_push_scene_payload_with(out, card_id, revision, node_count,
                                         false);
}

static void test_local_tap_payload_bounds(void)
{
    const uint8_t scene[] = {0xa3U, 0U, 1U, 1U, 0U, 2U, 0x80U};
    uint8_t payload[PROTOCOL_MAX_PAYLOAD_SIZE];
    protocol_message_t *message = malloc(sizeof(*message));
    assert(message != NULL);
    for (unsigned count = 0U; count <= 5U; ++count) {
        size_t length = build_push_scene_payload(payload, "card", 1U, 1U);
        payload[0] = 0xa4U;
        payload[length++] = 3U;
        payload[length++] = (uint8_t)(0x80U + count);
        for (unsigned i = 0U; i < count; ++i) {
            memcpy(payload + length, scene, sizeof(scene));
            length += sizeof(scene);
        }
        protocol_message_result_t result = decode_push_scene_frame(payload, length, message);
        assert((result == PROTOCOL_MESSAGE_OK) == (count >= 1U && count <= 4U));
        if (result == PROTOCOL_MESSAGE_OK) {
            assert(message->value.push_scene.tap_view_count == count);
            assert(message->value.push_scene.tap_wrap);
            /* The main scene was not replaced by the last decoded view. */
            assert(message->value.push_scene.scene.revision == 9U);
            payload[length - 1U] = 0xf5U;
            assert(decode_push_scene_frame(payload, length, message) != PROTOCOL_MESSAGE_OK);
        }
    }
    size_t length = build_push_scene_payload(payload, "card", 1U, 1U);
    payload[0] = 0xa4U;
    payload[length++] = 4U;
    payload[length++] = 0xf4U;
    assert(decode_push_scene_frame(payload, length, message) != PROTOCOL_MESSAGE_OK);
    /* The new event fields fit existing padding: event queue statics do not grow. */
    assert(sizeof(protocol_device_event_t) == 64U);
    free(message);
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

/* THE test for the one line the extraction to scene_decode_map() genuinely
 * added: cbor_value_leave_container() at the end of that function.
 *
 * Every other PushScene test and both fixtures end the payload map at key 2,
 * so the loop exits the moment the scene is decoded and a parent cursor left
 * parked inside the scene map is never read. Nothing would notice it. That
 * matters because docs/protocol/v1.md promises unknown integer keys are
 * SKIPPED so a later revision can add a field, and for this message that
 * promise rests entirely on that one line.
 *
 * Confirmed to fail with the leave_container call removed -- decoding the
 * trailing key 5 off a mispositioned cursor does not yield a payload the
 * decoder accepts. Do not "simplify" this back into the roundtrip test by
 * dropping the trailing key. */
static void test_push_scene_skips_an_unknown_key_after_the_scene(void)
{
    uint8_t payload[PROTOCOL_MAX_PAYLOAD_SIZE];
    size_t offset =
        build_push_scene_payload_with(payload, "clock", 6U, 2U, true);

    protocol_message_t *message = malloc(sizeof *message);
    assert(message != NULL);
    assert(decode_push_scene_frame(payload, offset, message) ==
           PROTOCOL_MESSAGE_OK);
    /* Not merely accepted -- the scene either side of the skipped key is
     * still fully and correctly decoded. */
    assert(strcmp(message->value.push_scene.card_id, "clock") == 0);
    assert(message->value.push_scene.revision == 6U);
    assert(message->value.push_scene.scene.revision == 9U);
    assert(message->value.push_scene.scene.node_count == 2U);
    assert(message->value.push_scene.scene.nodes[1].kind == SCENE_NODE_RECT);
    assert(message->value.push_scene.scene.nodes[1].value.rect.h == 11);
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
    assert(decode_ack_for(PROTOCOL_TYPE_ACTIVATE_CARD, true) ==
           PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
}

/* ------------------------------------------------------------------
 * The device's request-admission policy (protocol_message_request_gate).
 *
 * This is the one part of link/protocol_task.c's dispatch a host test can
 * reach, and it is here because the whole point of Task 10 is closing a gap
 * between what StatusResponse advertises and what dispatch answers.
 * ------------------------------------------------------------------ */

static void test_every_advertised_capability_is_actually_dispatchable(void)
{
    /* The defect this pins, stated as an invariant rather than as a list:
     * for every message type gated on a capability bit, the bit being
     * advertised in PROTOCOL_CURRENT_CAPABILITIES and the type being
     * dispatched must be the same fact. Bit 7 once sat defined-but-dark, so
     * a conforming host could not provision the device; bit 8 shipped set
     * while dispatch still answered UnsupportedMessage for type 19, which is
     * worse, because the host acts on the advertisement. */
    static const struct {
        protocol_message_type_t type;
        uint64_t capability;
    } gated[] = {
        {PROTOCOL_TYPE_NETWORK_CONFIG, PROTOCOL_CAPABILITY_NETWORKING},
        {PROTOCOL_TYPE_FACTORY_RESET, PROTOCOL_CAPABILITY_NETWORKING},
        {PROTOCOL_TYPE_ASSET_BEGIN, PROTOCOL_CAPABILITY_ASSET_TRANSFER},
        {PROTOCOL_TYPE_ASSET_CHUNK, PROTOCOL_CAPABILITY_ASSET_TRANSFER},
        {PROTOCOL_TYPE_ASSET_COMMIT, PROTOCOL_CAPABILITY_ASSET_TRANSFER},
        {PROTOCOL_TYPE_ASSET_RELEASE, PROTOCOL_CAPABILITY_ASSET_TRANSFER},
        {PROTOCOL_TYPE_PUSH_SCENE, PROTOCOL_CAPABILITY_SCENE_RENDER},
    };
    for (size_t i = 0U; i < sizeof gated / sizeof gated[0]; ++i) {
        assert((PROTOCOL_CURRENT_CAPABILITIES & gated[i].capability) != 0U);
        assert(protocol_message_request_gate(
                   gated[i].type, PROTOCOL_CURRENT_CAPABILITIES) ==
               PROTOCOL_REQUEST_DISPATCHABLE);
        /* And the gate is a gate: strip the bit and the same type is
         * refused for the capability, not accepted anyway. */
        assert(protocol_message_request_gate(
                   gated[i].type,
                   PROTOCOL_CURRENT_CAPABILITIES & ~gated[i].capability) ==
               PROTOCOL_REQUEST_MISSING_CAPABILITY);
    }
}

static void test_response_types_are_not_dispatchable_as_requests(void)
{
    static const protocol_message_type_t responses[] = {
        PROTOCOL_TYPE_STATUS_RESPONSE, PROTOCOL_TYPE_ACK,
        PROTOCOL_TYPE_HEARTBEAT_ACK,   PROTOCOL_TYPE_ERROR,
        PROTOCOL_TYPE_DEVICE_EVENT,
    };
    for (size_t i = 0U; i < sizeof responses / sizeof responses[0]; ++i) {
        assert(protocol_message_request_gate(
                   responses[i], PROTOCOL_CURRENT_CAPABILITIES) ==
               PROTOCOL_REQUEST_NOT_A_REQUEST);
    }
    /* PushScene must not be swept up by that rule -- it was, before Task 10,
     * and the device answered a type-19 frame with "response type sent as
     * request". */
    assert(protocol_message_request_gate(PROTOCOL_TYPE_PUSH_SCENE,
                                         PROTOCOL_CURRENT_CAPABILITIES) !=
           PROTOCOL_REQUEST_NOT_A_REQUEST);
}

/* Builds the roundtrip test's two-node scene with `binding` on the TEXT
 * node, so two calls differ in exactly one string. */
static size_t build_push_scene_payload_with_binding(uint8_t *payload,
                                                    const char *binding)
{
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
    offset = append_array_header_cbor(payload, offset, 1U);
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
    offset = append_text_cbor(payload, offset, binding);
    return offset;
}

/* The malformed-scene path, written to survive this plan's recurring trap.
 *
 * scene_decode() calls scene_model_validate(), which re-checks several of
 * the same bounds, so a payload that is merely "one past a cap" is refused
 * identically by a correct device and by a broken one -- a mutation sweep
 * found six survivors of exactly that shape. So the malformation here is a
 * binding the device cannot evaluate, which ONLY the decoder refuses:
 * scene_model_validate() checks that a binding array is NUL-terminated and
 * within SCENE_MAX_BINDING and deliberately does not parse it (see
 * ui/scene_view.c's header comment listing what the model does and does not
 * guarantee). One layer, not two.
 *
 * WHAT THIS TEST DOES NOT COVER, stated plainly because it would be easy to
 * read the assertions as covering it. The second assertion pins the GATE
 * TABLE, not the DISPATCH ARM. A device whose gate answers DISPATCHABLE
 * while dispatch_request()'s switch has no `case PROTOCOL_TYPE_PUSH_SCENE`
 * still passes this test. Nothing here reaches dispatch_push_scene() at all
 * -- not its scene_card_id restore on refusal, not its Busy/InvalidPayload
 * split, not its "the previous scene stays up" property -- because
 * link/protocol_task.c is an ESP-IDF-only translation unit that no host
 * binary can link, and extracting those decisions into core/ would mean
 * moving LVGL and lvgl_port calls there. That is a real gap; it is covered
 * only by Task 11 on the board. What the gate table being wrong DOES catch
 * is the specific defect Task 10 existed to close (bit advertised, type
 * declared not-a-request), and dispatch_request()'s `default` arm now
 * answers PROTOCOL_ERROR_INTERNAL rather than falling through silently, so
 * a missing arm is at least a visible error to the host instead of no
 * reply at all.
 */
static void test_a_malformed_scene_is_refused_by_content_not_by_type(void)
{
    uint8_t payload[PROTOCOL_MAX_PAYLOAD_SIZE];
    protocol_message_t *message = malloc(sizeof *message);
    assert(message != NULL);

    size_t offset =
        build_push_scene_payload_with_binding(payload, "shell:rm -rf /");
    assert(decode_push_scene_frame(payload, offset, message) ==
           PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
    /* Refused whole. link/protocol_task.c never reaches scene_view_show()
     * on this path, so the panel keeps whatever it was showing; there is no
     * half-decoded scene here for a careless dispatch to render. */
    assert(message->value.push_scene.scene.node_count == 0U);

    assert(protocol_message_request_gate(PROTOCOL_TYPE_PUSH_SCENE,
                                         PROTOCOL_CURRENT_CAPABILITIES) ==
           PROTOCOL_REQUEST_DISPATCHABLE);

    /* The same payload with an evaluable binding is accepted, so the binding
     * is the only reason the first one was not. */
    offset = build_push_scene_payload_with_binding(payload, "time:HH:mm");
    assert(decode_push_scene_frame(payload, offset, message) ==
           PROTOCOL_MESSAGE_OK);
    assert(message->value.push_scene.scene.node_count == 1U);
    free(message);
}

int main(void)
{
    test_crc();
    test_status_request_accepts_the_maximum_request_id();
    test_valid_fixtures();
    test_scene_fixtures_pin_new_fields_and_omitted_defaults();
    test_status_capability_handshake_and_legacy_defaults();
    test_status_last_ota_error_round_trip_and_legacy_default();
    test_status_asset_store_stats_round_trip_and_legacy_default();
    test_status_volatile_pool_stats_round_trip_and_legacy_default();
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
    test_config_brightness();
    test_asset_begin_roundtrips();
    test_asset_begin_rle_relationships_are_validated();
    test_asset_begin_rejects_short_digest();
    test_asset_chunk_rejects_oversize_data();
    test_asset_release_rejects_too_many_digests();
    test_ack_carries_already_present_only_for_asset_begin();
    test_current_capabilities_is_16352();
    test_volatile_asset_begin_is_gated_on_its_own_capability();
    test_push_scene_roundtrips();
    test_local_tap_payload_bounds();
    test_push_scene_rejects_a_scene_over_the_node_cap();
    test_push_scene_rejects_an_off_canvas_node();
    test_push_scene_skips_an_unknown_key_after_the_scene();
    test_push_scene_rejects_a_zero_revision();
    test_ack_for_push_scene_requires_a_revision();
    test_every_advertised_capability_is_actually_dispatchable();
    test_response_types_are_not_dispatchable_as_requests();
    test_a_malformed_scene_is_refused_by_content_not_by_type();
    puts("test_protocol: OK");
    return 0;
}
