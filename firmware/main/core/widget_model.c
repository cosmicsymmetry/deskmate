#include "widget_model.h"

#include <string.h>

static const protocol_apply_config_t *live_config(const widget_model_t *model)
{
    return model->configured ? &model->configs[model->live_config_index] : NULL;
}

static bool duplicate_card_ids(const protocol_apply_config_t *config)
{
    for (size_t i = 0U; i < config->card_count; ++i) {
        for (size_t j = i + 1U; j < config->card_count; ++j) {
            if (strcmp(config->cards[i].card_id, config->cards[j].card_id) == 0) {
                return true;
            }
        }
    }
    return false;
}

void widget_model_init(widget_model_t *model)
{
    if (model == NULL) {
        return;
    }
    memset(model, 0, sizeof(*model));
}

widget_model_config_result_t widget_model_check_config(
    const widget_model_t *model,
    const protocol_apply_config_t *config)
{
    if (model == NULL || config == NULL) {
        return WIDGET_MODEL_CONFIG_INVALID_ARGUMENT;
    }
    if (config->revision == 0U) {
        return WIDGET_MODEL_CONFIG_INVALID_VALUE;
    }
    const protocol_apply_config_t *current = live_config(model);
    if (current != NULL) {
        if (config->revision < current->revision) {
            return WIDGET_MODEL_CONFIG_STALE_REVISION;
        }
        if (config->revision == current->revision) {
            return WIDGET_MODEL_CONFIG_REPLAYED;
        }
    }
    if (config->card_count == 0U ||
        config->card_count > PROTOCOL_MAX_CONFIG_CARDS) {
        return WIDGET_MODEL_CONFIG_TOO_LARGE;
    }
    for (size_t i = 0U; i < config->card_count; ++i) {
        if (config->cards[i].card_id[0] == '\0') {
            return WIDGET_MODEL_CONFIG_INVALID_VALUE;
        }
    }
    if (duplicate_card_ids(config)) {
        return WIDGET_MODEL_CONFIG_DUPLICATE_ID;
    }
    return WIDGET_MODEL_CONFIG_APPLIED;
}

widget_model_config_result_t widget_model_apply_config(
    widget_model_t *model,
    const protocol_apply_config_t *config)
{
    widget_model_config_result_t result = widget_model_check_config(model, config);
    if (result != WIDGET_MODEL_CONFIG_APPLIED) {
        return result;
    }

    /* Stage into the buffer that is not live, then publish with an index swap,
     * so a reader never observes a half-written config. */
    uint8_t staging_index = model->configured ? (uint8_t)(1U - model->live_config_index) : 0U;
    protocol_apply_config_t *staging = &model->configs[staging_index];
    memcpy(staging, config, sizeof(*staging));

    /* Keep showing the same card across a config replace when it survives,
     * so an unrelated edit does not jump the loop. */
    size_t next_active = 0U;
    const protocol_card_config_t *active = widget_model_active_card(model);
    if (active != NULL) {
        for (size_t i = 0U; i < staging->card_count; ++i) {
            if (strcmp(staging->cards[i].card_id, active->card_id) == 0) {
                next_active = i;
                break;
            }
        }
    }

    model->live_config_index = staging_index;
    model->configured = true;
    model->active_card_index = next_active;

    /* A timer whose card left the configuration has nothing to resolve for. */
    if (model->timer.present) {
        bool still_present = false;
        for (size_t i = 0U; i < staging->card_count; ++i) {
            if (strcmp(staging->cards[i].card_id, model->timer.card_id) == 0) {
                still_present = true;
                break;
            }
        }
        if (!still_present) {
            memset(&model->timer, 0, sizeof(model->timer));
        }
    }
    return WIDGET_MODEL_CONFIG_APPLIED;
}

const protocol_apply_config_t *widget_model_config(const widget_model_t *model)
{
    return model == NULL ? NULL : live_config(model);
}

uint32_t widget_model_config_revision(const widget_model_t *model)
{
    const protocol_apply_config_t *config = widget_model_config(model);
    return config == NULL ? 0U : config->revision;
}

const protocol_card_config_t *widget_model_active_card(const widget_model_t *model)
{
    const protocol_apply_config_t *config = widget_model_config(model);
    if (config == NULL || model->active_card_index >= config->card_count) {
        return NULL;
    }
    return &config->cards[model->active_card_index];
}

bool widget_model_activate_card(widget_model_t *model, const char *card_id)
{
    const protocol_apply_config_t *config = widget_model_config(model);
    if (config == NULL || card_id == NULL) {
        return false;
    }
    for (size_t i = 0U; i < config->card_count; ++i) {
        if (strcmp(config->cards[i].card_id, card_id) == 0) {
            model->active_card_index = i;
            return true;
        }
    }
    return false;
}

widget_model_push_result_t widget_model_apply_timer(
    widget_model_t *model,
    const protocol_push_timer_t *push)
{
    if (model == NULL || push == NULL) {
        return WIDGET_MODEL_PUSH_INVALID_ARGUMENT;
    }
    const protocol_apply_config_t *config = widget_model_config(model);
    if (config == NULL) {
        return WIDGET_MODEL_PUSH_UNKNOWN_WIDGET;
    }
    if (push->revision == 0U) {
        return WIDGET_MODEL_PUSH_INVALID_ARGUMENT;
    }
    if (push->revision < model->latest_data_revision) {
        return WIDGET_MODEL_PUSH_STALE_REVISION;
    }
    bool known = false;
    for (size_t i = 0U; i < config->card_count; ++i) {
        if (strcmp(config->cards[i].card_id, push->card_id) == 0) {
            known = true;
            break;
        }
    }
    if (!known) {
        return WIDGET_MODEL_PUSH_UNKNOWN_WIDGET;
    }

    memset(&model->timer, 0, sizeof(model->timer));
    memcpy(model->timer.card_id, push->card_id, sizeof(model->timer.card_id));
    model->timer.card_id[sizeof(model->timer.card_id) - 1U] = '\0';
    model->timer.present = true;
    model->timer.total_ms = push->total_ms;
    model->timer.remaining_ms = push->remaining_ms;
    model->timer.running = push->running;
    model->latest_data_revision = push->revision;
    return WIDGET_MODEL_PUSH_ACCEPTED;
}

uint32_t widget_model_latest_data_revision(const widget_model_t *model)
{
    return model == NULL ? 0U : model->latest_data_revision;
}

bool widget_model_has_running_progress(const widget_model_t *model)
{
    return model != NULL && model->timer.present && model->timer.running;
}

const widget_model_timer_t *widget_model_timer(const widget_model_t *model,
                                               const char *card_id)
{
    if (model == NULL || card_id == NULL || !model->timer.present) {
        return NULL;
    }
    return strcmp(model->timer.card_id, card_id) == 0 ? &model->timer : NULL;
}
