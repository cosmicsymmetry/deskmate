#pragma once
#include <stdint.h>
#include <stdbool.h>
#include "esp_err.h"
#include "esp_lcd_panel_io.h"

// Brings up the QSPI bus, the CO5300 panel (via TCA9554-expander reset +
// vendor init), and LVGL 9 through esp_lvgl_port. After this returns ESP_OK,
// LVGL APIs are usable from any task, guarded by lvgl_port_lock()/unlock().
esp_err_t board_display_init(void);

// Panel IO handle, exposed for future direct DCS access (M1+)
// (esp_lcd_panel_io_tx_param against DCS 0x51).
esp_lcd_panel_io_handle_t board_display_io(void);

// Sets display brightness via DCS "Write Display Brightness" (0x51),
// 0 (off) - 255 (max). This is the single code path for brightness: both
// board_display_init()'s initial brightness and any later caller (config
// handling in later milestones, main.c's tap-zone test) go through this
// function rather than sending the DCS command directly.
esp_err_t board_display_set_brightness(uint8_t level);

// Toggles 180-degree display rotation (for the spec's cable-exit-either-side
// enclosure requirement). Uses LVGL 9 software rotation
// (lv_display_set_rotation) rather than the CO5300's own MADCTL mirror bits
// -- see board-notes.md ("Task 5: rotation path") for why: the panel's fixed
// x_gap column-address correction (BOARD_LCD_X_GAP) is applied unconditionally
// in the co5300 driver's draw_bitmap regardless of mirror state, so a
// hardware mirror risks moving the green uninitialized-RAM strip to the
// opposite edge in a way that cannot be verified without eyes on the panel.
// Software rotation never touches the panel's gap/addressing at all.
//
// Touch coordinates need no separate handling here: LVGL core itself remaps
// every pointer indev's coordinates for the display's current rotation
// unconditionally (lv_indev.c's indev_pointer_proc() ->
// lv_display_rotate_point(), independent of the sw_rotate flag above, which
// only governs esp_lvgl_port's own mirror/flush-buffer path). Do NOT add a
// driver-level touch mirror (e.g. esp_lcd_touch_set_mirror_x/y) on top of
// this -- an earlier version of this function did exactly that and it
// silently cancelled LVGL's own remap, leaving touch un-rotated while the
// image flipped. See board-notes.md ("Task 5: rotation path") for the full
// trace.
esp_err_t board_display_set_rotation_180(bool on);
