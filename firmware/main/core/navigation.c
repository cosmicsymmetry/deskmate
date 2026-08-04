#include "navigation.h"

#include <limits.h>

static uint32_t absolute_delta(int32_t left, int32_t right)
{
    int64_t delta = (int64_t)left - right;
    if (delta < 0) {
        delta = -delta;
    }
    return delta > UINT32_MAX ? UINT32_MAX : (uint32_t)delta;
}

static bool point_is_valid(navigation_point_t point,
                           int32_t width,
                           int32_t height)
{
    return width > 0 && height > 0 && point.x >= 0 && point.y >= 0 &&
           point.x < width && point.y < height;
}

navigation_gesture_t navigation_classify(
    navigation_point_t start,
    navigation_point_t end,
    uint64_t started_ms,
    uint64_t ended_ms,
    int32_t logical_width,
    int32_t logical_height)
{
    if (!point_is_valid(start, logical_width, logical_height) ||
        !point_is_valid(end, logical_width, logical_height) ||
        ended_ms < started_ms) {
        return NAVIGATION_GESTURE_NONE;
    }
    uint64_t duration = ended_ms - started_ms;
    uint32_t dx = absolute_delta(end.x, start.x);
    uint32_t dy = absolute_delta(end.y, start.y);
    if (duration <= NAVIGATION_TAP_MAX_DURATION_MS &&
        dx <= NAVIGATION_TAP_MAX_DISTANCE_PX &&
        dy <= NAVIGATION_TAP_MAX_DISTANCE_PX) {
        return NAVIGATION_GESTURE_TAP;
    }

    uint32_t swipe_threshold =
        (uint32_t)logical_width / NAVIGATION_SWIPE_WIDTH_DIVISOR;
    if (swipe_threshold < NAVIGATION_SWIPE_MIN_DISTANCE_PX) {
        swipe_threshold = NAVIGATION_SWIPE_MIN_DISTANCE_PX;
    }
    if (duration > NAVIGATION_SWIPE_MAX_DURATION_MS ||
        dx < swipe_threshold || (uint64_t)dx < (uint64_t)dy * 3U / 2U) {
        return NAVIGATION_GESTURE_NONE;
    }
    return end.x < start.x ? NAVIGATION_GESTURE_NEXT
                           : NAVIGATION_GESTURE_PREVIOUS;
}
