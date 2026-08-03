#include "timefmt.h"
#include <stdio.h>

static const char *DOW[] = { "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun" };
static const char *MON[] = { "Jan", "Feb", "Mar", "Apr", "May", "Jun",
                             "Jul", "Aug", "Sep", "Oct", "Nov", "Dec" };

void timefmt_hhmm(char out[6], int hour, int minute)
{
    snprintf(out, 6, "%02d:%02d", hour, minute);
}

void timefmt_date(char out[32], int year, int month, int day, int dow)
{
    (void)year;

    // dow/month arrive from host-pushed data starting M2; the spec requires
    // bad input to never crash firmware, so out-of-range values are deflected
    // to a "???" token rather than indexing DOW[]/MON[] out of bounds.
    const char *dow_str = (dow >= 0 && dow <= 6) ? DOW[dow] : "???";
    const char *mon_str = (month >= 1 && month <= 12) ? MON[month - 1] : "???";

    snprintf(out, 32, "%s, %s %d", dow_str, mon_str, day);
}
