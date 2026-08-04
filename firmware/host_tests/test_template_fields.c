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
    template_field_patch_t patch;
    protocol_push_data_t missing = {
        .widget_id = "timer",
        .revision = 1U,
    };
    assert(template_fields_resolve(&s_state, &s_staging, &missing, &patch) ==
           TEMPLATE_FIELDS_MISSING_REQUIRED);
    assert(patch.dirty_mask == 0U);
    assert(strcmp(template_fields_get(&s_state, "label")->value.text,
                  "Pomodoro") == 0);

    protocol_push_data_t push = progress_push();
    strcpy(add_field(&push, "label", PROTOCOL_FIELD_TEXT)->value.text,
           "Deep work");
    add_field(&push, "future", PROTOCOL_FIELD_BOOLEAN)->value.boolean = true;
    assert(template_fields_resolve(&s_state, &s_staging, &push, &patch) ==
           TEMPLATE_FIELDS_OK);
    assert(patch.dirty_mask != 0U);
    assert(patch.unknown_fields == 1U);
    assert(template_fields_get(&s_state, "duration_seconds")
               ->value.integer == 1500);
    assert(template_fields_get(&s_state, "remaining_seconds")
               ->value.integer == 1200);
    assert(template_fields_get(&s_state, "running")->value.boolean);
    assert(strcmp(template_fields_get(&s_state, "label")->value.text,
                  "Deep work") == 0);

    assert(template_fields_resolve(&s_state, &s_staging, &push, &patch) ==
           TEMPLATE_FIELDS_OK);
    assert(patch.dirty_mask == 0U);

    protocol_push_data_t invalid = push;
    invalid.fields[0].type = PROTOCOL_FIELD_TEXT;
    strcpy(invalid.fields[0].value.text, "wrong");
    assert(template_fields_resolve(&s_state, &s_staging, &invalid, &patch) ==
           TEMPLATE_FIELDS_WRONG_TYPE);
    assert(patch.dirty_mask == 0U);
    assert(template_fields_get(&s_state, "duration_seconds")
               ->value.integer == 1500);

    invalid = push;
    invalid.fields[1].value.integer = 1501;
    assert(template_fields_resolve(&s_state, &s_staging, &invalid, &patch) ==
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
    template_field_patch_t patch;
    assert(template_fields_resolve(&s_state, &s_staging, &push, &patch) ==
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
    assert(template_fields_resolve(&s_state, &s_staging, &push, &patch) ==
           TEMPLATE_FIELDS_OK);
    assert(strcmp(template_fields_get(&s_state, "row0_title")->value.text,
                  "Standup") == 0);

    memset(push.fields[0].value.text, 'x', 97U);
    push.fields[0].value.text[97] = '\0';
    assert(template_fields_resolve(&s_state, &s_staging, &push, &patch) ==
           TEMPLATE_FIELDS_OUT_OF_RANGE);
    assert(strcmp(template_fields_get(&s_state, "row0_title")->value.text,
                  "Standup") == 0);
}

int main(void)
{
    test_registry_and_defaults();
    test_progress_resolution_is_atomic();
    test_common_state_and_row_bounds();
    puts("test_template_fields: OK");
    return 0;
}
