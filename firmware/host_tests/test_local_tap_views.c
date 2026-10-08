#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include "core/local_tap_views.h"

typedef struct { bool allowed; const scene_t *shown; unsigned calls; } display_t;
static bool show(void *opaque, const scene_t *scene)
{
    display_t *display = opaque;
    ++display->calls;
    if (!display->allowed) return false;
    display->shown = scene;
    return true;
}
int main(void)
{
    scene_t *scenes = calloc(5U, sizeof(*scenes));
    assert(scenes != NULL);
    display_t display = {.allowed = true, .shown = scenes};
    local_tap_views_t ring = {.scenes = scenes, .count = 4U, .wrap = true};
    uint8_t index = 99U;
    for (unsigned i = 1U; i <= 10U; ++i) {
        assert(local_tap_views_advance(&ring, show, &display, &index));
        assert(index == i % 5U && display.shown == &scenes[index]);
    }
    ring.wrap = false;
    ring.count = 1U;
    assert(local_tap_views_advance(&ring, show, &display, &index));
    unsigned calls = display.calls;
    assert(local_tap_views_advance(&ring, show, &display, &index));
    assert(index == 1U && display.calls == calls);
    ring.index = 0U;
    display.allowed = false;
    index = 99U;
    assert(!local_tap_views_advance(&ring, show, &display, &index));
    assert(ring.index == 0U && index == 99U && display.shown == &scenes[1]);
    ring.count = 0U;
    assert(!local_tap_views_advance(&ring, show, &display, &index));
    ring.count = 5U;
    assert(!local_tap_views_advance(&ring, show, &display, &index));
    free(scenes);
    puts("test_local_tap_views: OK");
}
