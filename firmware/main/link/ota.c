#include "ota.h"

#include <stdatomic.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "board/display.h"
#include "core/ota_policy.h"
#include "esp_app_desc.h"
#include "esp_crt_bundle.h"
#include "esp_heap_caps.h"
#include "esp_http_client.h"
#include "esp_https_ota.h"
#include "esp_log.h"
#include "esp_lvgl_port.h"
#include "esp_ota_ops.h"
#include "esp_random.h"
#include "esp_system.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "link/net_link.h"
#include "link/net_store.h"
#include "link/protocol_task.h"
#include "link/wifi_station.h"
#include "lvgl.h"
#include "nvs.h"
#include "ui/ota_screen.h"
#include "ui/ui_runtime.h"

#define OTA_TASK_STACK_SIZE 12288U
#define OTA_TASK_PRIORITY 4U
#define OTA_TASK_CORE 1
#define OTA_HTTP_TIMEOUT_MS 45000
#define OTA_HTTP_BUFFER_SIZE 512
#define OTA_IMAGE_BUFFER_SIZE 4096
#define OTA_WIFI_WAIT_MS 60000U
#define OTA_WIFI_POLL_MS 1000U
#define OTA_OWNER_WAIT_MS 60000U
#define OTA_DEFERRED_POLL_MS 1000U
#define OTA_CHECK_INTERVAL_MS UINT32_C(86400000)
#define OTA_CHECK_JITTER_MS UINT32_C(3600000)
#define OTA_AUTHORIZATION_CAPACITY                                      \
    (PROTOCOL_MAX_DEVICE_TOKEN_LENGTH + sizeof("Bearer "))

typedef struct {
    esp_err_t transport_error;
} ota_http_diagnostic_t;

typedef struct {
    char bytes[OTA_POLICY_MAX_METADATA_LENGTH + 1U];
    size_t length;
    bool overflow;
    ota_http_diagnostic_t diagnostic;
} metadata_response_t;

static const char *TAG = "ota";
static atomic_int s_state = ATOMIC_VAR_INIT(PROTOCOL_OTA_IDLE);
static atomic_bool s_task_starting;
static TaskHandle_t s_task;
static portMUX_TYPE s_last_error_lock = portMUX_INITIALIZER_UNLOCKED;
static char s_last_error[PROTOCOL_MAX_DIAGNOSTIC_LENGTH + 1U];

static void set_state(protocol_ota_state_t state)
{
    atomic_store_explicit(&s_state, (int)state, memory_order_release);
}

protocol_ota_state_t ota_state(void)
{
    return (protocol_ota_state_t)atomic_load_explicit(
        &s_state, memory_order_acquire);
}

static void lock_last_error(void)
{
    portENTER_CRITICAL(&s_last_error_lock);
}

static void unlock_last_error(void)
{
    portEXIT_CRITICAL(&s_last_error_lock);
}

static void clear_last_error(void)
{
    lock_last_error();
    s_last_error[0] = '\0';
    unlock_last_error();
}

static void set_failure_detail(const char *stage, const char *detail)
{
    char formatted[PROTOCOL_MAX_DIAGNOSTIC_LENGTH + 1U];
    memset(formatted, 0, sizeof(formatted));
    ota_policy_format_failure(formatted, sizeof(formatted), stage, detail);
    lock_last_error();
    memcpy(s_last_error, formatted, sizeof(s_last_error));
    unlock_last_error();
    set_state(PROTOCOL_OTA_FAILED);
}

static void set_failure(const char *stage, esp_err_t error)
{
    set_failure_detail(stage, esp_err_to_name(error));
}

static void set_http_failure(const char *stage, int status)
{
    char detail[32];
    (void)snprintf(detail, sizeof(detail), "HTTP %d", status);
    set_failure_detail(stage, detail);
}

bool ota_copy_last_error(char *out, size_t out_capacity)
{
    if (out == NULL || out_capacity == 0U) {
        return false;
    }
    lock_last_error();
    size_t length = strnlen(s_last_error, sizeof(s_last_error));
    size_t copied = length < out_capacity - 1U ? length
                                               : out_capacity - 1U;
    memcpy(out, s_last_error, copied);
    out[copied] = '\0';
    unlock_last_error();
    return length != 0U;
}

static void capture_http_error(esp_http_client_event_t *event,
                               ota_http_diagnostic_t *diagnostic)
{
    if (event == NULL || diagnostic == NULL || event->client == NULL) {
        return;
    }
    int tls_code = 0;
    int tls_flags = 0;
    esp_err_t error = esp_http_client_get_and_clear_last_tls_error(
        event->client, &tls_code, &tls_flags);
    if (error != ESP_OK) {
        diagnostic->transport_error = error;
    }
}

static esp_err_t prefer_transport_error(
    esp_err_t result,
    const ota_http_diagnostic_t *diagnostic)
{
    if (result == ESP_FAIL && diagnostic != NULL &&
        diagnostic->transport_error != ESP_OK) {
        return diagnostic->transport_error;
    }
    return result;
}

static esp_err_t metadata_http_event(esp_http_client_event_t *event)
{
    if (event == NULL) {
        return ESP_OK;
    }
    metadata_response_t *response = event->user_data;
    if (event->event_id == HTTP_EVENT_ERROR) {
        if (response != NULL) {
            capture_http_error(event, &response->diagnostic);
        }
        return ESP_OK;
    }
    if (event->event_id != HTTP_EVENT_ON_DATA) {
        return ESP_OK;
    }
    if (response == NULL || event->data == NULL || event->data_len < 0) {
        return ESP_FAIL;
    }
    size_t incoming = (size_t)event->data_len;
    if (incoming > OTA_POLICY_MAX_METADATA_LENGTH - response->length) {
        response->overflow = true;
        return ESP_FAIL;
    }
    memcpy(response->bytes + response->length, event->data, incoming);
    response->length += incoming;
    response->bytes[response->length] = '\0';
    return ESP_OK;
}

static esp_err_t download_http_event(esp_http_client_event_t *event)
{
    if (event != NULL && event->event_id == HTTP_EVENT_ERROR) {
        capture_http_error(event, event->user_data);
    }
    return ESP_OK;
}

static esp_err_t fetch_metadata(const char *check_url,
                                const char *token,
                                ota_policy_metadata_t *metadata,
                                int *http_status)
{
    if (http_status != NULL) {
        *http_status = 0;
    }
    metadata_response_t *response = heap_caps_calloc(
        1U, sizeof(*response), MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    if (response == NULL) {
        return ESP_ERR_NO_MEM;
    }

    char authorization[OTA_AUTHORIZATION_CAPACITY];
    int authorization_length = snprintf(
        authorization, sizeof(authorization), "Bearer %s", token);
    if (authorization_length < 0 ||
        (size_t)authorization_length >= sizeof(authorization)) {
        memset(authorization, 0, sizeof(authorization));
        heap_caps_free(response);
        return ESP_ERR_INVALID_SIZE;
    }

    esp_http_client_config_t config = {
        .url = check_url,
        .method = HTTP_METHOD_GET,
        .timeout_ms = OTA_HTTP_TIMEOUT_MS,
        .disable_auto_redirect = true,
        .max_authorization_retries = -1,
        .event_handler = metadata_http_event,
        .user_data = response,
        .crt_bundle_attach = esp_crt_bundle_attach,
        .buffer_size = OTA_HTTP_BUFFER_SIZE,
        .buffer_size_tx = OTA_HTTP_BUFFER_SIZE,
    };

    /* HTTP_CLIENT's DEBUG header dump includes Authorization verbatim. Keep
     * the tag permanently silent before the secret is handed to it. */
    esp_log_level_set("HTTP_CLIENT", ESP_LOG_NONE);
    esp_http_client_handle_t client = esp_http_client_init(&config);
    if (client == NULL) {
        memset(authorization, 0, sizeof(authorization));
        heap_caps_free(response);
        return ESP_ERR_NO_MEM;
    }
    esp_err_t result = esp_http_client_set_header(
        client, "Authorization", authorization);
    memset(authorization, 0, sizeof(authorization));
    if (result == ESP_OK) {
        result = esp_http_client_perform(client);
    }
    int status = result == ESP_OK
                     ? esp_http_client_get_status_code(client)
                     : 0;
    if (http_status != NULL) {
        *http_status = status;
    }
    (void)esp_http_client_cleanup(client);

    result = prefer_transport_error(result, &response->diagnostic);

    if (result == ESP_OK && response->overflow) {
        result = ESP_ERR_INVALID_SIZE;
    } else if (result == ESP_OK && status == 204 && response->length == 0U) {
        result = ESP_ERR_NOT_FOUND;
    } else if (result == ESP_OK && status == 200 &&
               ota_policy_parse_metadata(response->bytes, response->length,
                                         metadata)) {
        result = ESP_OK;
    } else if (result == ESP_OK) {
        result = ESP_ERR_INVALID_RESPONSE;
    }

    memset(response, 0, sizeof(*response));
    heap_caps_free(response);
    return result;
}

static esp_err_t wait_for_wifi(void)
{
    uint32_t waited_ms = 0U;
    while (wifi_station_state() != PROTOCOL_WIFI_CONNECTED) {
        if (protocol_task_ota_blocked()) {
            return ESP_ERR_INVALID_STATE;
        }
        if (waited_ms >= OTA_WIFI_WAIT_MS) {
            return ESP_ERR_TIMEOUT;
        }
        vTaskDelay(pdMS_TO_TICKS(OTA_WIFI_POLL_MS));
        waited_ms += OTA_WIFI_POLL_MS;
    }
    return ESP_OK;
}

static esp_err_t wait_for_owner_state(void)
{
    uint32_t waited_ms = 0U;
    for (;;) {
        if (protocol_task_ota_blocked()) {
            return ESP_ERR_INVALID_STATE;
        }
        if (!ota_policy_should_wait_for_owner(
                protocol_task_owner_state_ready(), waited_ms,
                OTA_OWNER_WAIT_MS)) {
            /* This wait must remain bounded and fail open. A device whose
             * owner can never become ready must still be updatable rather
             * than being stranded forever by a bad configuration. */
            return ESP_OK;
        }
        vTaskDelay(pdMS_TO_TICKS(OTA_WIFI_POLL_MS));
        waited_ms += OTA_WIFI_POLL_MS;
    }
}

static uint8_t progress_percentage(esp_https_ota_handle_t handle,
                                   int image_size)
{
    int image_read = esp_https_ota_get_image_len_read(handle);
    if (image_read <= 0 || image_size <= 0) {
        return 0U;
    }
    uint64_t percentage = (uint64_t)(unsigned)image_read * UINT64_C(100) /
                          (uint64_t)(unsigned)image_size;
    return percentage > 99U ? 99U : (uint8_t)percentage;
}

static bool image_version_matches(const esp_app_desc_t *description,
                                  const char *expected)
{
    size_t actual_length = strnlen(description->version,
                                   sizeof(description->version));
    size_t expected_length = strnlen(
        expected, PROTOCOL_MAX_FIRMWARE_VERSION_LENGTH + 1U);
    return expected_length <= PROTOCOL_MAX_FIRMWARE_VERSION_LENGTH &&
           actual_length == expected_length &&
           memcmp(description->version, expected, actual_length) == 0;
}

static bool copy_running_version(char *out, size_t out_capacity)
{
    const esp_app_desc_t *description = esp_app_get_description();
    size_t length = strnlen(description->version,
                            sizeof(description->version));
    if (out == NULL || out_capacity <= length) {
        return false;
    }
    memcpy(out, description->version, length);
    out[length] = '\0';
    return length != 0U;
}

static esp_err_t reject_reinstall_of_failed_image(
    const esp_partition_t *update_partition,
    const char *candidate_version)
{
    esp_ota_img_states_t state;
    esp_err_t result = esp_ota_get_state_partition(update_partition, &state);
    if (result == ESP_ERR_NOT_FOUND || result == ESP_ERR_NOT_SUPPORTED) {
        return ESP_OK;
    }
    if (result != ESP_OK) {
        return result;
    }
    if (state != ESP_OTA_IMG_INVALID && state != ESP_OTA_IMG_ABORTED) {
        return ESP_OK;
    }

    esp_app_desc_t failed_description;
    memset(&failed_description, 0, sizeof(failed_description));
    result = esp_ota_get_partition_description(update_partition,
                                               &failed_description);
    if (result != ESP_OK) {
        /* An interrupted attempt to replace a rolled-back image erases this
         * descriptor while leaving the old ABORTED/INVALID otadata state.
         * With no readable image, there is no failed version to protect from
         * reinstall; allow a future candidate to repopulate the slot. */
        ESP_LOGW(TAG, "failed OTA slot has no readable image; allowing "
                      "replacement");
        return ESP_OK;
    }
    if (image_version_matches(&failed_description, candidate_version)) {
        ESP_LOGW(TAG, "server still advertises the image that rolled back; "
                      "leaving failed slot untouched");
        return ESP_ERR_OTA_ROLLBACK_FAILED;
    }
    return ESP_OK;
}

/* Reopens the link this download closed. Only the success path skips it, and
 * only because it reboots: every failure must hand the server its device back,
 * or one failed download would strand the display until the next reset. */
static void resume_link_if_suspended(bool suspended)
{
    if (!suspended) {
        return;
    }
    esp_err_t result = net_link_resume();
    if (result != ESP_OK) {
        ESP_LOGE(TAG, "could not resume the network link (%s); the device is "
                      "offline until the next reset", esp_err_to_name(result));
    }
}

static esp_err_t install_update(const char *server_url,
                                const char *running_version,
                                const ota_policy_metadata_t *metadata)
{
    if (strcmp(metadata->version, running_version) == 0) {
        set_failure("verify", ESP_ERR_INVALID_VERSION);
        return ESP_ERR_INVALID_VERSION;
    }
    char download_url[OTA_POLICY_MAX_URL_LENGTH + 1U];
    if (!ota_policy_build_download_url(server_url, metadata->path,
                                       download_url,
                                       sizeof(download_url))) {
        set_failure("begin", ESP_ERR_INVALID_ARG);
        return ESP_ERR_INVALID_ARG;
    }
    if (protocol_task_ota_blocked()) {
        return ESP_ERR_INVALID_STATE;
    }

    const esp_partition_t *running_partition = esp_ota_get_running_partition();
    const esp_partition_t *update_partition =
        esp_ota_get_next_update_partition(running_partition);
    if (running_partition == NULL || update_partition == NULL) {
        set_failure("begin", ESP_ERR_NOT_FOUND);
        return ESP_ERR_NOT_FOUND;
    }
    esp_err_t result = reject_reinstall_of_failed_image(
        update_partition, metadata->version);
    if (result != ESP_OK) {
        set_failure("verify", result);
        return result;
    }

    set_state(PROTOCOL_OTA_DOWNLOADING);
    bool screen_shown = ota_screen_show_progress(0U);
    if (!screen_shown) {
        set_failure("begin", ESP_ERR_NO_MEM);
        return ESP_ERR_NO_MEM;
    }

    ota_http_diagnostic_t http_diagnostic = {
        .transport_error = ESP_OK,
    };

    esp_http_client_config_t http_config = {
        .url = download_url,
        .method = HTTP_METHOD_GET,
        .timeout_ms = OTA_HTTP_TIMEOUT_MS,
        .disable_auto_redirect = true,
        .max_authorization_retries = -1,
        .crt_bundle_attach = esp_crt_bundle_attach,
        .buffer_size = OTA_IMAGE_BUFFER_SIZE,
        .buffer_size_tx = OTA_HTTP_BUFFER_SIZE,
        .keep_alive_enable = true,
        .event_handler = download_http_event,
        .user_data = &http_diagnostic,
    };
    esp_https_ota_config_t ota_config = {
        .http_config = &http_config,
        .buffer_caps = MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT,
        // `final` is deliberately left NULL rather than set equal to
        // `staging`. Naming the same partition twice -- the honest way to
        // describe a single-partition update -- walks into a hole in
        // esp_https_ota_begin(): when `final` is non-NULL it only assigns
        // handle->partition.final inside `if (staging != final)`, with no
        // else branch, so passing them equal leaves that pointer unset.
        // get_description_from_image() then dereferences
        // handle->partition.final->type and the device panics
        // (LoadProhibited) before writing a byte. Leaving `final` NULL takes
        // esp_https_ota_begin()'s own `final = staging` path, which is the
        // semantics we want. Observed on hardware 2026-08-19 against
        // ESP-IDF v5.5.5; see docs/hardware/board-notes.md.
        .partition = {
            .staging = update_partition,
        },
    };
    // Give the download the radio to itself. Two concurrent TLS sessions
    // exhaust the internal DMA memory the hardware AES accelerator needs, and
    // the download dies mid-transfer with "esp-aes: Failed to allocate
    // memory". ESP_ERR_INVALID_STATE means there was no network link to
    // suspend -- an OTA over the cable in local tier -- which is not a fault.
    esp_err_t suspend_result = net_link_suspend();
    bool link_suspended = suspend_result == ESP_OK;
    if (!link_suspended && suspend_result != ESP_ERR_INVALID_STATE) {
        ESP_LOGW(TAG, "could not suspend the network link (%s); downloading "
                      "alongside it", esp_err_to_name(suspend_result));
    }

    esp_https_ota_handle_t handle = NULL;
    result = esp_https_ota_begin(&ota_config, &handle);
    result = prefer_transport_error(result, &http_diagnostic);
    if (result != ESP_OK) {
        ota_screen_close();
        resume_link_if_suspended(link_suspended);
        set_failure("begin", result);
        return result;
    }
    int download_status = esp_https_ota_get_status_code(handle);
    if (download_status != 200) {
        result = ESP_ERR_INVALID_RESPONSE;
        set_http_failure("download", download_status);
        goto abort_update;
    }

    int image_size = esp_https_ota_get_image_size(handle);
    if (image_size <= 0 || (size_t)image_size > update_partition->size) {
        result = ESP_ERR_INVALID_SIZE;
        set_failure("verify", result);
        goto abort_update;
    }
    esp_app_desc_t new_description;
    memset(&new_description, 0, sizeof(new_description));
    result = esp_https_ota_get_img_desc(handle, &new_description);
    result = prefer_transport_error(result, &http_diagnostic);
    if (result != ESP_OK ||
        !image_version_matches(&new_description, metadata->version)) {
        if (result == ESP_OK) {
            result = ESP_ERR_INVALID_VERSION;
        }
        set_failure("verify", result);
        goto abort_update;
    }

    uint8_t last_percentage = 0U;
    int last_image_read = esp_https_ota_get_image_len_read(handle);
    int64_t started_at = esp_timer_get_time();
    int64_t last_progress_at = started_at;
    do {
        result = esp_https_ota_perform(handle);
        int64_t now = esp_timer_get_time();
        int image_read = esp_https_ota_get_image_len_read(handle);
        if (image_read > last_image_read) {
            last_image_read = image_read;
            last_progress_at = now;
        }
        if (ota_policy_download_timed_out(
                (uint64_t)(now - started_at),
                (uint64_t)(now - last_progress_at))) {
            result = ESP_ERR_TIMEOUT;
            set_failure("download", result);
            goto abort_update;
        }
        uint8_t percentage = progress_percentage(handle, image_size);
        if (percentage != last_percentage) {
            if (!ota_screen_show_progress(percentage)) {
                result = ESP_ERR_NO_MEM;
                set_failure("download", result);
                goto abort_update;
            }
            last_percentage = percentage;
        }
    } while (result == ESP_ERR_HTTPS_OTA_IN_PROGRESS);

    result = prefer_transport_error(result, &http_diagnostic);
    if (result != ESP_OK || !esp_https_ota_is_complete_data_received(handle)) {
        if (result == ESP_OK) {
            result = ESP_ERR_INVALID_SIZE;
        }
        set_failure("download", result);
        goto abort_update;
    }
    result = esp_https_ota_finish(handle);
    handle = NULL;
    if (result != ESP_OK) {
        ota_screen_close();
        resume_link_if_suspended(link_suspended);
        set_failure("verify", result);
        return result;
    }
    set_state(PROTOCOL_OTA_PENDING_VERIFY);
    (void)ota_screen_show_progress(100U);
    ESP_LOGI(TAG, "firmware image installed; rebooting into pending slot");
    vTaskDelay(pdMS_TO_TICKS(250U));
    esp_restart();
    return ESP_OK;

abort_update:
    (void)esp_https_ota_abort(handle);
    ota_screen_close();
    resume_link_if_suspended(link_suspended);
    return result;
}

static esp_err_t perform_check(void)
{
    protocol_network_config_t stored;
    net_store_load(&stored);
    if (stored.ssid[0] == '\0' || stored.server_url[0] == '\0' ||
        stored.token[0] == '\0') {
        memset(&stored, 0, sizeof(stored));
        set_state(PROTOCOL_OTA_IDLE);
        return ESP_ERR_NOT_FOUND;
    }
    if (protocol_task_ota_blocked()) {
        memset(&stored, 0, sizeof(stored));
        set_state(PROTOCOL_OTA_IDLE);
        return ESP_ERR_INVALID_STATE;
    }

    set_state(PROTOCOL_OTA_CHECKING);
    esp_err_t result = wait_for_wifi();
    if (result != ESP_OK) {
        memset(&stored, 0, sizeof(stored));
        if (result == ESP_ERR_INVALID_STATE) {
            set_state(PROTOCOL_OTA_IDLE);
        } else {
            set_failure("check", result);
        }
        return result;
    }
    result = wait_for_owner_state();
    if (result != ESP_OK) {
        memset(&stored, 0, sizeof(stored));
        set_state(PROTOCOL_OTA_IDLE);
        return result;
    }

    char running_version[PROTOCOL_MAX_FIRMWARE_VERSION_LENGTH + 1U];
    if (!copy_running_version(running_version, sizeof(running_version))) {
        memset(&stored, 0, sizeof(stored));
        set_failure("check", ESP_ERR_INVALID_VERSION);
        return ESP_ERR_INVALID_VERSION;
    }
    char check_url[OTA_POLICY_MAX_URL_LENGTH + 1U];
    if (!ota_policy_build_check_url(stored.server_url, running_version,
                                    check_url, sizeof(check_url))) {
        memset(&stored, 0, sizeof(stored));
        memset(running_version, 0, sizeof(running_version));
        set_failure("check", ESP_ERR_INVALID_ARG);
        return ESP_ERR_INVALID_ARG;
    }

    ota_policy_metadata_t metadata;
    memset(&metadata, 0, sizeof(metadata));
    int check_status = 0;
    result = fetch_metadata(check_url, stored.token, &metadata,
                            &check_status);
    memset(check_url, 0, sizeof(check_url));
    memset(stored.token, 0, sizeof(stored.token));
    if (result == ESP_ERR_NOT_FOUND) {
        memset(&stored, 0, sizeof(stored));
        memset(running_version, 0, sizeof(running_version));
        clear_last_error();
        set_state(PROTOCOL_OTA_IDLE);
        return ESP_OK;
    }
    if (result != ESP_OK) {
        memset(&stored, 0, sizeof(stored));
        memset(running_version, 0, sizeof(running_version));
        if (check_status != 0 && check_status != 200 &&
            check_status != 204) {
            set_http_failure("check", check_status);
        } else {
            set_failure("check", result);
        }
        return result;
    }

    clear_last_error();
    result = install_update(stored.server_url, running_version, &metadata);
    memset(&metadata, 0, sizeof(metadata));
    memset(&stored, 0, sizeof(stored));
    memset(running_version, 0, sizeof(running_version));
    if (result == ESP_ERR_INVALID_STATE) {
        set_state(PROTOCOL_OTA_IDLE);
    }
    return result;
}

static uint32_t next_check_delay_ms(void)
{
    uint32_t span = OTA_CHECK_JITTER_MS * 2U + 1U;
    uint32_t jitter = esp_random() % span;
    return OTA_CHECK_INTERVAL_MS - OTA_CHECK_JITTER_MS + jitter;
}

static void ota_task(void *argument)
{
    (void)argument;
    for (;;) {
        esp_err_t result = perform_check();
        if (result == ESP_ERR_INVALID_STATE) {
            ESP_LOGI(TAG, "firmware check deferred by active focus state");
            while (protocol_task_ota_blocked()) {
                (void)ulTaskNotifyTake(
                    pdTRUE, pdMS_TO_TICKS(OTA_DEFERRED_POLL_MS));
            }
            continue;
        } else if (result != ESP_OK && result != ESP_ERR_NOT_FOUND) {
            ESP_LOGW(TAG, "firmware check failed: %s",
                     esp_err_to_name(result));
        }
        uint32_t delay_ticks = ota_policy_delay_ticks(
            next_check_delay_ms(), configTICK_RATE_HZ);
        (void)ulTaskNotifyTake(pdTRUE, (TickType_t)delay_ticks);
    }
}

esp_err_t ota_check_now(void)
{
    bool blocked = protocol_task_ota_blocked();
    protocol_ota_state_t state = ota_state();
    if (state == PROTOCOL_OTA_CHECKING ||
        state == PROTOCOL_OTA_DOWNLOADING ||
        state == PROTOCOL_OTA_PENDING_VERIFY) {
        return ESP_ERR_INVALID_STATE;
    }
    if (s_task != NULL) {
        xTaskNotifyGive(s_task);
        return blocked ? ESP_ERR_INVALID_STATE : ESP_OK;
    }

    bool expected = false;
    if (!atomic_compare_exchange_strong_explicit(
            &s_task_starting, &expected, true, memory_order_acq_rel,
            memory_order_acquire)) {
        return ESP_ERR_INVALID_STATE;
    }
    BaseType_t created = xTaskCreatePinnedToCore(
        ota_task, "ota", OTA_TASK_STACK_SIZE, NULL, OTA_TASK_PRIORITY,
        &s_task, OTA_TASK_CORE);
    atomic_store_explicit(&s_task_starting, false, memory_order_release);
    if (created != pdPASS) {
        s_task = NULL;
        set_failure("task", ESP_ERR_NO_MEM);
        return ESP_ERR_NO_MEM;
    }
    return blocked ? ESP_ERR_INVALID_STATE : ESP_OK;
}

esp_err_t ota_mark_running_image_valid(void)
{
    const esp_partition_t *running = esp_ota_get_running_partition();
    if (running == NULL) {
        return ESP_ERR_NOT_FOUND;
    }
    esp_ota_img_states_t image_state;
    esp_err_t result = esp_ota_get_state_partition(running, &image_state);
    if (result == ESP_ERR_NOT_FOUND || result == ESP_ERR_NOT_SUPPORTED) {
        set_state(PROTOCOL_OTA_IDLE);
        return ESP_OK;
    }
    if (result != ESP_OK || image_state != ESP_OTA_IMG_PENDING_VERIFY) {
        if (result == ESP_OK) {
            set_state(PROTOCOL_OTA_IDLE);
        }
        return result;
    }
    set_state(PROTOCOL_OTA_PENDING_VERIFY);

    bool lvgl_running = false;
    if (lvgl_port_lock(0U)) {
        lvgl_running = lv_display_get_default() != NULL;
        lvgl_port_unlock();
    }
    nvs_stats_t nvs_stats;
    bool nvs_readable = nvs_get_stats(NULL, &nvs_stats) == ESP_OK;
    bool display_up = board_display_io() != NULL;
    bool ui_running = ui_runtime_is_initialized();
    bool protocol_running = protocol_task_is_running();
    if (!display_up || !lvgl_running || !ui_running || !protocol_running ||
        !nvs_readable) {
        ESP_LOGE(TAG,
                 "pending image local self-check failed: display=%u lvgl=%u "
                 "ui=%u protocol=%u nvs=%u",
                 (unsigned)display_up, (unsigned)lvgl_running,
                 (unsigned)ui_running, (unsigned)protocol_running,
                 (unsigned)nvs_readable);
        return ESP_ERR_INVALID_STATE;
    }

    result = esp_ota_mark_app_valid_cancel_rollback();
    if (result == ESP_OK) {
        set_state(PROTOCOL_OTA_IDLE);
        ESP_LOGI(TAG, "pending firmware image marked valid by local self-check");
    }
    return result;
}
