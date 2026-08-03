#pragma once
#include "esp_err.h"
#include "lvgl.h"

// Brings up the CST816S-family touch controller (silicon marked CST820 on
// this v2 board, see docs/hardware/board-notes.md) on the shared I2C bus and
// registers it as an LVGL pointer input device against `disp` (the display
// handle returned by lvgl_port_add_disp(), typically obtained in main.c via
// lv_display_get_default()). Must be called after board_display_init().
esp_err_t board_touch_init(lv_display_t *disp);
