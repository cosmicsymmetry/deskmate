/* Host tests for the device's model of the loop.
 *
 * Protocol v2 reduced this to what the device still decides for itself: which
 * cards exist, which one is live, and the timer a pomodoro's `timer.*` scene
 * bindings resolve against. The template/size-class/field-registry tests this
 * file used to carry went with the concepts they checked -- the device does not
 * render a face and so cannot have an opinion about one.
 */

#include <assert.h>
#include <stdio.h>
#include <string.h>

#include "core/widget_model.h"

static protocol_apply_config_t config_with(uint32_t revision,
                                           const char *const *card_ids,
                                           size_t count)
{
    protocol_apply_config_t config;
    memset(&config, 0, sizeof(config));
    config.revision = revision;
    config.rotation = 90U;
    config.card_count = count;
    for (size_t i = 0U; i < count; ++i) {
        snprintf(config.cards[i].card_id, sizeof(config.cards[i].card_id), "%s",
                 card_ids[i]);
        config.cards[i].tap_action = PROTOCOL_TAP_NONE;
    }
    return config;
}

static protocol_push_timer_t timer_push(const char *card_id, uint32_t revision,
                                        uint32_t total_ms,
                                        uint32_t remaining_ms, bool running)
{
    protocol_push_timer_t push;
    memset(&push, 0, sizeof(push));
    snprintf(push.card_id, sizeof(push.card_id), "%s", card_id);
    push.revision = revision;
    push.total_ms = total_ms;
    push.remaining_ms = remaining_ms;
    push.running = running;
    return push;
}

static void test_config_applies_and_activates(void)
{
    widget_model_t model;
    widget_model_init(&model);
    assert(widget_model_config(&model) == NULL);
    assert(widget_model_active_card(&model) == NULL);

    const char *ids[] = {"clock", "focus"};
    protocol_apply_config_t config = config_with(1U, ids, 2U);
    assert(widget_model_apply_config(&model, &config) ==
           WIDGET_MODEL_CONFIG_APPLIED);
    assert(widget_model_config_revision(&model) == 1U);
    /* The first card leads the loop until something activates another. */
    assert(strcmp(widget_model_active_card(&model)->card_id, "clock") == 0);

    assert(widget_model_activate_card(&model, "focus"));
    assert(strcmp(widget_model_active_card(&model)->card_id, "focus") == 0);
    assert(!widget_model_activate_card(&model, "absent"));
    assert(strcmp(widget_model_active_card(&model)->card_id, "focus") == 0);
}

static void test_replace_keeps_the_live_card_when_it_survives(void)
{
    widget_model_t model;
    widget_model_init(&model);
    const char *first[] = {"clock", "focus"};
    protocol_apply_config_t config = config_with(1U, first, 2U);
    assert(widget_model_apply_config(&model, &config) ==
           WIDGET_MODEL_CONFIG_APPLIED);
    assert(widget_model_activate_card(&model, "focus"));

    /* Reordering must not jump the loop off the card on the panel. */
    const char *reordered[] = {"focus", "clock"};
    protocol_apply_config_t next = config_with(2U, reordered, 2U);
    assert(widget_model_apply_config(&model, &next) ==
           WIDGET_MODEL_CONFIG_APPLIED);
    assert(strcmp(widget_model_active_card(&model)->card_id, "focus") == 0);

    /* But a card that leaves cannot stay on the panel. */
    const char *without[] = {"clock"};
    protocol_apply_config_t third = config_with(3U, without, 1U);
    assert(widget_model_apply_config(&model, &third) ==
           WIDGET_MODEL_CONFIG_APPLIED);
    assert(strcmp(widget_model_active_card(&model)->card_id, "clock") == 0);
}

static void test_revision_rules(void)
{
    widget_model_t model;
    widget_model_init(&model);
    const char *ids[] = {"clock"};
    protocol_apply_config_t config = config_with(5U, ids, 1U);
    assert(widget_model_apply_config(&model, &config) ==
           WIDGET_MODEL_CONFIG_APPLIED);

    protocol_apply_config_t replay = config_with(5U, ids, 1U);
    assert(widget_model_apply_config(&model, &replay) ==
           WIDGET_MODEL_CONFIG_REPLAYED);

    protocol_apply_config_t stale = config_with(4U, ids, 1U);
    assert(widget_model_apply_config(&model, &stale) ==
           WIDGET_MODEL_CONFIG_STALE_REVISION);

    protocol_apply_config_t zero = config_with(0U, ids, 1U);
    assert(widget_model_apply_config(&model, &zero) ==
           WIDGET_MODEL_CONFIG_INVALID_VALUE);

    const char *duplicate[] = {"clock", "clock"};
    protocol_apply_config_t dup = config_with(6U, duplicate, 2U);
    assert(widget_model_apply_config(&model, &dup) ==
           WIDGET_MODEL_CONFIG_DUPLICATE_ID);
}

static void test_preflight_does_not_mutate(void)
{
    widget_model_t model;
    widget_model_init(&model);
    const char *ids[] = {"clock"};
    protocol_apply_config_t config = config_with(1U, ids, 1U);
    assert(widget_model_check_config(&model, &config) ==
           WIDGET_MODEL_CONFIG_APPLIED);
    /* Checking is not applying. */
    assert(widget_model_config(&model) == NULL);
    assert(widget_model_config_revision(&model) == 0U);
}

static void test_timer_push_rules(void)
{
    widget_model_t model;
    widget_model_init(&model);
    const char *ids[] = {"clock", "focus"};
    protocol_apply_config_t config = config_with(1U, ids, 2U);
    assert(widget_model_apply_config(&model, &config) ==
           WIDGET_MODEL_CONFIG_APPLIED);

    protocol_push_timer_t unknown = timer_push("ghost", 1U, 10U, 5U, true);
    assert(widget_model_apply_timer(&model, &unknown) ==
           WIDGET_MODEL_PUSH_UNKNOWN_WIDGET);

    protocol_push_timer_t push = timer_push("focus", 4U, 1500U, 900U, true);
    assert(widget_model_apply_timer(&model, &push) ==
           WIDGET_MODEL_PUSH_ACCEPTED);
    assert(widget_model_latest_data_revision(&model) == 4U);

    const widget_model_timer_t *timer = widget_model_timer(&model, "focus");
    assert(timer != NULL);
    assert(timer->total_ms == 1500U);
    assert(timer->remaining_ms == 900U);
    assert(timer->running);
    /* The timer belongs to one card; another card has none. */
    assert(widget_model_timer(&model, "clock") == NULL);

    protocol_push_timer_t stale = timer_push("focus", 3U, 1500U, 800U, true);
    assert(widget_model_apply_timer(&model, &stale) ==
           WIDGET_MODEL_PUSH_STALE_REVISION);

    protocol_push_timer_t zero = timer_push("focus", 0U, 1500U, 800U, true);
    assert(widget_model_apply_timer(&model, &zero) ==
           WIDGET_MODEL_PUSH_INVALID_ARGUMENT);
}

static void test_timer_is_dropped_when_its_card_leaves(void)
{
    widget_model_t model;
    widget_model_init(&model);
    const char *ids[] = {"clock", "focus"};
    protocol_apply_config_t config = config_with(1U, ids, 2U);
    assert(widget_model_apply_config(&model, &config) ==
           WIDGET_MODEL_CONFIG_APPLIED);
    protocol_push_timer_t push = timer_push("focus", 1U, 1500U, 900U, true);
    assert(widget_model_apply_timer(&model, &push) ==
           WIDGET_MODEL_PUSH_ACCEPTED);
    assert(widget_model_has_running_progress(&model));

    const char *without[] = {"clock"};
    protocol_apply_config_t next = config_with(2U, without, 1U);
    assert(widget_model_apply_config(&model, &next) ==
           WIDGET_MODEL_CONFIG_APPLIED);
    /* A timer with no card cannot be resolved against, and must not keep the
     * OTA deferral gate held open. */
    assert(widget_model_timer(&model, "focus") == NULL);
    assert(!widget_model_has_running_progress(&model));
}

static void test_running_progress_is_visible_to_ota_policy(void)
{
    widget_model_t model;
    widget_model_init(&model);
    const char *ids[] = {"focus"};
    protocol_apply_config_t config = config_with(1U, ids, 1U);
    assert(widget_model_apply_config(&model, &config) ==
           WIDGET_MODEL_CONFIG_APPLIED);
    assert(!widget_model_has_running_progress(&model));

    protocol_push_timer_t running = timer_push("focus", 1U, 1500U, 900U, true);
    assert(widget_model_apply_timer(&model, &running) ==
           WIDGET_MODEL_PUSH_ACCEPTED);
    assert(widget_model_has_running_progress(&model));

    protocol_push_timer_t paused = timer_push("focus", 2U, 1500U, 900U, false);
    assert(widget_model_apply_timer(&model, &paused) ==
           WIDGET_MODEL_PUSH_ACCEPTED);
    assert(!widget_model_has_running_progress(&model));
}

int main(void)
{
    test_config_applies_and_activates();
    test_replace_keeps_the_live_card_when_it_survives();
    test_revision_rules();
    test_preflight_does_not_mutate();
    test_timer_push_rules();
    test_timer_is_dropped_when_its_card_leaves();
    test_running_progress_is_visible_to_ota_policy();
    printf("test_widget_model: OK\n");
    return 0;
}
