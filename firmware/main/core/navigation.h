#pragma once

#include <stdbool.h>
#include <stdint.h>

#define NAVIGATION_TAP_MAX_DISTANCE_PX 18
#define NAVIGATION_TAP_MAX_DURATION_MS 500U
#define NAVIGATION_SWIPE_MIN_DISTANCE_PX 56
#define NAVIGATION_SWIPE_WIDTH_DIVISOR 6
#define NAVIGATION_SWIPE_MAX_DURATION_MS 750U

typedef struct {
    int32_t x;
    int32_t y;
} navigation_point_t;

typedef enum {
    NAVIGATION_GESTURE_NONE = 0,
    NAVIGATION_GESTURE_TAP,
    NAVIGATION_GESTURE_PREVIOUS,
    NAVIGATION_GESTURE_NEXT,
} navigation_gesture_t;

/**
 * Classify points already transformed into LVGL's current logical canvas.
 * A leftward swipe advances to the next carousel screen; rightward selects
 * the previous screen. Invalid/out-of-bounds samples never produce an action.
 */
navigation_gesture_t navigation_classify(
    navigation_point_t start,
    navigation_point_t end,
    uint64_t started_ms,
    uint64_t ended_ms,
    int32_t logical_width,
    int32_t logical_height);
