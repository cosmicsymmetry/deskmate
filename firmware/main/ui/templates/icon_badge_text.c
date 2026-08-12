#include "template_internal.h"

#include <string.h>

#include "weather_icon.h"

enum {
    OBJ_TITLE,
    OBJ_BADGE,
    OBJ_ICON,
    OBJ_VALUE,
    OBJ_LABEL,
    OBJ_STATE,
};

/* `weather_icon_render` draws all eleven icons from absolute offsets inside a
 * 120px box (ICON_BOX in weather_icon.c). That artwork is verified on the
 * physical panel, so the box keeps its size — 15 * GRID, still on the grid —
 * rather than being scaled down around it. */
#define ICON_BOX 120
/* Icon and text are one cluster, and the cluster is centred rather than
 * pinned to the left margin. Anchored at the margin, a 120px icon and a
 * two-line reading left 200px of dead canvas to their right and the card
 * read half-finished. The block is a fixed 44 * GRID wide so the icon never
 * shifts when the reading changes length, which puts its left edge on
 * 6 * GRID and its right edge a margin clear of the panel. */
#define BLOCK_LEFT  (12 * DESKMATE_GRID)
#define TEXT_LEFT   (BLOCK_LEFT + ICON_BOX + 2 * DESKMATE_GRID)
#define TEXT_WIDTH  (24 * DESKMATE_GRID)
/* Title and badge share the top rail from opposite margins. */
#define RAIL_WIDTH (24 * DESKMATE_GRID)

static weather_icon_t s_current_icon = WEATHER_ICON_UNKNOWN;

/* Sets `text` in the largest tier that can both spell it and fit the text
 * column, and re-stacks the value/label pair around the canvas centre.
 *
 * A two-digit temperature at HERO is 124px and balances the 120px icon
 * beside it, which is what makes this card read as a weather instrument
 * rather than a caption with a picture; anything wider — "-12°" — falls to
 * DISPLAY on its own. HERO and DISPLAY are digits-only subsets (0-9 : - °
 * %), so a json-feed "yes" drops to BODY, and so does an over-long number:
 * LV_LABEL_LONG_DOT's ellipsis is itself outside the subset, which leaves
 * truncation to the only tier that can draw one.
 *
 * The label follows one tier down, floored at CAPTION, as on the big-number
 * face. The value's offset is measured from the label's line height so the
 * pair stays optically centred whichever tiers they land in. */
static void set_value_text(template_widget_view_t *view, const char *text)
{
    const lv_font_t *font = deskmate_number_font(text, TEXT_WIDTH);
    const lv_font_t *label_font = font == DESKMATE_FONT_BODY
                                      ? DESKMATE_FONT_CAPTION
                                      : DESKMATE_FONT_BODY;
    int32_t label_line = lv_font_get_line_height(label_font);
    lv_obj_set_style_text_font(view->objects[OBJ_VALUE], font, 0);
    lv_obj_set_height(view->objects[OBJ_VALUE],
                      lv_font_get_line_height(font));
    lv_label_set_text(view->objects[OBJ_VALUE], text);
    lv_obj_align(view->objects[OBJ_VALUE], LV_ALIGN_LEFT_MID, TEXT_LEFT,
                 -(2 * DESKMATE_GRID + label_line) / 2);
    lv_obj_set_style_text_font(view->objects[OBJ_LABEL], label_font, 0);
    lv_obj_set_height(view->objects[OBJ_LABEL], label_line);
    lv_obj_align_to(view->objects[OBJ_LABEL], view->objects[OBJ_VALUE],
                    LV_ALIGN_OUT_BOTTOM_LEFT, 0, 2 * DESKMATE_GRID);
}

bool icon_badge_text_create(template_widget_view_t *view,
                            lv_obj_t *parent,
                            protocol_size_class_t size)
{
    if (view == NULL || parent == NULL ||
        (size != PROTOCOL_SIZE_FULL && size != PROTOCOL_SIZE_STANDARD)) {
        return false;
    }
    memset(view, 0, sizeof(*view));
    view->root = lv_obj_create(parent);
    lv_obj_remove_style_all(view->root);
    lv_obj_remove_flag(view->root,
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->root, LV_PCT(100), LV_PCT(100));

    view->objects[OBJ_TITLE] = lv_label_create(view->root);
    lv_obj_set_style_text_font(view->objects[OBJ_TITLE],
                               DESKMATE_FONT_CAPTION, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_TITLE],
                                DESKMATE_COLOR_SECONDARY, 0);
    lv_label_set_long_mode(view->objects[OBJ_TITLE], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_TITLE], RAIL_WIDTH);
    lv_obj_set_height(view->objects[OBJ_TITLE],
                      lv_font_get_line_height(DESKMATE_FONT_CAPTION));
    lv_obj_align(view->objects[OBJ_TITLE], LV_ALIGN_TOP_LEFT, DESKMATE_MARGIN,
                 DESKMATE_MARGIN);
    lv_label_set_text(view->objects[OBJ_TITLE], "");

    /* The badge is this face's live token — the one thing that says which
     * reading you are looking at — so it takes the card's single accent. */
    view->objects[OBJ_BADGE] = lv_label_create(view->root);
    lv_obj_set_style_text_font(view->objects[OBJ_BADGE],
                               DESKMATE_FONT_CAPTION, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_BADGE],
                                DESKMATE_COLOR_ACCENT, 0);
    lv_label_set_long_mode(view->objects[OBJ_BADGE], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_BADGE], RAIL_WIDTH);
    lv_obj_set_height(view->objects[OBJ_BADGE],
                      lv_font_get_line_height(DESKMATE_FONT_CAPTION));
    lv_obj_set_style_text_align(view->objects[OBJ_BADGE],
                                LV_TEXT_ALIGN_RIGHT, 0);
    lv_obj_align(view->objects[OBJ_BADGE], LV_ALIGN_TOP_RIGHT,
                 -DESKMATE_MARGIN, DESKMATE_MARGIN);
    lv_label_set_text(view->objects[OBJ_BADGE], "");

    view->objects[OBJ_ICON] = lv_obj_create(view->root);
    lv_obj_remove_style_all(view->objects[OBJ_ICON]);
    lv_obj_remove_flag(view->objects[OBJ_ICON],
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->objects[OBJ_ICON], ICON_BOX, ICON_BOX);
    lv_obj_align(view->objects[OBJ_ICON], LV_ALIGN_LEFT_MID, BLOCK_LEFT, 0);

    view->objects[OBJ_VALUE] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_VALUE],
                                DESKMATE_COLOR_PRIMARY, 0);
    lv_label_set_long_mode(view->objects[OBJ_VALUE], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_VALUE], TEXT_WIDTH);

    view->objects[OBJ_LABEL] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_LABEL],
                                DESKMATE_COLOR_TERTIARY, 0);
    lv_label_set_long_mode(view->objects[OBJ_LABEL], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_LABEL], TEXT_WIDTH);
    lv_label_set_text(view->objects[OBJ_LABEL], "");

    set_value_text(view, "--");

    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_MID, 0,
                 -DESKMATE_MARGIN / 2);
    view->state_label = view->objects[OBJ_STATE];

    s_current_icon = WEATHER_ICON_UNKNOWN;
    weather_icon_render(view->objects[OBJ_ICON], s_current_icon,
                        DESKMATE_COLOR_PRIMARY, DESKMATE_COLOR_CANVAS);
    return true;
}

void icon_badge_text_patch(template_widget_view_t *view,
                           const template_field_state_t *fields,
                           uint16_t dirty_mask)
{
    (void)dirty_mask;
    if (view == NULL || view->root == NULL || fields == NULL) {
        return;
    }
    const template_field_value_t *title =
        template_fields_get(fields, "title");
    const template_field_value_t *badge =
        template_fields_get(fields, "badge");
    const template_field_value_t *value =
        template_fields_get(fields, "value");
    const template_field_value_t *label =
        template_fields_get(fields, "label");
    const template_field_value_t *icon =
        template_fields_get(fields, "icon");

    if (title != NULL) {
        lv_label_set_text(view->objects[OBJ_TITLE], title->value.text);
    }
    if (badge != NULL) {
        lv_label_set_text(view->objects[OBJ_BADGE], badge->value.text);
    }
    if (label != NULL) {
        lv_label_set_text(view->objects[OBJ_LABEL], label->value.text);
    }
    if (value != NULL) {
        set_value_text(view, value->value.text[0] != '\0'
                                 ? value->value.text
                                 : "--");
    }
    if (icon != NULL) {
        weather_icon_t next = weather_icon_from_name(icon->value.text);
        /* Only rebuild the artwork when the icon actually changes: every
         * rebuild deletes and recreates a dozen LVGL objects. */
        if (next != s_current_icon) {
            s_current_icon = next;
            weather_icon_render(view->objects[OBJ_ICON], next,
                                DESKMATE_COLOR_PRIMARY,
                                DESKMATE_COLOR_CANVAS);
        }
    }
}
