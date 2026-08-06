#include "weather_icon.h"

#include <string.h>

weather_icon_t weather_icon_from_name(const char *name)
{
    if (name == NULL) {
        return WEATHER_ICON_UNKNOWN;
    }
    static const struct {
        const char *name;
        weather_icon_t icon;
    } table[] = {
        { "sun", WEATHER_ICON_SUN },
        { "moon", WEATHER_ICON_MOON },
        { "cloud", WEATHER_ICON_CLOUD },
        { "cloud-sun", WEATHER_ICON_CLOUD_SUN },
        { "cloud-moon", WEATHER_ICON_CLOUD_MOON },
        { "rain", WEATHER_ICON_RAIN },
        { "drizzle", WEATHER_ICON_DRIZZLE },
        { "snow", WEATHER_ICON_SNOW },
        { "storm", WEATHER_ICON_STORM },
        { "fog", WEATHER_ICON_FOG },
        { "unknown", WEATHER_ICON_UNKNOWN },
    };
    for (size_t i = 0U; i < sizeof(table) / sizeof(table[0]); ++i) {
        if (strcmp(name, table[i].name) == 0) {
            return table[i].icon;
        }
    }
    return WEATHER_ICON_UNKNOWN;
}

#ifndef WEATHER_ICON_HOST_TEST
#include "lvgl.h"

#define ICON_BOX 120

static lv_obj_t *disc(lv_obj_t *parent, lv_color_t color, int16_t diameter,
                      int16_t x, int16_t y)
{
    lv_obj_t *obj = lv_obj_create(parent);
    lv_obj_remove_style_all(obj);
    lv_obj_remove_flag(obj, LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(obj, diameter, diameter);
    lv_obj_set_style_radius(obj, LV_RADIUS_CIRCLE, 0);
    lv_obj_set_style_bg_color(obj, color, 0);
    lv_obj_set_style_bg_opa(obj, LV_OPA_COVER, 0);
    lv_obj_align(obj, LV_ALIGN_CENTER, x, y);
    return obj;
}

static lv_obj_t *bar(lv_obj_t *parent, lv_color_t color, int16_t w, int16_t h,
                     int16_t x, int16_t y)
{
    lv_obj_t *obj = lv_obj_create(parent);
    lv_obj_remove_style_all(obj);
    lv_obj_remove_flag(obj, LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(obj, w, h);
    lv_obj_set_style_radius(obj, h / 2, 0);
    lv_obj_set_style_bg_color(obj, color, 0);
    lv_obj_set_style_bg_opa(obj, LV_OPA_COVER, 0);
    lv_obj_align(obj, LV_ALIGN_CENTER, x, y);
    return obj;
}

/* A cloud is three discs and a slab; every cloudy icon reuses it. */
static void draw_cloud(lv_obj_t *parent, lv_color_t color, int16_t dy)
{
    (void)disc(parent, color, 44, -18, dy);
    (void)disc(parent, color, 56, 4, dy - 8);
    (void)disc(parent, color, 40, 26, dy + 2);
    (void)bar(parent, color, 88, 26, 2, dy + 12);
}

static void draw_sun(lv_obj_t *parent, lv_color_t color, int16_t dx,
                     int16_t dy, int16_t diameter)
{
    (void)disc(parent, color, diameter, dx, dy);
}

static void draw_moon(lv_obj_t *parent, lv_color_t color, int16_t dx,
                      int16_t dy)
{
    /* Crescent: a lit disc with a background-coloured disc offset over it. */
    (void)disc(parent, color, 56, dx, dy);
    (void)disc(parent, lv_color_hex(0x000000), 48, dx + 16, dy - 8);
}

static void draw_drops(lv_obj_t *parent, lv_color_t color, int16_t count,
                       int16_t length)
{
    const int16_t xs[] = { -24, 0, 24 };
    for (int16_t i = 0; i < count && i < 3; ++i) {
        (void)bar(parent, color, 6, length, xs[i], 40);
    }
}

void weather_icon_render(lv_obj_t *container, weather_icon_t icon,
                         lv_color_t color)
{
    if (container == NULL) {
        return;
    }
    lv_obj_clean(container);
    lv_obj_set_size(container, ICON_BOX, ICON_BOX);

    switch (icon) {
    case WEATHER_ICON_SUN:
        draw_sun(container, color, 0, 0, 72);
        break;
    case WEATHER_ICON_MOON:
        draw_moon(container, color, 0, 0);
        break;
    case WEATHER_ICON_CLOUD:
        draw_cloud(container, color, 0);
        break;
    case WEATHER_ICON_CLOUD_SUN:
        draw_sun(container, color, -26, -30, 44);
        draw_cloud(container, color, 6);
        break;
    case WEATHER_ICON_CLOUD_MOON:
        draw_moon(container, color, -26, -30);
        draw_cloud(container, color, 6);
        break;
    case WEATHER_ICON_RAIN:
        draw_cloud(container, color, -12);
        draw_drops(container, color, 3, 22);
        break;
    case WEATHER_ICON_DRIZZLE:
        draw_cloud(container, color, -12);
        draw_drops(container, color, 2, 12);
        break;
    case WEATHER_ICON_SNOW:
        draw_cloud(container, color, -12);
        (void)disc(container, color, 10, -24, 42);
        (void)disc(container, color, 10, 0, 46);
        (void)disc(container, color, 10, 24, 42);
        break;
    case WEATHER_ICON_STORM:
        draw_cloud(container, color, -12);
        (void)bar(container, color, 10, 40, 0, 42);
        break;
    case WEATHER_ICON_FOG:
        draw_cloud(container, color, -16);
        (void)bar(container, color, 84, 8, -4, 30);
        (void)bar(container, color, 68, 8, 6, 46);
        break;
    case WEATHER_ICON_UNKNOWN:
    default:
        /* A hollow ring: unmistakably "no data", never a plausible-looking
         * wrong forecast. */
        (void)disc(container, color, 72, 0, 0);
        (void)disc(container, lv_color_hex(0x000000), 52, 0, 0);
        break;
    }
}
#endif
