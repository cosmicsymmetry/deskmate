#pragma once
#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>

/* One-time process init: lv_init + headless display. Returns false on failure. */
bool sim_init(void);

/* Scene rendering. */

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

/* Why sim_render_scene returns a status rather than a bool.
 *
 * Two very different things can go wrong, and framebuffer comparisons need
 * to distinguish them:
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
 * The binding context is assembled from the remaining arguments.
 * `orientation_flipped` selects the 270° mount.
 * Writes 448*368 RGB565 pixels (logical landscape) into out_pixels.
 *
 * Returns SIM_SCENE_OK, or the reason it failed -- see sim_scene_result_t
 * above. out_pixels is left untouched on every failure. Any asset a node
 * names must already be registered with sim_asset_register(). */
sim_scene_result_t sim_render_scene(const uint8_t *payload,
                                    size_t payload_length,
                                    int16_t utc_offset_minutes,
                                    int64_t now_unix_seconds,
                                    uint32_t timer_total_ms,
                                    uint32_t timer_remaining_ms,
                                    bool timer_running,
                                    bool orientation_flipped,
                                    uint16_t *out_pixels);
