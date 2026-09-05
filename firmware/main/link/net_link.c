#include "link/net_link.h"

#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "core/protocol_message.h"
#include "core/net_ring.h"
#include "core/reconnect_backoff.h"
#include "esp_crt_bundle.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_random.h"
#include "esp_timer.h"
#include "esp_websocket_client.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "link/net_store.h"
#include "link/protocol_task.h"
#include "link/usb_link.h"
#include "link/wifi_station.h"

#define NET_LINK_RX_RING_CAPACITY (USB_LINK_MAX_WIRE_FRAME_SIZE * 4U)
#define NET_LINK_TIMEOUT_MS 45000U
#define NET_LINK_STABLE_CONNECTION_MS 10000U
#define NET_LINK_AUTH_HEADER_CAPACITY                                      \
    (PROTOCOL_MAX_DEVICE_TOKEN_LENGTH + sizeof("Authorization: Bearer \r\n"))

static const char *TAG = "net_link";

static esp_websocket_client_handle_t s_client;
static uint8_t *s_rx_storage;
static net_ring_t s_rx_ring;
static SemaphoreHandle_t s_rx_ready;
static portMUX_TYPE s_rx_lock = portMUX_INITIALIZER_UNLOCKED;
static reconnect_backoff_t s_reconnect_backoff;
static bool s_connected;
static int64_t s_connected_at_us;

static uint32_t reconnect_random(void *context)
{
    (void)context;
    return esp_random();
}

static const char *websocket_error_type_name(
    esp_websocket_error_type_t error_type)
{
    switch (error_type) {
    case WEBSOCKET_ERROR_TYPE_NONE:
        return "none";
    case WEBSOCKET_ERROR_TYPE_TCP_TRANSPORT:
        return "tcp_transport";
    case WEBSOCKET_ERROR_TYPE_PONG_TIMEOUT:
        return "pong_timeout";
    case WEBSOCKET_ERROR_TYPE_HANDSHAKE:
        return "handshake";
    case WEBSOCKET_ERROR_TYPE_SERVER_CLOSE:
        return "server_close";
    default:
        return "unknown";
    }
}

static void ring_append(const uint8_t *data, size_t length)
{
    if (data == NULL || length == 0U || s_rx_storage == NULL) {
        return;
    }

    portENTER_CRITICAL(&s_rx_lock);
    size_t accepted = net_ring_write(&s_rx_ring, data, length);
    portEXIT_CRITICAL(&s_rx_lock);

    if (accepted != 0U) {
        (void)xSemaphoreGive(s_rx_ready);
    }
}

static size_t ring_drain(uint8_t *out, size_t capacity)
{
    portENTER_CRITICAL(&s_rx_lock);
    size_t count = net_ring_read(&s_rx_ring, out, capacity);
    portEXIT_CRITICAL(&s_rx_lock);
    return count;
}

static void configure_next_reconnect(esp_websocket_client_handle_t client)
{
    // A failure while the station has no IP says nothing about the server's
    // availability, so it must not advance the curve. net_link_start() runs
    // at protocol-task init, before DHCP has finished, so the first attempts
    // of every boot fail on DNS by construction
    // (ESP_ERR_ESP_TLS_CANNOT_RESOLVE_HOSTNAME). Counting those doublings
    // means arriving at the first attempt that could have worked already
    // heavily delayed: a reboot on 2026-08-19 spent 28 s offline that way,
    // most of it accumulated before the network existed. Hold at the 1 s
    // floor until the link could actually carry a connection.
    uint32_t delay_ms;
    if (wifi_station_state() != PROTOCOL_WIFI_CONNECTED) {
        reconnect_backoff_reset(&s_reconnect_backoff);
        delay_ms = reconnect_backoff_peek_delay_ms(&s_reconnect_backoff);
    } else {
        delay_ms = reconnect_backoff_next_delay_ms(&s_reconnect_backoff);
    }
    esp_err_t result = esp_websocket_client_set_reconnect_timeout(
        client, (int)delay_ms);
    if (result != ESP_OK) {
        ESP_LOGW(TAG, "failed to configure reconnect timeout: %s",
                 esp_err_to_name(result));
    }
}

static void websocket_event_handler(void *handler_arg,
                                    esp_event_base_t event_base,
                                    int32_t event_id,
                                    void *event_data)
{
    (void)handler_arg;
    (void)event_base;
    esp_websocket_event_data_t *event = event_data;

    if (event_id == WEBSOCKET_EVENT_CONNECTED) {
        portENTER_CRITICAL(&s_rx_lock);
        net_ring_clear(&s_rx_ring);
        portEXIT_CRITICAL(&s_rx_lock);
        protocol_task_reset_network_decoder();
        s_connected = true;
        s_connected_at_us = esp_timer_get_time();
        ESP_LOGI(TAG, "WebSocket connected");
    } else if (event_id == WEBSOCKET_EVENT_ERROR) {
        int status = event == NULL
            ? 0
            : event->error_handle.esp_ws_handshake_status_code;
        esp_websocket_error_type_t error_type = event == NULL
            ? WEBSOCKET_ERROR_TYPE_NONE
            : event->error_handle.error_type;
        if (status > 0 && status != 101) {
            ESP_LOGW(TAG, "WebSocket handshake rejected, status=%d, "
                     "error_type=%s", status,
                     websocket_error_type_name(error_type));
        } else {
            ESP_LOGW(TAG, "WebSocket TLS/transport failure, status=%d, "
                     "error_type=%s", status,
                     websocket_error_type_name(error_type));
        }
    } else if (event_id == WEBSOCKET_EVENT_DISCONNECTED ||
               event_id == WEBSOCKET_EVENT_CLOSED) {
        if (s_connected) {
            int64_t connected_us = esp_timer_get_time() - s_connected_at_us;
            if (connected_us >=
                (int64_t)NET_LINK_STABLE_CONNECTION_MS * 1000LL) {
                reconnect_backoff_reset(&s_reconnect_backoff);
            }
            s_connected = false;
        }
        if (event != NULL && event->client != NULL) {
            configure_next_reconnect(event->client);
        }
        if (event_id == WEBSOCKET_EVENT_CLOSED) {
            ESP_LOGI(TAG, "WebSocket closed cleanly");
        } else {
            ESP_LOGW(TAG, "WebSocket disconnected after an error");
        }
    } else if (event_id == WEBSOCKET_EVENT_DATA && event != NULL &&
               event->op_code == 2U && event->data_len > 0) {
        ring_append((const uint8_t *)event->data_ptr,
                    (size_t)event->data_len);
    }
}

static void release_start_resources(void)
{
    if (s_client != NULL) {
        (void)esp_websocket_unregister_events(
            s_client, WEBSOCKET_EVENT_ANY, websocket_event_handler);
        (void)esp_websocket_client_destroy(s_client);
        s_client = NULL;
    }
    if (s_rx_ready != NULL) {
        vSemaphoreDelete(s_rx_ready);
        s_rx_ready = NULL;
    }
    heap_caps_free(s_rx_storage);
    s_rx_storage = NULL;
    net_ring_init(&s_rx_ring, NULL, 0U);
}

esp_err_t net_link_start(void)
{
    if (s_client != NULL) {
        return ESP_ERR_INVALID_STATE;
    }

    protocol_network_config_t stored;
    net_store_load(&stored);
    if (net_store_current_tier() != PROTOCOL_TIER_NETWORKED ||
        stored.tier != PROTOCOL_TIER_NETWORKED ||
        stored.server_url[0] == '\0' || stored.token[0] == '\0') {
        memset(&stored, 0, sizeof(stored));
        return ESP_ERR_INVALID_STATE;
    }

    s_rx_storage = heap_caps_malloc(NET_LINK_RX_RING_CAPACITY,
                                    MALLOC_CAP_SPIRAM);
    if (s_rx_storage == NULL) {
        s_rx_storage = heap_caps_malloc(
            NET_LINK_RX_RING_CAPACITY, MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT);
        if (s_rx_storage != NULL) {
            ESP_LOGW(TAG, "PSRAM RX ring unavailable; using internal heap");
        }
    }
    if (s_rx_storage == NULL) {
        memset(&stored, 0, sizeof(stored));
        return ESP_ERR_NO_MEM;
    }
    s_rx_ready = xSemaphoreCreateBinary();
    if (s_rx_ready == NULL) {
        memset(&stored, 0, sizeof(stored));
        release_start_resources();
        return ESP_ERR_NO_MEM;
    }

    portENTER_CRITICAL(&s_rx_lock);
    net_ring_init(&s_rx_ring, s_rx_storage, NET_LINK_RX_RING_CAPACITY);
    portEXIT_CRITICAL(&s_rx_lock);
    reconnect_backoff_init(&s_reconnect_backoff, reconnect_random, NULL);
    s_connected = false;

    char authorization[NET_LINK_AUTH_HEADER_CAPACITY];
    int header_length = snprintf(authorization, sizeof(authorization),
                                 "Authorization: Bearer %s\r\n",
                                 stored.token);
    if (header_length < 0 || (size_t)header_length >= sizeof(authorization)) {
        memset(authorization, 0, sizeof(authorization));
        memset(&stored, 0, sizeof(stored));
        release_start_resources();
        return ESP_ERR_INVALID_STATE;
    }

    esp_websocket_client_config_t config = {
        .uri = stored.server_url,
        .headers = authorization,
        .crt_bundle_attach = esp_crt_bundle_attach,
        .enable_close_reconnect = true,
        .reconnect_timeout_ms = (int)reconnect_backoff_peek_delay_ms(
            &s_reconnect_backoff),
        .network_timeout_ms = (int)NET_LINK_TIMEOUT_MS,
    };
    s_client = esp_websocket_client_init(&config);
    memset(authorization, 0, sizeof(authorization));
    memset(&stored, 0, sizeof(stored));
    if (s_client == NULL) {
        release_start_resources();
        return ESP_ERR_NO_MEM;
    }

    esp_err_t result = esp_websocket_register_events(
        s_client, WEBSOCKET_EVENT_ANY, websocket_event_handler, NULL);
    if (result != ESP_OK) {
        release_start_resources();
        return result;
    }
    // IDF's transport_ws logs the complete HTTP upgrade request, including
    // our Authorization bearer token, at DEBUG and ERROR when a write fails.
    // ERROR is enabled in production, so only a full tag muzzle is safe.
    esp_log_level_set("transport_ws", ESP_LOG_NONE);
    result = esp_websocket_client_start(s_client);
    if (result != ESP_OK) {
        release_start_resources();
        return result;
    }
    return ESP_OK;
}

esp_err_t net_link_suspend(void)
{
    if (s_client == NULL) {
        return ESP_ERR_INVALID_STATE;
    }
    // stop(), not close(): the client is configured with
    // enable_close_reconnect, so a close would immediately dial again and
    // reintroduce the very second TLS session this exists to avoid.
    esp_err_t result = esp_websocket_client_stop(s_client);
    if (result != ESP_OK) {
        return result;
    }
    s_connected = false;
    // Drop anything half-received. Resuming re-dials from scratch, so a
    // frame straddling the suspend would otherwise be decoded against bytes
    // from a different connection -- the same reasoning as the CONNECTED
    // handler's clear.
    portENTER_CRITICAL(&s_rx_lock);
    net_ring_clear(&s_rx_ring);
    portEXIT_CRITICAL(&s_rx_lock);
    protocol_task_reset_network_decoder();
    ESP_LOGI(TAG, "WebSocket suspended for firmware download");
    return ESP_OK;
}

esp_err_t net_link_resume(void)
{
    if (s_client == NULL) {
        return ESP_ERR_INVALID_STATE;
    }
    esp_err_t result = esp_websocket_client_start(s_client);
    if (result != ESP_OK) {
        return result;
    }
    ESP_LOGI(TAG, "WebSocket resumed after firmware download");
    return ESP_OK;
}

static size_t net_link_read(uint8_t *out, size_t capacity,
                            TickType_t timeout_ticks)
{
    if (out == NULL || capacity == 0U || s_rx_storage == NULL ||
        s_rx_ready == NULL) {
        return 0U;
    }

    size_t received = ring_drain(out, capacity);
    if (received != 0U || timeout_ticks == 0U) {
        return received;
    }
    if (xSemaphoreTake(s_rx_ready, timeout_ticks) != pdTRUE) {
        return 0U;
    }
    return ring_drain(out, capacity);
}

static esp_err_t net_link_write_frame(const uint8_t *frame, size_t length,
                                      TickType_t timeout_ticks)
{
    if (s_client == NULL ||
        wifi_station_state() != PROTOCOL_WIFI_CONNECTED ||
        !esp_websocket_client_is_connected(s_client)) {
        return ESP_ERR_INVALID_STATE;
    }
    if (frame == NULL || length == 0U ||
        length > USB_LINK_MAX_WIRE_FRAME_SIZE || frame[length - 1U] != 0U) {
        return ESP_ERR_INVALID_ARG;
    }

    int sent = esp_websocket_client_send_bin(
        s_client, (const char *)frame, (int)length, timeout_ticks);
    return sent == (int)length ? ESP_OK : ESP_FAIL;
}

static uint32_t net_link_rx_dropped_bytes(void)
{
    portENTER_CRITICAL(&s_rx_lock);
    uint32_t dropped = net_ring_dropped_bytes(&s_rx_ring);
    portEXIT_CRITICAL(&s_rx_lock);
    return dropped;
}

static const link_transport_t s_net_transport = {
    .read = net_link_read,
    .write_frame = net_link_write_frame,
    .dropped_bytes = net_link_rx_dropped_bytes,
    .link_timeout_ms = NET_LINK_TIMEOUT_MS,
};

const link_transport_t *net_link_transport(void)
{
    return &s_net_transport;
}
