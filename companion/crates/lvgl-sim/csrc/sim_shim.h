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

/* ---------------------------------------------------------------------
 * Task 8 (stage 2a): scene rendering.
 * ------------------------------------------------------------------ */

/* One `field.<name>` binding's current value, as a provider would have
 * reported it. Looked up by name, exactly as the device's
 * scene_field_fn does. */
typedef struct {
    const char *name;
    const char *value;
} sim_scene_field_t;

/* Registers an asset blob under `digest` in the RAM-backed asset store,
 * idempotently per digest (a second registration of the same digest is a
 * no-op, not an error, so a case may register on every render). `kind` is
 * an asset_kind_t: 1 font, 2 icon font, 3 image. Returns false if the
 * store is full or the arguments are empty.
 *
 * Font assets reach ui/scene_view.c through font_registry_acquire();
 * image assets reach it through the resolver sim_render_scene installs
 * with scene_view_set_asset_resolver(). Both read this one store. */
bool sim_asset_register(const uint8_t *digest, const uint8_t *bytes,
                        uint32_t len, uint8_t kind);

/* Wraps `width` x `height` host-endian RGB565 `pixels` in the LVGL binary
 * image layout an `image` node expects -- an lv_image_header_t followed by
 * the pixel data, which is what scene_view.c's build_image() reads back.
 *
 * The header is built here rather than in Rust on purpose: it is a
 * bitfield struct whose byte layout is the compiler's business, and a
 * caller that guessed it wrong would produce a blob the renderer silently
 * refuses. Callers own the pixels; this owns only the wrapper.
 *
 * Returns the number of bytes written to `out`, or 0 if `out_capacity` is
 * too small or an argument is invalid. */
size_t sim_build_rgb565_image(int32_t width, int32_t height,
                              const uint16_t *pixels, uint8_t *out,
                              size_t out_capacity);

/* Why sim_render_scene returns a status rather than a bool.
 *
 * Two very different things can go wrong, and Task 9's parity gate is the
 * caller most likely to hit one of them:
 *
 *   - THE DECODER REFUSED THE BYTES. This is the failure mode the
 *     encode/decode design introduces: protocol::encode_scene_payload and
 *     core/scene_decode.c disagreeing about the wire shape. It says nothing
 *     about assets or drawing, and it must be diagnosable without a
 *     debugger, so scene_decode()'s scene_model_result_t is carried out
 *     whole -- one SIM_SCENE_ERR_DECODE_* per scene_model_result_t code,
 *     rather than folded into a single "decode failed".
 *   - THE RENDERER REFUSED THE SCENE. scene_view_show() returning false:
 *     an asset it could not acquire, or an allocation failure. Nothing to
 *     do with the bytes.
 *
 * Collapsing those into one bool made the difference invisible, which is
 * exactly the wrong trade for the one gate that has to explain itself. */
typedef enum {
    SIM_SCENE_OK = 0,
    /* NULL/empty payload or out_pixels. */
    SIM_SCENE_ERR_ARGUMENT,
    /* sim_init, the asset store, or the font registry could not be brought
     * up -- nothing about this scene. */
    SIM_SCENE_ERR_SETUP,
    /* scene_view_show() refused: an asset it could not acquire, or an
     * allocation failure. The previous screen is left up. */
    SIM_SCENE_ERR_SHOW,
    /* scene_decode() refusals. One per scene_model_result_t, in that
     * enum's own order, so a new model result cannot be silently mapped
     * onto an existing one. */
    SIM_SCENE_ERR_DECODE_ARGUMENT,
    SIM_SCENE_ERR_DECODE_NODE_COUNT,
    SIM_SCENE_ERR_DECODE_NODE_KIND,
    SIM_SCENE_ERR_DECODE_GEOMETRY,
    SIM_SCENE_ERR_DECODE_TEXT,
    SIM_SCENE_ERR_DECODE_FONT,
} sim_scene_result_t;

/* Decodes `payload` -- the standalone CBOR scene map firmware's
 * scene_decode() takes, as written by protocol::encode_scene_payload --
 * and renders it through ui/scene_view.c's interpreter.
 *
 * Going through the decoder rather than filling a scene_t over FFI is the
 * point: the device builds its scene_t from these same bytes with this
 * same code, so the parity gate compares two renders of one decode path,
 * not a render of the wire against a render of a hand-built struct.
 *
 * The binding context is assembled from the remaining arguments;
 * `fields` resolves `field.<name>` bindings and may be NULL when
 * `field_count` is 0. `orientation_flipped` selects the 270° mount, as in
 * sim_render. Writes 448*368 RGB565 pixels (logical landscape) into
 * out_pixels.
 *
 * Returns SIM_SCENE_OK, or the reason it failed -- see sim_scene_result_t
 * above. out_pixels is left untouched on every failure. Any asset a node
 * names must already be registered with sim_asset_register(). */
sim_scene_result_t sim_render_scene(const uint8_t *payload,
                                    size_t payload_length,
                                    int16_t utc_offset_minutes,
                                    int64_t now_unix_seconds,
                                    bool timer_active,
                                    uint32_t timer_remaining_ms,
                                    uint8_t timer_pct,
                                    const sim_scene_field_t *fields,
                                    size_t field_count,
                                    bool orientation_flipped,
                                    uint16_t *out_pixels);
