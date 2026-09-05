#include <assert.h>
#include <string.h>
#include <stdio.h>
#include "../main/core/timefmt.h"

int main(void)
{
    char buf[32];

    timefmt_hhmm(buf, 9, 5);
    assert(strcmp(buf, "09:05") == 0);
    timefmt_hhmm(buf, 23, 59);
    assert(strcmp(buf, "23:59") == 0);
    timefmt_hhmm(buf, 0, 0);
    assert(strcmp(buf, "00:00") == 0);

    timefmt_date(buf, 8, 3, 0); // dow: 0 = Monday
    assert(strcmp(buf, "Mon, Aug 3") == 0);
    timefmt_date(buf, 12, 31, 3); // Thursday
    assert(strcmp(buf, "Thu, Dec 31") == 0);

    // Out-of-range dow/month must never index DOW[]/MON[] out of bounds
    // (M2 will feed this host-pushed data; bad input must never crash
    // firmware per spec). Clamp/deflect to "???" tokens instead.
    timefmt_date(buf, 8, 3, -1);
    assert(strcmp(buf, "???, Aug 3") == 0);
    timefmt_date(buf, 8, 3, 7);
    assert(strcmp(buf, "???, Aug 3") == 0);
    timefmt_date(buf, 0, 3, 0);
    assert(strcmp(buf, "Mon, ??? 3") == 0);
    timefmt_date(buf, 13, 3, 0);
    assert(strcmp(buf, "Mon, ??? 3") == 0);
    timefmt_date(buf, 0, 3, -1);
    assert(strcmp(buf, "???, ??? 3") == 0);

    printf("test_timefmt: OK\n");
    return 0;
}
