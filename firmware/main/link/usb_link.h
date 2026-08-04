#pragma once

#include <stddef.h>
#include <stdint.h>

#include "esp_err.h"
#include "freertos/FreeRTOS.h"

#ifdef __cplusplus
extern "C" {
#endif

// Protocol v1 allows decoded frames up to 2048 bytes. COBS can add one code
// byte per 254 input bytes plus its initial code byte; the wire frame then has
// one trailing zero delimiter.
#define USB_LINK_MAX_WIRE_FRAME_SIZE 2058U

/** Install the native USB Serial/JTAG driver for log-free protocol traffic. */
esp_err_t usb_link_init(void);

/**
 * Read application-channel bytes from the driver's bounded RX ring.
 *
 * There is one intended reader: the protocol task. Returns zero on timeout or
 * invalid arguments.
 */
size_t usb_link_read(uint8_t *out, size_t capacity, TickType_t timeout_ticks);

/**
 * Write one complete, zero-delimited wire frame on the native CDC channel.
 *
 * There is one intended writer: the protocol task. The call is rejected when
 * the driver is not initialized, the frame is too large, or its delimiter is
 * missing. A disconnected or stalled host is reported through the deadline.
 */
esp_err_t usb_link_write_frame(const uint8_t *frame, size_t length,
                               TickType_t timeout_ticks);

/** Reserved transport-drop counter; native driver overflow is not observable. */
uint32_t usb_link_rx_dropped_bytes(void);

#ifdef __cplusplus
}
#endif
