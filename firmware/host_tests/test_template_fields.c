#include <assert.h>
#include <stdio.h>
#include <string.h>

#include "../main/core/template_fields.h"

static template_field_state_t s_state;
static template_field_state_t s_staging;

static protocol_field_t *add_field(protocol_push_data_t *push,
                                   const char *key,
                                   protocol_field_type_t type)
{
    assert(push->field_count < PROTOCOL_MAX_FIELD_COUNT);
    protocol_field_t *field = &push->fields[push->field_count++];
    memset(field, 0, sizeof(*field));
    assert(strlen(key) <= PROTOCOL_MAX_FIELD_KEY_LENGTH);
    strcpy(field->key, key);
    field->type = type;
    return field;
}

static protocol_push_data_t progress_push(void)
{
    protocol_push_data_t push = {
        .widget_id = "timer",
        .revision = 1U,
    };
    add_field(&push, "duration_seconds", PROTOCOL_FIELD_INTEGER)
        ->value.integer = 1500;
    add_field(&push, "remaining_seconds", PROTOCOL_FIELD_INTEGER)
        ->value.integer = 1200;
    add_field(&push, "running", PROTOCOL_FIELD_BOOLEAN)
        ->value.boolean = true;
    return push;
}

static void test_registry_and_defaults(void)
{
    size_t count = 0U;
    const template_field_descriptor_t *fields = template_fields_registry(
        PROTOCOL_TEMPLATE_DIGITAL_CLOCK, &count);
    assert(fields != NULL);
    assert(count == 4U);
    assert(strcmp(fields[0].name, "title") == 0);
    assert(fields[0].type == PROTOCOL_FIELD_TEXT);
    assert(!fields[0].required);
    assert(strcmp(fields[1].name, "show_seconds") == 0);
    assert(fields[1].default_value.boolean);

    assert(template_fields_init(PROTOCOL_TEMPLATE_DIGITAL_CLOCK, &s_state));
    const template_field_value_t *title = template_fields_get(&s_state,
                                                               "title");
    const template_field_value_t *seconds = template_fields_get(
        &s_state, "show_seconds");
    const template_field_value_t *stale = template_fields_get(&s_state,
                                                               "stale");
    assert(title != NULL && strcmp(title->value.text, "") == 0);
    assert(seconds != NULL && seconds->value.boolean);
    assert(stale != NULL && !stale->value.boolean);

    fields = template_fields_registry(PROTOCOL_TEMPLATE_PROGRESS_RING,
                                      &count);
    assert(fields != NULL && count == 6U);
    fields = template_fields_registry(PROTOCOL_TEMPLATE_ROW_LIST, &count);
    assert(fields != NULL && count == 13U);
    assert(template_fields_registry((protocol_template_kind_t)99, &count) ==
           NULL);
}

static void test_progress_resolution_is_atomic(void)
{
    assert(template_fields_init(PROTOCOL_TEMPLATE_PROGRESS_RING, &s_state));
    protocol_push_data_t missing = {
        .widget_id = "timer",
        .revision = 1U,
    };
    assert(template_fields_resolve(&s_state, &s_staging, &missing) ==
           TEMPLATE_FIELDS_MISSING_REQUIRED);
    assert(strcmp(template_fields_get(&s_state, "label")->value.text,
                  "Pomodoro") == 0);

    protocol_push_data_t push = progress_push();
    strcpy(add_field(&push, "label", PROTOCOL_FIELD_TEXT)->value.text,
           "Deep work");
    add_field(&push, "future", PROTOCOL_FIELD_BOOLEAN)->value.boolean = true;
    assert(template_fields_resolve(&s_state, &s_staging, &push) ==
           TEMPLATE_FIELDS_OK);
    assert(template_fields_get(&s_state, "duration_seconds")
               ->value.integer == 1500);
    assert(template_fields_get(&s_state, "remaining_seconds")
               ->value.integer == 1200);
    assert(template_fields_get(&s_state, "running")->value.boolean);
    assert(strcmp(template_fields_get(&s_state, "label")->value.text,
                  "Deep work") == 0);

    assert(template_fields_resolve(&s_state, &s_staging, &push) ==
           TEMPLATE_FIELDS_OK);

    protocol_push_data_t invalid = push;
    invalid.fields[0].type = PROTOCOL_FIELD_TEXT;
    strcpy(invalid.fields[0].value.text, "wrong");
    assert(template_fields_resolve(&s_state, &s_staging, &invalid) ==
           TEMPLATE_FIELDS_WRONG_TYPE);
    assert(template_fields_get(&s_state, "duration_seconds")
               ->value.integer == 1500);

    invalid = push;
    invalid.fields[1].value.integer = 1501;
    assert(template_fields_resolve(&s_state, &s_staging, &invalid) ==
           TEMPLATE_FIELDS_OUT_OF_RANGE);
    assert(template_fields_get(&s_state, "remaining_seconds")
               ->value.integer == 1200);
}

static void test_common_state_and_row_bounds(void)
{
    assert(template_fields_init(PROTOCOL_TEMPLATE_PROGRESS_RING, &s_state));
    protocol_push_data_t push = progress_push();
    add_field(&push, "stale", PROTOCOL_FIELD_BOOLEAN)->value.boolean = true;
    strcpy(add_field(&push, "error", PROTOCOL_FIELD_TEXT)->value.text,
           "provider offline");
    assert(template_fields_resolve(&s_state, &s_staging, &push) ==
           TEMPLATE_FIELDS_OK);
    assert(template_fields_get(&s_state, "stale")->value.boolean);
    assert(strcmp(template_fields_get(&s_state, "error")->value.text,
                  "provider offline") == 0);

    assert(template_fields_init(PROTOCOL_TEMPLATE_ROW_LIST, &s_state));
    memset(&push, 0, sizeof(push));
    strcpy(push.widget_id, "calendar");
    push.revision = 2U;
    strcpy(add_field(&push, "row0_title", PROTOCOL_FIELD_TEXT)->value.text,
           "Standup");
    strcpy(add_field(&push, "row0_time", PROTOCOL_FIELD_TEXT)->value.text,
           "10:00");
    assert(template_fields_resolve(&s_state, &s_staging, &push) ==
           TEMPLATE_FIELDS_OK);
    assert(strcmp(template_fields_get(&s_state, "row0_title")->value.text,
                  "Standup") == 0);

    memset(push.fields[0].value.text, 'x', 97U);
    push.fields[0].value.text[97] = '\0';
    assert(template_fields_resolve(&s_state, &s_staging, &push) ==
           TEMPLATE_FIELDS_OUT_OF_RANGE);
    assert(strcmp(template_fields_get(&s_state, "row0_title")->value.text,
                  "Standup") == 0);
}

static void test_extended_template_registries(void)
{
    size_t count = 0U;
    const template_field_descriptor_t *fields = NULL;

    fields = template_fields_registry(PROTOCOL_TEMPLATE_ANALOG_CLOCK, &count);
    assert(fields != NULL);
    assert(count == 4U);

    fields = template_fields_registry(PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL,
                                      &count);
    assert(fields != NULL);
    assert(count == 5U);

    fields = template_fields_registry(PROTOCOL_TEMPLATE_ICON_BADGE_TEXT,
                                      &count);
    assert(fields != NULL);
    assert(count == 10U);

    /* The uint16_t seen mask requires every registry to fit 16 fields. */
    for (int kind = PROTOCOL_TEMPLATE_DIGITAL_CLOCK;
         kind <= PROTOCOL_TEMPLATE_ICON_BADGE_TEXT; ++kind) {
        (void)template_fields_registry((protocol_template_kind_t)kind, &count);
        assert(count <= 16U);
    }
}

static void test_weather_registry_covers_every_provider_field(void)
{
    /* Every field the weather provider emits must remain declared. */
    static const char *emitted[] = {
        "title", "value", "label", "badge", "icon",
        "temperature_tenths", "apparent_temperature_tenths", "unit",
        "stale", "error",
    };
    size_t count = 0U;
    const template_field_descriptor_t *fields =
        template_fields_registry(PROTOCOL_TEMPLATE_ICON_BADGE_TEXT, &count);
    assert(fields != NULL);
    for (size_t i = 0U; i < sizeof(emitted) / sizeof(emitted[0]); ++i) {
        bool found = false;
        for (size_t j = 0U; j < count; ++j) {
            if (strcmp(fields[j].name, emitted[i]) == 0) {
                found = true;
                break;
            }
        }
        assert(found);
    }
}

static void test_big_number_label_defaults_are_safe(void)
{
    template_field_state_t state;
    assert(template_fields_init(PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL, &state));
    /* No required fields: an empty push must still yield a renderable state. */
    const template_field_value_t *value = template_fields_get(&state, "value");
    assert(value != NULL);
    assert(strcmp(value->value.text, "--") == 0);
}

static void test_icon_badge_text_temperature_bounds(void)
{
    /* Regression for a bounds defect: temperature_tenths/
     * apparent_temperature_tenths are tenths of a degree in the user's
     * selected unit, not always Celsius. A hot Fahrenheit reading (e.g.
     * 101.3 F == 1013 tenths) must not push the whole card stale. */
    template_field_state_t state;
    template_field_state_t staging;
    assert(template_fields_init(PROTOCOL_TEMPLATE_ICON_BADGE_TEXT, &state));

    protocol_push_data_t hot_fahrenheit = {
        .widget_id = "weather",
        .revision = 1U,
    };
    add_field(&hot_fahrenheit, "temperature_tenths", PROTOCOL_FIELD_INTEGER)
        ->value.integer = 1013;
    assert(template_fields_resolve(&state, &staging, &hot_fahrenheit) ==
           TEMPLATE_FIELDS_OK);
    assert(template_fields_get(&state, "temperature_tenths")->value.integer ==
           1013);

    protocol_push_data_t out_of_range = {
        .widget_id = "weather",
        .revision = 2U,
    };
    add_field(&out_of_range, "temperature_tenths", PROTOCOL_FIELD_INTEGER)
        ->value.integer = 25000;
    assert(template_fields_resolve(&state, &staging, &out_of_range) ==
           TEMPLATE_FIELDS_OUT_OF_RANGE);
}

/* Invariant: every template kind protocol_template_kind_valid() accepts
 * must have a non-empty registry entry. widget_model's config preflight
 * (widget_model_check_config) validates template kinds using only the
 * numeric range check, while the later apply-time field-init path
 * (widget_model_apply_config) depends on the registry. If a kind were ever
 * added to the enum without a matching registry entry, preflight would
 * report success and apply would then fail with a misleading
 * WIDGET_MODEL_CONFIG_INVALID_VALUE deep inside field init instead of the
 * correct WIDGET_MODEL_CONFIG_UNSUPPORTED_TEMPLATE at preflight. Do not
 * delete this test as redundant with test_extended_template_registries: it
 * is the one that pins the range check and the registry together so they
 * cannot silently drift apart again.
 */
static void test_every_valid_kind_has_a_registry(void)
{
    for (int kind = PROTOCOL_TEMPLATE_DIGITAL_CLOCK;
         kind <= PROTOCOL_TEMPLATE_ICON_BADGE_TEXT; ++kind) {
        protocol_template_kind_t template_kind = (protocol_template_kind_t)kind;
        assert(protocol_template_kind_valid(template_kind));

        size_t count = 0U;
        const template_field_descriptor_t *fields = template_fields_registry(
            template_kind, &count);
        assert(fields != NULL);
        assert(count > 0U);
    }
}

int main(void)
{
    test_registry_and_defaults();
    test_progress_resolution_is_atomic();
    test_common_state_and_row_bounds();
    test_extended_template_registries();
    test_weather_registry_covers_every_provider_field();
    test_big_number_label_defaults_are_safe();
    test_icon_badge_text_temperature_bounds();
    test_every_valid_kind_has_a_registry();
    puts("test_template_fields: OK");
    return 0;
}
