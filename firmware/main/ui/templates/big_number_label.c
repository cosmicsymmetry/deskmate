#include "template_internal.h"

#include <string.h>

enum {
    OBJ_TITLE,
    OBJ_VALUE,
    OBJ_LABEL,
    OBJ_STATE,
};

bool big_number_label_create(template_widget_view_t *view,
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
    lv_obj_set_width(view->objects[OBJ_TITLE], 392);
    lv_obj_align(view->objects[OBJ_TITLE], LV_ALIGN_TOP_LEFT, 28, 24);
    lv_label_set_text(view->objects[OBJ_TITLE], "");

    view->objects[OBJ_VALUE] = lv_label_create(view->root);
    lv_obj_set_style_text_font(view->objects[OBJ_VALUE],
                               &lv_font_montserrat_48, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_VALUE],
                                lv_color_hex(0xf2c14e), 0);
    /* 16 schema-legal characters exceed 448px in this font. Bound the box and
     * ellipsize, so an over-long value is visibly cut rather than silently
     * clipped at the panel edge. */
    lv_label_set_long_mode(view->objects[OBJ_VALUE], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_VALUE], 400);
    lv_obj_set_style_text_align(view->objects[OBJ_VALUE],
                                LV_TEXT_ALIGN_CENTER, 0);
    lv_obj_align(view->objects[OBJ_VALUE], LV_ALIGN_CENTER, 0, -18);
    lv_label_set_text(view->objects[OBJ_VALUE], "--");

    view->objects[OBJ_LABEL] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_LABEL],
                                lv_color_hex(0xb3b6c7), 0);
    lv_label_set_long_mode(view->objects[OBJ_LABEL], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_LABEL], 380);
    lv_obj_set_style_text_align(view->objects[OBJ_LABEL],
                                LV_TEXT_ALIGN_CENTER, 0);
    lv_obj_align_to(view->objects[OBJ_LABEL], view->objects[OBJ_VALUE],
                    LV_ALIGN_OUT_BOTTOM_MID, 0, 16);
    lv_label_set_text(view->objects[OBJ_LABEL], "");

    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_MID, 0, -14);
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
        lv_label_set_text(view->objects[OBJ_TITLE], title->value.text);
    }
    if (value != NULL) {
        /* The schema default is "--", so an absent value renders a stable
         * placeholder rather than stale pixels from the previous card. */
        lv_label_set_text(view->objects[OBJ_VALUE],
                          value->value.text[0] != '\0' ? value->value.text
                                                       : "--");
    }
    if (label != NULL) {
        lv_label_set_text(view->objects[OBJ_LABEL], label->value.text);
    }
    /* OBJ_VALUE now has a fixed 400px width and (under LV_LABEL_LONG_DOT) a
     * fixed single-line height, so its box no longer changes with content:
     * the create()-time lv_obj_align already positions it correctly for the
     * object's lifetime, and re-running that align here would always
     * recompute the same coordinates. It is intentionally not repeated.
     * OBJ_LABEL still depends on OBJ_VALUE's box, so it is kept re-anchored
     * beneath it in case that assumption ever changes. */
    lv_obj_align_to(view->objects[OBJ_LABEL], view->objects[OBJ_VALUE],
                    LV_ALIGN_OUT_BOTTOM_MID, 0, 16);
}
