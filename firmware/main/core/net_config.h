#pragma once

#include <stdbool.h>

#include "protocol_message.h"

typedef enum {
    NET_CONFIG_OK = 0,
    NET_CONFIG_ERR_MISSING_SSID,
    NET_CONFIG_ERR_MISSING_SERVER_URL,
    NET_CONFIG_ERR_MISSING_TOKEN,
    NET_CONFIG_ERR_INSECURE_URL,
    NET_CONFIG_ERR_INVALID_TIER,
    NET_CONFIG_ERR_INVALID_UTC_OFFSET,
} net_config_error_t;

/**
 * Validate a stored or received network config.
 *
 * Local tier requires nothing: a zeroed config is a valid factory-fresh local
 * device. Networked tier requires an SSID, a wss:// server URL and a token.
 * utc_offset_minutes is bounds-checked (PROTOCOL_MIN/MAX_UTC_OFFSET_MINUTES,
 * the same +/-840-minute range the USB TimeSync path already enforces)
 * regardless of tier, since local tier still joins WiFi for SNTP and applies
 * this same field -- an unchecked corrupt stored value could otherwise shift
 * the standalone clock by up to a full int16_t range. This is the sole gate:
 * rejected here means dispatch_network_config() never persists it, so a bad
 * value cannot reach NVS through the one production write path.
 */
net_config_error_t net_config_validate(const protocol_network_config_t *config);

/**
 * The tier the device may actually run in, which is never more privileged than
 * the tier the config asks for. Anything that fails validation degrades to
 * local, per the spec's rule that failure always lands on the standalone clock.
 */
protocol_tier_t net_config_effective_tier(const protocol_network_config_t *config);

/**
 * Whether a message arriving over USB may be acted on in the given tier.
 *
 * In networked tier the cable is a configurator, not an owner: it may read
 * status, write provisioning, and factory-reset, but may not apply config,
 * push data, sync time, activate screens or trigger interrupts.
 */
bool net_config_usb_message_allowed(protocol_tier_t tier,
                                    protocol_message_type_t type);
