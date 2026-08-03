#pragma once

// Standalone fallback clock screen (spec §3): big HH:MM, date line, and a
// dim "Connect deskmate app" hint. Reads the system clock via time()/
// localtime_r() -- until M1's time sync lands, this ticks up from the epoch
// (1970-01-01 00:00 UTC), which is expected for M0. Must be called after
// board_display_init() (needs an active LVGL display/screen).
void clock_screen_show(void);
