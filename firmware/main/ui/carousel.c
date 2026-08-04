#include "carousel.h"

#include <string.h>

#include "core/navigation.h"
#include "esp_timer.h"
#include "template_view.h"

typedef struct {
    device_event_queue_t *event_queue;
    lv_obj_t *screen;
    carousel_binding_t binding;
    navigation_point_t pressed_at;
    uint64_t pressed_ms;
    bool tracking;
} carousel_state_t;

static carousel_state_t s_carousel;

static uint64_t uptime_ms(void)
{
    return (uint64_t)esp_timer_get_time() / UINT64_C(1000);
}

static bool active_point(navigation_point_t *point)
{
    lv_indev_t *indev = lv_indev_active();
    if (indev == NULL || point == NULL) {
        return false;
    }
    lv_point_t lv_point;
    lv_indev_get_point(indev, &lv_point);
    point->x = lv_point.x;
    point->y = lv_point.y;
    return true;
}

static protocol_event_action_t tap_event_action(protocol_tap_action_t action)
{
    if (action == PROTOCOL_TAP_START_PAUSE) {
        return PROTOCOL_EVENT_ACTION_START_PAUSE;
    }
    if (action == PROTOCOL_TAP_RESET) {
        return PROTOCOL_EVENT_ACTION_RESET;
    }
    return PROTOCOL_EVENT_ACTION_NONE;
}

static void emit_event(navigation_gesture_t gesture)
{
    if (s_carousel.event_queue == NULL) {
        return;
    }
    protocol_device_event_t event = {0};
    if (s_carousel.binding.interrupt) {
        if (gesture != NAVIGATION_GESTURE_TAP) {
            return;
        }
        event.kind = PROTOCOL_EVENT_INTERRUPT_DISMISSED;
        event.action = PROTOCOL_EVENT_ACTION_DISMISS_INTERRUPT;
        event.has_interrupt_token = true;
        event.interrupt_token = s_carousel.binding.interrupt_token;
        strcpy(event.widget_id, s_carousel.binding.widget_id);
        strcpy(event.screen_id, s_carousel.binding.screen_id);
        (void)device_event_queue_push(s_carousel.event_queue, &event);
        return;
    }

    if (gesture == NAVIGATION_GESTURE_TAP) {
        event.action = tap_event_action(s_carousel.binding.tap_action);
        if (event.action == PROTOCOL_EVENT_ACTION_NONE) {
            return;
        }
        event.kind = PROTOCOL_EVENT_TAP;
        strcpy(event.widget_id, s_carousel.binding.widget_id);
        strcpy(event.screen_id, s_carousel.binding.screen_id);
        /* Optimistic feedback is intentionally reconciled by the next full
         * authoritative PushData snapshot, including when this enqueue drops. */
        template_view_apply_local_action(event.action);
    } else if (gesture == NAVIGATION_GESTURE_PREVIOUS) {
        if (strcmp(s_carousel.binding.previous_screen_id,
                   s_carousel.binding.screen_id) == 0) {
            return;
        }
        event.kind = PROTOCOL_EVENT_NAVIGATION;
        event.action = PROTOCOL_EVENT_ACTION_NAVIGATE_PREVIOUS;
        strcpy(event.widget_id, s_carousel.binding.previous_widget_id);
        strcpy(event.screen_id, s_carousel.binding.previous_screen_id);
    } else if (gesture == NAVIGATION_GESTURE_NEXT) {
        if (strcmp(s_carousel.binding.next_screen_id,
                   s_carousel.binding.screen_id) == 0) {
            return;
        }
        event.kind = PROTOCOL_EVENT_NAVIGATION;
        event.action = PROTOCOL_EVENT_ACTION_NAVIGATE_NEXT;
        strcpy(event.widget_id, s_carousel.binding.next_widget_id);
        strcpy(event.screen_id, s_carousel.binding.next_screen_id);
    } else {
        return;
    }
    (void)device_event_queue_push(s_carousel.event_queue, &event);
}

static void gesture_event_cb(lv_event_t *event)
{
    if (lv_event_get_current_target_obj(event) != s_carousel.screen) {
        return;
    }
    lv_event_code_t code = lv_event_get_code(event);
    if (code == LV_EVENT_PRESSED) {
        s_carousel.tracking = active_point(&s_carousel.pressed_at);
        s_carousel.pressed_ms = uptime_ms();
        return;
    }
    if (code == LV_EVENT_PRESS_LOST) {
        s_carousel.tracking = false;
        return;
    }
    if (code != LV_EVENT_RELEASED || !s_carousel.tracking) {
        return;
    }
    s_carousel.tracking = false;
    navigation_point_t released_at;
    lv_display_t *display = lv_display_get_default();
    if (!active_point(&released_at) || display == NULL) {
        return;
    }
    navigation_gesture_t gesture = navigation_classify(
        s_carousel.pressed_at, released_at, s_carousel.pressed_ms,
        uptime_ms(), lv_display_get_horizontal_resolution(display),
        lv_display_get_vertical_resolution(display));
    emit_event(gesture);
}

void carousel_init(device_event_queue_t *event_queue)
{
    memset(&s_carousel, 0, sizeof(s_carousel));
    s_carousel.event_queue = event_queue;
}

bool carousel_bind(lv_obj_t *screen, const carousel_binding_t *binding)
{
    if (screen == NULL || binding == NULL) {
        return false;
    }
    s_carousel.screen = screen;
    s_carousel.binding = *binding;
    s_carousel.tracking = false;
    lv_obj_add_event_cb(screen, gesture_event_cb, LV_EVENT_PRESSED, NULL);
    lv_obj_add_event_cb(screen, gesture_event_cb, LV_EVENT_RELEASED, NULL);
    lv_obj_add_event_cb(screen, gesture_event_cb, LV_EVENT_PRESS_LOST, NULL);
    return true;
}

void carousel_unbind(void)
{
    s_carousel.screen = NULL;
    memset(&s_carousel.binding, 0, sizeof(s_carousel.binding));
    s_carousel.tracking = false;
}
