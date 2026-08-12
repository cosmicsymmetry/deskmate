#include "clock_source.h"

#include <stdbool.h>
#include <time.h>

static bool s_override_active;
static int64_t s_override_unix_seconds;

int64_t clock_source_now(void)
{
    if (s_override_active) {
        return s_override_unix_seconds;
    }
    return (int64_t)time(NULL);
}

void clock_source_set_override(int64_t unix_seconds)
{
    s_override_active = true;
    s_override_unix_seconds = unix_seconds;
}

void clock_source_clear_override(void)
{
    s_override_active = false;
}
