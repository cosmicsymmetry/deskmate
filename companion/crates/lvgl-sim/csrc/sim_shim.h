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

/* Task 12: renders `text` centred on the 448x368 canvas using a runtime
 * font asset, exercising the same digest -> bytes -> lv_font_t path the
 * device's protocol_asset_resolver/font_registry does -- see sim_shim.c's
 * RAM-backed asset store shim. `digest` must be exactly ASSET_DIGEST_BYTES
 * (32) bytes; `ttf_bytes`/`ttf_len` is the font file to register under that
 * digest (registration is idempotent per digest, so re-registering the same
 * bytes under the same digest across calls is harmless). Returns false if
 * registration, font acquisition, or rendering fails. */
bool sim_render_asset_font(const uint8_t *digest,
                           const uint8_t *ttf_bytes,
                           uint32_t ttf_len,
                           int32_t pixel_size,
                           const char *text,
                           bool orientation_flipped,
                           uint16_t *out_pixels);
