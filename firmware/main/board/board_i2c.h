#pragma once
#include "driver/i2c_master.h"

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
