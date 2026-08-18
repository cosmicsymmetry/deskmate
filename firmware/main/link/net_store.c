#include "net_store.h"

#include <string.h>

#include "esp_log.h"
#include "nvs.h"

#include "core/net_config.h"

static const char *TAG = "net_store";
static const char *NVS_NAMESPACE = "deskmate";

static const char *KEY_SSID = "ssid";
static const char *KEY_PSK = "psk";
static const char *KEY_SERVER_URL = "server_url";
static const char *KEY_DEVICE_ID = "device_id";
static const char *KEY_TOKEN = "token";
static const char *KEY_UTC_OFFSET = "utc_offset";
static const char *KEY_TIER = "tier";

// The tier cache latches to the FIRST net_store_load() call and never moves
// again. Neither net_store_save() nor net_store_erase() touches it, and a
// later net_store_load() call (a future networking/status task reading the
// stored config, say) must not re-latch it either: a newly provisioned or
// erased config always takes effect on the next boot rather than
// hot-swapping the device's owner mid-session (see protocol_task.c's
// dispatch handlers). s_loaded is what makes that true regardless of how
// many times net_store_load() is called or by whom.
static protocol_tier_t s_current_tier = PROTOCOL_TIER_LOCAL;
static bool s_loaded = false;

// Reads one string field, logging (never the value -- it may be a secret)
// and leaving the field empty on any error other than "key not present",
// which is the ordinary factory-fresh/partial-write case.
static void load_string_field(nvs_handle_t handle,
                              const char *key,
                              char *out,
                              size_t capacity,
                              const char *field_name)
{
    size_t length = capacity;
    esp_err_t result = nvs_get_str(handle, key, out, &length);
    if (result != ESP_OK) {
        if (result != ESP_ERR_NVS_NOT_FOUND) {
            ESP_LOGW(TAG, "failed to read %s: %s", field_name,
                    esp_err_to_name(result));
        }
        out[0] = '\0';
    }
}

void net_store_load(protocol_network_config_t *out)
{
    if (out == NULL) {
        return;
    }
    memset(out, 0, sizeof(*out));

    nvs_handle_t handle;
    esp_err_t open_result = nvs_open(NVS_NAMESPACE, NVS_READONLY, &handle);
    if (open_result != ESP_OK) {
        if (open_result != ESP_ERR_NVS_NOT_FOUND) {
            ESP_LOGW(TAG, "failed to open namespace for read: %s",
                    esp_err_to_name(open_result));
        }
        // Factory-fresh or unreadable: out is already zeroed, which
        // net_config_effective_tier() below resolves to local tier.
        out->tier = net_config_effective_tier(out);
        if (!s_loaded) {
            s_current_tier = out->tier;
            s_loaded = true;
        }
        return;
    }

    load_string_field(handle, KEY_SSID, out->ssid, sizeof(out->ssid), "ssid");
    load_string_field(handle, KEY_PSK, out->psk, sizeof(out->psk), "psk");
    load_string_field(handle, KEY_SERVER_URL, out->server_url,
                      sizeof(out->server_url), "server_url");
    load_string_field(handle, KEY_DEVICE_ID, out->device_id,
                      sizeof(out->device_id), "device_id");
    load_string_field(handle, KEY_TOKEN, out->token, sizeof(out->token), "token");

    int16_t utc_offset = 0;
    esp_err_t offset_result = nvs_get_i16(handle, KEY_UTC_OFFSET, &utc_offset);
    if (offset_result == ESP_OK) {
        // net_config_validate() only gates the write path (dispatch_
        // network_config() calls it before net_store_save()); it cannot see
        // a value that reached flash before that check existed, or one a
        // corrupt/bit-flipped NVS record now returns. This is the read
        // path, so it is where an out-of-range stored value must be
        // sanitised rather than handed to wifi_station_bringup() verbatim --
        // otherwise a corrupt int16_t could shift the standalone clock by
        // up to +/-546 hours. UTC (0) is the honest fallback: the zone is
        // unreadable, not the time itself.
        if (utc_offset < PROTOCOL_MIN_UTC_OFFSET_MINUTES ||
            utc_offset > PROTOCOL_MAX_UTC_OFFSET_MINUTES) {
            ESP_LOGW(TAG, "utc_offset_minutes out of range; using UTC");
            out->utc_offset_minutes = 0;
        } else {
            out->utc_offset_minutes = utc_offset;
        }
    } else {
        if (offset_result != ESP_ERR_NVS_NOT_FOUND) {
            ESP_LOGW(TAG, "failed to read utc_offset_minutes: %s",
                    esp_err_to_name(offset_result));
        }
        out->utc_offset_minutes = 0;
    }

    int8_t tier_value = (int8_t)PROTOCOL_TIER_LOCAL;
    esp_err_t tier_result = nvs_get_i8(handle, KEY_TIER, &tier_value);
    if (tier_result == ESP_OK) {
        out->tier = (protocol_tier_t)tier_value;
    } else {
        if (tier_result != ESP_ERR_NVS_NOT_FOUND) {
            ESP_LOGW(TAG, "failed to read tier: %s", esp_err_to_name(tier_result));
        }
        out->tier = PROTOCOL_TIER_LOCAL;
    }

    nvs_close(handle);

    // A half-written, corrupt, or otherwise invalid config can never present
    // itself as networked: recompute the effective tier from validation
    // instead of trusting the raw stored value.
    out->tier = net_config_effective_tier(out);
    if (!s_loaded) {
        s_current_tier = out->tier;
        s_loaded = true;
    }
}

esp_err_t net_store_save(const protocol_network_config_t *config)
{
    if (config == NULL) {
        return ESP_ERR_INVALID_ARG;
    }

    nvs_handle_t handle;
    esp_err_t result = nvs_open(NVS_NAMESPACE, NVS_READWRITE, &handle);
    if (result != ESP_OK) {
        ESP_LOGW(TAG, "failed to open namespace for write: %s",
                esp_err_to_name(result));
        return result;
    }

    result = nvs_set_str(handle, KEY_SSID, config->ssid);
    if (result == ESP_OK) {
        result = nvs_set_str(handle, KEY_PSK, config->psk);
    }
    if (result == ESP_OK) {
        result = nvs_set_str(handle, KEY_SERVER_URL, config->server_url);
    }
    if (result == ESP_OK) {
        result = nvs_set_str(handle, KEY_DEVICE_ID, config->device_id);
    }
    if (result == ESP_OK) {
        result = nvs_set_str(handle, KEY_TOKEN, config->token);
    }
    if (result == ESP_OK) {
        result = nvs_set_i16(handle, KEY_UTC_OFFSET, config->utc_offset_minutes);
    }
    if (result == ESP_OK) {
        result = nvs_set_i8(handle, KEY_TIER, (int8_t)config->tier);
    }
    if (result == ESP_OK) {
        result = nvs_commit(handle);
    }
    if (result != ESP_OK) {
        ESP_LOGW(TAG, "failed to persist network config: %s",
                esp_err_to_name(result));
    }

    nvs_close(handle);
    return result;
}

esp_err_t net_store_erase(void)
{
    nvs_handle_t handle;
    esp_err_t result = nvs_open(NVS_NAMESPACE, NVS_READWRITE, &handle);
    if (result != ESP_OK) {
        if (result == ESP_ERR_NVS_NOT_FOUND) {
            // Nothing was ever stored: already factory-fresh.
            return ESP_OK;
        }
        ESP_LOGW(TAG, "failed to open namespace for erase: %s",
                esp_err_to_name(result));
        return result;
    }

    result = nvs_erase_all(handle);
    if (result == ESP_OK) {
        result = nvs_commit(handle);
    }
    if (result != ESP_OK) {
        ESP_LOGW(TAG, "failed to erase network config: %s",
                esp_err_to_name(result));
    }

    nvs_close(handle);
    return result;
}

protocol_tier_t net_store_current_tier(void)
{
    return s_current_tier;
}
