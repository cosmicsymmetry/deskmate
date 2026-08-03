#include <stdio.h>
#include "esp_log.h"
#include "board/board.h"

static const char *TAG = "deskmate";

void app_main(void)
{
    ESP_LOGI(TAG, "deskmate M0 boot");
    ESP_LOGI(TAG, "board: %dx%d LCD, QSPI CS=%d PCLK=%d D0-D3=%d,%d,%d,%d",
             BOARD_LCD_H_RES, BOARD_LCD_V_RES,
             BOARD_LCD_PIN_CS, BOARD_LCD_PIN_PCLK,
             BOARD_LCD_PIN_D0, BOARD_LCD_PIN_D1, BOARD_LCD_PIN_D2, BOARD_LCD_PIN_D3);
}
