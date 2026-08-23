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

typedef enum {
    SCENE_NODE_RECT = 1,
    SCENE_NODE_ARC = 2,
    SCENE_NODE_LINE = 3,
    SCENE_NODE_TEXT = 4,
    SCENE_NODE_IMAGE = 5,
    SCENE_NODE_GLYPH = 6,
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

typedef struct { int32_t x, y, w, h, radius; uint32_t fill; uint8_t opacity; }
    scene_rect_t;
typedef struct { int32_t cx, cy, r, start_deg, end_deg, width; uint32_t color;
                 bool rounded; char end_binding[SCENE_MAX_BINDING + 1U]; }
    scene_arc_t;
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

typedef struct {
    scene_node_kind_t kind;
    union {
        scene_rect_t rect;
        scene_arc_t arc;
        scene_line_t line;
        scene_text_t text;
        scene_image_t image;
        scene_glyph_t glyph;
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
