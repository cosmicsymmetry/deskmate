#include <assert.h>
#include <stdio.h>
#include <string.h>

#include "../main/ui/ui_command_queue.h"

static ui_command_queue_t s_queue;
static ui_command_t s_command;
static ui_command_t s_output;

static ui_command_t view_command(const char *widget_id,
                                 ui_command_type_t type,
                                 uint16_t dirty)
{
    ui_command_t command = {
        .type = type,
        .template_kind = PROTOCOL_TEMPLATE_DIGITAL_CLOCK,
        .size_class = PROTOCOL_SIZE_STANDARD,
        .dirty_mask = dirty,
    };
    strcpy(command.widget_id, widget_id);
    assert(template_fields_init(PROTOCOL_TEMPLATE_DIGITAL_CLOCK,
                                &command.fields));
    return command;
}

static void test_patch_coalescing(void)
{
    ui_command_queue_init(&s_queue);
    s_command = view_command("clock", UI_COMMAND_SHOW_VIEW, 1U);
    assert(ui_command_queue_push(&s_queue, &s_command));
    s_command = view_command("clock", UI_COMMAND_PATCH_VIEW, 2U);
    assert(ui_command_queue_push(&s_queue, &s_command));
    assert(ui_command_queue_coalesced(&s_queue) == 1U);
    assert(ui_command_queue_pop(&s_queue, &s_output));
    assert(s_output.type == UI_COMMAND_SHOW_VIEW);
    assert(s_output.size_class == PROTOCOL_SIZE_STANDARD);
    assert(s_output.dirty_mask == 3U);
    assert(!ui_command_queue_pop(&s_queue, &s_output));
}

static void test_view_supersedes_and_pressure_is_bounded(void)
{
    ui_command_queue_init(&s_queue);
    for (size_t i = 0U; i < UI_COMMAND_QUEUE_CAPACITY; ++i) {
        memset(&s_command, 0, sizeof(s_command));
        s_command.type = i % 2U == 0U ? UI_COMMAND_LINK_STATE
                                      : UI_COMMAND_TIME_OFFSET;
        if (s_command.type == UI_COMMAND_LINK_STATE) {
            s_command.online = true;
        }
        assert(ui_command_queue_push(&s_queue, &s_command));
        if (i >= 2U) {
            /* Same scalar kinds coalesce, so use distinct patch widgets. */
            s_command = view_command(i == 2U ? "a" : "b",
                                     UI_COMMAND_PATCH_VIEW, 1U);
            assert(ui_command_queue_push(&s_queue, &s_command));
        }
    }
    s_command = view_command("overflow", UI_COMMAND_PATCH_VIEW, 1U);
    assert(!ui_command_queue_push(&s_queue, &s_command));
    assert(ui_command_queue_dropped(&s_queue) == 1U);
    assert(ui_command_queue_high_water(&s_queue) ==
           UI_COMMAND_QUEUE_CAPACITY);

    s_command = view_command("latest", UI_COMMAND_SHOW_VIEW, UINT16_MAX);
    assert(ui_command_queue_push(&s_queue, &s_command));
    assert(ui_command_queue_pop(&s_queue, &s_output));
    assert(s_output.type == UI_COMMAND_LINK_STATE);
    assert(ui_command_queue_pop(&s_queue, &s_output));
    assert(s_output.type == UI_COMMAND_TIME_OFFSET);
    assert(ui_command_queue_pop(&s_queue, &s_output));
    assert(strcmp(s_output.widget_id, "latest") == 0);
    assert(!ui_command_queue_pop(&s_queue, &s_output));
    assert(ui_command_queue_coalesced(&s_queue) >= 4U);
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

static void test_offline_transition_cannot_be_starved_by_view_work(void)
{
    ui_command_queue_init(&s_queue);
    const char *ids[] = {"a", "b", "c", "d"};
    for (size_t i = 0U; i < UI_COMMAND_QUEUE_CAPACITY; ++i) {
        s_command = view_command(ids[i], UI_COMMAND_PATCH_VIEW, 1U);
        assert(ui_command_queue_push(&s_queue, &s_command));
    }
    memset(&s_command, 0, sizeof(s_command));
    s_command.type = UI_COMMAND_LINK_STATE;
    s_command.online = false;
    assert(ui_command_queue_push(&s_queue, &s_command));
    assert(ui_command_queue_pop(&s_queue, &s_output));
    assert(s_output.type == UI_COMMAND_LINK_STATE && !s_output.online);
    assert(!ui_command_queue_pop(&s_queue, &s_output));
    assert(ui_command_queue_dropped(&s_queue) == 0U);
    assert(ui_command_queue_coalesced(&s_queue) == 4U);
}

int main(void)
{
    test_patch_coalescing();
    test_view_supersedes_and_pressure_is_bounded();
    test_scalar_updates_replace_pending();
    test_offline_transition_cannot_be_starved_by_view_work();
    printf("test_ui_command_queue: OK (%zu-byte queue, capacity %u)\n",
           sizeof(s_queue), (unsigned)UI_COMMAND_QUEUE_CAPACITY);
    return 0;
}
