#pragma once
#include "esp_err.h"
#include "esp_lcd_panel_io.h"

// Brings up the QSPI bus, the CO5300 panel (via TCA9554-expander reset +
// vendor init), and LVGL 9 through esp_lvgl_port. After this returns ESP_OK,
// LVGL APIs are usable from any task, guarded by lvgl_port_lock()/unlock().
esp_err_t board_display_init(void);

// Panel IO handle, exposed for Task 5's brightness command
// (esp_lcd_panel_io_tx_param against DCS 0x51).
esp_lcd_panel_io_handle_t board_display_io(void);
