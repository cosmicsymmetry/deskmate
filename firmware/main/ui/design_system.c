#include "design_system.h"

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
    lv_label_set_long_mode(label, LV_LABEL_LONG_DOT);
    lv_obj_set_width(label, w);
    lv_obj_set_height(label, lv_font_get_line_height(font));
    lv_obj_set_style_text_align(label, align, 0);
    lv_label_set_text(label, "");
    lv_obj_set_pos(label, x, y);
    return label;
}
