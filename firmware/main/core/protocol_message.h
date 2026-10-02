#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "core/asset_store.h"
#include "core/scene_model.h"
#include "protocol_frame.h"

#define PROTOCOL_LINK_TIMEOUT_MS 10000U
/* A card id is an identifier the host chose, not free text. Protocol v1 had
 * one of these per name for the same thing -- a widget, a screen and a card --
 * all 32; config schema v10 settled them as one card. */
#define PROTOCOL_MAX_CARD_ID_LENGTH 32U
#define PROTOCOL_MAX_CONFIG_CARDS 8U
#define PROTOCOL_MAX_DIAGNOSTIC_LENGTH 96U
#define PROTOCOL_MAX_INTERRUPT_REASON_LENGTH 96U
#define PROTOCOL_MAX_FIRMWARE_VERSION_LENGTH 32U
#define PROTOCOL_MIN_UNIX_SECONDS INT64_C(1577836800)
#define PROTOCOL_MAX_UNIX_SECONDS INT64_C(4102444800)
#define PROTOCOL_MIN_UTC_OFFSET_MINUTES (-840)
#define PROTOCOL_MAX_UTC_OFFSET_MINUTES 840
#define PROTOCOL_MAX_VERSION 2U
#define PROTOCOL_CAPABILITY_CORE_WIDGETS (UINT64_C(1) << 0)
#define PROTOCOL_CAPABILITY_CONFIG_ROTATION (UINT64_C(1) << 1)
#define PROTOCOL_CAPABILITY_DASHBOARD_LAYOUTS (UINT64_C(1) << 2)
#define PROTOCOL_CAPABILITY_EXTENDED_TEMPLATES (UINT64_C(1) << 3)
#define PROTOCOL_CAPABILITY_HOST_TAP_ACTIONS (UINT64_C(1) << 4)
#define PROTOCOL_CAPABILITY_ASSET_TRANSFER (UINT64_C(1) << 5)
#define PROTOCOL_CAPABILITY_FIRMWARE_UPDATE (UINT64_C(1) << 6)
#define PROTOCOL_CAPABILITY_NETWORKING (UINT64_C(1) << 7)
/* Gates PushScene (type 19). A host that does not see this bit must not
 * send one -- the same contract NetworkConfig and FactoryReset have under
 * bit 7. Defining a bit is not switching it on: bit 7 sat defined-but-dark
 * for most of V2 and the constant read 75 instead of 203, so the value
 * below is pinned by a test in both languages. */
#define PROTOCOL_CAPABILITY_SCENE_RENDER (UINT64_C(1) << 8)
/* A build advertising this bit accepts `AssetBegin { volatile: true }` and
 * resolves committed volatile image bytes. Bit 5 cannot carry that promise:
 * deployed bit-5 builds explicitly reject volatile begins. */
#define PROTOCOL_CAPABILITY_VOLATILE_ASSETS (UINT64_C(1) << 9)
/* A build advertising this bit honours AssetBegin's `encoding` and
 * `decoded_length` on the DURABLE tier too, not only the volatile one. Bit 9
 * cannot carry that promise for the same reason bit 5 could not carry bit 9's:
 * a deployed bit-9 build's durable branch ignores `encoding` and writes wire
 * bytes to flash verbatim, so an RLE565 durable asset would be stored as if the
 * compressed bytes were pixels. */
#define PROTOCOL_CAPABILITY_DURABLE_ASSET_ENCODING (UINT64_C(1) << 10)
#define PROTOCOL_CAPABILITY_DISPLAY_BRIGHTNESS (UINT64_C(1) << 11)
#define PROTOCOL_MIN_DISPLAY_BRIGHTNESS 26U
/* Protocol v2 retired bits 0-4: they described a device that rendered
 * templates itself, which this one has not since stage 3a. The numbers are NOT
 * re-used -- a bit's meaning is its identity. */
#define PROTOCOL_CURRENT_CAPABILITIES                            \
    (PROTOCOL_CAPABILITY_ASSET_TRANSFER |                        \
     PROTOCOL_CAPABILITY_FIRMWARE_UPDATE |                       \
     PROTOCOL_CAPABILITY_NETWORKING |                            \
     PROTOCOL_CAPABILITY_SCENE_RENDER |                          \
     PROTOCOL_CAPABILITY_VOLATILE_ASSETS |                       \
     PROTOCOL_CAPABILITY_DURABLE_ASSET_ENCODING |                 \
     PROTOCOL_CAPABILITY_DISPLAY_BRIGHTNESS)
#define PROTOCOL_MAX_SSID_LENGTH 32U
#define PROTOCOL_MAX_PSK_LENGTH 64U
#define PROTOCOL_MAX_SERVER_URL_LENGTH 128U
#define PROTOCOL_MAX_DEVICE_TOKEN_LENGTH 128U
#define PROTOCOL_MAX_DEVICE_ID_LENGTH 32U
#define PROTOCOL_MAX_IP_LENGTH 15U
/* 1920, not the 2034-byte envelope cap: the CBOR map header, the 32-byte
 * digest with its bstr header, the offset key/value, and the data bstr
 * header cost roughly 46 bytes; this leaves deliberate margin. */
#define PROTOCOL_MAX_ASSET_CHUNK_BYTES 1920U
#define PROTOCOL_MAX_ASSET_DIGESTS 32U
#define PROTOCOL_ASSET_ENCODING_RAW 0U
#define PROTOCOL_ASSET_ENCODING_RLE565 1U
#define PROTOCOL_VOLATILE_IMAGE_DECODED_LENGTH 329740U
typedef enum {
    PROTOCOL_TYPE_STATUS_REQUEST = 1,
    PROTOCOL_TYPE_STATUS_RESPONSE = 2,
    PROTOCOL_TYPE_TIME_SYNC = 3,
    PROTOCOL_TYPE_ACK = 4,
    PROTOCOL_TYPE_PUSH_TIMER = 5,
    PROTOCOL_TYPE_HEARTBEAT = 6,
    PROTOCOL_TYPE_HEARTBEAT_ACK = 7,
    PROTOCOL_TYPE_ERROR = 8,
    PROTOCOL_TYPE_APPLY_CONFIG = 9,
    PROTOCOL_TYPE_ACTIVATE_CARD = 10,
    PROTOCOL_TYPE_TRIGGER_INTERRUPT = 11,
    PROTOCOL_TYPE_DEVICE_EVENT = 12,
    PROTOCOL_TYPE_NETWORK_CONFIG = 13,
    PROTOCOL_TYPE_FACTORY_RESET = 14,
    PROTOCOL_TYPE_ASSET_BEGIN = 15,
    PROTOCOL_TYPE_ASSET_CHUNK = 16,
    PROTOCOL_TYPE_ASSET_COMMIT = 17,
    PROTOCOL_TYPE_ASSET_RELEASE = 18,
    PROTOCOL_TYPE_PUSH_SCENE = 19,
} protocol_message_type_t;

typedef enum {
    PROTOCOL_ERROR_MALFORMED_FRAME = 1,
    PROTOCOL_ERROR_BAD_CHECKSUM = 2,
    PROTOCOL_ERROR_VERSION_MISMATCH = 3,
    PROTOCOL_ERROR_UNSUPPORTED_MESSAGE = 4,
    PROTOCOL_ERROR_INVALID_PAYLOAD = 5,
    PROTOCOL_ERROR_INVALID_TIME = 6,
    PROTOCOL_ERROR_STALE_REVISION = 7,
    PROTOCOL_ERROR_BUSY = 8,
    PROTOCOL_ERROR_INTERNAL = 9,
    PROTOCOL_ERROR_UNKNOWN_WIDGET = 10,
    PROTOCOL_ERROR_CONFIG_TOO_LARGE = 14,
    /* Returned when a message is valid but not permitted on this transport
     * in the device's current tier. */
    PROTOCOL_ERROR_WRONG_TIER = 15,
} protocol_error_code_t;

typedef enum {
    PROTOCOL_TAP_NONE = 0,
    PROTOCOL_TAP_START_PAUSE = 1,
    PROTOCOL_TAP_RESET = 2,
} protocol_tap_action_t;

typedef enum {
    PROTOCOL_INTERRUPT_DISABLED = 0,
    PROTOCOL_INTERRUPT_ENABLED = 1,
} protocol_interrupt_policy_t;

typedef enum {
    PROTOCOL_EVENT_TAP = 1,
    PROTOCOL_EVENT_NAVIGATION = 2,
    PROTOCOL_EVENT_INTERRUPT_DISMISSED = 3,
} protocol_event_kind_t;

typedef enum {
    PROTOCOL_EVENT_ACTION_NONE = 0,
    PROTOCOL_EVENT_ACTION_START_PAUSE = 1,
    PROTOCOL_EVENT_ACTION_RESET = 2,
    PROTOCOL_EVENT_ACTION_NAVIGATE_PREVIOUS = 3,
    PROTOCOL_EVENT_ACTION_NAVIGATE_NEXT = 4,
    PROTOCOL_EVENT_ACTION_DISMISS_INTERRUPT = 5,
} protocol_event_action_t;

typedef enum {
    PROTOCOL_TIER_LOCAL = 0,
    PROTOCOL_TIER_NETWORKED = 1,
} protocol_tier_t;

typedef enum {
    PROTOCOL_WIFI_DOWN = 0,
    PROTOCOL_WIFI_CONNECTING = 1,
    PROTOCOL_WIFI_CONNECTED = 2,
    PROTOCOL_WIFI_FAILED = 3,
} protocol_wifi_state_t;

typedef enum {
    PROTOCOL_OTA_IDLE = 0,
    PROTOCOL_OTA_CHECKING = 1,
    PROTOCOL_OTA_DOWNLOADING = 2,
    PROTOCOL_OTA_PENDING_VERIFY = 3,
    PROTOCOL_OTA_FAILED = 4,
} protocol_ota_state_t;

typedef struct {
    char ssid[PROTOCOL_MAX_SSID_LENGTH + 1U];
    char psk[PROTOCOL_MAX_PSK_LENGTH + 1U];
    char server_url[PROTOCOL_MAX_SERVER_URL_LENGTH + 1U];
    char device_id[PROTOCOL_MAX_DEVICE_ID_LENGTH + 1U];
    char token[PROTOCOL_MAX_DEVICE_TOKEN_LENGTH + 1U];
    int16_t utc_offset_minutes;
    protocol_tier_t tier;
} protocol_network_config_t;

typedef struct {
    uint8_t digest[ASSET_DIGEST_BYTES];
    asset_kind_t kind;
    uint32_t total_length;
    bool volatile_tier;
    uint8_t encoding;
    bool has_decoded_length;
    uint32_t decoded_length;
} protocol_asset_begin_t;

typedef struct {
    uint8_t digest[ASSET_DIGEST_BYTES];
    uint32_t offset;
    uint8_t data[PROTOCOL_MAX_ASSET_CHUNK_BYTES];
    size_t data_length;
} protocol_asset_chunk_t;

typedef struct {
    uint8_t digest[ASSET_DIGEST_BYTES];
} protocol_asset_commit_t;

typedef struct {
    uint8_t digests[PROTOCOL_MAX_ASSET_DIGESTS][ASSET_DIGEST_BYTES];
    size_t digest_count;
} protocol_asset_release_t;

/* One card in the loop. Protocol v2 carries only what the device decides for
 * itself: which card this is, and what a tap does. */
typedef struct {
    char card_id[PROTOCOL_MAX_CARD_ID_LENGTH + 1U];
    protocol_tap_action_t tap_action;
} protocol_card_config_t;

typedef struct {
    uint32_t revision;
    uint16_t rotation;
    uint8_t brightness;
    bool has_brightness;
    size_t card_count;
    protocol_card_config_t cards[PROTOCOL_MAX_CONFIG_CARDS];
} protocol_apply_config_t;

typedef struct {
    char card_id[PROTOCOL_MAX_CARD_ID_LENGTH + 1U];
} protocol_activate_card_t;

typedef struct {
    char card_id[PROTOCOL_MAX_CARD_ID_LENGTH + 1U];
    uint32_t token;
    char reason[PROTOCOL_MAX_INTERRUPT_REASON_LENGTH + 1U];
} protocol_trigger_interrupt_t;

/* Something the person at the panel did, reported upward. Protocol v1 carried
 * two identifiers, widget_id (key 2) and screen_id (key 3), and every producer
 * set them to the same card. v2 states the one card id in key 2; key 3 is
 * retired rather than reused. */
typedef struct {
    uint64_t sequence;
    protocol_event_kind_t kind;
    char card_id[PROTOCOL_MAX_CARD_ID_LENGTH + 1U];
    protocol_event_action_t action;
    bool has_interrupt_token;
    uint32_t interrupt_token;
} protocol_device_event_t;

/* The timer a card's `timer.*` scene bindings resolve against. Protocol v2
 * replaced PushData's arbitrary field bag with this: its only consumer read
 * three keys out of that bag by name, and nothing has bound `field.*` since
 * config schema v9. */
typedef struct {
    char card_id[PROTOCOL_MAX_CARD_ID_LENGTH + 1U];
    uint32_t revision;
    uint32_t total_ms;
    uint32_t remaining_ms;
    bool running;
} protocol_push_timer_t;

/* PushScene: one card's whole display list, replacing whatever that card
 * drew before. `scene` is embedded by value rather than pointed at because
 * protocol_message_t is the decoder's single destination and nothing here
 * allocates -- but it is ~6 KB, which makes protocol_message_t ~6.4 KB.
 * link/protocol_task.c holds its one instance inside the PSRAM-allocated
 * protocol_context_t, so this costs no internal DRAM and nothing on the
 * 8 KiB task stack; a new caller putting a protocol_message_t on a stack
 * would, and must not. */
typedef struct {
    char card_id[PROTOCOL_MAX_CARD_ID_LENGTH + 1U];
    uint32_t revision;
    scene_t scene;
} protocol_push_scene_t;

typedef struct {
    int64_t unix_seconds;
    int16_t utc_offset_minutes;
} protocol_time_sync_t;

typedef struct {
    uint8_t acknowledged_type;
    bool has_revision;
    uint32_t revision;
    bool has_already_present;
    bool already_present;
} protocol_ack_t;

typedef struct {
    uint8_t protocol_version;
    uint8_t max_protocol_version;
    uint64_t capabilities;
    char firmware_version[PROTOCOL_MAX_FIRMWARE_VERSION_LENGTH + 1U];
    uint64_t uptime_ms;
    uint32_t free_heap;
    uint16_t display_width;
    uint16_t display_height;
    uint8_t brightness;
    uint16_t rotation;
    bool online;
    uint32_t latest_revision;
    uint32_t valid_frames;
    uint32_t malformed_frames;
    uint32_t crc_errors;
    uint32_t overflow_frames;
    uint32_t dropped_responses;
    uint32_t rx_dropped_bytes;
    uint32_t dropped_events;
    uint32_t event_queue_high_water;
    uint32_t dropped_ui_commands;
    uint32_t ui_queue_high_water;
    uint32_t config_revision;
    uint32_t latest_interrupt_token;
    protocol_tier_t tier;
    protocol_wifi_state_t wifi_state;
    int8_t wifi_rssi;
    char ip[PROTOCOL_MAX_IP_LENGTH + 1U];
    protocol_ota_state_t ota_state;
    bool has_last_network_error;
    char last_network_error[PROTOCOL_MAX_DIAGNOSTIC_LENGTH + 1U];
    bool has_last_ota_error;
    char last_ota_error[PROTOCOL_MAX_DIAGNOSTIC_LENGTH + 1U];
    bool has_asset_store_stats;
    uint32_t asset_store_used_bytes;
    uint32_t asset_store_free_bytes;
    uint32_t asset_count;
    /* Key 32. The durable stats above describe flash; these describe the
     * volatile frame pool and the PSRAM heap it draws from, which is the only
     * place a host can learn whether the pool has room. */
    bool has_volatile_asset_stats;
    uint32_t volatile_committed_count;
    uint32_t volatile_slot_capacity;
    uint32_t volatile_used_bytes;
    uint32_t psram_free_bytes;
    uint32_t psram_low_water_bytes;
} protocol_status_response_t;

typedef struct {
    uint64_t uptime_ms;
} protocol_heartbeat_ack_t;

typedef struct {
    protocol_error_code_t code;
    char diagnostic[PROTOCOL_MAX_DIAGNOSTIC_LENGTH + 1U];
} protocol_error_response_t;

typedef struct {
    uint8_t type;
    union {
        protocol_status_response_t status;
        protocol_time_sync_t time_sync;
        protocol_ack_t ack;
        protocol_push_timer_t push_timer;
        protocol_heartbeat_ack_t heartbeat_ack;
        protocol_error_response_t error;
        protocol_apply_config_t apply_config;
        protocol_activate_card_t activate_card;
        protocol_trigger_interrupt_t trigger_interrupt;
        protocol_device_event_t device_event;
        protocol_network_config_t network_config;
        protocol_asset_begin_t asset_begin;
        protocol_asset_chunk_t asset_chunk;
        protocol_asset_commit_t asset_commit;
        protocol_asset_release_t asset_release;
        protocol_push_scene_t push_scene;
    } value;
} protocol_message_t;

typedef enum {
    PROTOCOL_MESSAGE_OK = 0,
    PROTOCOL_MESSAGE_ERR_ARGUMENT,
    PROTOCOL_MESSAGE_ERR_CBOR,
    PROTOCOL_MESSAGE_ERR_VERSION,
    PROTOCOL_MESSAGE_ERR_UNSUPPORTED_TYPE,
    PROTOCOL_MESSAGE_ERR_INVALID_VALUE,
    PROTOCOL_MESSAGE_ERR_INVALID_TIME,
    PROTOCOL_MESSAGE_ERR_MISSING_FIELD,
    PROTOCOL_MESSAGE_ERR_DUPLICATE_KEY,
    PROTOCOL_MESSAGE_ERR_TOO_LARGE,
    PROTOCOL_MESSAGE_ERR_FRAME,
    PROTOCOL_MESSAGE_ERR_INVALID_REQUEST_ID,
    PROTOCOL_MESSAGE_ERR_CONFIG_TOO_LARGE,
    PROTOCOL_MESSAGE_ERR_UNKNOWN_WIDGET,
} protocol_message_result_t;

typedef enum {
    PROTOCOL_REQUEST_DISPATCHABLE = 0,
    /* A response-only type (Ack, Error, StatusResponse, HeartbeatAck,
     * DeviceEvent) arriving as a request. */
    PROTOCOL_REQUEST_NOT_A_REQUEST,
    /* A request type whose capability bit this build does not advertise. */
    PROTOCOL_REQUEST_MISSING_CAPABILITY,
} protocol_request_gate_t;

/* The device's request-admission policy: whether a build advertising
 * `capabilities` dispatches `type` when it arrives as a request.
 *
 * It lives here, rather than inline in link/protocol_task.c where it is
 * used, for one reason: it is the only part of dispatch a host test can
 * reach, and the failure it guards against has bitten this project once
 * already in mirror image. Bit 7 was defined and never set, so a conforming
 * host could not provision the device at all; the inverse -- a bit
 * advertised whose messages are refused -- is worse, because the host acts
 * on the advertisement. Keeping the table and PROTOCOL_CURRENT_CAPABILITIES
 * in the same header lets test_protocol.c assert the two agree, for every
 * bit, rather than for whichever one someone remembered.
 *
 * `capabilities` is a parameter and not PROTOCOL_CURRENT_CAPABILITIES so
 * that a build profile omitting a bit is refused explicitly rather than
 * silently accepting messages its firmware did not compile support for. */
protocol_request_gate_t protocol_message_request_gate(
    protocol_message_type_t type,
    uint64_t capabilities);

/* The four asset-transfer message types remain gated by bit 5. This second
 * payload-aware gate adds bit 9 only for the volatile form of AssetBegin, so
 * an older bit-5 device never accepts a tier it cannot actually store. */
protocol_request_gate_t protocol_asset_begin_request_gate(
    const protocol_asset_begin_t *begin,
    uint64_t capabilities);

protocol_message_result_t protocol_message_decode(
    const protocol_frame_t *frame,
    protocol_message_t *message);

protocol_message_result_t protocol_message_encode(
    uint32_t request_id,
    const protocol_message_t *message,
    uint8_t *wire,
    size_t wire_capacity,
    size_t *wire_length);
