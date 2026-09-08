#include "scene_model.h"

#include <string.h>

/* A node's geometry must lie fully inside the canvas. Reject rather than
 * clip: a host that computed an off-canvas layout has a bug, and clipping
 * would hide it instead of surfacing it. Every bound check here is written
 * as `x > LIMIT - extent` rather than `x + extent > LIMIT` so a
 * maliciously or accidentally huge coordinate cannot wrap an int32_t before
 * comparison; each `extent > LIMIT` guard runs first so the subtraction
 * that follows it can never go negative. */

static bool rect_within_canvas(int32_t x, int32_t y, int32_t w, int32_t h)
{
    if (x < 0 || y < 0 || w < 0 || h < 0) {
        return false;
    }
    if (w > SCENE_CANVAS_WIDTH || h > SCENE_CANVAS_HEIGHT) {
        return false;
    }
    if (x > SCENE_CANVAS_WIDTH - w) {
        return false;
    }
    if (y > SCENE_CANVAS_HEIGHT - h) {
        return false;
    }
    return true;
}

static bool arc_within_canvas(const scene_arc_t *arc)
{
    if (arc->r < 0 || arc->cx < 0 || arc->cy < 0 || arc->width < 0) {
        return false;
    }
    if (arc->r > SCENE_CANVAS_WIDTH || arc->r > SCENE_CANVAS_HEIGHT) {
        return false;
    }
    /* cx/cy must be at least r so the (unchecked) `cx - r` implied by the
     * arc's bounding box cannot go negative. */
    if (arc->cx < arc->r || arc->cy < arc->r) {
        return false;
    }
    if (arc->cx > SCENE_CANVAS_WIDTH - arc->r) {
        return false;
    }
    if (arc->cy > SCENE_CANVAS_HEIGHT - arc->r) {
        return false;
    }
    return true;
}

static bool fixed_line_within_canvas(const scene_line_t *line)
{
    if (line->point_count < 2U || line->point_count > SCENE_MAX_LINE_POINTS) {
        return false;
    }
    if (line->width < 0) {
        return false;
    }
    for (uint32_t i = 0U; i < line->point_count; i++) {
        if (line->xs[i] < 0 || line->xs[i] > SCENE_CANVAS_WIDTH) {
            return false;
        }
        if (line->ys[i] < 0 || line->ys[i] > SCENE_CANVAS_HEIGHT) {
            return false;
        }
    }
    return true;
}

static bool bound_line_within_canvas(const scene_line_t *line)
{
    if (line->point_count != 0U || line->width < 0 || line->length < 0) {
        return false;
    }
    if (line->pivot_x < 0 || line->pivot_x > SCENE_CANVAS_WIDTH ||
        line->pivot_y < 0 || line->pivot_y > SCENE_CANVAS_HEIGHT) {
        return false;
    }
    /* The binding can eventually point in every direction. Bounding the
     * whole radius, rather than only today's endpoint, keeps every refresh
     * inside the same canvas contract as a fixed line. */
    if (line->length > line->pivot_x ||
        line->length > SCENE_CANVAS_WIDTH - line->pivot_x ||
        line->length > line->pivot_y ||
        line->length > SCENE_CANVAS_HEIGHT - line->pivot_y) {
        return false;
    }
    return true;
}

static bool text_within_canvas(const scene_text_t *text)
{
    if (text->x < 0 || text->w < 0) {
        return false;
    }
    if (text->w > SCENE_CANVAS_WIDTH) {
        return false;
    }
    if (text->x > SCENE_CANVAS_WIDTH - text->w) {
        return false;
    }
    /* baseline_y is the type's baseline, not a box top -- see the field
     * comment in scene_model.h -- but it still has to sit on the canvas. */
    if (text->baseline_y < 0 || text->baseline_y > SCENE_CANVAS_HEIGHT) {
        return false;
    }
    return true;
}

static bool scale_within_canvas(const scene_scale_t *scale)
{
    return rect_within_canvas(scale->x, scale->y, scale->box, scale->box);
}

/* A label is content-sized by LVGL, so core cannot know its right/bottom edge
 * without doing exactly the font arithmetic this node exists to keep off the
 * host. The anchor itself and every style extent are still bounded before
 * LVGL sees them. */
static bool label_within_canvas(const scene_label_t *label)
{
    if (label->y < 0 || label->y >= SCENE_CANVAS_HEIGHT) {
        return false;
    }
    switch (label->horizontal_anchor) {
    case SCENE_LABEL_ANCHOR_LEFT:
    case SCENE_LABEL_ANCHOR_CENTER:
        if (label->x < 0 || label->x >= SCENE_CANVAS_WIDTH) {
            return false;
        }
        break;
    case SCENE_LABEL_ANCHOR_RIGHT:
        /* Box edges use the same half-open convention as fixed rectangles:
         * a right edge of SCENE_CANVAS_WIDTH lands on the last pixel. */
        if (label->x <= 0 || label->x > SCENE_CANVAS_WIDTH) {
            return false;
        }
        break;
    default:
        return false;
    }
    if (label->radius < 0 || label->radius > SCENE_CANVAS_HEIGHT ||
        label->pad_hor < 0 || label->pad_hor > SCENE_CANVAS_WIDTH ||
        label->pad_ver < 0 || label->pad_ver > SCENE_CANVAS_HEIGHT ||
        label->letter_space < 0 ||
        label->letter_space > SCENE_CANVAS_WIDTH) {
        return false;
    }
    return true;
}

static bool rot_rect_within_canvas(const scene_rot_rect_t *rect)
{
    if (!rect_within_canvas(rect->x, rect->y, rect->w, rect->h)) {
        return false;
    }
    if (rect->has_clip &&
        !rect_within_canvas(rect->clip.x, rect->clip.y,
                            rect->clip.w, rect->clip.h)) {
        return false;
    }
    /* A transform pivot is in the object's local frame but may sit outside
     * the object: analog_clock.c rotates an 8px tick around y=160. Keep the
     * untrusted value canvas-bounded on each axis. Since the object itself is
     * also canvas-bounded, every corner-to-pivot delta remains canvas-scale
     * before LVGL's rotation matrix mixes the axes; arbitrary int32 values
     * can never reach that arithmetic. */
    if (rect->pivot_x < 0 || rect->pivot_x > SCENE_CANVAS_WIDTH ||
        rect->pivot_y < 0 || rect->pivot_y > SCENE_CANVAS_HEIGHT) {
        return false;
    }
    /* One signed turn covers every distinct transform while bounding the
     * value handed to LVGL. Both endpoints are legal and equivalent to 0. */
    return rect->rotation >= -3600 && rect->rotation <= 3600;
}

/* Bounds only -- see scene_scale_t's field comment in scene_model.h for the
 * full justification. total_tick_count in [2, SCENE_SCALE_MAX_TOTAL_TICKS]
 * (0/1 draws nothing, same reasoning as line_within_canvas rejecting a line
 * under 2 points); major_tick_every in [1, total_tick_count] (0 is a
 * meaningless "no major ticks" LVGL already tolerates, but this model
 * rejects it rather than accept a value with no visible effect; anything
 * past total_tick_count draws the same single major tick at index 0 as
 * major_tick_every == total_tick_count does, so it is rejected as
 * redundant). */
static bool scale_ticks_valid(const scene_scale_t *scale)
{
    if (scale->total_tick_count < 2U ||
        scale->total_tick_count > SCENE_SCALE_MAX_TOTAL_TICKS) {
        return false;
    }
    if (scale->major_tick_every < 1U ||
        scale->major_tick_every > scale->total_tick_count) {
        return false;
    }
    return true;
}

static bool glyph_within_canvas(const scene_glyph_t *glyph)
{
    if (glyph->x < 0 || glyph->size < 0) {
        return false;
    }
    if (glyph->size > SCENE_CANVAS_WIDTH || glyph->size > SCENE_CANVAS_HEIGHT) {
        return false;
    }
    if (glyph->x > SCENE_CANVAS_WIDTH - glyph->size) {
        return false;
    }
    /* baseline_y is a baseline, not a box edge -- same reasoning as
     * text_within_canvas above. This module has no font metrics, so it
     * cannot know a baseline-anchored glyph's true vertical extent; only
     * the baseline itself is checked against the canvas, and the renderer
     * clips. */
    if (glyph->baseline_y < 0 || glyph->baseline_y > SCENE_CANVAS_HEIGHT) {
        return false;
    }
    return true;
}

/* The array backing a text/binding buffer is not guaranteed NUL-terminated
 * by anything upstream (it arrives over the wire in a later task) -- so
 * this has to search for the terminator within the array bound with
 * memchr rather than assume one and call strlen, which would read past
 * the array on an untrusted payload that omitted it. */
static bool nul_terminated(const char *buf, size_t buf_size)
{
    return memchr(buf, '\0', buf_size) != NULL;
}

bool scene_model_parse_rotation_binding(const char *text,
                                        scene_rotation_binding_t *out)
{
    if (text == NULL || out == NULL) {
        return false;
    }
    if (text[0] == '\0') {
        *out = SCENE_ROTATION_BINDING_NONE;
        return true;
    }
    if (strcmp(text, "time:hour") == 0) {
        *out = SCENE_ROTATION_BINDING_HOUR;
        return true;
    }
    if (strcmp(text, "time:minute") == 0) {
        *out = SCENE_ROTATION_BINDING_MINUTE;
        return true;
    }
    if (strcmp(text, "time:second") == 0) {
        *out = SCENE_ROTATION_BINDING_SECOND;
        return true;
    }
    return false;
}

bool scene_model_parse_angle_binding(const char *text,
                                     scene_angle_binding_t *out)
{
    if (text == NULL || out == NULL) {
        return false;
    }
    if (text[0] == '\0') {
        *out = SCENE_ANGLE_BINDING_NONE;
        return true;
    }
    if (strcmp(text, "time:angle:hour") == 0) {
        *out = SCENE_ANGLE_BINDING_HOUR;
        return true;
    }
    if (strcmp(text, "time:angle:minute") == 0) {
        *out = SCENE_ANGLE_BINDING_MINUTE;
        return true;
    }
    return false;
}

/* The origin, in [0, 360). Normalised here because
 * lv_arc_set_rotation() reduces with a `while` loop (lv_arc.c:271-272), which
 * an untrusted wire value near INT32_MAX would spin through ~6 million
 * times. See scene_arc_t's field comment for the angle convention. */
int32_t scene_model_arc_origin(int32_t degrees)
{
    int32_t value = degrees % 360;
    if (value < 0) {
        value += 360;
    }
    return value;
}

/* The clockwise sweep from start_deg to end_deg, in (0, 360], or 0.
 *
 * Derived from the RAW difference, before either endpoint is folded, which
 * is what preserves a full turn: end_deg - start_deg == 360 is a whole
 * circle, and so is any other nonzero exact multiple. start_deg == end_deg
 * is NOT a full turn -- it is a degenerate empty arc, and drawing nothing is
 * the right answer for it. A negative difference is read the way LVGL reads
 * one of its own (lv_arc.c:225-226): as the clockwise sweep that lands on
 * end_deg.
 *
 * The subtraction widens to int64 first because both endpoints are untrusted
 * int32s and int32 difference can wrap. */
int32_t scene_model_arc_span(int32_t start_deg, int32_t end_deg)
{
    int64_t raw = (int64_t)end_deg - (int64_t)start_deg;
    int32_t span = (int32_t)(raw % 360);

    if (span < 0) {
        span += 360;
    }
    if (span == 0 && raw != 0) {
        span = 360;
    }
    return span;
}

static scene_model_result_t validate_font(const scene_font_ref_t *font)
{
    switch (font->kind) {
    case SCENE_FONT_BAKED:
        switch (font->baked) {
        case SCENE_FONT_CAPTION:
        case SCENE_FONT_BODY:
        case SCENE_FONT_DISPLAY:
        case SCENE_FONT_HERO:
            return SCENE_MODEL_OK;
        default:
            return SCENE_MODEL_ERR_FONT;
        }
    case SCENE_FONT_ASSET:
        if (font->pixel_size < 8 || font->pixel_size > 200) {
            return SCENE_MODEL_ERR_FONT;
        }
        return SCENE_MODEL_OK;
    default:
        return SCENE_MODEL_ERR_FONT;
    }
}

scene_model_result_t scene_model_validate(const scene_t *scene)
{
    if (scene == NULL) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    if (scene->node_count > SCENE_MAX_NODES) {
        return SCENE_MODEL_ERR_NODE_COUNT;
    }

    for (uint32_t i = 0U; i < scene->node_count; i++) {
        const scene_node_t *node = &scene->nodes[i];

        switch (node->kind) {
        case SCENE_NODE_RECT:
            if (!rect_within_canvas(node->value.rect.x, node->value.rect.y,
                                     node->value.rect.w, node->value.rect.h) ||
                (node->value.rect.has_clip &&
                 !rect_within_canvas(node->value.rect.clip.x,
                                     node->value.rect.clip.y,
                                     node->value.rect.clip.w,
                                     node->value.rect.clip.h))) {
                return SCENE_MODEL_ERR_GEOMETRY;
            }
            break;

        case SCENE_NODE_ARC:
            if (!arc_within_canvas(&node->value.arc)) {
                return SCENE_MODEL_ERR_GEOMETRY;
            }
            if (!nul_terminated(node->value.arc.end_binding,
                                 sizeof node->value.arc.end_binding)) {
                return SCENE_MODEL_ERR_TEXT;
            }
            break;

        case SCENE_NODE_LINE: {
            const scene_line_t *line = &node->value.line;
            scene_angle_binding_t binding;
            if (!nul_terminated(line->angle_binding,
                                sizeof line->angle_binding) ||
                !scene_model_parse_angle_binding(line->angle_binding,
                                                 &binding)) {
                return SCENE_MODEL_ERR_TEXT;
            }
            bool geometry_ok = binding == SCENE_ANGLE_BINDING_NONE
                ? fixed_line_within_canvas(line)
                : bound_line_within_canvas(line);
            if (!geometry_ok) {
                return SCENE_MODEL_ERR_GEOMETRY;
            }
            break;
        }

        case SCENE_NODE_TEXT: {
            const scene_text_t *text = &node->value.text;
            scene_model_result_t font_result;

            if (!text_within_canvas(text)) {
                return SCENE_MODEL_ERR_GEOMETRY;
            }
            if (!nul_terminated(text->value.literal, sizeof text->value.literal) ||
                !nul_terminated(text->value.binding, sizeof text->value.binding)) {
                return SCENE_MODEL_ERR_TEXT;
            }
            font_result = validate_font(&text->font);
            if (font_result != SCENE_MODEL_OK) {
                return font_result;
            }
            switch (text->align) {
            case SCENE_ALIGN_LEFT:
            case SCENE_ALIGN_CENTER:
            case SCENE_ALIGN_RIGHT:
                break;
            default:
                return SCENE_MODEL_ERR_GEOMETRY;
            }
            break;
        }

        case SCENE_NODE_IMAGE:
            if (!rect_within_canvas(node->value.image.x, node->value.image.y,
                                     node->value.image.w, node->value.image.h)) {
                return SCENE_MODEL_ERR_GEOMETRY;
            }
            break;

        case SCENE_NODE_GLYPH:
            if (!glyph_within_canvas(&node->value.glyph)) {
                return SCENE_MODEL_ERR_GEOMETRY;
            }
            if (!nul_terminated(node->value.glyph.name,
                                 sizeof node->value.glyph.name)) {
                return SCENE_MODEL_ERR_TEXT;
            }
            break;

        case SCENE_NODE_SCALE:
            if (!scale_within_canvas(&node->value.scale)) {
                return SCENE_MODEL_ERR_GEOMETRY;
            }
            if (!scale_ticks_valid(&node->value.scale)) {
                return SCENE_MODEL_ERR_GEOMETRY;
            }
            break;

        case SCENE_NODE_LABEL: {
            const scene_label_t *label = &node->value.label;
            scene_model_result_t font_result;

            if (!label_within_canvas(label)) {
                return SCENE_MODEL_ERR_GEOMETRY;
            }
            if (!nul_terminated(label->value.literal,
                                sizeof label->value.literal) ||
                !nul_terminated(label->value.binding,
                                sizeof label->value.binding)) {
                return SCENE_MODEL_ERR_TEXT;
            }
            font_result = validate_font(&label->font);
            if (font_result != SCENE_MODEL_OK) {
                return font_result;
            }
            break;
        }

        case SCENE_NODE_ROT_RECT: {
            const scene_rot_rect_t *rect = &node->value.rot_rect;
            scene_rotation_binding_t binding;

            if (!rot_rect_within_canvas(rect)) {
                return SCENE_MODEL_ERR_GEOMETRY;
            }
            if (!nul_terminated(rect->rotation_binding,
                                sizeof rect->rotation_binding) ||
                !scene_model_parse_rotation_binding(rect->rotation_binding,
                                                    &binding)) {
                return SCENE_MODEL_ERR_TEXT;
            }
            break;
        }

        default:
            return SCENE_MODEL_ERR_NODE_KIND;
        }
    }

    return SCENE_MODEL_OK;
}
