#include <assert.h>
#include <stdio.h>
#include <time.h>

#include "../main/core/clock_source.h"

int main(void)
{
    /* Default: tracks the real clock within slack. */
    int64_t real = (int64_t)time(NULL);
    int64_t got = clock_source_now();
    assert(got >= real && got <= real + 2);

    /* Override wins exactly, repeatedly. */
    clock_source_set_override(1755000000);
    assert(clock_source_now() == 1755000000);
    assert(clock_source_now() == 1755000000);

    /* Clearing returns to the real clock. */
    clock_source_clear_override();
    real = (int64_t)time(NULL);
    got = clock_source_now();
    assert(got >= real && got <= real + 2);

    printf("test_clock_source: OK\n");
    return 0;
}
