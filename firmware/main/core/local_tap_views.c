#include "core/local_tap_views.h"

#include <stddef.h>

bool local_tap_views_advance(local_tap_views_t *views,
                             local_tap_show_fn show, void *context,
                             uint8_t *reported_index)
{
    if (views == NULL || views->scenes == NULL || views->count == 0U ||
        views->count > 4U || views->index > views->count || show == NULL ||
        reported_index == NULL) return false;
    uint8_t next = views->index;
    if (next < views->count) ++next;
    else if (views->wrap) next = 0U;
    if (next != views->index && !show(context, &views->scenes[next])) return false;
    views->index = next;
    *reported_index = next;
    return true;
}
