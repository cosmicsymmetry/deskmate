#pragma once

#include <stdbool.h>

#include "esp_err.h"

#include "core/protocol_message.h"

/* Firmware-only diagnostic trigger: a StatusRequest carrying this reserved
 * request ID over native USB schedules an OTA check without changing the v1
 * wire schema. The ordinary StatusResponse is still returned. */
#define OTA_CHECK_STATUS_REQUEST_ID UINT32_MAX

/**
 * Confirm the running image so the bootloader stops holding a rollback.
 *
 * The self-check is deliberately local-only: display up, LVGL running,
 * protocol task alive, NVS readable. Requiring "reached the server" would roll
 * back a perfectly good image across the whole fleet during any outage --
 * network reachability is a runtime condition, never a validity condition.
 */
esp_err_t ota_mark_running_image_valid(void);

/**
 * Ask the server whether a newer image exists and install it if so.
 *
 * Returns ESP_ERR_INVALID_STATE while an interrupt is live or a pomodoro is
 * running, but keeps the request pending and retries shortly after that focus
 * state ends.
 */
esp_err_t ota_check_now(void);

protocol_ota_state_t ota_state(void);
