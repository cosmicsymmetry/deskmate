#include <assert.h>
#include <stdio.h>
#include <string.h>

#include "../main/core/device_event_queue.h"

static device_event_queue_t s_queue;
static protocol_device_event_t s_event;

static protocol_device_event_t navigation_event(const char *screen_id)
{
    protocol_device_event_t event = {
        .kind = PROTOCOL_EVENT_NAVIGATION,
        .action = PROTOCOL_EVENT_ACTION_NAVIGATE_NEXT,
    };
    strcpy(event.widget_id, "clock");
    strcpy(event.screen_id, screen_id);
    return event;
}

static void test_fifo_and_pressure_gap(void)
{
    device_event_queue_init(&s_queue);
    for (size_t i = 0U; i < DEVICE_EVENT_QUEUE_CAPACITY; ++i) {
        s_event = navigation_event("screen");
        assert(device_event_queue_push(&s_queue, &s_event));
    }
    assert(device_event_queue_high_water(&s_queue) ==
           DEVICE_EVENT_QUEUE_CAPACITY);
    assert(!device_event_queue_push(&s_queue, &s_event));
    assert(device_event_queue_dropped(&s_queue) == 1U);
    assert(device_event_queue_latest_sequence(&s_queue) == 9U);

    for (uint64_t expected = 1U;
         expected <= DEVICE_EVENT_QUEUE_CAPACITY; ++expected) {
        assert(device_event_queue_pop(&s_queue, &s_event));
        assert(s_event.sequence == expected);
    }
    assert(!device_event_queue_pop(&s_queue, &s_event));

    assert(device_event_queue_push(&s_queue, &s_event));
    assert(device_event_queue_pop(&s_queue, &s_event));
    assert(s_event.sequence == 10U);
}

static void test_invalid_arguments_do_not_consume_sequence(void)
{
    device_event_queue_init(&s_queue);
    assert(!device_event_queue_push(NULL, &s_event));
    assert(!device_event_queue_push(&s_queue, NULL));
    assert(device_event_queue_latest_sequence(&s_queue) == 0U);
}

int main(void)
{
    test_fifo_and_pressure_gap();
    test_invalid_arguments_do_not_consume_sequence();
    printf("test_device_event_queue: OK (%zu-byte queue, capacity %u)\n",
           sizeof(s_queue), (unsigned)DEVICE_EVENT_QUEUE_CAPACITY);
    return 0;
}
