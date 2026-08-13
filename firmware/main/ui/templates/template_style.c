/* The shared visual grammar every face composes from, so the complication
 * language stays one language rather than six dialects. See spec §6.2
 * (amended 2026-08-13) for the direction these primitives encode. */

#include "template_internal.h"

deskmate_palette_t deskmate_palette(protocol_template_kind_t kind)
{
    switch (kind) {
    /* Both clock faces share one identity: they are the same reading in two
     * notations, and a carousel that hued them apart would imply otherwise.
     * 28° amber-orange, clear of stale-gold's 45°. */
    case PROTOCOL_TEMPLATE_DIGITAL_CLOCK:
    case PROTOCOL_TEMPLATE_ANALOG_CLOCK:
        return (deskmate_palette_t){
            .hue = lv_color_hex(0xff8f2e),
            .tint = lv_color_hex(0xffc48a),
            .ink = lv_color_hex(0x1c1105),
        };
    /* A countdown is the one face that should read as urgent. */
    case PROTOCOL_TEMPLATE_PROGRESS_RING:
        return (deskmate_palette_t){
            .hue = lv_color_hex(0xff5a3d),
            .tint = lv_color_hex(0xff9a85),
            .ink = lv_color_hex(0x200803),
        };
    /* Teal reads as "scheduled, not urgent" against the countdown. */
    case PROTOCOL_TEMPLATE_ROW_LIST:
        return (deskmate_palette_t){
            .hue = lv_color_hex(0x2fd9c0),
            .tint = lv_color_hex(0x8ee9dc),
            .ink = lv_color_hex(0x04211d),
        };
    /* Sky: the icon face is a weather card in every shipped playlist. */
    case PROTOCOL_TEMPLATE_ICON_BADGE_TEXT:
        return (deskmate_palette_t){
            .hue = lv_color_hex(0x35b6f5),
            .tint = lv_color_hex(0x94d8fa),
            .ink = lv_color_hex(0x041a24),
        };
    /* Violet: the only cool hue far from every other face, which suits the
     * one face whose content is arbitrary. */
    case PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL:
        return (deskmate_palette_t){
            .hue = lv_color_hex(0x8b6cff),
            .tint = lv_color_hex(0xc0b0ff),
            .ink = lv_color_hex(0x0f0726),
        };
    default:
        break;
    }
    return (deskmate_palette_t){
        .hue = lv_color_hex(0xff8f2e),
        .tint = lv_color_hex(0xffc48a),
        .ink = lv_color_hex(0x1c1105),
    };
}

lv_obj_t *deskmate_module(lv_obj_t *parent, int32_t x, int32_t y, int32_t w,
                          int32_t h)
{
    lv_obj_t *module = lv_obj_create(parent);
    lv_obj_remove_style_all(module);
    lv_obj_remove_flag(module,
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(module, w, h);
    lv_obj_set_pos(module, x, y);
    lv_obj_set_style_bg_color(module, DESKMATE_COLOR_SURFACE, 0);
    lv_obj_set_style_bg_opa(module, LV_OPA_COVER, 0);
    lv_obj_set_style_radius(module, DESKMATE_RADIUS_MODULE, 0);
    return module;
}

lv_obj_t *deskmate_chip(lv_obj_t *parent, int32_t x, int32_t y,
                        lv_color_t fill, lv_color_t ink)
{
    lv_obj_t *chip = lv_label_create(parent);
    lv_obj_set_style_text_font(chip, DESKMATE_FONT_CAPTION, 0);
    lv_obj_set_style_text_color(chip, ink, 0);
    lv_obj_set_style_bg_color(chip, fill, 0);
    lv_obj_set_style_bg_opa(chip, LV_OPA_COVER, 0);
    lv_obj_set_style_radius(chip, DESKMATE_RADIUS_CHIP, 0);
    /* Vertical padding is what makes the pill DESKMATE_CHIP_HEIGHT tall
     * around a CAPTION line, so the header rail lands on the grid. */
    lv_obj_set_style_pad_hor(chip, 2 * DESKMATE_GRID, 0);
    lv_obj_set_style_pad_ver(
        chip,
        (DESKMATE_CHIP_HEIGHT -
         lv_font_get_line_height(DESKMATE_FONT_CAPTION)) / 2,
        0);
    lv_obj_set_style_text_letter_space(chip, 1, 0);
    lv_label_set_text(chip, "");
    lv_obj_set_pos(chip, x, y);
    return chip;
}

lv_obj_t *deskmate_eyebrow(lv_obj_t *parent, const char *text, int32_t x,
                           int32_t y, lv_color_t color)
{
    lv_obj_t *label = lv_label_create(parent);
    lv_obj_set_style_text_font(label, DESKMATE_FONT_CAPTION, 0);
    lv_obj_set_style_text_color(label, color, 0);
    lv_obj_set_style_text_letter_space(label, DESKMATE_EYEBROW_TRACKING, 0);
    lv_label_set_text(label, text);
    lv_obj_set_pos(label, x, y);
    return label;
}

lv_obj_t *deskmate_label_box(lv_obj_t *parent, int32_t x, int32_t y,
                             int32_t w, lv_text_align_t align,
                             lv_color_t color, const lv_font_t *font)
{
    lv_obj_t *label = lv_label_create(parent);
    lv_obj_set_style_text_font(label, font, 0);
    lv_obj_set_style_text_color(label, color, 0);
    /* Width alone is not enough: with LV_SIZE_CONTENT height LVGL breaks an
     * over-long run onto further lines instead of ellipsizing, so the height
     * is pinned to exactly one line of the tier. */
    lv_label_set_long_mode(label, LV_LABEL_LONG_DOT);
    lv_obj_set_width(label, w);
    lv_obj_set_height(label, lv_font_get_line_height(font));
    lv_obj_set_style_text_align(label, align, 0);
    lv_label_set_text(label, "");
    lv_obj_set_pos(label, x, y);
    return label;
}
