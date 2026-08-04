#include <assert.h>
#include <stdio.h>

#include "../main/core/navigation.h"

#define LOGICAL_WIDTH 448
#define LOGICAL_HEIGHT 368

static navigation_gesture_t classify(navigation_point_t start,
                                     navigation_point_t end,
                                     uint64_t duration_ms)
{
    return navigation_classify(start, end, 1000U, 1000U + duration_ms,
                               LOGICAL_WIDTH, LOGICAL_HEIGHT);
}

static void test_tap_and_swipe_boundaries(void)
{
    navigation_point_t center = {224, 184};
    assert(classify(center, (navigation_point_t){242, 166}, 500U) ==
           NAVIGATION_GESTURE_TAP);
    assert(classify(center, (navigation_point_t){243, 184}, 100U) ==
           NAVIGATION_GESTURE_NONE);

    /* 448 / 6 truncates to the frozen 74-pixel primary threshold. */
    assert(classify(center, (navigation_point_t){150, 184}, 750U) ==
           NAVIGATION_GESTURE_NEXT);
    assert(classify(center, (navigation_point_t){298, 184}, 750U) ==
           NAVIGATION_GESTURE_PREVIOUS);
    assert(classify(center, (navigation_point_t){151, 184}, 200U) ==
           NAVIGATION_GESTURE_NONE);
    assert(classify(center, (navigation_point_t){150, 184}, 751U) ==
           NAVIGATION_GESTURE_NONE);
}

static void test_horizontal_dominance_and_invalid_samples(void)
{
    navigation_point_t start = {300, 100};
    assert(classify(start, (navigation_point_t){200, 180}, 200U) ==
           NAVIGATION_GESTURE_NONE);
    assert(classify(start, (navigation_point_t){200, 166}, 200U) ==
           NAVIGATION_GESTURE_NEXT);
    assert(navigation_classify((navigation_point_t){-1, 0},
                               (navigation_point_t){100, 0}, 0U, 1U,
                               LOGICAL_WIDTH, LOGICAL_HEIGHT) ==
           NAVIGATION_GESTURE_NONE);
    assert(navigation_classify(start, (navigation_point_t){200, 100},
                               2U, 1U, LOGICAL_WIDTH, LOGICAL_HEIGHT) ==
           NAVIGATION_GESTURE_NONE);
}

static navigation_point_t panel_to_logical(navigation_point_t panel,
                                           int rotation_degrees)
{
    /* Mirrors LVGL pointer processing for the 368x448 native panel. The
     * production classifier receives these already-logical coordinates. */
    if (rotation_degrees == 90) {
        return (navigation_point_t){panel.y, 367 - panel.x};
    }
    return (navigation_point_t){447 - panel.y, panel.x};
}

static void test_rotation_correct_logical_coordinates(void)
{
    navigation_point_t start_90 = panel_to_logical(
        (navigation_point_t){184, 320}, 90);
    navigation_point_t end_90 = panel_to_logical(
        (navigation_point_t){184, 200}, 90);
    assert(classify(start_90, end_90, 200U) == NAVIGATION_GESTURE_NEXT);

    navigation_point_t start_270 = panel_to_logical(
        (navigation_point_t){184, 127}, 270);
    navigation_point_t end_270 = panel_to_logical(
        (navigation_point_t){184, 247}, 270);
    assert(classify(start_270, end_270, 200U) == NAVIGATION_GESTURE_NEXT);
}

int main(void)
{
    test_tap_and_swipe_boundaries();
    test_horizontal_dominance_and_invalid_samples();
    test_rotation_correct_logical_coordinates();
    puts("test_navigation: OK");
    return 0;
}
