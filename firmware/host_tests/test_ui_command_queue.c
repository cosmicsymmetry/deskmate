#include <assert.h>
#include <stdio.h>
#include <string.h>

#include "../main/ui/ui_command_queue.h"

static ui_command_queue_t s_queue;
static ui_command_t s_command;
static ui_command_t s_output;

/* A UI command no longer carries a template field registry snapshot. Scenes
 * bypass this queue, so its only payloads are the card fallback and scalar
 * link/time state. */
_Static_assert(sizeof(ui_command_t) <= 8U,
               "shipping UI commands still carry a C-template payload");

static void test_card_fallback_maps_to_standalone_clock(void)
{
    assert(ui_command_action(UI_COMMAND_SHOW_CARD_FALLBACK) ==
           UI_COMMAND_ACTION_SHOW_STANDALONE_CLOCK);
    assert(ui_command_action(UI_COMMAND_LINK_STATE) ==
           UI_COMMAND_ACTION_APPLY_LINK_STATE);
    assert(ui_command_action(UI_COMMAND_TIME_OFFSET) ==
           UI_COMMAND_ACTION_APPLY_TIME_OFFSET);
}

static void test_card_fallback_round_trips_through_queue(void)
{
    ui_command_queue_init(&s_queue);
    memset(&s_command, 0, sizeof(s_command));
    s_command.type = UI_COMMAND_SHOW_CARD_FALLBACK;
    assert(ui_command_queue_push(&s_queue, &s_command));
    assert(ui_command_queue_pop(&s_queue, &s_output));
    assert(s_output.type == UI_COMMAND_SHOW_CARD_FALLBACK);
    assert(!ui_command_queue_pop(&s_queue, &s_output));
}

static void test_duplicate_card_fallbacks_coalesce(void)
{
    ui_command_queue_init(&s_queue);
    memset(&s_command, 0, sizeof(s_command));
    s_command.type = UI_COMMAND_SHOW_CARD_FALLBACK;
    assert(ui_command_queue_push(&s_queue, &s_command));
    assert(ui_command_queue_push(&s_queue, &s_command));
    assert(ui_command_queue_pop(&s_queue, &s_output));
    assert(s_output.type == UI_COMMAND_SHOW_CARD_FALLBACK);
    assert(!ui_command_queue_pop(&s_queue, &s_output));
    assert(ui_command_queue_coalesced(&s_queue) == 1U);
}

static void test_scalar_updates_replace_pending(void)
{
    ui_command_queue_init(&s_queue);
    memset(&s_command, 0, sizeof(s_command));
    s_command.type = UI_COMMAND_LINK_STATE;
    s_command.online = false;
    assert(ui_command_queue_push(&s_queue, &s_command));
    s_command.online = true;
    assert(ui_command_queue_push(&s_queue, &s_command));
    assert(ui_command_queue_pop(&s_queue, &s_output));
    assert(s_output.online);
    assert(ui_command_queue_coalesced(&s_queue) == 1U);
}

static void test_scene_cancels_only_the_card_fallback(void)
{
    ui_command_queue_init(&s_queue);
    memset(&s_command, 0, sizeof(s_command));
    s_command.type = UI_COMMAND_LINK_STATE;
    s_command.online = false;
    assert(ui_command_queue_push(&s_queue, &s_command));
    s_command.type = UI_COMMAND_SHOW_CARD_FALLBACK;
    assert(ui_command_queue_push(&s_queue, &s_command));

    /* A successful PushScene removes the provisional card fallback, but it
     * must never cancel the authoritative offline transition for host loss. */
    assert(ui_command_queue_discard_card_fallbacks(&s_queue));
    assert(ui_command_queue_pop(&s_queue, &s_output));
    assert(s_output.type == UI_COMMAND_LINK_STATE && !s_output.online);
    assert(!ui_command_queue_pop(&s_queue, &s_output));
}

static void test_offline_transition_cannot_be_starved(void)
{
    ui_command_queue_init(&s_queue);
    memset(&s_command, 0, sizeof(s_command));
    s_command.type = UI_COMMAND_SHOW_CARD_FALLBACK;
    assert(ui_command_queue_push(&s_queue, &s_command));

    s_command.type = UI_COMMAND_TIME_OFFSET;
    s_command.utc_offset_minutes = 240;
    assert(ui_command_queue_push(&s_queue, &s_command));

    s_command.type = UI_COMMAND_LINK_STATE;
    s_command.online = false;
    assert(ui_command_queue_push(&s_queue, &s_command));

    assert(ui_command_queue_pop(&s_queue, &s_output));
    assert(s_output.type == UI_COMMAND_TIME_OFFSET);
    assert(ui_command_queue_pop(&s_queue, &s_output));
    assert(s_output.type == UI_COMMAND_LINK_STATE && !s_output.online);
    assert(!ui_command_queue_pop(&s_queue, &s_output));
    assert(ui_command_queue_dropped(&s_queue) == 0U);
    assert(ui_command_queue_coalesced(&s_queue) == 1U);
}

static void test_brightness_coalesces_and_survives_host_loss(void)
{
    ui_command_queue_init(&s_queue);
    ui_command_t command = {.type = UI_COMMAND_BRIGHTNESS, .brightness = 26U};
    assert(ui_command_action(command.type) == UI_COMMAND_ACTION_APPLY_BRIGHTNESS);
    assert(ui_command_queue_push(&s_queue, &command));
    command.brightness = 255U;
    assert(ui_command_queue_push(&s_queue, &command));
    command.type = UI_COMMAND_SHOW_CARD_FALLBACK;
    assert(ui_command_queue_push(&s_queue, &command));
    command.type = UI_COMMAND_LINK_STATE;
    command.online = false;
    assert(ui_command_queue_push(&s_queue, &command));
    assert(ui_command_queue_pop(&s_queue, &s_output));
    assert(s_output.type == UI_COMMAND_BRIGHTNESS && s_output.brightness == 255U);
    assert(ui_command_queue_pop(&s_queue, &s_output));
    assert(s_output.type == UI_COMMAND_LINK_STATE && !s_output.online);
    assert(!ui_command_queue_pop(&s_queue, &s_output));
    assert(ui_command_queue_dropped(&s_queue) == 0U);
}

int main(void)
{
    test_card_fallback_maps_to_standalone_clock();
    test_card_fallback_round_trips_through_queue();
    test_duplicate_card_fallbacks_coalesce();
    test_scalar_updates_replace_pending();
    test_brightness_coalesces_and_survives_host_loss();
    test_scene_cancels_only_the_card_fallback();
    test_offline_transition_cannot_be_starved();
    printf("test_ui_command_queue: OK (%zu-byte queue, capacity %u)\n",
           sizeof(s_queue), (unsigned)UI_COMMAND_QUEUE_CAPACITY);
    return 0;
}
