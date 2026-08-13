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
/* The icon sits on its own surface tile, which is what gives this face its
 * module: an icon alone on the canvas read as a picture next to a caption,
 * and the tile makes it a reading. Two grid units of surface on every side
 * of the artwork. */
#define ICON_TILE   (19 * DESKMATE_GRID)
#define ICON_INSET  ((ICON_TILE - ICON_BOX) / 2)

#define TEXT_LEFT  (DESKMATE_MARGIN + ICON_TILE + 2 * DESKMATE_GRID)
#define TEXT_WIDTH (28 * DESKMATE_GRID)

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
    const deskmate_palette_t palette =
        deskmate_palette(PROTOCOL_TEMPLATE_ICON_BADGE_TEXT);
    view->root = lv_obj_create(parent);
    lv_obj_remove_style_all(view->root);
    lv_obj_remove_flag(view->root,
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->root, LV_PCT(100), LV_PCT(100));

    view->objects[OBJ_TITLE] = deskmate_chip(
        view->root, DESKMATE_MARGIN, 2 * DESKMATE_GRID, palette.hue,
        palette.ink);

    /* The badge is this face's live token — the one thing that says which
     * reading you are looking at — so it takes a filled chip rather than
     * coloured text, and sits opposite the title on the same rail. */
    view->objects[OBJ_BADGE] =
        deskmate_chip(view->root, 0, 0, palette.hue, palette.ink);
    lv_obj_align(view->objects[OBJ_BADGE], LV_ALIGN_TOP_RIGHT,
                 -DESKMATE_MARGIN, 2 * DESKMATE_GRID);

    lv_obj_t *icon_tile = deskmate_module(view->root, DESKMATE_MARGIN, 0,
                                          ICON_TILE, ICON_TILE);
    lv_obj_align(icon_tile, LV_ALIGN_LEFT_MID, DESKMATE_MARGIN, 0);

    view->objects[OBJ_ICON] = lv_obj_create(icon_tile);
    lv_obj_remove_style_all(view->objects[OBJ_ICON]);
    lv_obj_remove_flag(view->objects[OBJ_ICON],
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->objects[OBJ_ICON], ICON_BOX, ICON_BOX);
    lv_obj_set_pos(view->objects[OBJ_ICON], ICON_INSET, ICON_INSET);

    view->objects[OBJ_VALUE] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_VALUE],
                                DESKMATE_COLOR_PRIMARY, 0);
    lv_label_set_long_mode(view->objects[OBJ_VALUE], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_VALUE], TEXT_WIDTH);

    view->objects[OBJ_LABEL] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_LABEL], palette.tint, 0);
    lv_label_set_long_mode(view->objects[OBJ_LABEL], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_LABEL], TEXT_WIDTH);
    lv_label_set_text(view->objects[OBJ_LABEL], "");

    set_value_text(view, "--");

    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_MID, 0,
                 -2 * DESKMATE_GRID);
    view->state_label = view->objects[OBJ_STATE];

    /* The artwork is drawn in the face hue on the tile's surface. The
     * background argument is load-bearing, not cosmetic: the crescent moon
     * and the unknown ring are cut out by punching a background-coloured
     * disc over a lit one, so it must be the colour actually behind the
     * icon — which is now the module, not the canvas. */
    view->icon_current = WEATHER_ICON_UNKNOWN;
    weather_icon_render(view->objects[OBJ_ICON], view->icon_current,
                        palette.hue, DESKMATE_COLOR_SURFACE);
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
        deskmate_chip_set_text(view->objects[OBJ_TITLE], title->value.text);
    }
    if (badge != NULL) {
        deskmate_chip_set_text(view->objects[OBJ_BADGE], badge->value.text);
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
        if (next != view->icon_current) {
            view->icon_current = next;
            weather_icon_render(
                view->objects[OBJ_ICON], next,
                deskmate_palette(PROTOCOL_TEMPLATE_ICON_BADGE_TEXT).hue,
                DESKMATE_COLOR_SURFACE);
        }
    }
}
