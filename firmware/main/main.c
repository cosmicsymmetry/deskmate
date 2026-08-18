#include "esp_log.h"
#include "nvs_flash.h"
#include "board/board.h"
#include "board/display.h"
#include "board/touch.h"
#include "link/usb_link.h"
#include "link/protocol_task.h"
#include "link/wifi_station.h"
#include "ui/clock_screen.h"
#include "ui/ui_runtime.h"
#include "lvgl.h"

static const char *TAG = "deskmate";

// M0's fallback brightness level (0-255 DCS "Write Display Brightness"
// value); later milestones will make this configurable.
#define BOARD_INIT_BRIGHTNESS 200

void app_main(void)
{
    ESP_LOGI(TAG, "deskmate M2 boot");
    ESP_LOGI(TAG, "board: %dx%d LCD, QSPI CS=%d PCLK=%d D0-D3=%d,%d,%d,%d",
             BOARD_LCD_H_RES, BOARD_LCD_V_RES,
             BOARD_LCD_PIN_CS, BOARD_LCD_PIN_PCLK,
             BOARD_LCD_PIN_D0, BOARD_LCD_PIN_D1, BOARD_LCD_PIN_D2, BOARD_LCD_PIN_D3);

    esp_err_t nvs_status = nvs_flash_init();
    if (nvs_status == ESP_ERR_NVS_NO_FREE_PAGES ||
        nvs_status == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        // A truncated or version-shifted namespace is recoverable by erasing
        // it: the device returns to factory-fresh local tier, which is the
        // failure direction the spec requires. But the erase itself can also
        // fail (e.g. a flash read error) -- ESP_ERROR_CHECK()'ing that would
        // abort before board_display_init() and panic-reboot-loop the device
        // without ever reaching the standalone clock, the same class of bug
        // already fixed below for nvs_flash_init() itself. Degrade the same
        // way: only retry nvs_flash_init() if the erase succeeded, otherwise
        // log and fall through to the same "continue without NVS" handling.
        esp_err_t erase_status = nvs_flash_erase();
        if (erase_status == ESP_OK) {
            nvs_status = nvs_flash_init();
        } else {
            ESP_LOGE(TAG, "nvs_flash_erase failed (%s); continuing without NVS",
                    esp_err_to_name(erase_status));
            nvs_status = erase_status;
        }
    }
    if (nvs_status != ESP_OK) {
        // Any other NVS failure (corruption, a flash read error, no memory)
        // must not abort boot here: aborting before board_display_init()
        // would panic-reboot-loop the device without ever reaching the
        // standalone clock, which is the one thing that must always survive
        // NVS trouble. net_store_load()'s own nvs_open() will fail the same
        // way and already degrades to local tier on its own, so continuing
        // is safe -- provisioning/persistence are simply unavailable this
        // boot.
        ESP_LOGE(TAG, "nvs_flash_init failed (%s); continuing without NVS",
                esp_err_to_name(nvs_status));
    }

    ESP_ERROR_CHECK(board_display_init());
    ESP_LOGI(TAG, "board_display_init OK, io=%p", (void *)board_display_io());

    // Touch registers itself against the lv_display_t LVGL already created
    // inside board_display_init() (via lvgl_port_add_disp()); display.h
    // deliberately doesn't expose that handle, so pull it back out through
    // LVGL's own default-display accessor instead of changing display.h.
    // The clock screen doesn't consume touch input yet, but the input
    // pipeline must keep running for later milestones (soak-tested in
    // Task 7).
    lv_display_t *disp = lv_display_get_default();
    ESP_ERROR_CHECK(board_touch_init(disp));
    ESP_LOGI(TAG, "board_touch_init OK");

    ESP_ERROR_CHECK(board_display_set_brightness(BOARD_INIT_BRIGHTNESS));

    clock_screen_show();
    ESP_LOGI(TAG, "clock screen shown, LVGL task running");
    ESP_ERROR_CHECK(ui_runtime_init());

    // wifi_station_bringup() only starts the join attempt (or no-ops when no
    // SSID is stored); it never blocks on the network, so the clock screen
    // above is already showing whether or not WiFi ever comes up. Any
    // failure here is diagnostic only -- boot must not abort on it.
    esp_err_t wifi_status = wifi_station_bringup();
    ESP_LOGI(TAG, "wifi_station_bringup: %s", esp_err_to_name(wifi_status));

    // The native USB Serial/JTAG driver allocates bounded RX/TX rings from
    // internal RAM. Install it only after LVGL has secured its DMA and
    // software-rotation buffers so the M0 display path remains deterministic.
    ESP_ERROR_CHECK(usb_link_init());
    ESP_LOGI(TAG, "native USB Serial/JTAG protocol link ready");
    ESP_ERROR_CHECK(protocol_task_start());
    ESP_LOGI(TAG, "protocol task running");
}
