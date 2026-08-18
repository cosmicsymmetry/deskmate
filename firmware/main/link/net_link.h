#pragma once

#include "esp_err.h"

#include "link/link_transport.h"

/**
 * Connect to the provisioned server URL, presenting the stored bearer token in
 * the WebSocket handshake.
 *
 * Returns ESP_ERR_INVALID_STATE when the device is not in networked tier or
 * has no usable config -- both are ordinary conditions, not faults.
 */
esp_err_t net_link_start(void);

/** The network transport. `link_timeout_ms` is 45000. */
const link_transport_t *net_link_transport(void);
