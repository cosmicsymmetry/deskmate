#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "esp_err.h"

#include "core/protocol_message.h"

/**
 * Bring up the station using the persisted credentials and start SNTP.
 *
 * Returns ESP_OK when the attempt started, not when the join succeeded --
 * progress is observed through wifi_station_state(). With no stored SSID this
 * is a no-op returning ESP_OK: a device with no credentials is a working local
 * device, not an error.
 *
 * Named wifi_station_bringup() rather than the more obvious
 * wifi_station_start(): esp_wifi's own closed-source net80211 blob
 * (libnet80211.a) already exports a global C symbol called
 * wifi_station_start(), and the linker treats that as a multiple-definition
 * error against any app function of the same name. Discovered at link time
 * during this task; see wifi_station.c.
 */
esp_err_t wifi_station_bringup(void);

/**
 * Pump work that must not run on the caller SNTP notifies from.
 *
 * SNTP's sync callback runs on the lwIP tcpip task (unpinned, priority 18),
 * which can preempt either the LVGL task or the protocol task mid-critical-
 * section on either core. ui_runtime_set_utc_offset_minutes() -- and
 * everything it calls into -- uses bare spinlocks that are only safe because
 * their only two callers today (LVGL on core 0, protocol on core 1) can never
 * preempt each other. Calling it from the tcpip task risks spinning forever
 * on a lock its holder can no longer run to release, i.e. a watchdog reboot.
 *
 * So the SNTP callback only records that a sync happened; this function,
 * called from wifi_station_poll()'s intended caller (the protocol task's own
 * loop, which already ticks on a bounded read timeout), performs the actual
 * UI-facing call on a context that is safe to touch those locks. Must only
 * ever be called from the LVGL task or the protocol task -- do not call this
 * from a new context without re-checking that invariant.
 */
void wifi_station_poll(void);

protocol_wifi_state_t wifi_station_state(void);

/** Sampled live from the driver; 0 when there is no current association. */
int8_t wifi_station_rssi(void);

/**
 * Copies the current dotted-quad IPv4 (or an empty string when not
 * connected) into `out`, a buffer of `len` bytes owned by the caller,
 * NUL-terminated within that bound. No-op if `out` is NULL or `len` is 0.
 */
void wifi_station_copy_ip(char *out, size_t len);

/** True once SNTP has set the system clock at least once. */
bool wifi_station_time_synced(void);
