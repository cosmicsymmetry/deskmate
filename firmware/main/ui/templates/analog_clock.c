#include "template_internal.h"

#include <string.h>
#include <time.h>

#include "core/clock_source.h"

enum {
    OBJ_FACE,
    OBJ_HOUR,
    OBJ_MINUTE,
    OBJ_SECOND,
    OBJ_HUB,
    OBJ_STATE,
};

/* The dial fills the short axis between the canvas margins. */
#define FACE_DIAMETER (368 - 2 * DESKMATE_MARGIN)
#define FACE_RADIUS   (FACE_DIAMETER / 2)

/* Hand lengths measured from the hub, all grid multiples, each ending on a
 * feature of the dial rather than in mid-air: the minute hand reaches the
 * major ticks' inner ends (r = 144), the second hand the minor ticks'
 * (r = 152), and the hour hand stops at 0.72 of the minute hand so the two
 * never read as one. */
#define HAND_HOUR_LEN   (13 * DESKMATE_GRID)
#define HAND_MINUTE_LEN (18 * DESKMATE_GRID)
#define HAND_SECOND_LEN (19 * DESKMATE_GRID)

#define HAND_HOUR_WIDTH   6
#define HAND_MINUTE_WIDTH 4
#define HAND_SECOND_WIDTH 2
#define HUB_DIAMETER      (2 * DESKMATE_GRID)

/* Deep enough to seat the longer of the two tick lengths with a grid unit
 * of surface showing on either side of it. */
#define CHAPTER_RING_WIDTH (4 * DESKMATE_GRID)

#define TICK_MAJOR_LEN   (2 * DESKMATE_GRID)
#define TICK_MINOR_LEN   DESKMATE_GRID
#define TICK_MAJOR_WIDTH 4
/* The eight minor ticks sit at 30-degree steps, so unlike the cardinals they
 * are drawn rotated and lose contrast to anti-aliasing: a 2px minor spreads
 * over three columns at partial coverage and all but vanishes in TERTIARY.
 * Three pixels keeps a whole lit column at the centre of the stroke. */
#define TICK_MINOR_WIDTH 3

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
    const deskmate_palette_t palette =
        deskmate_palette(PROTOCOL_TEMPLATE_ANALOG_CLOCK);
    view->clock_show_seconds = true;
    view->root = lv_obj_create(parent);
    lv_obj_remove_style_all(view->root);
    lv_obj_remove_flag(view->root,
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->root, LV_PCT(100), LV_PCT(100));

    /* A chapter ring: the surface the hour marks are set into. The
     * complication language groups related marks onto a surface, but a
     * filled disc would light 320px of AMOLED that the design otherwise
     * keeps off, so the surface is an annulus just deep enough to hold the
     * ticks. Untracked: nothing patches it. */
    lv_obj_t *chapter = lv_arc_create(view->root);
    lv_obj_set_size(chapter, FACE_DIAMETER, FACE_DIAMETER);
    lv_obj_center(chapter);
    lv_arc_set_bg_angles(chapter, 0, 360);
    lv_obj_remove_style(chapter, NULL, LV_PART_KNOB);
    lv_obj_remove_flag(chapter, LV_OBJ_FLAG_CLICKABLE);
    lv_obj_set_style_arc_color(chapter, DESKMATE_COLOR_SURFACE,
                               LV_PART_MAIN);
    lv_obj_set_style_arc_width(chapter, CHAPTER_RING_WIDTH, LV_PART_MAIN);
    lv_obj_set_style_arc_opa(chapter, LV_OPA_TRANSP, LV_PART_INDICATOR);

    /* An invisible container: the coordinate frame the ticks and hands are
     * placed in. */
    view->objects[OBJ_FACE] = lv_obj_create(view->root);
    lv_obj_remove_style_all(view->objects[OBJ_FACE]);
    lv_obj_remove_flag(view->objects[OBJ_FACE],
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->objects[OBJ_FACE], FACE_DIAMETER, FACE_DIAMETER);
    lv_obj_center(view->objects[OBJ_FACE]);

    /* Twelve ticks, children of the face and therefore untracked. The four
     * cardinals are longer, wider and brighter so the quarters can be read
     * without counting. Each tick is a rectangle whose transform pivot is
     * pushed down to the face centre, so setting its rotation swings it onto
     * its hour. The face has no border or padding, so the pivot offset is
     * exactly the radius. */
    for (int i = 0; i < 12; ++i) {
        lv_obj_t *tick = lv_obj_create(view->objects[OBJ_FACE]);
        lv_obj_remove_style_all(tick);
        lv_obj_remove_flag(tick,
                           LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
        bool major = (i % 3) == 0;
        int32_t width = major ? TICK_MAJOR_WIDTH : TICK_MINOR_WIDTH;
        lv_obj_set_size(tick, width, major ? TICK_MAJOR_LEN : TICK_MINOR_LEN);
        /* Twelve o'clock carries the face hue: a dial needs a stated "up",
         * and one marker carrying it is cheaper than numerals the hero
         * subset could not set anyway. Taking the hue rather than plain
         * white also ties the dial to the digital clock it shares an
         * identity with. */
        lv_obj_set_style_bg_color(tick,
                                  i == 0 ? palette.hue
                                         : (major ? DESKMATE_COLOR_SECONDARY
                                                  : DESKMATE_COLOR_TERTIARY),
                                  0);
        lv_obj_set_style_bg_opa(tick, LV_OPA_COVER, 0);
        lv_obj_set_style_transform_pivot_x(tick, width / 2, 0);
        lv_obj_set_style_transform_pivot_y(tick, FACE_RADIUS, 0);
        lv_obj_align(tick, LV_ALIGN_TOP_MID, 0, 0);
        lv_obj_set_style_transform_rotation(tick, i * 300, 0);
    }

    view->objects[OBJ_HOUR] =
        make_hand(view->objects[OBJ_FACE], HAND_HOUR_LEN, HAND_HOUR_WIDTH,
                  DESKMATE_COLOR_PRIMARY);
    lv_obj_align(view->objects[OBJ_HOUR], LV_ALIGN_CENTER, 0,
                 -HAND_HOUR_LEN / 2);

    view->objects[OBJ_MINUTE] =
        make_hand(view->objects[OBJ_FACE], HAND_MINUTE_LEN, HAND_MINUTE_WIDTH,
                  DESKMATE_COLOR_PRIMARY);
    lv_obj_align(view->objects[OBJ_MINUTE], LV_ALIGN_CENTER, 0,
                 -HAND_MINUTE_LEN / 2);

    view->objects[OBJ_SECOND] =
        make_hand(view->objects[OBJ_FACE], HAND_SECOND_LEN, HAND_SECOND_WIDTH,
                  palette.hue);
    lv_obj_align(view->objects[OBJ_SECOND], LV_ALIGN_CENTER, 0,
                 -HAND_SECOND_LEN / 2);

    view->objects[OBJ_HUB] = lv_obj_create(view->objects[OBJ_FACE]);
    lv_obj_remove_style_all(view->objects[OBJ_HUB]);
    lv_obj_remove_flag(view->objects[OBJ_HUB],
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->objects[OBJ_HUB], HUB_DIAMETER, HUB_DIAMETER);
    lv_obj_set_style_radius(view->objects[OBJ_HUB], LV_RADIUS_CIRCLE, 0);
    lv_obj_set_style_bg_color(view->objects[OBJ_HUB], palette.hue, 0);
    lv_obj_set_style_bg_opa(view->objects[OBJ_HUB], LV_OPA_COVER, 0);
    lv_obj_center(view->objects[OBJ_HUB]);

    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_MID, 0,
                 -2 * DESKMATE_GRID);
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
    /* `title` is still a schema and wire field — it names the card in the
     * companion's library — but no clock face draws it, so nothing here
     * reads it. */
    const template_field_value_t *show =
        template_fields_get(fields, "show_seconds");
    if (show != NULL) {
        view->clock_show_seconds = show->value.boolean;
        if (view->clock_show_seconds) {
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
    if (view->clock_show_seconds) {
        set_hand_angle(view->objects[OBJ_SECOND], now.tm_sec * 6);
    }
}
