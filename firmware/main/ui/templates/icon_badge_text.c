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

static weather_icon_t s_current_icon = WEATHER_ICON_UNKNOWN;

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
    lv_obj_set_style_text_color(view->objects[OBJ_TITLE],
                                lv_color_hex(0x8f93a8), 0);
    lv_label_set_long_mode(view->objects[OBJ_TITLE], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_TITLE], 190);
    lv_obj_align(view->objects[OBJ_TITLE], LV_ALIGN_TOP_LEFT, 28, 22);
    lv_label_set_text(view->objects[OBJ_TITLE], "");

    view->objects[OBJ_BADGE] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_BADGE],
                                lv_color_hex(0x8f93a8), 0);
    lv_label_set_long_mode(view->objects[OBJ_BADGE], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_BADGE], 190);
    lv_obj_set_style_text_align(view->objects[OBJ_BADGE],
                                LV_TEXT_ALIGN_RIGHT, 0);
    lv_obj_align(view->objects[OBJ_BADGE], LV_ALIGN_TOP_RIGHT, -28, 22);
    lv_label_set_text(view->objects[OBJ_BADGE], "");

    view->objects[OBJ_ICON] = lv_obj_create(view->root);
    lv_obj_remove_style_all(view->objects[OBJ_ICON]);
    lv_obj_remove_flag(view->objects[OBJ_ICON],
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->objects[OBJ_ICON], 120, 120);
    lv_obj_align(view->objects[OBJ_ICON], LV_ALIGN_LEFT_MID, 44, 6);

    view->objects[OBJ_VALUE] = lv_label_create(view->root);
    lv_obj_set_style_text_font(view->objects[OBJ_VALUE],
                               &lv_font_montserrat_48, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_VALUE],
                                lv_color_hex(0xf2c14e), 0);
    /* Only ~232px of canvas remains to the right of the icon, and 16
     * schema-legal characters in this font exceed that comfortably. Bound and
     * ellipsize rather than letting it run off the panel edge. */
    lv_label_set_long_mode(view->objects[OBJ_VALUE], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_VALUE], 204);
    lv_obj_align(view->objects[OBJ_VALUE], LV_ALIGN_LEFT_MID, 216, -14);
    lv_label_set_text(view->objects[OBJ_VALUE], "--");

    view->objects[OBJ_LABEL] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_LABEL],
                                lv_color_hex(0xb3b6c7), 0);
    lv_label_set_long_mode(view->objects[OBJ_LABEL], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_LABEL], 200);
    lv_obj_align(view->objects[OBJ_LABEL], LV_ALIGN_LEFT_MID, 216, 40);
    lv_label_set_text(view->objects[OBJ_LABEL], "");

    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_MID, 0, -12);
    view->state_label = view->objects[OBJ_STATE];

    s_current_icon = WEATHER_ICON_UNKNOWN;
    weather_icon_render(view->objects[OBJ_ICON], s_current_icon,
                        lv_color_hex(0xe6e8f0), lv_color_hex(0x101020));
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
    if (value != NULL) {
        lv_label_set_text(view->objects[OBJ_VALUE],
                          value->value.text[0] != '\0' ? value->value.text
                                                       : "--");
    }
    if (label != NULL) {
        lv_label_set_text(view->objects[OBJ_LABEL], label->value.text);
    }
    if (icon != NULL) {
        weather_icon_t next = weather_icon_from_name(icon->value.text);
        /* Only rebuild the artwork when the icon actually changes: every
         * rebuild deletes and recreates a dozen LVGL objects. */
        if (next != s_current_icon) {
            s_current_icon = next;
            weather_icon_render(view->objects[OBJ_ICON], next,
                                lv_color_hex(0xe6e8f0), lv_color_hex(0x101020));
        }
    }
}
