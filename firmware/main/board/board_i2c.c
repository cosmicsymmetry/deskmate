#include "board_i2c.h"
#include "board.h"
#include "esp_check.h"
#include "esp_io_expander_tca9554.h"
#include "esp_log.h"

static const char *TAG = "board_i2c";

static i2c_master_bus_handle_t s_bus = NULL;
static esp_io_expander_handle_t s_expander = NULL;

i2c_master_bus_handle_t board_i2c_bus(void)
{
    if (s_bus != NULL) {
        return s_bus;
    }

    const i2c_master_bus_config_t bus_config = {
        .i2c_port = BOARD_I2C_PORT,
        .sda_io_num = BOARD_I2C_PIN_SDA,
        .scl_io_num = BOARD_I2C_PIN_SCL,
        .clk_source = I2C_CLK_SRC_DEFAULT,
        .glitch_ignore_cnt = 7,
        .flags.enable_internal_pullup = true,
    };

    esp_err_t err = i2c_new_master_bus(&bus_config, &s_bus);
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "i2c_new_master_bus failed: %s", esp_err_to_name(err));
        s_bus = NULL;
        return NULL;
    }

    ESP_LOGI(TAG, "I2C bus ready on port %d (SDA=%d, SCL=%d)",
              BOARD_I2C_PORT, BOARD_I2C_PIN_SDA, BOARD_I2C_PIN_SCL);
    return s_bus;
}

esp_io_expander_handle_t board_io_expander(void)
{
    if (s_expander != NULL) {
        return s_expander;
    }

    i2c_master_bus_handle_t bus = board_i2c_bus();
    if (bus == NULL) {
        return NULL;
    }

    // NOTE: esp_io_expander_new_i2c_tca9554() unconditionally resets the
    // physical chip's DIR/OUTPUT registers to 0xFF as part of construction.
    // This must only ever run once per boot -- see the big comment in
    // board_i2c.h. Callers must use board_io_expander(), never call
    // esp_io_expander_new_i2c_tca9554() themselves.
    esp_err_t err = esp_io_expander_new_i2c_tca9554(
        bus, ESP_IO_EXPANDER_I2C_TCA9554_ADDRESS_000, &s_expander);
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "esp_io_expander_new_i2c_tca9554 failed: %s", esp_err_to_name(err));
        s_expander = NULL;
        return NULL;
    }

    ESP_LOGI(TAG, "TCA9554 IO expander ready at 0x%02x",
              ESP_IO_EXPANDER_I2C_TCA9554_ADDRESS_000);
    return s_expander;
}
