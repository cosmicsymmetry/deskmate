#include "template_internal.h"

#include <string.h>

enum {
    OBJ_TITLE,
    OBJ_VALUE,
    OBJ_LABEL,
    OBJ_STATE,
};

/* The value is centred on the canvas axis and bounded by the margins. */
#define CONTENT_WIDTH (448 - 2 * DESKMATE_MARGIN)

/* Sets `text` in the largest tier it can legally and legibly occupy, and
 * re-stacks the value/label pair around the canvas centre.
 *
 * The tier ladder is unchanged. `value` is free-form text off the wire: a
 * json-feed card can put "yes" in it as easily as "8123". The HERO and
 * DISPLAY tiers are digits-only subsets (0-9 : - ° %), so alphabetic text
 * set in them would render as placeholder boxes — and so would
 * LV_LABEL_LONG_DOT's own ellipsis, since '.' is not in the subset either.
 * `deskmate_number_font` therefore steps down to BODY both for text those
 * tiers cannot spell and for numbers too long to fit, which leaves ellipsis
 * duty entirely with the tier that can draw one. */
static void set_value_text(template_widget_view_t *view, const char *text)
{
    const lv_font_t *font = deskmate_number_font(text, CONTENT_WIDTH);
    lv_obj_set_style_text_font(view->objects[OBJ_VALUE], font, 0);
    lv_obj_set_height(view->objects[OBJ_VALUE],
                      lv_font_get_line_height(font));
    lv_label_set_text(view->objects[OBJ_VALUE], text);
    /* The label is a footnote to the value, so it tracks it one tier down,
     * floored at CAPTION. At CAPTION under a 96px number it read as a stray
     * mark; at BODY under a 28px fallback value it would compete with it. */
    const lv_font_t *label_font = font == DESKMATE_FONT_BODY
                                      ? DESKMATE_FONT_CAPTION
                                      : DESKMATE_FONT_BODY;
    int32_t label_line = lv_font_get_line_height(label_font);
    lv_obj_set_style_text_font(view->objects[OBJ_LABEL], label_font, 0);
    /* Padding scales with the tier so the pill hugs whichever type it ends
     * up carrying. */
    lv_obj_set_style_pad_ver(view->objects[OBJ_LABEL], DESKMATE_GRID / 2, 0);
    /* The pair is centred as a unit: the value's own offset is measured
     * from the label's rendered height, so neither tier combination leaves
     * the block sitting high or low. */
    lv_obj_align(view->objects[OBJ_VALUE], LV_ALIGN_CENTER, 0,
                 -(2 * DESKMATE_GRID + label_line + DESKMATE_GRID) / 2);
    lv_obj_align_to(view->objects[OBJ_LABEL], view->objects[OBJ_VALUE],
                    LV_ALIGN_OUT_BOTTOM_MID, 0, 2 * DESKMATE_GRID);
}

bool big_number_label_create(template_widget_view_t *view,
                             lv_obj_t *parent,
                             protocol_size_class_t size)
{
    if (view == NULL || parent == NULL ||
        (size != PROTOCOL_SIZE_FULL && size != PROTOCOL_SIZE_STANDARD)) {
        return false;
    }
    memset(view, 0, sizeof(*view));
    const deskmate_palette_t palette =
        deskmate_palette(PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL);
    view->root = lv_obj_create(parent);
    lv_obj_remove_style_all(view->root);
    lv_obj_remove_flag(view->root,
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->root, LV_PCT(100), LV_PCT(100));

    view->objects[OBJ_TITLE] = deskmate_chip(
        view->root, DESKMATE_MARGIN, 2 * DESKMATE_GRID, palette.hue,
        palette.ink);

    view->objects[OBJ_VALUE] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_VALUE],
                                DESKMATE_COLOR_PRIMARY, 0);
    lv_label_set_long_mode(view->objects[OBJ_VALUE], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_VALUE], CONTENT_WIDTH);
    lv_obj_set_style_text_align(view->objects[OBJ_VALUE],
                                LV_TEXT_ALIGN_CENTER, 0);

    /* The label sits on its own surface pill rather than as loose type: it
     * is the unit the value is measured in, and giving it an edge stops it
     * reading as a stray caption under a very large number. */
    view->objects[OBJ_LABEL] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_LABEL], palette.tint, 0);
    lv_obj_set_style_bg_color(view->objects[OBJ_LABEL],
                              DESKMATE_COLOR_SURFACE, 0);
    lv_obj_set_style_bg_opa(view->objects[OBJ_LABEL], LV_OPA_COVER, 0);
    lv_obj_set_style_radius(view->objects[OBJ_LABEL], DESKMATE_RADIUS_CHIP,
                            0);
    lv_obj_set_style_pad_hor(view->objects[OBJ_LABEL], 2 * DESKMATE_GRID, 0);
    lv_label_set_text(view->objects[OBJ_LABEL], "");

    /* The schema default is "--", so a card that never sends a value shows a
     * stable placeholder rather than stale pixels from the previous card. */
    set_value_text(view, "--");

    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_MID, 0,
                 -2 * DESKMATE_GRID);
    view->state_label = view->objects[OBJ_STATE];
    return true;
}

void big_number_label_patch(template_widget_view_t *view,
                            const template_field_state_t *fields,
                            uint16_t dirty_mask)
{
    (void)dirty_mask;
    if (view == NULL || view->root == NULL || fields == NULL) {
        return;
    }
    const template_field_value_t *title =
        template_fields_get(fields, "title");
    const template_field_value_t *value =
        template_fields_get(fields, "value");
    const template_field_value_t *label =
        template_fields_get(fields, "label");
    if (title != NULL) {
        deskmate_chip_set_text(view->objects[OBJ_TITLE], title->value.text);
    }
    if (label != NULL) {
        /* An empty label must hide the pill, not leave an empty surface
         * floating under the value. */
        deskmate_chip_set_text(view->objects[OBJ_LABEL], label->value.text);
    }
    if (value != NULL) {
        set_value_text(view, value->value.text[0] != '\0'
                                 ? value->value.text
                                 : "--");
    }
}
