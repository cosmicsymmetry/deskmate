#pragma once

#include <stdbool.h>
#include <stdint.h>
#include "core/scene_model.h"

/* scenes[0] is the primary face. Storage is owned by the caller, and every
 * operation is serialized in the UI context. No allocation or globals. */
typedef struct {
    scene_t *scenes;
    uint8_t count;
    uint8_t index;
    bool wrap;
} local_tap_views_t;

typedef bool (*local_tap_show_fn)(void *context, const scene_t *scene);

/* False means no local answer (absent ring, invalid state, or show refused).
 * Neither index nor the caller's reported index changes on a refusal. */
bool local_tap_views_advance(local_tap_views_t *views,
                             local_tap_show_fn show, void *context,
                             uint8_t *reported_index);
