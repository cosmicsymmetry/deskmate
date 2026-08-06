#include <assert.h>
#include <stdio.h>
#include <string.h>

#include "../main/ui/templates/weather_icon.h"

static void test_known_names_map(void)
{
    assert(weather_icon_from_name("sun") == WEATHER_ICON_SUN);
    assert(weather_icon_from_name("moon") == WEATHER_ICON_MOON);
    assert(weather_icon_from_name("cloud") == WEATHER_ICON_CLOUD);
    assert(weather_icon_from_name("cloud-sun") == WEATHER_ICON_CLOUD_SUN);
    assert(weather_icon_from_name("cloud-moon") == WEATHER_ICON_CLOUD_MOON);
    assert(weather_icon_from_name("rain") == WEATHER_ICON_RAIN);
    assert(weather_icon_from_name("drizzle") == WEATHER_ICON_DRIZZLE);
    assert(weather_icon_from_name("snow") == WEATHER_ICON_SNOW);
    assert(weather_icon_from_name("storm") == WEATHER_ICON_STORM);
    assert(weather_icon_from_name("fog") == WEATHER_ICON_FOG);
    assert(weather_icon_from_name("unknown") == WEATHER_ICON_UNKNOWN);
}

static void test_unrecognised_input_falls_back(void)
{
    assert(weather_icon_from_name("") == WEATHER_ICON_UNKNOWN);
    assert(weather_icon_from_name("SUN") == WEATHER_ICON_UNKNOWN);
    assert(weather_icon_from_name("sunny") == WEATHER_ICON_UNKNOWN);
    assert(weather_icon_from_name("../../etc/passwd") == WEATHER_ICON_UNKNOWN);
    assert(weather_icon_from_name(NULL) == WEATHER_ICON_UNKNOWN);
}

int main(void)
{
    test_known_names_map();
    test_unrecognised_input_falls_back();
    printf("test_weather_icon: OK\n");
    return 0;
}
