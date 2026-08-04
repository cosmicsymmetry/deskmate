#pragma once

#include <stdbool.h>
#include <stdint.h>

// Standalone fallback clock screen (spec §3): big HH:MM, date line, and a
// dim "Connect deskmate app" hint. During M1 acceptance, releasing in the
// visual top half cycles brightness and releasing in the visual bottom half
// flips the landscape orientation by 180°. Reads the system clock via time()/
// gmtime_r() after applying M1's fixed UTC offset. Until a time sync lands,
// this ticks up from the epoch
// (1970-01-01 00:00 UTC), which is expected for M0. Must be called after
// board_display_init() (needs an active LVGL display/screen).
void clock_screen_show(void);

// Same operation when the caller already runs in LVGL context. It must not
// be called while holding a non-LVGL task context because it does not acquire
// lvgl_port's lock.
void clock_screen_show_in_lvgl(void);

// Thread-safe external entry points. Both retain state if the screen is
// rebuilt; the LVGL timer consumes their fixed atomic mailboxes in its own
// task context without allocating per update.
void clock_screen_set_online(bool online);
void clock_screen_set_utc_offset_minutes(int16_t offset_minutes);
