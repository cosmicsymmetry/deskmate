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
    snprintf(out, 32, "%s, %s %d", DOW[dow], MON[month - 1], day);
}
