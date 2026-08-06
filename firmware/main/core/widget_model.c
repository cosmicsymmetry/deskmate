#include "widget_model.h"

#include <limits.h>
#include <string.h>

_Static_assert(sizeof(widget_model_t) <= 48U * 1024U,
               "fixed widget model exceeded its M2 RAM budget");

static bool bounded_text(const char *text,
                         size_t capacity,
                         size_t minimum_length)
{
    const char *end = memchr(text, '\0', capacity);
    return end != NULL && (size_t)(end - text) >= minimum_length;
}

static widget_model_config_result_t validate_config(
    const protocol_apply_config_t *config)
{
    if (config == NULL || config->revision == 0U ||
        config->widget_count == 0U || config->screen_count == 0U) {
        return WIDGET_MODEL_CONFIG_INVALID_VALUE;
    }
    if (config->widget_count > PROTOCOL_MAX_CONFIG_WIDGETS ||
        config->screen_count > PROTOCOL_MAX_CONFIG_SCREENS) {
        return WIDGET_MODEL_CONFIG_TOO_LARGE;
    }
    if (config->rotation != 90U && config->rotation != 270U) {
        return WIDGET_MODEL_CONFIG_INVALID_VALUE;
    }
    for (size_t i = 0U; i < config->widget_count; ++i) {
        const protocol_widget_config_t *widget = &config->widgets[i];
        if (!bounded_text(widget->widget_id, sizeof(widget->widget_id), 1U)) {
            return WIDGET_MODEL_CONFIG_INVALID_VALUE;
        }
        size_t field_count = 0U;
        if (template_fields_registry(widget->template_kind, &field_count) ==
            NULL) {
            return WIDGET_MODEL_CONFIG_UNSUPPORTED_TEMPLATE;
        }
        if (widget->size_class == PROTOCOL_SIZE_TILE ||
            (widget->size_class != PROTOCOL_SIZE_FULL &&
             widget->size_class != PROTOCOL_SIZE_STANDARD)) {
            return WIDGET_MODEL_CONFIG_UNSUPPORTED_SIZE_CLASS;
        }
        if (widget->tap_action < PROTOCOL_TAP_NONE ||
            widget->tap_action > PROTOCOL_TAP_RESET ||
            widget->interrupt_policy < PROTOCOL_INTERRUPT_DISABLED ||
            widget->interrupt_policy > PROTOCOL_INTERRUPT_ENABLED ||
            (widget->template_kind != PROTOCOL_TEMPLATE_PROGRESS_RING &&
             widget->tap_action != PROTOCOL_TAP_NONE)) {
            return WIDGET_MODEL_CONFIG_INVALID_VALUE;
        }
        for (size_t j = 0U; j < i; ++j) {
            if (strcmp(widget->widget_id, config->widgets[j].widget_id) == 0) {
                return WIDGET_MODEL_CONFIG_DUPLICATE_ID;
            }
        }
    }
    for (size_t i = 0U; i < config->screen_count; ++i) {
        const protocol_screen_config_t *screen = &config->screens[i];
        if (!bounded_text(screen->screen_id, sizeof(screen->screen_id), 1U) ||
            !bounded_text(screen->widget_id, sizeof(screen->widget_id), 1U)) {
            return WIDGET_MODEL_CONFIG_INVALID_VALUE;
        }
        for (size_t j = 0U; j < i; ++j) {
            if (strcmp(screen->screen_id, config->screens[j].screen_id) == 0) {
                return WIDGET_MODEL_CONFIG_DUPLICATE_ID;
            }
        }
        bool found = false;
        for (size_t j = 0U; j < config->widget_count; ++j) {
            if (strcmp(screen->widget_id, config->widgets[j].widget_id) == 0) {
                found = true;
                break;
            }
        }
        if (!found) {
            return WIDGET_MODEL_CONFIG_UNKNOWN_WIDGET;
        }
    }
    return WIDGET_MODEL_CONFIG_APPLIED;
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
    widget_model_config_result_t validation = validate_config(config);
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

    memset(model->widget_has_data[staging_index], 0,
           sizeof(model->widget_has_data[staging_index]));
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

bool widget_model_navigate(widget_model_t *model,
                           widget_navigation_t direction)
{
    const protocol_apply_config_t *config = widget_model_config(model);
    if (config == NULL || config->screen_count == 0U ||
        (direction != WIDGET_NAVIGATE_PREVIOUS &&
         direction != WIDGET_NAVIGATE_NEXT)) {
        return false;
    }
    if (direction == WIDGET_NAVIGATE_NEXT) {
        model->active_screen_index =
            (model->active_screen_index + 1U) % config->screen_count;
    } else if (model->active_screen_index == 0U) {
        model->active_screen_index = config->screen_count - 1U;
    } else {
        --model->active_screen_index;
    }
    return true;
}

widget_model_push_result_t widget_model_apply_push(
    widget_model_t *model,
    const protocol_push_data_t *push,
    widget_model_update_t *update)
{
    if (model == NULL || push == NULL || update == NULL ||
        push->revision == 0U) {
        return WIDGET_MODEL_PUSH_INVALID_ARGUMENT;
    }
    memset(update, 0, sizeof(*update));
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
    template_field_patch_t patch;
    template_fields_result_t result = template_fields_resolve(
        &model->widget_fields[model->live_config_index][index],
        &model->field_staging, push, &patch);
    if (result != TEMPLATE_FIELDS_OK) {
        return result == TEMPLATE_FIELDS_INVALID_ARGUMENT
                   ? WIDGET_MODEL_PUSH_INVALID_ARGUMENT
                   : WIDGET_MODEL_PUSH_INVALID_FIELDS;
    }
    update->widget_index = index;
    update->dirty_mask = patch.dirty_mask;
    update->unknown_fields = patch.unknown_fields;
    model->latest_data_revision = push->revision;
    model->widget_has_data[model->live_config_index][index] = true;
    if (patch.unknown_fields > UINT32_MAX - model->unknown_field_count) {
        model->unknown_field_count = UINT32_MAX;
    } else {
        model->unknown_field_count += (uint32_t)patch.unknown_fields;
    }
    return WIDGET_MODEL_PUSH_ACCEPTED;
}

uint32_t widget_model_latest_data_revision(const widget_model_t *model)
{
    return model != NULL ? model->latest_data_revision : 0U;
}

uint32_t widget_model_unknown_field_count(const widget_model_t *model)
{
    return model != NULL ? model->unknown_field_count : 0U;
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

bool widget_model_widget_has_data(const widget_model_t *model,
                                  const char *widget_id)
{
    const protocol_apply_config_t *config = widget_model_config(model);
    if (config == NULL || widget_id == NULL) {
        return false;
    }
    size_t index = widget_index(config, widget_id);
    return index < config->widget_count &&
           model->widget_has_data[model->live_config_index][index];
}
