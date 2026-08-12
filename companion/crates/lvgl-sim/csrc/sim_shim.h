#pragma once
#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>

typedef struct {
    const char *name;
    int type; /* 0 text, 1 integer, 2 boolean — mirrors PROTOCOL_FIELD_* */
    const char *text;
    int64_t integer;
    bool boolean;
} sim_field_t;

/* One-time process init: lv_init + headless display. Returns false on failure. */
bool sim_init(void);

/* Renders one template with the given fields at a pinned instant.
 * orientation_flipped selects the 270° (180°-flipped landscape) mounting.
 * Writes 448*368 RGB565 pixels (logical landscape) into out_pixels.
 * Returns false on unknown template or field resolution failure. */
bool sim_render(int template_kind,
                const sim_field_t *fields,
                size_t field_count,
                int16_t utc_offset_minutes,
                int64_t now_unix_seconds,
                bool orientation_flipped,
                uint16_t *out_pixels);
