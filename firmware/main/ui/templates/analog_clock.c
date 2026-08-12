#include "template_internal.h"

#include <string.h>
#include <time.h>

#include "core/clock_source.h"

enum {
    OBJ_TITLE,
    OBJ_FACE,
    OBJ_HOUR,
    OBJ_MINUTE,
    OBJ_SECOND,
    OBJ_HUB,
    OBJ_STATE,
};

#define FACE_DIAMETER 300
#define HAND_HOUR_LEN 82
#define HAND_MINUTE_LEN 120
#define HAND_SECOND_LEN 132

static bool s_show_seconds = true;

static lv_obj_t *make_hand(lv_obj_t *parent, int16_t length, int16_t width,
                           lv_color_t color)
{
    lv_obj_t *hand = lv_obj_create(parent);
    lv_obj_remove_style_all(hand);
    lv_obj_remove_flag(hand, LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(hand, width, length);
    lv_obj_set_style_radius(hand, width / 2, 0);
    lv_obj_set_style_bg_color(hand, color, 0);
    lv_obj_set_style_bg_opa(hand, LV_OPA_COVER, 0);
    /* Pivot at the bottom centre so rotation sweeps around the hub. */
    lv_obj_set_style_transform_pivot_x(hand, width / 2, 0);
    lv_obj_set_style_transform_pivot_y(hand, length, 0);
    return hand;
}

static void set_hand_angle(lv_obj_t *hand, int32_t degrees)
{
    /* LVGL transform_rotation is in 0.1 degree units. */
    lv_obj_set_style_transform_rotation(hand, degrees * 10, 0);
}

bool analog_clock_create(template_widget_view_t *view,
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
    lv_obj_align(view->objects[OBJ_TITLE], LV_ALIGN_TOP_MID, 0, 14);

    view->objects[OBJ_FACE] = lv_obj_create(view->root);
    lv_obj_remove_style_all(view->objects[OBJ_FACE]);
    lv_obj_remove_flag(view->objects[OBJ_FACE],
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->objects[OBJ_FACE], FACE_DIAMETER, FACE_DIAMETER);
    lv_obj_set_style_radius(view->objects[OBJ_FACE], LV_RADIUS_CIRCLE, 0);
    lv_obj_set_style_border_width(view->objects[OBJ_FACE], 3, 0);
    lv_obj_set_style_border_color(view->objects[OBJ_FACE],
                                  lv_color_hex(0x3a3d4d), 0);
    lv_obj_center(view->objects[OBJ_FACE]);

    /* Twelve ticks, children of the face and therefore untracked. */
    for (int i = 0; i < 12; ++i) {
        lv_obj_t *tick = lv_obj_create(view->objects[OBJ_FACE]);
        lv_obj_remove_style_all(tick);
        lv_obj_remove_flag(tick,
                           LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
        bool major = (i % 3) == 0;
        lv_obj_set_size(tick, major ? 6 : 3, major ? 18 : 10);
        lv_obj_set_style_bg_color(
            tick, lv_color_hex(major ? 0xe6e8f0 : 0x6a6d80), 0);
        lv_obj_set_style_bg_opa(tick, LV_OPA_COVER, 0);
        lv_obj_set_style_transform_pivot_x(tick, (major ? 6 : 3) / 2, 0);
        /* The pivot is measured from the tick's own top edge and must land on the
         * face centre. LV_ALIGN_TOP_MID positions against the face's CONTENT box,
         * which lv_obj_get_content_coords insets by the 3px border set above, so a
         * tick's top sits at face-top + 8 + 3, not face-top + 8. Subtracting the
         * border width here keeps the twelve ticks concentric with the hands and hub,
         * which use centre alignment and are unaffected. */
        lv_obj_set_style_transform_pivot_y(tick, FACE_DIAMETER / 2 - 8 - 3, 0);
        lv_obj_align(tick, LV_ALIGN_TOP_MID, 0, 8);
        lv_obj_set_style_transform_rotation(tick, i * 300, 0);
    }

    view->objects[OBJ_HOUR] =
        make_hand(view->objects[OBJ_FACE], HAND_HOUR_LEN, 10,
                  lv_color_hex(0xffffff));
    lv_obj_align(view->objects[OBJ_HOUR], LV_ALIGN_CENTER, 0,
                 -HAND_HOUR_LEN / 2);

    view->objects[OBJ_MINUTE] =
        make_hand(view->objects[OBJ_FACE], HAND_MINUTE_LEN, 7,
                  lv_color_hex(0xffffff));
    lv_obj_align(view->objects[OBJ_MINUTE], LV_ALIGN_CENTER, 0,
                 -HAND_MINUTE_LEN / 2);

    view->objects[OBJ_SECOND] =
        make_hand(view->objects[OBJ_FACE], HAND_SECOND_LEN, 3,
                  lv_color_hex(0xf2c14e));
    lv_obj_align(view->objects[OBJ_SECOND], LV_ALIGN_CENTER, 0,
                 -HAND_SECOND_LEN / 2);

    view->objects[OBJ_HUB] = lv_obj_create(view->objects[OBJ_FACE]);
    lv_obj_remove_style_all(view->objects[OBJ_HUB]);
    lv_obj_remove_flag(view->objects[OBJ_HUB],
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->objects[OBJ_HUB], 16, 16);
    lv_obj_set_style_radius(view->objects[OBJ_HUB], LV_RADIUS_CIRCLE, 0);
    lv_obj_set_style_bg_color(view->objects[OBJ_HUB],
                              lv_color_hex(0xf2c14e), 0);
    lv_obj_set_style_bg_opa(view->objects[OBJ_HUB], LV_OPA_COVER, 0);
    lv_obj_center(view->objects[OBJ_HUB]);

    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_MID, 0, -10);
    view->state_label = view->objects[OBJ_STATE];
    return true;
}

void analog_clock_patch(template_widget_view_t *view,
                        const template_field_state_t *fields,
                        uint16_t dirty_mask)
{
    (void)dirty_mask;
    if (view == NULL || view->root == NULL || fields == NULL) {
        return;
    }
    const template_field_value_t *title =
        template_fields_get(fields, "title");
    const template_field_value_t *show =
        template_fields_get(fields, "show_seconds");
    if (title != NULL) {
        lv_label_set_text(view->objects[OBJ_TITLE], title->value.text);
    }
    if (show != NULL) {
        s_show_seconds = show->value.boolean;
        if (s_show_seconds) {
            lv_obj_remove_flag(view->objects[OBJ_SECOND], LV_OBJ_FLAG_HIDDEN);
        } else {
            lv_obj_add_flag(view->objects[OBJ_SECOND], LV_OBJ_FLAG_HIDDEN);
        }
    }
}

void analog_clock_tick(template_widget_view_t *view,
                       int16_t utc_offset_minutes)
{
    if (view == NULL || view->root == NULL) {
        return;
    }
    time_t local = (time_t)clock_source_now() +
                   (time_t)utc_offset_minutes * 60;
    struct tm now;
    if (gmtime_r(&local, &now) == NULL) {
        return;
    }
    int32_t hour12 = now.tm_hour % 12;
    set_hand_angle(view->objects[OBJ_HOUR],
                   (hour12 * 30) + (now.tm_min / 2));
    set_hand_angle(view->objects[OBJ_MINUTE], now.tm_min * 6);
    if (s_show_seconds) {
        set_hand_angle(view->objects[OBJ_SECOND], now.tm_sec * 6);
    }
}
