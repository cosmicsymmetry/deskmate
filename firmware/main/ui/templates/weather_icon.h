#pragma once

/* The closed weather icon vocabulary. The host may send any string; anything
 * outside this set renders WEATHER_ICON_UNKNOWN. Keep this list in sync with
 * the weather provider in companion/crates/providers/src/weather.rs. */
typedef enum {
    WEATHER_ICON_UNKNOWN = 0,
    WEATHER_ICON_SUN,
    WEATHER_ICON_MOON,
    WEATHER_ICON_CLOUD,
    WEATHER_ICON_CLOUD_SUN,
    WEATHER_ICON_CLOUD_MOON,
    WEATHER_ICON_RAIN,
    WEATHER_ICON_DRIZZLE,
    WEATHER_ICON_SNOW,
    WEATHER_ICON_STORM,
    WEATHER_ICON_FOG,
} weather_icon_t;

/* Pure mapping, no LVGL dependency, NULL-safe. */
weather_icon_t weather_icon_from_name(const char *name);

#ifndef WEATHER_ICON_HOST_TEST
#include "lvgl.h"

/* Clears `container`'s children and draws `icon` into it using LVGL
 * primitives. Idempotent. The container must already be sized. */
void weather_icon_render(lv_obj_t *container, weather_icon_t icon,
                         lv_color_t color);
#endif
