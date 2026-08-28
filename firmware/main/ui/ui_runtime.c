#include "ui_runtime.h"

#include <stdatomic.h>
#include <string.h>

#include "clock_screen.h"
#include "carousel.h"
#include "esp_check.h"
#include "esp_log.h"
#include "esp_lvgl_port.h"
#include "lvgl.h"
#include "ota_screen.h"
#include "scene_view.h"
#include "ui_command_queue.h"

#ifndef DESKMATE_LV_CONF_CANARY
#error "LVGL is not reading firmware/lv_conf.h — CONFIG_LV_CONF_SKIP must be n"
#endif

#define UI_COMMAND_POLL_MS 20U

static const char *TAG = "ui_runtime";
static ui_command_queue_t s_queue;
static ui_command_t s_publish_command;
static ui_command_t s_consume_command;
static atomic_flag s_publish_lock = ATOMIC_FLAG_INIT;
static lv_timer_t *s_command_timer;
static bool s_initialized;

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
    if (ota_screen_active_in_lvgl() &&
        (command->type == UI_COMMAND_SHOW_STANDALONE ||
         command->type == UI_COMMAND_SHOW_CARD_FALLBACK)) {
        /* The OTA task owns the panel until it reboots or restores the clock. */
        return;
    }
    switch (ui_command_action(command->type)) {
    case UI_COMMAND_ACTION_SHOW_STANDALONE_CLOCK:
        carousel_unbind();
        clock_screen_show_in_lvgl();
        /* Loading the clock deletes the scene's screen (auto_del), which is
         * what drops its hold on every asset font it acquired. This covers
         * the one case that does not: clock_screen_show_in_lvgl()
         * early-returns when the clock is already the active screen, which
         * would leave a built-but-unshown scene alive. */
        (void)scene_view_destroy();
        break;
    case UI_COMMAND_ACTION_APPLY_LINK_STATE: {
        /* A scene is host content, so losing the host restores the
         * standalone clock rather than leaving a frozen face up. */
        bool host_content_was_active = scene_view_active();
        clock_screen_set_online(command->online);
        if (!command->online && host_content_was_active) {
            carousel_unbind();
            clock_screen_show_in_lvgl();
            (void)scene_view_destroy();
        }
        break;
    }
    case UI_COMMAND_ACTION_APPLY_TIME_OFFSET:
        clock_screen_set_utc_offset_minutes(command->utc_offset_minutes);
        break;
    case UI_COMMAND_ACTION_NONE:
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

bool ui_runtime_is_initialized(void)
{
    return s_initialized;
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

bool ui_runtime_show_card_fallback(void)
{
    if (!s_initialized) {
        return false;
    }
    lock_publisher();
    memset(&s_publish_command, 0, sizeof(s_publish_command));
    s_publish_command.type = UI_COMMAND_SHOW_CARD_FALLBACK;
    bool accepted = publish_scalar(UI_COMMAND_SHOW_CARD_FALLBACK);
    unlock_publisher();
    return accepted;
}

bool ui_runtime_discard_card_fallbacks(void)
{
    if (!s_initialized) {
        return false;
    }
    return ui_command_queue_discard_card_fallbacks(&s_queue);
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
