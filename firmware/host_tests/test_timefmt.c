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

    timefmt_date(buf, 2026, 8, 3, 0); // dow: 0 = Monday
    assert(strcmp(buf, "Mon, Aug 3") == 0);
    timefmt_date(buf, 2026, 12, 31, 3); // Thursday
    assert(strcmp(buf, "Thu, Dec 31") == 0);

    printf("test_timefmt: OK\n");
    return 0;
}
