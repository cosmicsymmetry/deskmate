#include <assert.h>
#include <stdio.h>
#include <string.h>

#include "../main/core/link_state.h"
#include "../main/core/widget_model.h"

static widget_model_t s_model;

static protocol_apply_config_t config_two_screens(uint32_t revision)
{
    protocol_apply_config_t config = {
        .revision = revision,
        .rotation = 90U,
        .widget_count = 2U,
        .widgets = {
            {
                .widget_id = "clock",
                .template_kind = PROTOCOL_TEMPLATE_DIGITAL_CLOCK,
                .size_class = PROTOCOL_SIZE_FULL,
                .tap_action = PROTOCOL_TAP_NONE,
                .interrupt_policy = PROTOCOL_INTERRUPT_DISABLED,
            },
            {
                .widget_id = "timer",
                .template_kind = PROTOCOL_TEMPLATE_PROGRESS_RING,
                .size_class = PROTOCOL_SIZE_STANDARD,
                .tap_action = PROTOCOL_TAP_START_PAUSE,
                .interrupt_policy = PROTOCOL_INTERRUPT_ENABLED,
            },
        },
        .screen_count = 2U,
        .screens = {
            {.screen_id = "home", .widget_id = "clock"},
            {.screen_id = "focus", .widget_id = "timer"},
        },
    };
    return config;
}

static protocol_push_data_t clock_push(uint32_t revision)
{
    protocol_push_data_t push = {
        .widget_id = "clock",
        .revision = revision,
        .field_count = 1U,
        .fields = {{
            .key = "show_seconds",
            .type = PROTOCOL_FIELD_BOOLEAN,
            .value.boolean = false,
        }},
    };
    return push;
}

static protocol_push_data_t timer_push(uint32_t revision)
{
    protocol_push_data_t push = {
        .widget_id = "timer",
        .revision = revision,
        .field_count = 3U,
        .fields = {
            {.key = "duration_seconds", .type = PROTOCOL_FIELD_INTEGER,
             .value.integer = 1500},
            {.key = "remaining_seconds", .type = PROTOCOL_FIELD_INTEGER,
             .value.integer = 900},
            {.key = "running", .type = PROTOCOL_FIELD_BOOLEAN,
             .value.boolean = true},
        },
    };
    return push;
}

static void test_atomic_config_and_activation(void)
{
    widget_model_init(&s_model);
    assert(widget_model_config(&s_model) == NULL);
    assert(widget_model_active_screen(&s_model) == NULL);
    protocol_apply_config_t config = config_two_screens(1U);
    assert(widget_model_apply_config(&s_model, &config) ==
           WIDGET_MODEL_CONFIG_APPLIED);
    assert(widget_model_config(&s_model)->revision == 1U);
    assert(strcmp(widget_model_active_screen(&s_model)->screen_id, "home") ==
           0);

    assert(widget_model_activate_screen(&s_model, "focus"));
    assert(!widget_model_activate_screen(&s_model, "missing"));
    assert(strcmp(widget_model_active_screen(&s_model)->screen_id, "focus") ==
           0);

    protocol_apply_config_t invalid = config_two_screens(2U);
    strcpy(invalid.widgets[1].widget_id, "clock");
    assert(widget_model_apply_config(&s_model, &invalid) ==
           WIDGET_MODEL_CONFIG_DUPLICATE_ID);
    assert(widget_model_config(&s_model)->revision == 1U);
    assert(strcmp(widget_model_active_screen(&s_model)->screen_id, "focus") ==
           0);

    invalid = config_two_screens(2U);
    strcpy(invalid.screens[1].widget_id, "missing");
    assert(widget_model_apply_config(&s_model, &invalid) ==
           WIDGET_MODEL_CONFIG_UNKNOWN_WIDGET);
    assert(widget_model_config(&s_model)->revision == 1U);

    invalid = config_two_screens(2U);
    invalid.widgets[0].template_kind = (protocol_template_kind_t)99;
    assert(widget_model_apply_config(&s_model, &invalid) ==
           WIDGET_MODEL_CONFIG_UNSUPPORTED_TEMPLATE);
    invalid = config_two_screens(2U);
    invalid.widgets[0].size_class = PROTOCOL_SIZE_TILE;
    assert(widget_model_apply_config(&s_model, &invalid) ==
           WIDGET_MODEL_CONFIG_UNSUPPORTED_SIZE_CLASS);
    invalid = config_two_screens(2U);
    invalid.widget_count = PROTOCOL_MAX_CONFIG_WIDGETS + 1U;
    assert(widget_model_apply_config(&s_model, &invalid) ==
           WIDGET_MODEL_CONFIG_TOO_LARGE);
    invalid = config_two_screens(2U);
    invalid.rotation = 180U;
    assert(widget_model_apply_config(&s_model, &invalid) ==
           WIDGET_MODEL_CONFIG_INVALID_VALUE);
    assert(widget_model_config(&s_model)->revision == 1U);

    protocol_apply_config_t replacement = config_two_screens(2U);
    replacement.screens[0] = config.screens[1];
    replacement.screens[1] = config.screens[0];
    assert(widget_model_apply_config(&s_model, &replacement) ==
           WIDGET_MODEL_CONFIG_APPLIED);
    assert(strcmp(widget_model_active_screen(&s_model)->screen_id, "focus") ==
           0);
    assert(widget_model_apply_config(&s_model, &config) ==
           WIDGET_MODEL_CONFIG_STALE_REVISION);
}

static void test_replay_and_global_data_revisions(void)
{
    widget_model_init(&s_model);
    protocol_apply_config_t config = config_two_screens(4U);
    assert(widget_model_apply_config(&s_model, &config) ==
           WIDGET_MODEL_CONFIG_APPLIED);
    protocol_push_data_t push = clock_push(10U);
    assert(widget_model_apply_push(&s_model, &push) ==
           WIDGET_MODEL_PUSH_ACCEPTED);
    assert(widget_model_latest_data_revision(&s_model) == 10U);
    assert(!template_fields_get(widget_model_widget_fields(&s_model, "clock"),
                                "show_seconds")->value.boolean);

    assert(widget_model_apply_config(&s_model, &config) ==
           WIDGET_MODEL_CONFIG_REPLAYED);
    assert(!template_fields_get(widget_model_widget_fields(&s_model, "clock"),
                                "show_seconds")->value.boolean);

    push = timer_push(11U);
    assert(widget_model_apply_push(&s_model, &push) ==
           WIDGET_MODEL_PUSH_ACCEPTED);
    push = clock_push(11U);
    assert(widget_model_apply_push(&s_model, &push) ==
           WIDGET_MODEL_PUSH_STALE_REVISION);

    push = timer_push(12U);
    push.fields[1].value.integer = 1600;
    assert(widget_model_apply_push(&s_model, &push) ==
           WIDGET_MODEL_PUSH_INVALID_FIELDS);
    assert(widget_model_latest_data_revision(&s_model) == 11U);

    push = clock_push(12U);
    strcpy(push.widget_id, "missing");
    assert(widget_model_apply_push(&s_model, &push) ==
           WIDGET_MODEL_PUSH_UNKNOWN_WIDGET);
    assert(widget_model_latest_data_revision(&s_model) == 11U);

    push = clock_push(12U);
    push.field_count = 2U;
    strcpy(push.fields[1].key, "future_field");
    push.fields[1].type = PROTOCOL_FIELD_BOOLEAN;
    push.fields[1].value.boolean = true;
    assert(widget_model_apply_push(&s_model, &push) ==
           WIDGET_MODEL_PUSH_ACCEPTED);
    assert(!template_fields_get(widget_model_widget_fields(&s_model, "clock"),
                                "show_seconds")->value.boolean);
    assert(widget_model_latest_data_revision(&s_model) == 12U);

    protocol_apply_config_t changed_same_revision = config;
    changed_same_revision.rotation = 270U;
    assert(widget_model_apply_config(&s_model, &changed_same_revision) ==
           WIDGET_MODEL_CONFIG_STALE_REVISION);
    assert(!template_fields_get(widget_model_widget_fields(&s_model, "clock"),
                                "show_seconds")->value.boolean);

    protocol_apply_config_t replacement = config_two_screens(5U);
    assert(widget_model_apply_config(&s_model, &replacement) ==
           WIDGET_MODEL_CONFIG_APPLIED);
    assert(template_fields_get(widget_model_widget_fields(&s_model, "clock"),
                               "show_seconds")->value.boolean);
    assert(widget_model_latest_data_revision(&s_model) == 12U);
}

static void test_config_preflight_does_not_mutate_model(void)
{
    widget_model_init(&s_model);
    protocol_apply_config_t config = config_two_screens(1U);
    assert(widget_model_check_config(&s_model, &config) ==
           WIDGET_MODEL_CONFIG_APPLIED);
    assert(widget_model_config(&s_model) == NULL);

    assert(widget_model_apply_config(&s_model, &config) ==
           WIDGET_MODEL_CONFIG_APPLIED);
    assert(widget_model_activate_screen(&s_model, "focus"));
    assert(widget_model_check_config(&s_model, &config) ==
           WIDGET_MODEL_CONFIG_REPLAYED);
    assert(strcmp(widget_model_active_screen(&s_model)->screen_id, "focus") ==
           0);

    protocol_apply_config_t invalid = config_two_screens(2U);
    strcpy(invalid.widgets[1].widget_id, "clock");
    assert(widget_model_check_config(&s_model, &invalid) ==
           WIDGET_MODEL_CONFIG_DUPLICATE_ID);
    assert(widget_model_config_revision(&s_model) == 1U);

    protocol_apply_config_t changed_same_revision = config;
    changed_same_revision.rotation = 270U;
    assert(widget_model_check_config(&s_model, &changed_same_revision) ==
           WIDGET_MODEL_CONFIG_STALE_REVISION);
    assert(widget_model_config(&s_model)->rotation == 90U);

    protocol_apply_config_t replacement = config_two_screens(2U);
    assert(widget_model_check_config(&s_model, &replacement) ==
           WIDGET_MODEL_CONFIG_APPLIED);
    assert(widget_model_config_revision(&s_model) == 1U);
    assert(strcmp(widget_model_active_screen(&s_model)->screen_id, "focus") ==
           0);
}

static void build_single_widget_config(protocol_apply_config_t *config,
                                       const char *id,
                                       protocol_template_kind_t template_kind,
                                       uint32_t revision)
{
    memset(config, 0, sizeof(*config));
    config->revision = revision;
    config->rotation = 90U;
    config->widget_count = 1U;
    strcpy(config->widgets[0].widget_id, id);
    config->widgets[0].template_kind = template_kind;
    config->widgets[0].size_class = PROTOCOL_SIZE_FULL;
    config->widgets[0].tap_action = PROTOCOL_TAP_NONE;
    config->widgets[0].interrupt_policy = PROTOCOL_INTERRUPT_DISABLED;
    config->screen_count = 1U;
    strcpy(config->screens[0].screen_id, id);
    strcpy(config->screens[0].widget_id, id);
}

static void test_extended_templates_are_accepted(void)
{
    widget_model_t model;
    widget_model_init(&model);

    const protocol_template_kind_t kinds[] = {
        PROTOCOL_TEMPLATE_ANALOG_CLOCK,
        PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL,
        PROTOCOL_TEMPLATE_ICON_BADGE_TEXT,
    };
    for (size_t i = 0U; i < sizeof(kinds) / sizeof(kinds[0]); ++i) {
        protocol_apply_config_t config;
        build_single_widget_config(&config, "card", kinds[i],
                                   (uint32_t)(i + 1U));
        assert(widget_model_apply_config(&model, &config) ==
               WIDGET_MODEL_CONFIG_APPLIED);
    }

    /* An out-of-range kind must still be refused. */
    protocol_apply_config_t bad;
    build_single_widget_config(&bad, "card", (protocol_template_kind_t)7,
                               99U);
    assert(widget_model_apply_config(&model, &bad) ==
           WIDGET_MODEL_CONFIG_UNSUPPORTED_TEMPLATE);
}

static void test_timeout_retains_replay_state(void)
{
    widget_model_init(&s_model);
    link_state_t link;
    link_state_init(&link);
    protocol_apply_config_t config = config_two_screens(1U);
    assert(widget_model_apply_config(&s_model, &config) ==
           WIDGET_MODEL_CONFIG_APPLIED);
    protocol_push_data_t push = clock_push(7U);
    assert(widget_model_apply_push(&s_model, &push) ==
           WIDGET_MODEL_PUSH_ACCEPTED);

    assert(link_state_note_valid_request(&link, 100U));
    assert(link_state_poll(&link, 10100U));
    assert(!link.online);
    assert(widget_model_config(&s_model)->revision == 1U);
    assert(widget_model_latest_data_revision(&s_model) == 7U);
    assert(strcmp(widget_model_active_screen(&s_model)->screen_id, "home") ==
           0);

    assert(link_state_note_valid_request(&link, 12000U));
    assert(widget_model_apply_config(&s_model, &config) ==
           WIDGET_MODEL_CONFIG_REPLAYED);
    push = clock_push(8U);
    assert(widget_model_apply_push(&s_model, &push) ==
           WIDGET_MODEL_PUSH_ACCEPTED);

    widget_model_init(&s_model);
    assert(widget_model_config_revision(&s_model) == 0U);
    assert(widget_model_latest_data_revision(&s_model) == 0U);
    assert(widget_model_apply_push(&s_model, &push) ==
           WIDGET_MODEL_PUSH_UNKNOWN_WIDGET);
}

static void test_running_progress_is_visible_to_ota_policy(void)
{
    widget_model_init(&s_model);
    assert(!widget_model_has_running_progress(&s_model));
    protocol_apply_config_t config = config_two_screens(1U);
    assert(widget_model_apply_config(&s_model, &config) ==
           WIDGET_MODEL_CONFIG_APPLIED);
    assert(!widget_model_has_running_progress(&s_model));

    protocol_push_data_t push = timer_push(1U);
    assert(widget_model_apply_push(&s_model, &push) ==
           WIDGET_MODEL_PUSH_ACCEPTED);
    assert(widget_model_has_running_progress(&s_model));

    push = timer_push(2U);
    push.fields[2].value.boolean = false;
    assert(widget_model_apply_push(&s_model, &push) ==
           WIDGET_MODEL_PUSH_ACCEPTED);
    assert(!widget_model_has_running_progress(&s_model));
}

int main(void)
{
    test_atomic_config_and_activation();
    test_replay_and_global_data_revisions();
    test_config_preflight_does_not_mutate_model();
    test_extended_templates_are_accepted();
    test_timeout_retains_replay_state();
    test_running_progress_is_visible_to_ota_policy();
    printf("test_widget_model: OK (%zu-byte fixed model)\n", sizeof(s_model));
    return 0;
}
