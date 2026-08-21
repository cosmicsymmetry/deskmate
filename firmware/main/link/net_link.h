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

/**
 * Close the WebSocket so a firmware download can have the radio to itself,
 * and reopen it afterwards.
 *
 * This board cannot afford two concurrent TLS sessions. The hardware AES
 * accelerator's DMA buffers must come from internal memory -- PSRAM cannot
 * serve DMA -- and internal memory is already committed to LVGL and WiFi. A
 * second handshake alongside a live protocol link fails with
 * "esp-aes: Failed to allocate memory", and the download dies mid-transfer
 * (observed on hardware 2026-08-19). Suspending is also honest about what is
 * happening: the device is about to reboot into a new image, so there is
 * nothing for the server to say to it in the meantime.
 *
 * Both return ESP_ERR_INVALID_STATE when no client exists, which is the
 * ordinary case in local tier rather than a fault.
 */
esp_err_t net_link_suspend(void);
esp_err_t net_link_resume(void);
