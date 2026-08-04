#include "template_internal.h"

#include <stdio.h>
#include <string.h>

enum {
    OBJ_TITLE,
    OBJ_ROW0_TITLE,
    OBJ_ROW0_TIME,
    OBJ_ROW1_TITLE,
    OBJ_ROW1_TIME,
    OBJ_ROW2_TITLE,
    OBJ_ROW2_TIME,
    OBJ_ROW3_TITLE,
    OBJ_ROW3_TIME,
    OBJ_ROW4_TITLE,
    OBJ_ROW4_TIME,
    OBJ_STATE,
};

bool row_list_create(template_widget_view_t *view,
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
                                lv_color_hex(0xffffff), 0);
    lv_obj_align(view->objects[OBJ_TITLE], LV_ALIGN_TOP_LEFT, 20, 12);
    for (size_t row = 0U; row < 5U; ++row) {
        size_t title_index = OBJ_ROW0_TITLE + row * 2U;
        size_t time_index = title_index + 1U;
        int32_t y = 48 + (int32_t)row *
                            (size == PROTOCOL_SIZE_FULL ? 53 : 47);
        view->objects[title_index] = lv_label_create(view->root);
        lv_obj_set_width(view->objects[title_index], 300);
        lv_label_set_long_mode(view->objects[title_index],
                               LV_LABEL_LONG_MODE_DOTS);
        lv_obj_set_style_text_color(view->objects[title_index],
                                    lv_color_hex(0xe7e8ef), 0);
        lv_obj_align(view->objects[title_index], LV_ALIGN_TOP_LEFT, 20, y);
        view->objects[time_index] = lv_label_create(view->root);
        lv_obj_set_width(view->objects[time_index], 90);
        lv_obj_set_style_text_align(view->objects[time_index],
                                    LV_TEXT_ALIGN_RIGHT, 0);
        lv_obj_set_style_text_color(view->objects[time_index],
                                    lv_color_hex(0x8f93a8), 0);
        lv_obj_align(view->objects[time_index], LV_ALIGN_TOP_RIGHT, -20, y);
    }
    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_RIGHT, -20, -8);
    view->state_label = view->objects[OBJ_STATE];
    return true;
}

void row_list_patch(template_widget_view_t *view,
                    const template_field_state_t *fields,
                    uint16_t dirty_mask)
{
    (void)dirty_mask;
    if (view == NULL || view->root == NULL || fields == NULL) {
        return;
    }
    const template_field_value_t *title = template_fields_get(fields,
                                                               "title");
    if (title != NULL) {
        lv_label_set_text(view->objects[OBJ_TITLE], title->value.text);
    }
    for (size_t row = 0U; row < 5U; ++row) {
        char name[16];
        snprintf(name, sizeof(name), "row%u_title", (unsigned)row);
        const template_field_value_t *row_title = template_fields_get(fields,
                                                                      name);
        snprintf(name, sizeof(name), "row%u_time", (unsigned)row);
        const template_field_value_t *row_time = template_fields_get(fields,
                                                                     name);
        size_t title_index = OBJ_ROW0_TITLE + row * 2U;
        size_t time_index = title_index + 1U;
        lv_label_set_text(view->objects[title_index],
                          row_title != NULL ? row_title->value.text : "");
        lv_label_set_text(view->objects[time_index],
                          row_time != NULL ? row_time->value.text : "");
        if (row_title == NULL || row_title->value.text[0] == '\0') {
            lv_obj_add_flag(view->objects[title_index], LV_OBJ_FLAG_HIDDEN);
            lv_obj_add_flag(view->objects[time_index], LV_OBJ_FLAG_HIDDEN);
        } else {
            lv_obj_remove_flag(view->objects[title_index], LV_OBJ_FLAG_HIDDEN);
            lv_obj_remove_flag(view->objects[time_index], LV_OBJ_FLAG_HIDDEN);
        }
    }
}
