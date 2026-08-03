#pragma once
#include "driver/gpio.h"
#include "driver/spi_master.h"
#include "driver/i2c_master.h"

// Waveshare ESP32-S3 Touch AMOLED 1.8 v2 (CO5300 panel, CST820/CST816S-family touch)
// Values sourced from the Waveshare wiki + schematic + waveshareteam GitHub demo sources —
// see docs/hardware/board-notes.md for the full derivation and citations.
#define BOARD_LCD_H_RES        368
#define BOARD_LCD_V_RES        448
#define BOARD_LCD_QSPI_HOST    SPI2_HOST
#define BOARD_LCD_PIN_PCLK     GPIO_NUM_11   // net QSPI_SCL
#define BOARD_LCD_PIN_CS       GPIO_NUM_12   // net LCD_CS
#define BOARD_LCD_PIN_D0       GPIO_NUM_4    // net QSPI_SIO0
#define BOARD_LCD_PIN_D1       GPIO_NUM_5    // net QSPI_SI1
#define BOARD_LCD_PIN_D2       GPIO_NUM_6    // net QSPI_SI2
#define BOARD_LCD_PIN_D3       GPIO_NUM_7    // net QSPI_SI3
#define BOARD_LCD_PIN_RST      GPIO_NUM_NC   // net LCD_RESET -> TCA9554 IO-expander EXIO0, not a direct ESP32 GPIO
#define BOARD_I2C_PORT         I2C_NUM_0
#define BOARD_I2C_PIN_SDA      GPIO_NUM_15   // net ESP32_SDA (shared bus: touch, RTC, IMU, IO expander)
#define BOARD_I2C_PIN_SCL      GPIO_NUM_14   // net ESP32_SCL (shared bus: touch, RTC, IMU, IO expander)
#define BOARD_TOUCH_PIN_INT    GPIO_NUM_21   // net TP_INT
#define BOARD_TOUCH_PIN_RST    GPIO_NUM_NC   // net TP_RESET -> TCA9554 IO-expander EXIO2, not a direct ESP32 GPIO
