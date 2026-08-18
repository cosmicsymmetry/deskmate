#include "link/net_link.h"

#include <limits.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "core/protocol_message.h"
#include "esp_crt_bundle.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_random.h"
#include "esp_websocket_client.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "link/net_store.h"
#include "link/usb_link.h"
#include "link/wifi_station.h"

#define NET_LINK_RX_RING_CAPACITY (USB_LINK_MAX_WIRE_FRAME_SIZE * 4U)
#define NET_LINK_TIMEOUT_MS 45000U
#define NET_LINK_RECONNECT_MIN_DELAY_MS 1000U
#define NET_LINK_RECONNECT_MAX_DELAY_MS 60000U
#define NET_LINK_AUTH_HEADER_CAPACITY                                      \
    (PROTOCOL_MAX_DEVICE_TOKEN_LENGTH + sizeof("Authorization: Bearer \r\n"))

static const char *TAG = "net_link";

static esp_websocket_client_handle_t s_client;
static uint8_t *s_rx_ring;
static size_t s_rx_head;
static size_t s_rx_tail;
static size_t s_rx_used;
static uint32_t s_rx_dropped_bytes;
static SemaphoreHandle_t s_rx_ready;
static portMUX_TYPE s_rx_lock = portMUX_INITIALIZER_UNLOCKED;
static uint32_t s_reconnect_delay_ms = NET_LINK_RECONNECT_MIN_DELAY_MS;

static uint32_t jittered_delay_ms(uint32_t base_ms)
{
    uint32_t percent = 80U + (esp_random() % 41U);
    return (uint32_t)(((uint64_t)base_ms * percent) / 100U);
}

static void advance_reconnect_delay(void)
{
    uint32_t doubled = s_reconnect_delay_ms * 2U;
    s_reconnect_delay_ms = (doubled > NET_LINK_RECONNECT_MAX_DELAY_MS ||
                            doubled < s_reconnect_delay_ms)
        ? NET_LINK_RECONNECT_MAX_DELAY_MS
        : doubled;
}

static void add_dropped_bytes_locked(size_t count)
{
    if (count > UINT32_MAX - s_rx_dropped_bytes) {
        s_rx_dropped_bytes = UINT32_MAX;
    } else {
        s_rx_dropped_bytes += (uint32_t)count;
    }
}

static void ring_append(const uint8_t *data, size_t length)
{
    if (data == NULL || length == 0U || s_rx_ring == NULL) {
        return;
    }

    portENTER_CRITICAL(&s_rx_lock);
    size_t available = NET_LINK_RX_RING_CAPACITY - s_rx_used;
    size_t accepted = length < available ? length : available;
    size_t first = accepted;
    size_t until_wrap = NET_LINK_RX_RING_CAPACITY - s_rx_head;
    if (first > until_wrap) {
        first = until_wrap;
    }
    if (first != 0U) {
        memcpy(s_rx_ring + s_rx_head, data, first);
    }
    size_t second = accepted - first;
    if (second != 0U) {
        memcpy(s_rx_ring, data + first, second);
    }
    s_rx_head = (s_rx_head + accepted) % NET_LINK_RX_RING_CAPACITY;
    s_rx_used += accepted;
    add_dropped_bytes_locked(length - accepted);
    portEXIT_CRITICAL(&s_rx_lock);

    if (accepted != 0U) {
        (void)xSemaphoreGive(s_rx_ready);
    }
}

static size_t ring_drain(uint8_t *out, size_t capacity)
{
    portENTER_CRITICAL(&s_rx_lock);
    size_t count = capacity < s_rx_used ? capacity : s_rx_used;
    size_t first = count;
    size_t until_wrap = NET_LINK_RX_RING_CAPACITY - s_rx_tail;
    if (first > until_wrap) {
        first = until_wrap;
    }
    if (first != 0U) {
        memcpy(out, s_rx_ring + s_rx_tail, first);
    }
    size_t second = count - first;
    if (second != 0U) {
        memcpy(out + first, s_rx_ring, second);
    }
    s_rx_tail = (s_rx_tail + count) % NET_LINK_RX_RING_CAPACITY;
    s_rx_used -= count;
    portEXIT_CRITICAL(&s_rx_lock);
    return count;
}

static void configure_next_reconnect(esp_websocket_client_handle_t client)
{
    uint32_t delay_ms = jittered_delay_ms(s_reconnect_delay_ms);
    esp_err_t result = esp_websocket_client_set_reconnect_timeout(
        client, (int)delay_ms);
    if (result != ESP_OK) {
        ESP_LOGW(TAG, "failed to configure reconnect timeout: %s",
                 esp_err_to_name(result));
    }
    advance_reconnect_delay();
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
        s_reconnect_delay_ms = NET_LINK_RECONNECT_MIN_DELAY_MS;
        ESP_LOGI(TAG, "WebSocket connected");
    } else if (event_id == WEBSOCKET_EVENT_DISCONNECTED ||
               event_id == WEBSOCKET_EVENT_CLOSED) {
        if (event != NULL && event->client != NULL) {
            configure_next_reconnect(event->client);
        }
        ESP_LOGW(TAG, "WebSocket disconnected");
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
    heap_caps_free(s_rx_ring);
    s_rx_ring = NULL;
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

    s_rx_ring = heap_caps_malloc(NET_LINK_RX_RING_CAPACITY,
                                 MALLOC_CAP_SPIRAM);
    if (s_rx_ring == NULL) {
        s_rx_ring = heap_caps_malloc(NET_LINK_RX_RING_CAPACITY,
                                     MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT);
        if (s_rx_ring != NULL) {
            ESP_LOGW(TAG, "PSRAM RX ring unavailable; using internal heap");
        }
    }
    if (s_rx_ring == NULL) {
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
    s_rx_head = 0U;
    s_rx_tail = 0U;
    s_rx_used = 0U;
    s_rx_dropped_bytes = 0U;
    portEXIT_CRITICAL(&s_rx_lock);
    s_reconnect_delay_ms = NET_LINK_RECONNECT_MIN_DELAY_MS;

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
        .reconnect_timeout_ms = (int)jittered_delay_ms(
            NET_LINK_RECONNECT_MIN_DELAY_MS),
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
    result = esp_websocket_client_start(s_client);
    if (result != ESP_OK) {
        release_start_resources();
        return result;
    }
    return ESP_OK;
}

static size_t net_link_read(uint8_t *out, size_t capacity,
                            TickType_t timeout_ticks)
{
    if (out == NULL || capacity == 0U || s_rx_ring == NULL ||
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
    uint32_t dropped = s_rx_dropped_bytes;
    portEXIT_CRITICAL(&s_rx_lock);
    return dropped;
}

static const link_transport_t s_net_transport = {
    .name = "network",
    .read = net_link_read,
    .write_frame = net_link_write_frame,
    .dropped_bytes = net_link_rx_dropped_bytes,
    .link_timeout_ms = NET_LINK_TIMEOUT_MS,
};

const link_transport_t *net_link_transport(void)
{
    return &s_net_transport;
}
