#pragma once

#include "esp_err.h"

#include "core/protocol_message.h"

/**
 * Load the persisted network config.
 *
 * On a factory-fresh device, on a missing namespace, or on any read error, the
 * config is zeroed and the tier is local. This function never fails in a way
 * the caller must handle differently -- the failure direction is always toward
 * local tier and the standalone clock.
 */
void net_store_load(protocol_network_config_t *out);

/** Persist a validated config. Returns ESP_ERR_INVALID_ARG on a NULL argument. */
esp_err_t net_store_save(const protocol_network_config_t *config);

/** Erase the namespace, returning the device to factory-fresh local tier. */
esp_err_t net_store_erase(void);

/** The effective tier of the persisted config, cached after first load. */
protocol_tier_t net_store_current_tier(void);
