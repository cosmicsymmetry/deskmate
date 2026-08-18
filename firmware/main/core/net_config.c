#include "net_config.h"

#include <string.h>

net_config_error_t net_config_validate(const protocol_network_config_t *config)
{
    if (config == NULL) {
        return NET_CONFIG_ERR_MISSING_SSID;
    }

    switch (config->tier) {
        case PROTOCOL_TIER_LOCAL:
            return NET_CONFIG_OK;
        case PROTOCOL_TIER_NETWORKED:
            break;
        default:
            return NET_CONFIG_ERR_INVALID_TIER;
    }

    if (config->ssid[0] == '\0') {
        return NET_CONFIG_ERR_MISSING_SSID;
    }
    if (config->server_url[0] == '\0') {
        return NET_CONFIG_ERR_MISSING_SERVER_URL;
    }
    if (strncmp(config->server_url, "wss://", 6) != 0) {
        return NET_CONFIG_ERR_INSECURE_URL;
    }
    if (config->token[0] == '\0') {
        return NET_CONFIG_ERR_MISSING_TOKEN;
    }

    return NET_CONFIG_OK;
}

protocol_tier_t net_config_effective_tier(const protocol_network_config_t *config)
{
    if (config == NULL) {
        return PROTOCOL_TIER_LOCAL;
    }

    if (net_config_validate(config) != NET_CONFIG_OK) {
        return PROTOCOL_TIER_LOCAL;
    }

    return config->tier;
}

bool net_config_usb_message_allowed(protocol_tier_t tier,
                                    protocol_message_type_t type)
{
    if (tier == PROTOCOL_TIER_LOCAL) {
        return true;
    }

    switch (type) {
        case PROTOCOL_TYPE_STATUS_REQUEST:
        case PROTOCOL_TYPE_STATUS_RESPONSE:
        case PROTOCOL_TYPE_ACK:
        case PROTOCOL_TYPE_HEARTBEAT:
        case PROTOCOL_TYPE_HEARTBEAT_ACK:
        case PROTOCOL_TYPE_ERROR:
        case PROTOCOL_TYPE_DEVICE_EVENT:
        case PROTOCOL_TYPE_NETWORK_CONFIG:
        case PROTOCOL_TYPE_FACTORY_RESET:
            return true;
        case PROTOCOL_TYPE_TIME_SYNC:
        case PROTOCOL_TYPE_PUSH_DATA:
        case PROTOCOL_TYPE_APPLY_CONFIG:
        case PROTOCOL_TYPE_ACTIVATE_SCREEN:
        case PROTOCOL_TYPE_TRIGGER_INTERRUPT:
        default:
            return false;
    }
}
