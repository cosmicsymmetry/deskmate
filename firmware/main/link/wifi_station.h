#pragma once

#include <stdbool.h>
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

protocol_wifi_state_t wifi_station_state(void);
int8_t wifi_station_rssi(void);

/** Dotted-quad IPv4, or an empty string when not connected. */
const char *wifi_station_ip(void);

/** True once SNTP has set the system clock at least once. */
bool wifi_station_time_synced(void);
