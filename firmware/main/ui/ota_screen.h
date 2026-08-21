#pragma once

#include <stdbool.h>
#include <stdint.h>

/** Show or update the full-screen OTA takeover. Safe from non-LVGL tasks. */
bool ota_screen_show_progress(uint8_t percentage);

/** Return to the standalone clock after a failed/aborted update. */
void ota_screen_close(void);

/** Called only by the LVGL UI consumer while it already owns the LVGL lock. */
bool ota_screen_active_in_lvgl(void);
