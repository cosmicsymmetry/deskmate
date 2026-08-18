#pragma once

#include <stddef.h>
#include <stdint.h>

#include "esp_err.h"
#include "freertos/FreeRTOS.h"

/**
 * One protocol transport. The protocol task holds exactly one at a time,
 * selected by tier, and knows nothing about cables or sockets.
 *
 * `read` returns zero on timeout. `write_frame` receives one complete,
 * zero-delimited wire frame -- the COBS framing is identical on every
 * transport, so protocol_frame.c is unchanged.
 */
typedef struct {
    const char *name;
    size_t (*read)(uint8_t *out, size_t capacity, TickType_t timeout_ticks);
    esp_err_t (*write_frame)(const uint8_t *frame, size_t length,
                             TickType_t timeout_ticks);
    uint32_t (*dropped_bytes)(void);
    uint32_t link_timeout_ms;
} link_transport_t;
