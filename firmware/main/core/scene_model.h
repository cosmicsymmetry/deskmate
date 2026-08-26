#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "core/asset_store.h"   /* ASSET_DIGEST_BYTES */

#define SCENE_CANVAS_WIDTH   448
#define SCENE_CANVAS_HEIGHT  368
#define SCENE_MAX_NODES      24U
#define SCENE_MAX_TEXT_BYTES 128U
#define SCENE_MAX_LINE_POINTS 8U
#define SCENE_MAX_GLYPH_NAME 32U
#define SCENE_MAX_BINDING    48U
/* A SCALE node's total_tick_count is bounded here, not just clamped at the
 * renderer, because LVGL redraws every tick on each invalidation with no cap
 * of its own -- see scene_scale_t's field comment for why 361. */
#define SCENE_SCALE_MAX_TOTAL_TICKS 361U

typedef enum {
    SCENE_NODE_RECT = 1,
    SCENE_NODE_ARC = 2,
    SCENE_NODE_LINE = 3,
    SCENE_NODE_TEXT = 4,
    SCENE_NODE_IMAGE = 5,
    SCENE_NODE_GLYPH = 6,
    SCENE_NODE_SCALE = 7,
    SCENE_NODE_LABEL = 8,
    SCENE_NODE_ROT_RECT = 9,
} scene_node_kind_t;

/* The four faces baked by tools/genfonts.sh. Kept as a tier rather than a
 * pixel size because the shipped templates address them by role, and because
 * a tier resolves to the identical lv_font_t the C templates use -- which is
 * what makes the stage-2a parity gate winnable. */
typedef enum {
    SCENE_FONT_CAPTION = 1,  /* deskmate_font_18 */
    SCENE_FONT_BODY = 2,     /* deskmate_font_28 */
    SCENE_FONT_DISPLAY = 3,  /* deskmate_font_56 */
    SCENE_FONT_HERO = 4,     /* deskmate_font_96 */
} scene_font_tier_t;

typedef enum {
    SCENE_FONT_BAKED = 1,
    SCENE_FONT_ASSET = 2,
} scene_font_kind_t;

typedef struct {
    scene_font_kind_t kind;
    scene_font_tier_t baked;              /* when kind == SCENE_FONT_BAKED */
    uint8_t digest[ASSET_DIGEST_BYTES];   /* when kind == SCENE_FONT_ASSET */
    int32_t pixel_size;                   /* when kind == SCENE_FONT_ASSET */
} scene_font_ref_t;

/* A text node's string is either a literal or a binding the device evaluates
 * locally. The binding form is what keeps a clock ticking with the link down. */
typedef enum {
    SCENE_VALUE_LITERAL = 1,
    SCENE_VALUE_BINDING = 2,
} scene_value_kind_t;

typedef struct {
    scene_value_kind_t kind;
    char literal[SCENE_MAX_TEXT_BYTES + 1U];
    char binding[SCENE_MAX_BINDING + 1U];
} scene_value_t;

typedef enum {
    SCENE_ALIGN_LEFT = 1,
    SCENE_ALIGN_CENTER = 2,
    SCENE_ALIGN_RIGHT = 3,
} scene_align_t;

typedef enum {
    SCENE_LABEL_ANCHOR_LEFT = 0,
    SCENE_LABEL_ANCHOR_CENTER = 1,
    SCENE_LABEL_ANCHOR_RIGHT = 2,
} scene_label_anchor_t;

typedef struct { int32_t x, y, w, h, radius; uint32_t fill; uint8_t opacity; }
    scene_rect_t;
/* `start_deg`/`end_deg` are absolute canvas angles in LVGL's convention: 0
 * degrees is 3 o'clock and they increase CLOCKWISE. `start_deg` is the
 * origin and the arc sweeps clockwise from it to `end_deg`; a difference
 * that comes out negative is read as the clockwise sweep landing on
 * `end_deg`, so 270 -> 90 sweeps from twelve o'clock through three (the
 * right half), never anticlockwise through nine. LVGL's own words for the
 * scale, from lv_arc.h: "0 deg: right, 90 bottom".
 *
 * A FULL TURN is spelled as a nonzero exact multiple of 360: a ring starting
 * at twelve o'clock is `start_deg = 270, end_deg = 630`. It is NOT spelled
 * `start_deg == end_deg`, which is a degenerate empty arc and draws nothing.
 * The renderer derives the sweep from the raw difference before folding
 * either endpoint, precisely so that spelling survives -- see
 * scene_model_arc_origin()/scene_model_arc_span() below. Emitting `end_deg =
 * start_deg + 359` to dodge the question instead leaves a gap that is
 * plainly visible at a gauge's stroke width.
 *
 * `end_binding` scales that sweep rather than replacing it: the arc is drawn
 * from `start_deg` through `end_deg - start_deg` times the bound percentage,
 * so the declared pair is the 100% geometry. */
typedef struct { int32_t cx, cy, r, start_deg, end_deg, width; uint32_t color;
                 bool rounded; char end_binding[SCENE_MAX_BINDING + 1U]; }
    scene_arc_t;

/* The origin and sweep an ARC node is drawn with -- pure integer math, used
 * by ui/scene_view.c's ARC renderer and host-tested directly in
 * test_scene_model.c, with zero LVGL dependency. See scene_arc_t's field
 * comment above for the angle convention these implement. */
int32_t scene_model_arc_origin(int32_t degrees);
int32_t scene_model_arc_span(int32_t start_deg, int32_t end_deg);
typedef struct { int32_t xs[SCENE_MAX_LINE_POINTS], ys[SCENE_MAX_LINE_POINTS];
                 uint32_t point_count; int32_t width; uint32_t color; }
    scene_line_t;
/* `baseline_y` is the type's baseline, not the box top. The shipped templates
 * position by baseline (see digital_clock.c's baseline_offset()), so a
 * box-anchored node could not reproduce their pixels. */
typedef struct { int32_t x, baseline_y, w; scene_align_t align;
                 scene_font_ref_t font; uint32_t color; scene_value_t value;
                 bool ellipsize; }
    scene_text_t;
typedef struct { int32_t x, y, w, h; uint8_t digest[ASSET_DIGEST_BYTES];
                 bool recolor; uint32_t color; }
    scene_image_t;
typedef struct { int32_t x, baseline_y, size; uint8_t digest[ASSET_DIGEST_BYTES];
                 char name[SCENE_MAX_GLYPH_NAME + 1U]; uint32_t color; }
    scene_glyph_t;

/* SCENE_NODE_SCALE reproduces `lv_scale_create` the way digital_clock.c's
 * dial does it (digital_clock.c:103-131): a square LV_SCALE_MODE_ROUND_INNER
 * scale with its labels hidden and its own arc invisible, drawn as ticks
 * only. `(x, y)` is the scale's box top-left in absolute canvas
 * coordinates, same as every other node; `box` is a single side length,
 * not a `w`/`h` pair, because every scale this node has ever needed to draw
 * is square -- `lv_obj_set_size(dial, DIAL_BOX, DIAL_BOX)` passes the same
 * value twice, so a second dimension would be a wire field nobody sets.
 *
 * The renderer pins the rest of the dial to the exact constants
 * digital_clock.c uses -- angle range 360, rotation 270 (twelve o'clock),
 * value range 0..720, minor tick width/length, major tick width/length/
 * opacity, and the fixed grey minor-tick colour -- because nothing in that
 * file ever varies them. Only `total_tick_count`, `major_tick_every`, and
 * the major tick's colour do (per widget instance and per accent palette),
 * so only those are fields here.
 *
 * `total_tick_count` and `major_tick_every` are bounded at validation
 * (SCENE_SCALE_MAX_TOTAL_TICKS, and major_tick_every in
 * [1, total_tick_count]) because LVGL redraws every tick on each
 * invalidation with no cap of its own: an untrusted host asking for
 * billions of ticks would stall the render task, not merely draw an ugly
 * dial. 361 is one tick per degree of the full circle the renderer always
 * draws (angle_range is fixed at 360), so nothing legitimate needs more.
 *
 * `major_tick_color` is the major tick's colour only -- `palette.hue` in
 * digital_clock.c, the one thing that changes between two clock cards with
 * different accent colours. The minor ticks stay the fixed
 * DESKMATE_COLOR_TERTIARY grey every template uses, so that colour is a
 * renderer default, not a field.
 *
 * BOX ORIGIN: LVGL positions everything drawn on a scale -- its ticks, and
 * any needle -- from the scale object's own centre, which is `(x + box/2, y
 * + box/2)` here, not this node's top-left and not the canvas origin. This
 * node's two clock hands are ordinary LINE nodes in absolute canvas
 * coordinates (scene_view.c does not reparent them under the scale the way
 * digital_clock.c parents its lv_line hands under the lv_scale), so
 * whatever emits their endpoints must derive that centre itself; this node
 * carries no needle. */
typedef struct { int32_t x, y, box; uint32_t total_tick_count;
                 uint32_t major_tick_every; uint32_t major_tick_color; }
    scene_scale_t;

/* A content-sized lv_label carrying the complete style needed to reproduce
 * deskmate_chip(), deskmate_eyebrow(), and BigNumberLabel's pill. The device
 * deliberately owns text measurement: the rendered box depends on glyph
 * advances, 4.4-format kern pairs, and letter_space. x is the left edge,
 * horizontal centre, or right edge of the finished box according to
 * horizontal_anchor; y is always the top edge. */
typedef struct { int32_t x, y; scene_label_anchor_t horizontal_anchor;
                 scene_font_ref_t font; scene_value_t value;
                 uint32_t ink, fill; uint8_t fill_opacity; int32_t radius;
                 int32_t pad_hor, pad_ver, letter_space; bool hide_when_empty; }
    scene_label_t;

/* A rectangle rotated through LVGL's object transform, not drawn as a line.
 * rotation is in tenths of a degree, exactly as
 * lv_obj_set_style_transform_rotation() accepts it. A non-empty binding is
 * one of time:hour, time:minute, or time:second. */
typedef struct { int32_t x, y, w, h, radius; uint32_t fill;
                 int32_t pivot_x, pivot_y, rotation;
                 char rotation_binding[SCENE_MAX_BINDING + 1U]; }
    scene_rot_rect_t;

typedef enum {
    SCENE_ROTATION_BINDING_NONE = 0,
    SCENE_ROTATION_BINDING_HOUR = 1,
    SCENE_ROTATION_BINDING_MINUTE = 2,
    SCENE_ROTATION_BINDING_SECOND = 3,
} scene_rotation_binding_t;

/* Parses the deliberately closed rotation-binding vocabulary. Empty is the
 * legal fixed-angle form and returns SCENE_ROTATION_BINDING_NONE. */
bool scene_model_parse_rotation_binding(const char *text,
                                        scene_rotation_binding_t *out);

typedef struct {
    scene_node_kind_t kind;
    union {
        scene_rect_t rect;
        scene_arc_t arc;
        scene_line_t line;
        scene_text_t text;
        scene_image_t image;
        scene_glyph_t glyph;
        scene_scale_t scale;
        scene_label_t label;
        scene_rot_rect_t rot_rect;
    } value;
} scene_node_t;

typedef struct {
    uint32_t revision;
    uint32_t background;
    uint32_t node_count;
    scene_node_t nodes[SCENE_MAX_NODES];
} scene_t;

typedef enum {
    SCENE_MODEL_OK = 0,
    SCENE_MODEL_ERR_ARGUMENT,
    SCENE_MODEL_ERR_NODE_COUNT,
    SCENE_MODEL_ERR_NODE_KIND,
    SCENE_MODEL_ERR_GEOMETRY,
    SCENE_MODEL_ERR_TEXT,
    SCENE_MODEL_ERR_FONT,
} scene_model_result_t;

scene_model_result_t scene_model_validate(const scene_t *scene);

/* Decodes one scene from an untrusted CBOR payload into caller-owned
 * storage. Implemented in core/scene_decode.c, which is the only file in
 * core/ that includes TinyCBOR -- this header and scene_model.c stay free
 * of it so the simulator and host tests can build scenes directly.
 *
 * The wire shape, key by key, is documented at the top of scene_decode.c.
 *
 * A scene arriving NESTED inside another message -- PushScene's key 2 --
 * cannot come through here, because this function validates its buffer as
 * a complete standalone payload. core/scene_decode.h declares
 * scene_decode_map(), the shared body both entry points run, for that
 * caller. Everything below applies identically to it.
 *
 * `out` is caller-owned deliberately. sizeof(scene_t) is ~6 KB
 * (SCENE_MAX_NODES x 256 bytes), which on this board has to be placed on
 * purpose -- link/protocol_task.c PSRAM-allocates its context for exactly
 * this reason -- and must never land on a task stack or in .bss by
 * accident. This function creates no scene_t of its own.
 *
 * VALIDATION CONTRACT -- exact, and relied upon by callers:
 *
 *   On SCENE_MODEL_OK, `*out` has ALREADY PASSED scene_model_validate().
 *   scene_decode() calls it once, on the fully populated scene, before
 *   returning OK. A caller that obtained a scene from here does not need
 *   to validate it again. (ui/scene_view.c still validates, because
 *   scene_view_show() also accepts scenes built directly in C by the
 *   simulator and the host tests, which never pass through here.)
 *
 *   On ANY non-OK result, `out->node_count` is 0. The scene is refused
 *   WHOLE -- a payload whose last node carries an unparseable binding
 *   yields no nodes at all, never a half-rendered panel -- and what is
 *   left behind is an empty scene, which is safe to hand anywhere,
 *   though it draws nothing.
 *
 * Result mapping. scene_model_result_t has no CBOR-specific code and none
 * is added, so every structural failure lands on ERR_ARGUMENT:
 *
 *   SCENE_MODEL_ERR_ARGUMENT   NULL/empty argument, or the payload is not
 *                              a well-formed, canonical, complete CBOR
 *                              scene map: wrong major type, missing
 *                              required key, duplicate or non-increasing
 *                              key, invalid UTF-8, trailing bytes, a
 *                              byte string of the wrong length, or
 *                              nesting past CBOR_PARSER_MAX_RECURSIONS.
 *   SCENE_MODEL_ERR_NODE_COUNT more than SCENE_MAX_NODES nodes.
 *   SCENE_MODEL_ERR_NODE_KIND  a node kind outside scene_node_kind_t.
 *   SCENE_MODEL_ERR_TEXT       a text, binding, or glyph name past its
 *                              cap, or a binding scene_binding_parse()
 *                              rejects.
 *   SCENE_MODEL_ERR_GEOMETRY   a numeric field outside the range of the
 *                              C type that holds it, a line coordinate
 *                              array longer than SCENE_MAX_LINE_POINTS,
 *                              or a line's two coordinate arrays
 *                              disagreeing in length.
 *   ...plus anything scene_model_validate() itself returns. */
scene_model_result_t scene_decode(const uint8_t *payload, size_t length,
                                  scene_t *out);
