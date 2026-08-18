#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "protocol_frame.h"

#define PROTOCOL_LINK_TIMEOUT_MS 10000U
#define PROTOCOL_MAX_WIDGET_ID_LENGTH 32U
#define PROTOCOL_MAX_SCREEN_ID_LENGTH 32U
#define PROTOCOL_MAX_CONFIG_WIDGETS 8U
#define PROTOCOL_MAX_CONFIG_SCREENS 8U
#define PROTOCOL_MAX_FIELD_COUNT 16U
#define PROTOCOL_MAX_FIELD_KEY_LENGTH 32U
#define PROTOCOL_MAX_FIELD_TEXT_LENGTH 128U
#define PROTOCOL_MAX_DIAGNOSTIC_LENGTH 96U
#define PROTOCOL_MAX_INTERRUPT_REASON_LENGTH 96U
#define PROTOCOL_MAX_FIRMWARE_VERSION_LENGTH 32U
#define PROTOCOL_MIN_UNIX_SECONDS INT64_C(1577836800)
#define PROTOCOL_MAX_UNIX_SECONDS INT64_C(4102444800)
#define PROTOCOL_MIN_UTC_OFFSET_MINUTES (-840)
#define PROTOCOL_MAX_UTC_OFFSET_MINUTES 840
#define PROTOCOL_MAX_VERSION 1U
#define PROTOCOL_CAPABILITY_CORE_WIDGETS (UINT64_C(1) << 0)
#define PROTOCOL_CAPABILITY_CONFIG_ROTATION (UINT64_C(1) << 1)
#define PROTOCOL_CAPABILITY_DASHBOARD_LAYOUTS (UINT64_C(1) << 2)
#define PROTOCOL_CAPABILITY_EXTENDED_TEMPLATES (UINT64_C(1) << 3)
#define PROTOCOL_CAPABILITY_HOST_TAP_ACTIONS (UINT64_C(1) << 4)
#define PROTOCOL_CAPABILITY_ASSET_TRANSFER (UINT64_C(1) << 5)
#define PROTOCOL_CAPABILITY_FIRMWARE_UPDATE (UINT64_C(1) << 6)
#define PROTOCOL_CAPABILITY_NETWORKING (UINT64_C(1) << 7)
#define PROTOCOL_LEGACY_CAPABILITIES PROTOCOL_CAPABILITY_CORE_WIDGETS
#define PROTOCOL_CURRENT_CAPABILITIES                            \
    (PROTOCOL_CAPABILITY_CORE_WIDGETS |                          \
     PROTOCOL_CAPABILITY_CONFIG_ROTATION |                       \
     PROTOCOL_CAPABILITY_EXTENDED_TEMPLATES)
#define PROTOCOL_MAX_SSID_LENGTH 32U
#define PROTOCOL_MAX_PSK_LENGTH 64U
#define PROTOCOL_MAX_SERVER_URL_LENGTH 128U
#define PROTOCOL_MAX_DEVICE_TOKEN_LENGTH 128U
#define PROTOCOL_MAX_DEVICE_ID_LENGTH 32U
#define PROTOCOL_MAX_IP_LENGTH 15U

typedef enum {
    PROTOCOL_TYPE_STATUS_REQUEST = 1,
    PROTOCOL_TYPE_STATUS_RESPONSE = 2,
    PROTOCOL_TYPE_TIME_SYNC = 3,
    PROTOCOL_TYPE_ACK = 4,
    PROTOCOL_TYPE_PUSH_DATA = 5,
    PROTOCOL_TYPE_HEARTBEAT = 6,
    PROTOCOL_TYPE_HEARTBEAT_ACK = 7,
    PROTOCOL_TYPE_ERROR = 8,
    PROTOCOL_TYPE_APPLY_CONFIG = 9,
    PROTOCOL_TYPE_ACTIVATE_SCREEN = 10,
    PROTOCOL_TYPE_TRIGGER_INTERRUPT = 11,
    PROTOCOL_TYPE_DEVICE_EVENT = 12,
    PROTOCOL_TYPE_NETWORK_CONFIG = 13,
    PROTOCOL_TYPE_FACTORY_RESET = 14,
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
    PROTOCOL_ERROR_UNKNOWN_SCREEN = 11,
    PROTOCOL_ERROR_UNSUPPORTED_TEMPLATE = 12,
    PROTOCOL_ERROR_UNSUPPORTED_SIZE_CLASS = 13,
    PROTOCOL_ERROR_CONFIG_TOO_LARGE = 14,
    /* Returned when a message is valid but not permitted on this transport
     * in the device's current tier. */
    PROTOCOL_ERROR_WRONG_TIER = 15,
} protocol_error_code_t;

typedef enum {
    PROTOCOL_TEMPLATE_DIGITAL_CLOCK = 1,
    PROTOCOL_TEMPLATE_PROGRESS_RING = 2,
    PROTOCOL_TEMPLATE_ROW_LIST = 3,
    PROTOCOL_TEMPLATE_ANALOG_CLOCK = 4,
    PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL = 5,
    PROTOCOL_TEMPLATE_ICON_BADGE_TEXT = 6,
} protocol_template_kind_t;

typedef enum {
    PROTOCOL_SIZE_FULL = 1,
    PROTOCOL_SIZE_STANDARD = 2,
    PROTOCOL_SIZE_TILE = 3,
} protocol_size_class_t;

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
    char widget_id[PROTOCOL_MAX_WIDGET_ID_LENGTH + 1U];
    protocol_template_kind_t template_kind;
    protocol_size_class_t size_class;
    protocol_tap_action_t tap_action;
    protocol_interrupt_policy_t interrupt_policy;
} protocol_widget_config_t;

typedef struct {
    char screen_id[PROTOCOL_MAX_SCREEN_ID_LENGTH + 1U];
    char widget_id[PROTOCOL_MAX_WIDGET_ID_LENGTH + 1U];
} protocol_screen_config_t;

typedef struct {
    uint32_t revision;
    uint16_t rotation;
    size_t widget_count;
    protocol_widget_config_t widgets[PROTOCOL_MAX_CONFIG_WIDGETS];
    size_t screen_count;
    protocol_screen_config_t screens[PROTOCOL_MAX_CONFIG_SCREENS];
} protocol_apply_config_t;

typedef struct {
    char screen_id[PROTOCOL_MAX_SCREEN_ID_LENGTH + 1U];
} protocol_activate_screen_t;

typedef struct {
    char widget_id[PROTOCOL_MAX_WIDGET_ID_LENGTH + 1U];
    uint32_t token;
    char reason[PROTOCOL_MAX_INTERRUPT_REASON_LENGTH + 1U];
} protocol_trigger_interrupt_t;

typedef struct {
    uint64_t sequence;
    protocol_event_kind_t kind;
    char widget_id[PROTOCOL_MAX_WIDGET_ID_LENGTH + 1U];
    char screen_id[PROTOCOL_MAX_SCREEN_ID_LENGTH + 1U];
    protocol_event_action_t action;
    bool has_interrupt_token;
    uint32_t interrupt_token;
} protocol_device_event_t;

typedef enum {
    PROTOCOL_FIELD_TEXT = 0,
    PROTOCOL_FIELD_INTEGER,
    PROTOCOL_FIELD_BOOLEAN,
} protocol_field_type_t;

typedef struct {
    char key[PROTOCOL_MAX_FIELD_KEY_LENGTH + 1U];
    protocol_field_type_t type;
    union {
        char text[PROTOCOL_MAX_FIELD_TEXT_LENGTH + 1U];
        int64_t integer;
        bool boolean;
    } value;
} protocol_field_t;

typedef struct {
    char widget_id[PROTOCOL_MAX_WIDGET_ID_LENGTH + 1U];
    uint32_t revision;
    size_t field_count;
    protocol_field_t fields[PROTOCOL_MAX_FIELD_COUNT];
} protocol_push_data_t;

typedef struct {
    int64_t unix_seconds;
    int16_t utc_offset_minutes;
} protocol_time_sync_t;

typedef struct {
    uint8_t acknowledged_type;
    bool has_revision;
    uint32_t revision;
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
        protocol_push_data_t push_data;
        protocol_heartbeat_ack_t heartbeat_ack;
        protocol_error_response_t error;
        protocol_apply_config_t apply_config;
        protocol_activate_screen_t activate_screen;
        protocol_trigger_interrupt_t trigger_interrupt;
        protocol_device_event_t device_event;
        protocol_network_config_t network_config;
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
    PROTOCOL_MESSAGE_ERR_UNSUPPORTED_TEMPLATE,
    PROTOCOL_MESSAGE_ERR_UNSUPPORTED_SIZE_CLASS,
    PROTOCOL_MESSAGE_ERR_CONFIG_TOO_LARGE,
    PROTOCOL_MESSAGE_ERR_UNKNOWN_WIDGET,
} protocol_message_result_t;

bool protocol_template_kind_valid(protocol_template_kind_t kind);

protocol_message_result_t protocol_message_decode(
    const protocol_frame_t *frame,
    protocol_message_t *message);

protocol_message_result_t protocol_message_encode(
    uint32_t request_id,
    const protocol_message_t *message,
    uint8_t *wire,
    size_t wire_capacity,
    size_t *wire_length);
