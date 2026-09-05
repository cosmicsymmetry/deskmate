#include "widget_model.h"

#include <string.h>

#include "core/apply_config_validation.h"

_Static_assert(sizeof(widget_model_t) <= 48U * 1024U,
               "fixed widget model exceeded its M2 RAM budget");

static widget_model_config_result_t config_validation_result(
    apply_config_validation_result_t result)
{
    switch (result) {
    case APPLY_CONFIG_VALID:
        return WIDGET_MODEL_CONFIG_APPLIED;
    case APPLY_CONFIG_TOO_LARGE:
        return WIDGET_MODEL_CONFIG_TOO_LARGE;
    case APPLY_CONFIG_DUPLICATE_ID:
        return WIDGET_MODEL_CONFIG_DUPLICATE_ID;
    case APPLY_CONFIG_UNKNOWN_WIDGET:
        return WIDGET_MODEL_CONFIG_UNKNOWN_WIDGET;
    case APPLY_CONFIG_UNSUPPORTED_TEMPLATE:
        return WIDGET_MODEL_CONFIG_UNSUPPORTED_TEMPLATE;
    case APPLY_CONFIG_UNSUPPORTED_SIZE_CLASS:
        return WIDGET_MODEL_CONFIG_UNSUPPORTED_SIZE_CLASS;
    case APPLY_CONFIG_INVALID_ARGUMENT:
        return WIDGET_MODEL_CONFIG_INVALID_ARGUMENT;
    case APPLY_CONFIG_INVALID_VALUE:
    default:
        return WIDGET_MODEL_CONFIG_INVALID_VALUE;
    }
}

static bool configs_equal(const protocol_apply_config_t *left,
                          const protocol_apply_config_t *right)
{
    if (left->revision != right->revision ||
        left->rotation != right->rotation ||
        left->widget_count != right->widget_count ||
        left->screen_count != right->screen_count) {
        return false;
    }
    for (size_t i = 0U; i < left->widget_count; ++i) {
        const protocol_widget_config_t *a = &left->widgets[i];
        const protocol_widget_config_t *b = &right->widgets[i];
        if (strcmp(a->widget_id, b->widget_id) != 0 ||
            a->template_kind != b->template_kind ||
            a->size_class != b->size_class ||
            a->tap_action != b->tap_action ||
            a->interrupt_policy != b->interrupt_policy) {
            return false;
        }
    }
    for (size_t i = 0U; i < left->screen_count; ++i) {
        const protocol_screen_config_t *a = &left->screens[i];
        const protocol_screen_config_t *b = &right->screens[i];
        if (strcmp(a->screen_id, b->screen_id) != 0 ||
            strcmp(a->widget_id, b->widget_id) != 0) {
            return false;
        }
    }
    return true;
}

static size_t screen_index(const protocol_apply_config_t *config,
                           const char *screen_id)
{
    for (size_t i = 0U; i < config->screen_count; ++i) {
        if (strcmp(config->screens[i].screen_id, screen_id) == 0) {
            return i;
        }
    }
    return config->screen_count;
}

static size_t widget_index(const protocol_apply_config_t *config,
                           const char *widget_id)
{
    for (size_t i = 0U; i < config->widget_count; ++i) {
        if (strcmp(config->widgets[i].widget_id, widget_id) == 0) {
            return i;
        }
    }
    return config->widget_count;
}

void widget_model_init(widget_model_t *model)
{
    if (model != NULL) {
        memset(model, 0, sizeof(*model));
    }
}

const protocol_apply_config_t *widget_model_config(
    const widget_model_t *model)
{
    return model != NULL && model->configured
               ? &model->configs[model->live_config_index]
               : NULL;
}

uint32_t widget_model_config_revision(const widget_model_t *model)
{
    const protocol_apply_config_t *config = widget_model_config(model);
    return config != NULL ? config->revision : 0U;
}

widget_model_config_result_t widget_model_check_config(
    const widget_model_t *model,
    const protocol_apply_config_t *config)
{
    if (model == NULL || config == NULL) {
        return WIDGET_MODEL_CONFIG_INVALID_ARGUMENT;
    }
    widget_model_config_result_t validation = config_validation_result(
        apply_config_validate(config));
    if (validation != WIDGET_MODEL_CONFIG_APPLIED) {
        return validation;
    }

    const protocol_apply_config_t *live = widget_model_config(model);
    if (live != NULL && config->revision <= live->revision) {
        if (config->revision == live->revision &&
            configs_equal(live, config)) {
            return WIDGET_MODEL_CONFIG_REPLAYED;
        }
        return WIDGET_MODEL_CONFIG_STALE_REVISION;
    }
    return WIDGET_MODEL_CONFIG_APPLIED;
}

widget_model_config_result_t widget_model_apply_config(
    widget_model_t *model,
    const protocol_apply_config_t *config)
{
    widget_model_config_result_t result = widget_model_check_config(model,
                                                                     config);
    if (result != WIDGET_MODEL_CONFIG_APPLIED) {
        return result;
    }

    uint8_t staging_index = model->configured
                                ? (uint8_t)(model->live_config_index ^ 1U)
                                : 1U;
    protocol_apply_config_t *staging = &model->configs[staging_index];
    *staging = *config;

    const protocol_apply_config_t *live = widget_model_config(model);
    char active_screen_id[PROTOCOL_MAX_SCREEN_ID_LENGTH + 1U] = {0};
    if (live != NULL && model->active_screen_index < live->screen_count) {
        strcpy(active_screen_id,
               live->screens[model->active_screen_index].screen_id);
    }
    size_t next_active = active_screen_id[0] == '\0'
                             ? staging->screen_count
                             : screen_index(staging, active_screen_id);
    if (next_active == staging->screen_count) {
        next_active = 0U;
    }

    for (size_t i = 0U; i < staging->widget_count; ++i) {
        bool initialized = template_fields_init(
            staging->widgets[i].template_kind,
            &model->widget_fields[staging_index][i]);
        if (!initialized) {
            return WIDGET_MODEL_CONFIG_INVALID_VALUE;
        }
    }
    for (size_t i = staging->widget_count;
         i < PROTOCOL_MAX_CONFIG_WIDGETS; ++i) {
        memset(&model->widget_fields[staging_index][i], 0,
               sizeof(model->widget_fields[staging_index][i]));
    }
    model->live_config_index = staging_index;
    model->configured = true;
    model->active_screen_index = next_active;
    return WIDGET_MODEL_CONFIG_APPLIED;
}

const protocol_screen_config_t *widget_model_active_screen(
    const widget_model_t *model)
{
    const protocol_apply_config_t *config = widget_model_config(model);
    if (config == NULL || model->active_screen_index >= config->screen_count) {
        return NULL;
    }
    return &config->screens[model->active_screen_index];
}

bool widget_model_activate_screen(widget_model_t *model,
                                  const char *screen_id)
{
    const protocol_apply_config_t *config = widget_model_config(model);
    if (config == NULL || screen_id == NULL) {
        return false;
    }
    size_t index = screen_index(config, screen_id);
    if (index == config->screen_count) {
        return false;
    }
    model->active_screen_index = index;
    return true;
}

widget_model_push_result_t widget_model_apply_push(
    widget_model_t *model,
    const protocol_push_data_t *push)
{
    if (model == NULL || push == NULL || push->revision == 0U) {
        return WIDGET_MODEL_PUSH_INVALID_ARGUMENT;
    }
    if (push->revision <= model->latest_data_revision) {
        return WIDGET_MODEL_PUSH_STALE_REVISION;
    }
    const protocol_apply_config_t *config = widget_model_config(model);
    if (config == NULL) {
        return WIDGET_MODEL_PUSH_UNKNOWN_WIDGET;
    }
    size_t index = widget_index(config, push->widget_id);
    if (index == config->widget_count) {
        return WIDGET_MODEL_PUSH_UNKNOWN_WIDGET;
    }
    template_fields_result_t result = template_fields_resolve(
        &model->widget_fields[model->live_config_index][index],
        &model->field_staging, push);
    if (result != TEMPLATE_FIELDS_OK) {
        return result == TEMPLATE_FIELDS_INVALID_ARGUMENT
                   ? WIDGET_MODEL_PUSH_INVALID_ARGUMENT
                   : WIDGET_MODEL_PUSH_INVALID_FIELDS;
    }
    model->latest_data_revision = push->revision;
    return WIDGET_MODEL_PUSH_ACCEPTED;
}

uint32_t widget_model_latest_data_revision(const widget_model_t *model)
{
    return model != NULL ? model->latest_data_revision : 0U;
}

const template_field_state_t *widget_model_widget_fields(
    const widget_model_t *model,
    const char *widget_id)
{
    const protocol_apply_config_t *config = widget_model_config(model);
    if (config == NULL || widget_id == NULL) {
        return NULL;
    }
    size_t index = widget_index(config, widget_id);
    return index < config->widget_count
               ? &model->widget_fields[model->live_config_index][index]
               : NULL;
}

bool widget_model_has_running_progress(const widget_model_t *model)
{
    const protocol_apply_config_t *config = widget_model_config(model);
    if (config == NULL) {
        return false;
    }
    for (size_t index = 0U; index < config->widget_count; ++index) {
        if (config->widgets[index].template_kind !=
            PROTOCOL_TEMPLATE_PROGRESS_RING) {
            continue;
        }
        const template_field_state_t *fields =
            &model->widget_fields[model->live_config_index][index];
        const template_field_value_t *running = template_fields_get(
            fields, "running");
        if (running != NULL && running->type == PROTOCOL_FIELD_BOOLEAN &&
            running->value.boolean) {
            return true;
        }
    }
    return false;
}
