#include <assert.h>
#include <string.h>

#include "../main/core/net_config.h"

static protocol_network_config_t valid_networked(void)
{
    protocol_network_config_t config;
    memset(&config, 0, sizeof(config));
    strcpy(config.ssid, "home-network");
    strcpy(config.psk, "correct horse battery staple");
    strcpy(config.server_url, "wss://deskmate.example.com/v1/device/link");
    strcpy(config.device_id, "dev-0001");
    strcpy(config.token, "abc123");
    config.utc_offset_minutes = 240;
    config.tier = PROTOCOL_TIER_NETWORKED;
    return config;
}

static void test_valid_networked_config_passes(void)
{
    protocol_network_config_t config = valid_networked();
    assert(net_config_validate(&config) == NET_CONFIG_OK);
    assert(net_config_effective_tier(&config) == PROTOCOL_TIER_NETWORKED);
}

// A networked device with no server URL cannot reach anyone. The rule from the
// spec is that every failure lands in local tier with the standalone clock, so
// this must degrade rather than be rejected into an unusable state.
static void test_networked_without_server_url_falls_back_to_local(void)
{
    protocol_network_config_t config = valid_networked();
    config.server_url[0] = '\0';
    assert(net_config_validate(&config) == NET_CONFIG_ERR_MISSING_SERVER_URL);
    assert(net_config_effective_tier(&config) == PROTOCOL_TIER_LOCAL);
}

static void test_networked_without_token_falls_back_to_local(void)
{
    protocol_network_config_t config = valid_networked();
    config.token[0] = '\0';
    assert(net_config_validate(&config) == NET_CONFIG_ERR_MISSING_TOKEN);
    assert(net_config_effective_tier(&config) == PROTOCOL_TIER_LOCAL);
}

static void test_networked_without_ssid_falls_back_to_local(void)
{
    protocol_network_config_t config = valid_networked();
    config.ssid[0] = '\0';
    assert(net_config_validate(&config) == NET_CONFIG_ERR_MISSING_SSID);
    assert(net_config_effective_tier(&config) == PROTOCOL_TIER_LOCAL);
}

// A plaintext ws:// URL would carry the bearer token in the clear.
static void test_non_tls_server_url_is_rejected(void)
{
    protocol_network_config_t config = valid_networked();
    strcpy(config.server_url, "ws://deskmate.example.com/v1/device/link");
    assert(net_config_validate(&config) == NET_CONFIG_ERR_INSECURE_URL);
    assert(net_config_effective_tier(&config) == PROTOCOL_TIER_LOCAL);
}

// Local tier needs no credentials at all -- an empty config is a valid local
// device, which is what a factory-fresh board is.
static void test_empty_config_is_a_valid_local_device(void)
{
    protocol_network_config_t config;
    memset(&config, 0, sizeof(config));
    config.tier = PROTOCOL_TIER_LOCAL;
    assert(net_config_validate(&config) == NET_CONFIG_OK);
    assert(net_config_effective_tier(&config) == PROTOCOL_TIER_LOCAL);
}

// Local tier still joins WiFi for SNTP and firmware, so an SSID is meaningful
// there, but its absence is not an error.
static void test_local_tier_without_ssid_is_not_an_error(void)
{
    protocol_network_config_t config;
    memset(&config, 0, sizeof(config));
    config.tier = PROTOCOL_TIER_LOCAL;
    strcpy(config.ssid, "home-network");
    assert(net_config_validate(&config) == NET_CONFIG_OK);
}

// A corrupt or malicious utc_offset_minutes must not reach the standalone
// clock unfiltered. Checked regardless of tier: local tier still joins WiFi
// for SNTP and applies this same field via wifi_station_bringup().
static void test_out_of_range_utc_offset_on_networked_config_falls_back_to_local(void)
{
    protocol_network_config_t config = valid_networked();
    config.utc_offset_minutes = 841;
    assert(net_config_validate(&config) == NET_CONFIG_ERR_INVALID_UTC_OFFSET);
    assert(net_config_effective_tier(&config) == PROTOCOL_TIER_LOCAL);
}

static void test_out_of_range_utc_offset_on_local_config_is_rejected(void)
{
    protocol_network_config_t config;
    memset(&config, 0, sizeof(config));
    config.tier = PROTOCOL_TIER_LOCAL;
    config.utc_offset_minutes = -841;
    assert(net_config_validate(&config) == NET_CONFIG_ERR_INVALID_UTC_OFFSET);
    assert(net_config_effective_tier(&config) == PROTOCOL_TIER_LOCAL);
}

// The +/-840 boundary values themselves (the same range the USB TimeSync
// path enforces) must still validate.
static void test_boundary_utc_offsets_are_valid(void)
{
    protocol_network_config_t config;
    memset(&config, 0, sizeof(config));
    config.tier = PROTOCOL_TIER_LOCAL;
    config.utc_offset_minutes = 840;
    assert(net_config_validate(&config) == NET_CONFIG_OK);
    config.utc_offset_minutes = -840;
    assert(net_config_validate(&config) == NET_CONFIG_OK);
}

// A corrupted or half-written NVS record could produce a tier byte that is
// neither LOCAL nor NETWORKED. That must not validate OK, and it must not be
// handed back verbatim as the effective tier -- it degrades to local like any
// other failure.
static void test_out_of_range_tier_on_otherwise_valid_config_degrades_to_local(void)
{
    protocol_network_config_t config = valid_networked();
    config.tier = (protocol_tier_t)7;
    assert(net_config_validate(&config) == NET_CONFIG_ERR_INVALID_TIER);
    assert(net_config_effective_tier(&config) == PROTOCOL_TIER_LOCAL);
}

static void test_out_of_range_tier_on_empty_config_degrades_to_local(void)
{
    protocol_network_config_t config;
    memset(&config, 0, sizeof(config));
    config.tier = (protocol_tier_t)7;
    assert(net_config_validate(&config) == NET_CONFIG_ERR_INVALID_TIER);
    assert(net_config_effective_tier(&config) == PROTOCOL_TIER_LOCAL);
}

// NULL is documented as degrading to local without crashing; exercise it
// directly rather than only by inspection.
static void test_null_config_is_handled_without_crashing(void)
{
    assert(net_config_validate(NULL) == NET_CONFIG_ERR_MISSING_SSID);
    assert(net_config_effective_tier(NULL) == PROTOCOL_TIER_LOCAL);
}

// The restricted USB message set in networked tier. Status, provisioning and
// reset are always allowed; anything that would make the cable an owner is not.
static void test_usb_message_gate_in_networked_tier(void)
{
    assert(net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                          PROTOCOL_TYPE_STATUS_REQUEST));
    assert(net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                          PROTOCOL_TYPE_NETWORK_CONFIG));
    assert(net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                          PROTOCOL_TYPE_FACTORY_RESET));
    assert(net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                          PROTOCOL_TYPE_HEARTBEAT));

    assert(!net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                           PROTOCOL_TYPE_APPLY_CONFIG));
    assert(!net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                           PROTOCOL_TYPE_PUSH_DATA));
    assert(!net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                           PROTOCOL_TYPE_TIME_SYNC));
    assert(!net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                           PROTOCOL_TYPE_ACTIVATE_SCREEN));
    assert(!net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                           PROTOCOL_TYPE_TRIGGER_INTERRUPT));
}

// In local tier the cable owns the device, so nothing is gated.
static void test_usb_message_gate_in_local_tier_allows_everything(void)
{
    assert(net_config_usb_message_allowed(PROTOCOL_TIER_LOCAL,
                                          PROTOCOL_TYPE_APPLY_CONFIG));
    assert(net_config_usb_message_allowed(PROTOCOL_TIER_LOCAL,
                                          PROTOCOL_TYPE_PUSH_DATA));
    assert(net_config_usb_message_allowed(PROTOCOL_TIER_LOCAL,
                                          PROTOCOL_TYPE_TIME_SYNC));
}

int main(void)
{
    test_valid_networked_config_passes();
    test_networked_without_server_url_falls_back_to_local();
    test_networked_without_token_falls_back_to_local();
    test_networked_without_ssid_falls_back_to_local();
    test_non_tls_server_url_is_rejected();
    test_empty_config_is_a_valid_local_device();
    test_local_tier_without_ssid_is_not_an_error();
    test_out_of_range_tier_on_otherwise_valid_config_degrades_to_local();
    test_out_of_range_tier_on_empty_config_degrades_to_local();
    test_out_of_range_utc_offset_on_networked_config_falls_back_to_local();
    test_out_of_range_utc_offset_on_local_config_is_rejected();
    test_boundary_utc_offsets_are_valid();
    test_null_config_is_handled_without_crashing();
    test_usb_message_gate_in_networked_tier();
    test_usb_message_gate_in_local_tier_allows_everything();
    return 0;
}
