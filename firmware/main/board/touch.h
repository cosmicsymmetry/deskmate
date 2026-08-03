#pragma once
#include "esp_err.h"
#include "esp_lcd_touch.h"
#include "lvgl.h"

// Brings up the CST816S-family touch controller (silicon marked CST820 on
// this v2 board, see docs/hardware/board-notes.md) on the shared I2C bus and
// registers it as an LVGL pointer input device against `disp` (the display
// handle returned by lvgl_port_add_disp(), typically obtained in main.c via
// lv_display_get_default()). Must be called after board_display_init().
esp_err_t board_touch_init(lv_display_t *disp);

// Touch controller handle, exposed for Task 5's rotation handling
// (esp_lcd_touch_set_mirror_x/y) -- display.c's board_display_set_rotation_180()
// calls this so display + touch rotation stay wired through one entry point.
// Returns NULL if board_touch_init() hasn't completed yet.
esp_lcd_touch_handle_t board_touch_handle(void);
