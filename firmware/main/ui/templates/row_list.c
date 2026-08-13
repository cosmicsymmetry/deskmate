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
    OBJ_EMPTY,
    OBJ_STATE,
};

/* Rows begin 5 * GRID under the title's cap line and step by 7 * GRID, which
 * leaves a 3 * GRID channel between one row's descenders and the next row's
 * caps: enough to scan the column without the list feeling ventilated. The
 * fifth row's box ends at y = 323, clear of the state footer at 334. */
#define ROW_FIRST_Y (DESKMATE_MARGIN + 5 * DESKMATE_GRID)
#define ROW_PITCH   (7 * DESKMATE_GRID)

/* A fixed right-hand column for the times, so they form an edge rather than
 * ragging with the row titles' lengths. 12 * GRID, not the 10 that "09:00"
 * literally needs: BODY is Inter Regular with proportional figures, so a
 * five-character time measures anywhere from 72px to 80px depending on which
 * digits it contains, and the wider column keeps the widest of them clear of
 * the ellipsis. */
#define TIME_COLUMN_WIDTH (12 * DESKMATE_GRID)
#define TIME_GUTTER       (2 * DESKMATE_GRID)
#define ROW_TITLE_WIDTH \
    (448 - 2 * DESKMATE_MARGIN - TIME_COLUMN_WIDTH - TIME_GUTTER)

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
    lv_obj_set_style_text_font(view->objects[OBJ_TITLE],
                               DESKMATE_FONT_CAPTION, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_TITLE],
                                DESKMATE_COLOR_SECONDARY, 0);
    lv_label_set_long_mode(view->objects[OBJ_TITLE], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_TITLE], 448 - 2 * DESKMATE_MARGIN);
    lv_obj_set_height(view->objects[OBJ_TITLE],
                      lv_font_get_line_height(DESKMATE_FONT_CAPTION));
    lv_obj_align(view->objects[OBJ_TITLE], LV_ALIGN_TOP_LEFT, DESKMATE_MARGIN,
                 DESKMATE_MARGIN);
    lv_label_set_text(view->objects[OBJ_TITLE], "");

    const int32_t body_line = lv_font_get_line_height(DESKMATE_FONT_BODY);
    for (size_t row = 0U; row < 5U; ++row) {
        size_t title_index = OBJ_ROW0_TITLE + row * 2U;
        size_t time_index = title_index + 1U;
        int32_t y = ROW_FIRST_Y + (int32_t)row * ROW_PITCH;

        view->objects[title_index] = lv_label_create(view->root);
        lv_obj_set_style_text_font(view->objects[title_index],
                                   DESKMATE_FONT_BODY, 0);
        lv_obj_set_style_text_color(view->objects[title_index],
                                    DESKMATE_COLOR_PRIMARY, 0);
        lv_label_set_long_mode(view->objects[title_index], LV_LABEL_LONG_DOT);
        lv_obj_set_width(view->objects[title_index], ROW_TITLE_WIDTH);
        /* The height must be pinned, not just the width. With a width alone
         * the label keeps LV_SIZE_CONTENT height, and LVGL force-breaks an
         * over-long run onto further lines instead of ellipsizing — which is
         * why a 128-character row title used to render as five wrapped lines
         * on top of the rows beneath it. One pinned line makes
         * LV_LABEL_LONG_DOT actually fire. */
        lv_obj_set_height(view->objects[title_index], body_line);
        lv_obj_align(view->objects[title_index], LV_ALIGN_TOP_LEFT,
                     DESKMATE_MARGIN, y);

        view->objects[time_index] = lv_label_create(view->root);
        lv_obj_set_style_text_font(view->objects[time_index],
                                   DESKMATE_FONT_BODY, 0);
        lv_obj_set_style_text_color(view->objects[time_index],
                                    DESKMATE_COLOR_SECONDARY, 0);
        lv_label_set_long_mode(view->objects[time_index], LV_LABEL_LONG_DOT);
        lv_obj_set_width(view->objects[time_index], TIME_COLUMN_WIDTH);
        lv_obj_set_height(view->objects[time_index], body_line);
        lv_obj_set_style_text_align(view->objects[time_index],
                                    LV_TEXT_ALIGN_RIGHT, 0);
        lv_obj_align(view->objects[time_index], LV_ALIGN_TOP_RIGHT,
                     -DESKMATE_MARGIN, y);
    }

    /* An empty list should say so rather than show a bare title. */
    view->objects[OBJ_EMPTY] = lv_label_create(view->root);
    lv_obj_set_style_text_font(view->objects[OBJ_EMPTY], DESKMATE_FONT_BODY,
                               0);
    lv_obj_set_style_text_color(view->objects[OBJ_EMPTY],
                                DESKMATE_COLOR_TERTIARY, 0);
    lv_obj_set_height(view->objects[OBJ_EMPTY], body_line);
    lv_obj_align(view->objects[OBJ_EMPTY], LV_ALIGN_TOP_LEFT, DESKMATE_MARGIN,
                 ROW_FIRST_Y);
    lv_label_set_text(view->objects[OBJ_EMPTY], "Nothing to show");
    lv_obj_add_flag(view->objects[OBJ_EMPTY], LV_OBJ_FLAG_HIDDEN);

    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_MID, 0,
                 -2 * DESKMATE_GRID);
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
    size_t shown = 0U;
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
            ++shown;
            lv_obj_remove_flag(view->objects[title_index], LV_OBJ_FLAG_HIDDEN);
            lv_obj_remove_flag(view->objects[time_index], LV_OBJ_FLAG_HIDDEN);
        }
    }
    if (shown == 0U) {
        lv_obj_remove_flag(view->objects[OBJ_EMPTY], LV_OBJ_FLAG_HIDDEN);
    } else {
        lv_obj_add_flag(view->objects[OBJ_EMPTY], LV_OBJ_FLAG_HIDDEN);
    }
}
