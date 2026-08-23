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

static bool line_within_canvas(const scene_line_t *line)
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
                                     node->value.rect.w, node->value.rect.h)) {
                return SCENE_MODEL_ERR_GEOMETRY;
            }
            break;

        case SCENE_NODE_ARC:
            if (!arc_within_canvas(&node->value.arc)) {
                return SCENE_MODEL_ERR_GEOMETRY;
            }
            break;

        case SCENE_NODE_LINE:
            if (!line_within_canvas(&node->value.line)) {
                return SCENE_MODEL_ERR_GEOMETRY;
            }
            break;

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
            break;

        default:
            return SCENE_MODEL_ERR_NODE_KIND;
        }
    }

    return SCENE_MODEL_OK;
}
