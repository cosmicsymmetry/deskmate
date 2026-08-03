#pragma once
#include "driver/i2c_master.h"
#include "esp_io_expander.h"

// Shared I2C bus accessor for the board's single I2C bus (BOARD_I2C_PIN_SDA/SCL).
// This bus carries the TCA9554 IO expander (LCD/touch reset lines), the touch
// controller, the RTC, and the IMU (see docs/hardware/board-notes.md). Lazily
// created on first call so both display.c (Task 3, for the TCA9554 expander)
// and the touch driver (Task 4) can share one bus handle instead of each
// initializing I2C_NUM_0 independently, which would fail on the second init.
//
// Not thread-safe to call concurrently from multiple tasks on first use;
// app_main() calls it once during single-threaded bring-up before any other
// task touches it.
i2c_master_bus_handle_t board_i2c_bus(void);

// Shared TCA9554 IO-expander accessor. ALL access to the expander (EXIO0
// LCD_RESET, EXIO2 TP_RESET, and any other EXIOn in future milestones) MUST
// go through this function rather than calling
// esp_io_expander_new_i2c_tca9554() directly. Reason: that constructor
// unconditionally resets the *physical* chip's DIR and OUTPUT registers to
// 0xFF (see esp_io_expander_tca9554.c: esp_io_expander_new_i2c_tca9554() ->
// reset()) every time it is called, and each returned handle only tracks its
// own cached register shadow -- it never re-reads silicon. So a second,
// independent handle created later (e.g. by Task 4 for EXIO2/touch reset)
// would silently stomp whatever EXIO0/LCD_RESET was left at, and neither
// handle would know. Lazily created once and kept for the lifetime of the
// app (a singleton, not freed -- there is no del() call needed for
// something that lives as long as app_main()'s process does).
esp_io_expander_handle_t board_io_expander(void);
