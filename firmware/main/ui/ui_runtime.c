#include "ui_runtime.h"

#include <stdatomic.h>
#include <string.h>

#include "clock_screen.h"
#include "carousel.h"
#include "esp_check.h"
#include "esp_log.h"
#include "esp_lvgl_port.h"
#include "lvgl.h"
#include "template_view.h"
#include "ui_command_queue.h"

#define UI_COMMAND_POLL_MS 20U

static const char *TAG = "ui_runtime";
static ui_command_queue_t s_queue;
static ui_command_t s_publish_command;
static ui_command_t s_consume_command;
static atomic_flag s_publish_lock = ATOMIC_FLAG_INIT;
static lv_timer_t *s_command_timer;
static bool s_initialized;
static char s_active_widget_id[PROTOCOL_MAX_WIDGET_ID_LENGTH + 1U];

_Static_assert(sizeof(s_queue) + sizeof(s_publish_command) +
                   sizeof(s_consume_command) <= 16U * 1024U,
               "fixed UI mailbox exceeded its M2 RAM budget");

static void lock_publisher(void)
{
    while (atomic_flag_test_and_set_explicit(&s_publish_lock,
                                              memory_order_acquire)) {
    }
}

static void unlock_publisher(void)
{
    atomic_flag_clear_explicit(&s_publish_lock, memory_order_release);
}

static bool copy_widget_id(char *destination, const char *widget_id)
{
    if (widget_id == NULL) {
        return false;
    }
    const char *end = memchr(widget_id, '\0',
                             PROTOCOL_MAX_WIDGET_ID_LENGTH + 1U);
    if (end == NULL || end == widget_id) {
        return false;
    }
    size_t length = (size_t)(end - widget_id);
    memcpy(destination, widget_id, length + 1U);
    return true;
}

static bool publish_scalar(ui_command_type_t type)
{
    bool accepted = ui_command_queue_push(&s_queue, &s_publish_command);
    if (!accepted) {
        ESP_LOGW(TAG, "UI queue full; dropped command type=%u",
                 (unsigned)type);
    }
    return accepted;
}

static void consume_command(const ui_command_t *command)
{
    switch (command->type) {
    case UI_COMMAND_SHOW_STANDALONE:
        template_view_deactivate();
        carousel_unbind();
        clock_screen_show_in_lvgl();
        s_active_widget_id[0] = '\0';
        break;
    case UI_COMMAND_SHOW_VIEW:
        if (template_view_show(command->template_kind, command->size_class,
                               &command->fields)) {
            memcpy(s_active_widget_id, command->widget_id,
                   sizeof(s_active_widget_id));
            carousel_binding_t binding = {
                .tap_action = command->view_context.tap_action,
                .interrupt = command->view_context.interrupt,
                .interrupt_token = command->view_context.interrupt_token,
            };
            memcpy(binding.screen_id, command->view_context.screen_id,
                   sizeof(binding.screen_id));
            memcpy(binding.widget_id, command->widget_id,
                   sizeof(binding.widget_id));
            memcpy(binding.previous_screen_id,
                   command->view_context.previous_screen_id,
                   sizeof(binding.previous_screen_id));
            memcpy(binding.previous_widget_id,
                   command->view_context.previous_widget_id,
                   sizeof(binding.previous_widget_id));
            memcpy(binding.next_screen_id,
                   command->view_context.next_screen_id,
                   sizeof(binding.next_screen_id));
            memcpy(binding.next_widget_id,
                   command->view_context.next_widget_id,
                   sizeof(binding.next_widget_id));
            (void)carousel_bind(template_view_screen(), &binding);
        } else {
            ESP_LOGE(TAG, "failed to create template=%u size=%u",
                     (unsigned)command->template_kind,
                     (unsigned)command->size_class);
        }
        break;
    case UI_COMMAND_PATCH_VIEW:
        if (strcmp(s_active_widget_id, command->widget_id) == 0 &&
            !template_view_patch(command->template_kind, &command->fields,
                                 command->dirty_mask)) {
            ESP_LOGW(TAG, "rejected UI patch for widget=%s",
                     command->widget_id);
        }
        break;
    case UI_COMMAND_LINK_STATE: {
        bool template_was_active = template_view_active();
        clock_screen_set_online(command->online);
        template_view_set_online(command->online);
        if (!command->online && template_was_active) {
            template_view_deactivate();
            carousel_unbind();
            clock_screen_show_in_lvgl();
            s_active_widget_id[0] = '\0';
        }
        break;
    }
    case UI_COMMAND_TIME_OFFSET:
        clock_screen_set_utc_offset_minutes(command->utc_offset_minutes);
        template_view_set_utc_offset_minutes(command->utc_offset_minutes);
        break;
    case UI_COMMAND_INTERRUPTS:
        template_view_set_interrupts(command->interrupt_active,
                                     command->interrupt_pending);
        break;
    default:
        break;
    }
}

void ui_runtime_set_event_queue(device_event_queue_t *event_queue)
{
    carousel_init(event_queue);
}

static void command_timer_cb(lv_timer_t *timer)
{
    (void)timer;
    while (ui_command_queue_pop(&s_queue, &s_consume_command)) {
        consume_command(&s_consume_command);
    }
}

esp_err_t ui_runtime_init(void)
{
    ESP_RETURN_ON_FALSE(!s_initialized, ESP_ERR_INVALID_STATE, TAG,
                        "UI runtime already initialized");
    ui_command_queue_init(&s_queue);
    atomic_flag_clear(&s_publish_lock);
    memset(&s_publish_command, 0, sizeof(s_publish_command));
    memset(&s_consume_command, 0, sizeof(s_consume_command));
    s_active_widget_id[0] = '\0';

    lvgl_port_lock(0);
    s_command_timer = lv_timer_create(command_timer_cb,
                                      UI_COMMAND_POLL_MS, NULL);
    lvgl_port_unlock();
    ESP_RETURN_ON_FALSE(s_command_timer != NULL, ESP_ERR_NO_MEM, TAG,
                        "create UI command timer");
    s_initialized = true;
    ESP_LOGI(TAG, "fixed UI queue ready: %u slots, %u bytes",
             (unsigned)UI_COMMAND_QUEUE_CAPACITY,
             (unsigned)sizeof(s_queue));
    return ESP_OK;
}

bool ui_runtime_show_view(const char *widget_id,
                          protocol_template_kind_t template_kind,
                          protocol_size_class_t size_class,
                          const template_field_state_t *fields,
                          const ui_view_context_t *view_context)
{
    if (!s_initialized || fields == NULL || view_context == NULL) {
        return false;
    }
    lock_publisher();
    memset(&s_publish_command, 0, sizeof(s_publish_command));
    s_publish_command.type = UI_COMMAND_SHOW_VIEW;
    bool valid = copy_widget_id(s_publish_command.widget_id, widget_id);
    if (valid) {
        s_publish_command.template_kind = template_kind;
        s_publish_command.size_class = size_class;
        s_publish_command.fields = *fields;
        s_publish_command.dirty_mask = UINT16_MAX;
        s_publish_command.view_context = *view_context;
    }
    bool accepted = valid && publish_scalar(UI_COMMAND_SHOW_VIEW);
    unlock_publisher();
    return accepted;
}

bool ui_runtime_patch_view(const char *widget_id,
                           protocol_template_kind_t template_kind,
                           const template_field_state_t *fields,
                           uint16_t dirty_mask)
{
    if (!s_initialized || fields == NULL) {
        return false;
    }
    lock_publisher();
    memset(&s_publish_command, 0, sizeof(s_publish_command));
    s_publish_command.type = UI_COMMAND_PATCH_VIEW;
    bool valid = copy_widget_id(s_publish_command.widget_id, widget_id);
    if (valid) {
        s_publish_command.template_kind = template_kind;
        s_publish_command.fields = *fields;
        s_publish_command.dirty_mask = dirty_mask;
    }
    bool accepted = valid && publish_scalar(UI_COMMAND_PATCH_VIEW);
    unlock_publisher();
    return accepted;
}

bool ui_runtime_show_standalone(void)
{
    if (!s_initialized) {
        return false;
    }
    lock_publisher();
    memset(&s_publish_command, 0, sizeof(s_publish_command));
    s_publish_command.type = UI_COMMAND_SHOW_STANDALONE;
    bool accepted = publish_scalar(UI_COMMAND_SHOW_STANDALONE);
    unlock_publisher();
    return accepted;
}

bool ui_runtime_set_online(bool online)
{
    if (!s_initialized) {
        return false;
    }
    lock_publisher();
    memset(&s_publish_command, 0, sizeof(s_publish_command));
    s_publish_command.type = UI_COMMAND_LINK_STATE;
    s_publish_command.online = online;
    bool accepted = publish_scalar(UI_COMMAND_LINK_STATE);
    unlock_publisher();
    return accepted;
}

bool ui_runtime_set_utc_offset_minutes(int16_t offset_minutes)
{
    if (!s_initialized) {
        return false;
    }
    lock_publisher();
    memset(&s_publish_command, 0, sizeof(s_publish_command));
    s_publish_command.type = UI_COMMAND_TIME_OFFSET;
    s_publish_command.utc_offset_minutes = offset_minutes;
    bool accepted = publish_scalar(UI_COMMAND_TIME_OFFSET);
    unlock_publisher();
    return accepted;
}

bool ui_runtime_set_interrupts(bool active, bool pending)
{
    if (!s_initialized) {
        return false;
    }
    lock_publisher();
    memset(&s_publish_command, 0, sizeof(s_publish_command));
    s_publish_command.type = UI_COMMAND_INTERRUPTS;
    s_publish_command.interrupt_active = active;
    s_publish_command.interrupt_pending = pending;
    bool accepted = publish_scalar(UI_COMMAND_INTERRUPTS);
    unlock_publisher();
    return accepted;
}

uint32_t ui_runtime_dropped_commands(void)
{
    return ui_command_queue_dropped(&s_queue);
}

uint32_t ui_runtime_coalesced_commands(void)
{
    return ui_command_queue_coalesced(&s_queue);
}

uint32_t ui_runtime_queue_high_water(void)
{
    return ui_command_queue_high_water(&s_queue);
}
