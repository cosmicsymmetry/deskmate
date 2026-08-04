#include "link/usb_link.h"

#include <stdbool.h>
#include <stdatomic.h>

#include "driver/usb_serial_jtag.h"
#include "esp_check.h"
#include "freertos/task.h"

#define USB_LINK_RX_QUEUE_CAPACITY 4096U
#define USB_LINK_TX_QUEUE_CAPACITY 4096U

static const char *TAG = "usb_link";
static atomic_bool s_initialized;

esp_err_t usb_link_init(void)
{
    if (atomic_load(&s_initialized)) {
        return ESP_ERR_INVALID_STATE;
    }

    usb_serial_jtag_driver_config_t config = {
        .rx_buffer_size = USB_LINK_RX_QUEUE_CAPACITY,
        .tx_buffer_size = USB_LINK_TX_QUEUE_CAPACITY,
    };
    ESP_RETURN_ON_ERROR(usb_serial_jtag_driver_install(&config), TAG,
                        "install native USB Serial/JTAG driver");

    atomic_store(&s_initialized, true);
    return ESP_OK;
}

size_t usb_link_read(uint8_t *out, size_t capacity, TickType_t timeout_ticks)
{
    if (!atomic_load(&s_initialized) || out == NULL || capacity == 0) {
        return 0;
    }

    int received = usb_serial_jtag_read_bytes(out, capacity, timeout_ticks);
    return received > 0 ? (size_t)received : 0;
}

esp_err_t usb_link_write_frame(const uint8_t *frame, size_t length,
                               TickType_t timeout_ticks)
{
    ESP_RETURN_ON_FALSE(atomic_load(&s_initialized), ESP_ERR_INVALID_STATE,
                        TAG, "USB link is not initialized");
    ESP_RETURN_ON_FALSE(frame != NULL && length > 0 &&
                            length <= USB_LINK_MAX_WIRE_FRAME_SIZE,
                        ESP_ERR_INVALID_ARG, TAG, "invalid TX frame");
    ESP_RETURN_ON_FALSE(frame[length - 1U] == 0, ESP_ERR_INVALID_ARG, TAG,
                        "TX frame is missing its zero delimiter");

    TickType_t started = xTaskGetTickCount();
    size_t sent = 0;
    while (sent < length) {
        TickType_t elapsed = xTaskGetTickCount() - started;
        if (elapsed >= timeout_ticks) {
            return ESP_ERR_TIMEOUT;
        }
        int written = usb_serial_jtag_write_bytes(frame + sent, length - sent,
                                                   timeout_ticks - elapsed);
        if (written <= 0) {
            return ESP_ERR_TIMEOUT;
        }
        sent += (size_t)written;
    }

    TickType_t elapsed = xTaskGetTickCount() - started;
    if (elapsed >= timeout_ticks) {
        return ESP_ERR_TIMEOUT;
    }
    return usb_serial_jtag_wait_tx_done(timeout_ticks - elapsed);
}

uint32_t usb_link_rx_dropped_bytes(void)
{
    // The native peripheral driver owns its fixed RX ring and does not expose
    // an overflow counter. Protocol-level overlong and malformed input is
    // counted by the incremental decoder instead.
    return 0;
}
