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
    OBJ_CARD0,
    OBJ_CARD1,
    OBJ_CARD2,
    OBJ_CARD3,
    OBJ_CARD4,
    OBJ_COUNT,
    OBJ_EMPTY,
    OBJ_STATE,
};

/* Each entry is its own surface module rather than a line of text on the
 * canvas. Five modules of 6 * GRID on a 7 * GRID pitch start a grid unit
 * under the header chip and end at y = 328, which leaves the shared state
 * footer its own band. */
#define ROW_FIRST_Y (7 * DESKMATE_GRID)
#define ROW_PITCH   (7 * DESKMATE_GRID)
#define ROW_HEIGHT  (6 * DESKMATE_GRID)
#define ROW_X       DESKMATE_MARGIN
#define ROW_WIDTH   (448 - 2 * DESKMATE_MARGIN)

/* A leading bar in the face hue turns a row into an entry: it gives the eye
 * a repeating left edge to run down, which a bare time column does not. */
#define BAR_X      DESKMATE_GRID
#define BAR_WIDTH  4
#define BAR_HEIGHT (4 * DESKMATE_GRID)

/* The time takes a fixed column so the entries form an edge instead of
 * ragging with their titles' lengths. */
#define TIME_X     (3 * DESKMATE_GRID)
#define TIME_WIDTH (11 * DESKMATE_GRID)
#define TITLE_X    (16 * DESKMATE_GRID)
#define TITLE_WIDTH (ROW_WIDTH - TITLE_X - 2 * DESKMATE_GRID)

#define COUNT_BOX (4 * DESKMATE_GRID)
#define COUNT_X   (448 - DESKMATE_MARGIN - COUNT_BOX)

bool row_list_create(template_widget_view_t *view,
                     lv_obj_t *parent,
                     protocol_size_class_t size)
{
    if (view == NULL || parent == NULL ||
        (size != PROTOCOL_SIZE_FULL && size != PROTOCOL_SIZE_STANDARD)) {
        return false;
    }
    memset(view, 0, sizeof(*view));
    const deskmate_palette_t palette =
        deskmate_palette(PROTOCOL_TEMPLATE_ROW_LIST);
    view->root = lv_obj_create(parent);
    lv_obj_remove_style_all(view->root);
    lv_obj_remove_flag(view->root,
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->root, LV_PCT(100), LV_PCT(100));

    view->objects[OBJ_TITLE] = deskmate_chip(
        view->root, DESKMATE_MARGIN, 2 * DESKMATE_GRID, palette.hue,
        palette.ink);

    /* How many entries there are is worth stating: the list is capped at
     * five, so "3" and "5" mean different things about what is missing. The
     * count is read off the rows already on screen, not off the wire. */
    view->objects[OBJ_COUNT] = lv_label_create(view->root);
    lv_obj_set_size(view->objects[OBJ_COUNT], COUNT_BOX, COUNT_BOX);
    lv_obj_set_pos(view->objects[OBJ_COUNT], COUNT_X, 2 * DESKMATE_GRID);
    lv_obj_set_style_text_font(view->objects[OBJ_COUNT],
                               DESKMATE_FONT_CAPTION, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_COUNT], palette.hue, 0);
    lv_obj_set_style_text_align(view->objects[OBJ_COUNT],
                                LV_TEXT_ALIGN_CENTER, 0);
    lv_obj_set_style_border_color(view->objects[OBJ_COUNT], palette.hue, 0);
    lv_obj_set_style_border_width(view->objects[OBJ_COUNT], 2, 0);
    lv_obj_set_style_border_opa(view->objects[OBJ_COUNT], LV_OPA_COVER, 0);
    lv_obj_set_style_radius(view->objects[OBJ_COUNT], LV_RADIUS_CIRCLE, 0);
    lv_obj_set_style_pad_top(
        view->objects[OBJ_COUNT],
        (COUNT_BOX - lv_font_get_line_height(DESKMATE_FONT_CAPTION)) / 2, 0);
    lv_label_set_text(view->objects[OBJ_COUNT], "");

    const int32_t body_line = lv_font_get_line_height(DESKMATE_FONT_BODY);
    const int32_t text_y = (ROW_HEIGHT - body_line) / 2;
    const size_t card_slot[5] = { OBJ_CARD0, OBJ_CARD1, OBJ_CARD2, OBJ_CARD3,
                                  OBJ_CARD4 };
    for (size_t row = 0U; row < 5U; ++row) {
        int32_t y = ROW_FIRST_Y + (int32_t)row * ROW_PITCH;
        lv_obj_t *card =
            deskmate_module(view->root, ROW_X, y, ROW_WIDTH, ROW_HEIGHT);
        lv_obj_set_style_radius(card, 2 * DESKMATE_GRID, 0);
        view->objects[card_slot[row]] = card;

        lv_obj_t *bar = lv_obj_create(card);
        lv_obj_remove_style_all(bar);
        lv_obj_remove_flag(bar,
                           LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
        lv_obj_set_size(bar, BAR_WIDTH, BAR_HEIGHT);
        lv_obj_set_pos(bar, BAR_X, (ROW_HEIGHT - BAR_HEIGHT) / 2);
        lv_obj_set_style_bg_color(bar, palette.hue, 0);
        lv_obj_set_style_bg_opa(bar, LV_OPA_COVER, 0);
        lv_obj_set_style_radius(bar, BAR_WIDTH / 2, 0);

        view->objects[OBJ_ROW0_TIME + row * 2U] = deskmate_label_box(
            card, TIME_X, text_y, TIME_WIDTH, LV_TEXT_ALIGN_LEFT,
            palette.hue, DESKMATE_FONT_BODY);
        view->objects[OBJ_ROW0_TITLE + row * 2U] = deskmate_label_box(
            card, TITLE_X, text_y, TITLE_WIDTH, LV_TEXT_ALIGN_LEFT,
            DESKMATE_COLOR_PRIMARY, DESKMATE_FONT_BODY);
    }

    /* An empty list should say so in the same grammar as a full one, so the
     * message sits in a module at the first entry's slot rather than as
     * loose text on the canvas. */
    lv_obj_t *empty_card = deskmate_module(view->root, ROW_X, ROW_FIRST_Y,
                                           ROW_WIDTH, ROW_HEIGHT);
    lv_obj_set_style_radius(empty_card, 2 * DESKMATE_GRID, 0);
    view->objects[OBJ_EMPTY] = empty_card;
    lv_obj_t *empty_text = deskmate_label_box(
        empty_card, TIME_X, text_y, ROW_WIDTH - 2 * TIME_X,
        LV_TEXT_ALIGN_LEFT, DESKMATE_COLOR_TERTIARY, DESKMATE_FONT_BODY);
    lv_label_set_text(empty_text, "Nothing to show");
    lv_obj_add_flag(empty_card, LV_OBJ_FLAG_HIDDEN);

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
    const size_t card_slot[5] = { OBJ_CARD0, OBJ_CARD1, OBJ_CARD2, OBJ_CARD3,
                                  OBJ_CARD4 };
    const template_field_value_t *title = template_fields_get(fields,
                                                               "title");
    if (title != NULL) {
        deskmate_chip_set_text(view->objects[OBJ_TITLE], title->value.text);
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
        size_t time_index = OBJ_ROW0_TIME + row * 2U;
        lv_label_set_text(view->objects[title_index],
                          row_title != NULL ? row_title->value.text : "");
        lv_label_set_text(view->objects[time_index],
                          row_time != NULL ? row_time->value.text : "");
        /* Hiding the module hides its children with it. */
        if (row_title == NULL || row_title->value.text[0] == '\0') {
            lv_obj_add_flag(view->objects[card_slot[row]],
                            LV_OBJ_FLAG_HIDDEN);
        } else {
            ++shown;
            lv_obj_remove_flag(view->objects[card_slot[row]],
                               LV_OBJ_FLAG_HIDDEN);
        }
    }
    if (shown == 0U) {
        lv_obj_remove_flag(view->objects[OBJ_EMPTY], LV_OBJ_FLAG_HIDDEN);
        lv_obj_add_flag(view->objects[OBJ_COUNT], LV_OBJ_FLAG_HIDDEN);
    } else {
        lv_obj_add_flag(view->objects[OBJ_EMPTY], LV_OBJ_FLAG_HIDDEN);
        char count_text[8];
        snprintf(count_text, sizeof(count_text), "%u", (unsigned)shown);
        lv_label_set_text(view->objects[OBJ_COUNT], count_text);
        lv_obj_remove_flag(view->objects[OBJ_COUNT], LV_OBJ_FLAG_HIDDEN);
    }
}
