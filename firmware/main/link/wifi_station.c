#include "wifi_station.h"

#include <stdatomic.h>
#include <string.h>
#include <sys/time.h>

#include "esp_event.h"
#include "esp_log.h"
#include "esp_netif.h"
#include "esp_random.h"
#include "esp_sntp.h"
#include "esp_timer.h"
#include "esp_wifi.h"

#include "link/net_store.h"
#include "ui/ui_runtime.h"

// Plan-wide reconnect backoff: 1s initial, doubling, capped at 60s. Jitter is
// applied on top (see jittered_delay_ms) so a fleet does not resynchronise
// onto the network in lockstep the instant an outage ends.
#define WIFI_RECONNECT_MIN_DELAY_MS 1000U
#define WIFI_RECONNECT_MAX_DELAY_MS 60000U

static const char *TAG = "wifi_station";

// protocol_wifi_state_t is int-sized; stored as atomic_int and cast back on
// read, matching this codebase's existing convention for a small enum/scalar
// shared between the LVGL/protocol tasks and another context (see
// clock_screen.c's s_utc_offset_minutes).
// Zero-initialized statics already match PROTOCOL_WIFI_DOWN / 0 / false, so
// no explicit initializer is needed (and none is used elsewhere in this
// codebase for the same atomic pattern -- see clock_screen.c).
static atomic_int s_wifi_state;
static atomic_int s_wifi_rssi;
static atomic_bool s_time_synced;

// s_ip is mutated by the event-loop task (on IP_EVENT_STA_GOT_IP /
// WIFI_EVENT_STA_DISCONNECTED) and read by the protocol task when it builds
// a status response. Guard both sides with the same short spinlock so a
// reader never observes a torn dotted-quad string; the critical sections are
// a handful of byte copies, so busy-waiting is appropriate (see
// ui_runtime.c's lock_publisher()/unlock_publisher() for the same pattern).
static atomic_flag s_ip_lock = ATOMIC_FLAG_INIT;
static char s_ip[PROTOCOL_MAX_IP_LENGTH + 1U];
static char s_ip_snapshot[PROTOCOL_MAX_IP_LENGTH + 1U];

// Loaded once at wifi_station_start() and never a secret; only the SSID/PSK
// are sensitive and neither is retained beyond building the driver's config.
static int16_t s_utc_offset_minutes;

static esp_timer_handle_t s_reconnect_timer;
static uint32_t s_reconnect_delay_ms = WIFI_RECONNECT_MIN_DELAY_MS;

static void lock_ip(void)
{
    while (atomic_flag_test_and_set_explicit(&s_ip_lock, memory_order_acquire)) {
    }
}

static void unlock_ip(void)
{
    atomic_flag_clear_explicit(&s_ip_lock, memory_order_release);
}

static void set_ip(const char *ip)
{
    lock_ip();
    if (ip == NULL) {
        s_ip[0] = '\0';
    } else {
        size_t length = strnlen(ip, PROTOCOL_MAX_IP_LENGTH);
        memcpy(s_ip, ip, length);
        s_ip[length] = '\0';
    }
    unlock_ip();
}

static uint32_t jittered_delay_ms(uint32_t base_ms)
{
    // Scale factor in [80, 120] percent, i.e. +/-20%.
    uint32_t percent = 80U + (esp_random() % 41U);
    return (uint32_t)(((uint64_t)base_ms * percent) / 100U);
}

static void schedule_reconnect(void)
{
    uint32_t delay_ms = jittered_delay_ms(s_reconnect_delay_ms);
    esp_err_t result = esp_timer_start_once(s_reconnect_timer,
                                            (uint64_t)delay_ms * 1000ULL);
    if (result != ESP_OK) {
        ESP_LOGW(TAG, "failed to arm reconnect timer: %s",
                esp_err_to_name(result));
    }
    uint32_t doubled = s_reconnect_delay_ms * 2U;
    s_reconnect_delay_ms = (doubled > WIFI_RECONNECT_MAX_DELAY_MS ||
                            doubled < s_reconnect_delay_ms)
        ? WIFI_RECONNECT_MAX_DELAY_MS
        : doubled;
}

static void reconnect_timer_cb(void *arg)
{
    (void)arg;
    esp_err_t result = esp_wifi_connect();
    if (result != ESP_OK) {
        ESP_LOGW(TAG, "reconnect attempt failed to start: %s",
                esp_err_to_name(result));
    }
}

static void time_sync_notification_cb(struct timeval *tv)
{
    (void)tv;
    atomic_store_explicit(&s_time_synced, true, memory_order_relaxed);
    // The system clock now holds UTC: SNTP calls settimeofday() with the raw
    // server time here, exactly as protocol_task.c's dispatch_time_sync()
    // does for a USB host's TimeSync message. Applying the provisioned
    // utc_offset_minutes happens only at this UI-facing call -- it is never
    // folded into the system clock itself -- so this call and a later (or
    // earlier) USB TimeSync's own ui_runtime_set_utc_offset_minutes() call
    // simply overwrite the same display-only offset rather than compounding:
    // whichever source synced most recently wins, and neither can double-
    // apply an offset because clock_source_now() is always pure UTC.
    ui_runtime_set_utc_offset_minutes(s_utc_offset_minutes);
    ESP_LOGI(TAG, "SNTP time sync complete");
}

static void start_sntp_if_needed(void)
{
    if (esp_sntp_enabled()) {
        return;
    }
    esp_sntp_setoperatingmode(ESP_SNTP_OPMODE_POLL);
    esp_sntp_setservername(0, "pool.ntp.org");
    esp_sntp_set_time_sync_notification_cb(time_sync_notification_cb);
    esp_sntp_init();
}

static void wifi_event_handler(void *handler_arg, esp_event_base_t base,
                               int32_t event_id, void *event_data)
{
    (void)handler_arg;
    (void)base;
    (void)event_data;
    if (event_id == WIFI_EVENT_STA_START) {
        esp_err_t result = esp_wifi_connect();
        if (result != ESP_OK) {
            ESP_LOGW(TAG, "initial connect failed to start: %s",
                    esp_err_to_name(result));
        }
    } else if (event_id == WIFI_EVENT_STA_DISCONNECTED) {
        atomic_store_explicit(&s_wifi_state, PROTOCOL_WIFI_CONNECTING,
                              memory_order_relaxed);
        set_ip(NULL);
        schedule_reconnect();
    }
}

static void ip_event_handler(void *handler_arg, esp_event_base_t base,
                             int32_t event_id, void *event_data)
{
    (void)handler_arg;
    (void)base;
    if (event_id != IP_EVENT_STA_GOT_IP) {
        return;
    }
    const ip_event_got_ip_t *event = (const ip_event_got_ip_t *)event_data;

    // A successful join resets the backoff so the *next* disconnect starts
    // from 1s again rather than continuing to climb.
    s_reconnect_delay_ms = WIFI_RECONNECT_MIN_DELAY_MS;

    wifi_ap_record_t ap_info;
    memset(&ap_info, 0, sizeof(ap_info));
    int8_t rssi = 0;
    if (esp_wifi_sta_get_ap_info(&ap_info) == ESP_OK) {
        rssi = ap_info.rssi;
    }
    atomic_store_explicit(&s_wifi_rssi, rssi, memory_order_relaxed);

    char ip_text[PROTOCOL_MAX_IP_LENGTH + 1U];
    esp_ip4addr_ntoa(&event->ip_info.ip, ip_text, sizeof(ip_text));
    set_ip(ip_text);

    atomic_store_explicit(&s_wifi_state, PROTOCOL_WIFI_CONNECTED,
                          memory_order_relaxed);
    ESP_LOGI(TAG, "WiFi connected, ip=%s rssi=%d", ip_text, (int)rssi);

    start_sntp_if_needed();
}

esp_err_t wifi_station_bringup(void)
{
    protocol_network_config_t config;
    net_store_load(&config);
    s_utc_offset_minutes = config.utc_offset_minutes;

    if (config.ssid[0] == '\0') {
        memset(&config, 0, sizeof(config));
        ESP_LOGI(TAG, "no stored WiFi credentials; staying local");
        return ESP_OK;
    }

    ESP_LOGI(TAG, "starting WiFi station, ssid=%s", config.ssid);

    esp_err_t result = esp_netif_init();
    if (result != ESP_OK && result != ESP_ERR_INVALID_STATE) {
        memset(&config, 0, sizeof(config));
        return result;
    }
    result = esp_event_loop_create_default();
    if (result != ESP_OK && result != ESP_ERR_INVALID_STATE) {
        memset(&config, 0, sizeof(config));
        return result;
    }
    if (esp_netif_create_default_wifi_sta() == NULL) {
        memset(&config, 0, sizeof(config));
        return ESP_FAIL;
    }

    wifi_init_config_t init_config = WIFI_INIT_CONFIG_DEFAULT();
    result = esp_wifi_init(&init_config);
    if (result != ESP_OK) {
        memset(&config, 0, sizeof(config));
        return result;
    }

    result = esp_event_handler_instance_register(
        WIFI_EVENT, ESP_EVENT_ANY_ID, &wifi_event_handler, NULL, NULL);
    if (result != ESP_OK) {
        memset(&config, 0, sizeof(config));
        return result;
    }
    result = esp_event_handler_instance_register(
        IP_EVENT, IP_EVENT_STA_GOT_IP, &ip_event_handler, NULL, NULL);
    if (result != ESP_OK) {
        memset(&config, 0, sizeof(config));
        return result;
    }

    const esp_timer_create_args_t timer_args = {
        .callback = &reconnect_timer_cb,
        .name = "wifi_reconnect",
    };
    result = esp_timer_create(&timer_args, &s_reconnect_timer);
    if (result != ESP_OK) {
        memset(&config, 0, sizeof(config));
        return result;
    }

    result = esp_wifi_set_mode(WIFI_MODE_STA);
    if (result != ESP_OK) {
        memset(&config, 0, sizeof(config));
        return result;
    }

    wifi_config_t wifi_config;
    memset(&wifi_config, 0, sizeof(wifi_config));
    size_t ssid_len = strnlen(config.ssid, sizeof(config.ssid) - 1U);
    if (ssid_len > sizeof(wifi_config.sta.ssid)) {
        ssid_len = sizeof(wifi_config.sta.ssid);
    }
    memcpy(wifi_config.sta.ssid, config.ssid, ssid_len);
    size_t psk_len = strnlen(config.psk, sizeof(config.psk) - 1U);
    if (psk_len > sizeof(wifi_config.sta.password)) {
        psk_len = sizeof(wifi_config.sta.password);
    }
    memcpy(wifi_config.sta.password, config.psk, psk_len);

    // The PSK has now been handed to the driver's own config storage; it
    // does not need to keep living on this stack.
    memset(&config, 0, sizeof(config));

    result = esp_wifi_set_config(WIFI_IF_STA, &wifi_config);
    memset(&wifi_config, 0, sizeof(wifi_config));
    if (result != ESP_OK) {
        return result;
    }

    result = esp_wifi_start();
    if (result != ESP_OK) {
        return result;
    }

    // The join has started, not succeeded; wifi_station_state() reports
    // progress from here via the event handlers above.
    atomic_store_explicit(&s_wifi_state, PROTOCOL_WIFI_CONNECTING,
                          memory_order_relaxed);
    return ESP_OK;
}

protocol_wifi_state_t wifi_station_state(void)
{
    return (protocol_wifi_state_t)atomic_load_explicit(&s_wifi_state,
                                                        memory_order_relaxed);
}

int8_t wifi_station_rssi(void)
{
    return (int8_t)atomic_load_explicit(&s_wifi_rssi, memory_order_relaxed);
}

const char *wifi_station_ip(void)
{
    lock_ip();
    memcpy(s_ip_snapshot, s_ip, sizeof(s_ip_snapshot));
    unlock_ip();
    return s_ip_snapshot;
}

bool wifi_station_time_synced(void)
{
    return atomic_load_explicit(&s_time_synced, memory_order_relaxed);
}
