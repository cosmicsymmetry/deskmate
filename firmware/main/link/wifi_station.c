#include "wifi_station.h"

#include <stdatomic.h>
#include <string.h>
#include <sys/time.h>

#include "esp_event.h"
#include "esp_idf_version.h"
#include "esp_log.h"
#include "esp_netif.h"
#include "esp_random.h"
#include "esp_sntp.h"
#include "esp_timer.h"
#include "esp_wifi.h"

#include "core/reconnect_backoff.h"
#include "link/net_store.h"
#include "ui/ui_runtime.h"

static const char *TAG = "wifi_station";

// protocol_wifi_state_t is int-sized; stored as atomic_int and cast back on
// read, matching this codebase's existing convention for a small enum/scalar
// shared between the LVGL/protocol tasks and another context (see
// clock_screen.c's s_utc_offset_minutes).
// Zero-initialized statics already match PROTOCOL_WIFI_DOWN / false, so no
// explicit initializer is needed (and none is used elsewhere in this
// codebase for the same atomic pattern -- see clock_screen.c).
static atomic_int s_wifi_state;
static atomic_bool s_time_synced;

// Set by time_sync_notification_cb() (the lwIP tcpip task) and cleared by
// wifi_station_poll() (the protocol task). This is the entire hand-off: the
// tcpip task never calls ui_runtime_set_utc_offset_minutes() itself. See the
// header comment on wifi_station_poll() for why.
static atomic_bool s_offset_apply_pending;

// s_ip is mutated by the event-loop task (on IP_EVENT_STA_GOT_IP /
// WIFI_EVENT_STA_DISCONNECTED) and read by the protocol task when it builds
// a status response. Guard both sides with the same short spinlock so a
// reader never observes a torn dotted-quad string; the critical sections are
// a handful of byte copies, so busy-waiting is appropriate (see
// ui_runtime.c's lock_publisher()/unlock_publisher() for the same pattern).
static atomic_flag s_ip_lock = ATOMIC_FLAG_INIT;
static char s_ip[PROTOCOL_MAX_IP_LENGTH + 1U];

// Loaded once at wifi_station_bringup() and never a secret; only the SSID/PSK
// are sensitive and neither is retained beyond building the driver's config.
static int16_t s_utc_offset_minutes;

static esp_timer_handle_t s_reconnect_timer;
static reconnect_backoff_t s_reconnect_backoff;

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

static uint32_t reconnect_random(void *context)
{
    (void)context;
    return esp_random();
}

static void schedule_reconnect(void)
{
    uint32_t delay_ms = reconnect_backoff_next_delay_ms(
        &s_reconnect_backoff);
    esp_err_t result = esp_timer_start_once(s_reconnect_timer,
                                            (uint64_t)delay_ms * 1000ULL);
    if (result != ESP_OK) {
        ESP_LOGW(TAG, "failed to arm reconnect timer: %s",
                esp_err_to_name(result));
    }
}

static void reconnect_timer_cb(void *arg)
{
    (void)arg;
    // FAILED is a report, not a terminal state: each new attempt clears it
    // back to CONNECTING (the ruling this task implements), even if the
    // previous disconnect was classified as a credential failure -- the
    // password may since have been fixed, or the AP may be back in range.
    // If this attempt fails for the same reason, the next
    // WIFI_EVENT_STA_DISCONNECTED will re-classify and set FAILED again.
    atomic_store_explicit(&s_wifi_state, PROTOCOL_WIFI_CONNECTING,
                          memory_order_relaxed);
    esp_err_t result = esp_wifi_connect();
    if (result != ESP_OK) {
        // A synchronous failure here means no association was even
        // attempted, so no WIFI_EVENT_STA_DISCONNECTED will follow to
        // schedule the next retry -- without re-arming here, the state
        // machine would silently stall in CONNECTING until reboot.
        ESP_LOGW(TAG, "reconnect attempt failed to start: %s",
                esp_err_to_name(result));
        schedule_reconnect();
    }
}

static void time_sync_notification_cb(struct timeval *tv)
{
    (void)tv;
    // This callback runs on the lwIP tcpip task (unpinned, priority 18) --
    // see wifi_station_poll()'s doc comment for why it must not touch
    // ui_runtime directly. Only atomics are touched here; the actual UI call
    // happens later, on the protocol task, via wifi_station_poll().
    atomic_store_explicit(&s_time_synced, true, memory_order_relaxed);
    atomic_store_explicit(&s_offset_apply_pending, true, memory_order_relaxed);
    ESP_LOGI(TAG, "SNTP time sync complete");
}

void wifi_station_poll(void)
{
    bool pending = atomic_exchange_explicit(&s_offset_apply_pending, false,
                                            memory_order_relaxed);
    if (!pending) {
        return;
    }
    // The system clock now holds UTC: SNTP calls settimeofday() with the raw
    // server time, exactly as protocol_task.c's dispatch_time_sync() does
    // for a USB host's TimeSync message. Applying the provisioned
    // utc_offset_minutes happens only at this UI-facing call -- it is never
    // folded into the system clock itself -- so this call and a later (or
    // earlier) USB TimeSync's own ui_runtime_set_utc_offset_minutes() call
    // simply overwrite the same display-only offset rather than compounding:
    // whichever source synced most recently wins, and neither can double-
    // apply an offset because clock_source_now() is always pure UTC.
    if (!ui_runtime_set_utc_offset_minutes(s_utc_offset_minutes)) {
        // The publish was dropped (e.g. a momentarily full UI command
        // queue). The atomic_exchange above already consumed the pending
        // flag, so without this the offset would silently wait for the
        // *next* SNTP sync -- roughly an hour -- to be applied at all.
        // Re-arm it so the next protocol-task tick retries.
        atomic_store_explicit(&s_offset_apply_pending, true,
                              memory_order_relaxed);
    }
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

// Disconnect reasons that mean "this credential/target is wrong," not "the
// link briefly dropped." Reported as PROTOCOL_WIFI_FAILED so a human
// consulting wifi_state can tell a bad SSID/passphrase apart from an
// out-of-range router at a glance, instead of seeing "connecting" forever
// either way. This is a report, not a terminal state: retries continue on
// the same backoff regardless of reason, and the next WIFI_EVENT_STA_START
// or IP_EVENT_STA_GOT_IP clears it back to CONNECTING/CONNECTED as normal.
//
// The three IDF-5.5 codes below (WIFI_REASON_NO_AP_FOUND_{W_COMPATIBLE_
// SECURITY,IN_AUTHMODE_THRESHOLD,IN_RSSI_THRESHOLD}) are wifi_err_reason_t
// enum constants, not preprocessor macros, so #ifdef cannot detect whether
// an older esp_wifi_types_generic.h defines them -- an #ifdef guard on an
// enumerator is always false regardless of whether the symbol exists, which
// would silently compile away the intended check rather than fail loud.
// ESP_IDF_VERSION_VAL, by contrast, is a real macro (esp_idf_version.h), so
// gating on it gets a genuine compile-time cross-check of the symbolic
// names on IDF >=5.5 (this project targets 5.5.5) while still compiling
// against the project's idf: '>=5.3' floor: below 5.5 the block simply
// compiles out and no classification is needed for codes that release
// doesn't emit.
static bool disconnect_reason_is_credential_failure(uint8_t reason)
{
    switch (reason) {
    case WIFI_REASON_AUTH_EXPIRE:
    case WIFI_REASON_4WAY_HANDSHAKE_TIMEOUT:
    case WIFI_REASON_NO_AP_FOUND:
    case WIFI_REASON_AUTH_FAIL:
    case WIFI_REASON_HANDSHAKE_TIMEOUT:
        return true;
#if ESP_IDF_VERSION >= ESP_IDF_VERSION_VAL(5, 5, 0)
    case WIFI_REASON_NO_AP_FOUND_W_COMPATIBLE_SECURITY:
    case WIFI_REASON_NO_AP_FOUND_IN_AUTHMODE_THRESHOLD:
    case WIFI_REASON_NO_AP_FOUND_IN_RSSI_THRESHOLD:
        return true;
#endif
    default:
        return false;
    }
}

static void wifi_event_handler(void *handler_arg, esp_event_base_t base,
                               int32_t event_id, void *event_data)
{
    (void)handler_arg;
    (void)base;
    if (event_id == WIFI_EVENT_STA_START) {
        esp_err_t result = esp_wifi_connect();
        if (result != ESP_OK) {
            // Same give-up risk as reconnect_timer_cb(): a synchronous
            // failure here means no WIFI_EVENT_STA_DISCONNECTED is coming to
            // schedule a retry, so this must arm the backoff itself.
            ESP_LOGW(TAG, "initial connect failed to start: %s",
                    esp_err_to_name(result));
            schedule_reconnect();
        }
    } else if (event_id == WIFI_EVENT_STA_DISCONNECTED) {
        const wifi_event_sta_disconnected_t *disconnected =
            (const wifi_event_sta_disconnected_t *)event_data;
        uint8_t reason = (disconnected != NULL) ? disconnected->reason : 0U;
        // Reason code and SSID only -- never the passphrase.
        ESP_LOGW(TAG, "WiFi disconnected, reason=%u", (unsigned)reason);
        protocol_wifi_state_t next_state =
            disconnect_reason_is_credential_failure(reason)
                ? PROTOCOL_WIFI_FAILED
                : PROTOCOL_WIFI_CONNECTING;
        atomic_store_explicit(&s_wifi_state, next_state,
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
    reconnect_backoff_reset(&s_reconnect_backoff);

    char ip_text[PROTOCOL_MAX_IP_LENGTH + 1U];
    esp_ip4addr_ntoa(&event->ip_info.ip, ip_text, sizeof(ip_text));
    set_ip(ip_text);

    atomic_store_explicit(&s_wifi_state, PROTOCOL_WIFI_CONNECTED,
                          memory_order_relaxed);
    ESP_LOGI(TAG, "WiFi connected, ip=%s", ip_text);

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

    reconnect_backoff_init(&s_reconnect_backoff, reconnect_random, NULL);

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

    // Factory reset (net_store_erase()) only clears our own NVS namespace.
    // esp_wifi's default WIFI_STORAGE_FLASH would otherwise persist the PSK
    // into the driver's own nvs.net80211 namespace, which survives that
    // reset -- "factory reset" must actually mean no credentials remain on
    // flash. We already own persistence ourselves (net_store), so RAM-only
    // driver storage is strictly correct here, not just cheaper.
    result = esp_wifi_set_storage(WIFI_STORAGE_RAM);
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
    // Capped at sizeof(field) - 1, not sizeof(field): wifi_sta_config_t has
    // no ssid_len companion field, so a legal 32-character SSID filling the
    // 32-byte ssid[] entirely would leave no NUL before the password[] bytes
    // that follow it in the struct. Whether the closed-source net80211 blob
    // bounds its own read of ssid[] is unverifiable from source, so leaving
    // at least one zero byte (already present from the memset above) is
    // cheap insurance. Same reasoning for the 64-byte password[].
    size_t ssid_len = strnlen(config.ssid, sizeof(config.ssid) - 1U);
    if (ssid_len > sizeof(wifi_config.sta.ssid) - 1U) {
        ssid_len = sizeof(wifi_config.sta.ssid) - 1U;
    }
    memcpy(wifi_config.sta.ssid, config.ssid, ssid_len);
    size_t psk_len = strnlen(config.psk, sizeof(config.psk) - 1U);
    if (psk_len > sizeof(wifi_config.sta.password) - 1U) {
        psk_len = sizeof(wifi_config.sta.password) - 1U;
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
    // Sampled live rather than cached from the join-time event: a cached
    // value would keep reporting the last-seen RSSI while CONNECTING or
    // FAILED with no live link at all, which defeats the "wrong password
    // versus out of range" judgement PROTOCOL_WIFI_FAILED exists to enable.
    // esp_wifi_sta_get_ap_info() is a thread-safe public esp_wifi API and
    // itself returns ESP_ERR_WIFI_NOT_CONNECT (among other errors) whenever
    // there is no current association, so the "not connected" case falls
    // out of the same call rather than needing separate bookkeeping.
    wifi_ap_record_t ap_info;
    memset(&ap_info, 0, sizeof(ap_info));
    if (esp_wifi_sta_get_ap_info(&ap_info) != ESP_OK) {
        return 0;
    }
    return ap_info.rssi;
}

void wifi_station_copy_ip(char *out, size_t len)
{
    if (out == NULL || len == 0U) {
        return;
    }
    lock_ip();
    size_t source_length = strnlen(s_ip, sizeof(s_ip));
    size_t copy_length = (source_length > len - 1U) ? len - 1U : source_length;
    memcpy(out, s_ip, copy_length);
    out[copy_length] = '\0';
    unlock_ip();
}

bool wifi_station_time_synced(void)
{
    return atomic_load_explicit(&s_time_synced, memory_order_relaxed);
}
