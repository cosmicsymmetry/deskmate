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

/**
 * Persist a validated config. Returns ESP_ERR_INVALID_ARG on a NULL
 * argument.
 *
 * This is not an atomic write: each field is its own nvs_set_*() call, and
 * on current ESP-IDF those calls already write through to flash --
 * nvs_commit() does not buffer them and cannot roll a partial write back.
 * A power cut partway through can leave a mix of new and old field values
 * on flash. The mitigation is deliberate field ordering: `tier` is written
 * last, so an interrupted write cannot make a previously-local config
 * present as networked, and any other partial mix still degrades safely
 * through net_config_effective_tier() on the next load.
 */
esp_err_t net_store_save(const protocol_network_config_t *config);

/** Erase the namespace, returning the device to factory-fresh local tier. */
esp_err_t net_store_erase(void);

/**
 * The effective tier of the persisted config, latched to the first
 * net_store_load() call for the life of the process and never updated by
 * any later call (from this or any other caller) or by net_store_save()/
 * net_store_erase(). A newly provisioned or erased config only takes
 * effect on the next boot.
 */
protocol_tier_t net_store_current_tier(void);
