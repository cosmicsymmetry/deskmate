#include "protocol_task.h"

#include <limits.h>
#include <stdbool.h>
#include <stdint.h>
#include <string.h>
#include <sys/time.h>

#include "board/board.h"
#include "board/display.h"
#include "core/device_event_queue.h"
#include "core/interrupt_state.h"
#include "core/link_state.h"
#include "core/protocol_frame.h"
#include "core/protocol_message.h"
#include "core/widget_model.h"
#include "esp_app_desc.h"
#include "esp_check.h"
#include "esp_log.h"
#include "esp_system.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "link/usb_link.h"
#include "ui/ui_runtime.h"

#define PROTOCOL_TASK_STACK_SIZE 8192U
#define PROTOCOL_TASK_PRIORITY 5U
#define PROTOCOL_TASK_CORE 1
#define PROTOCOL_READ_CHUNK_SIZE 512U
#define PROTOCOL_READ_TIMEOUT_MS 50U
#define PROTOCOL_WRITE_TIMEOUT_MS 200U
#define PROTOCOL_EVENT_WRITE_TIMEOUT_MS 10U
#define PROTOCOL_EVENTS_PER_POLL 2U

typedef struct {
    protocol_decoder_t decoder;
    link_state_t link;
    widget_model_t model;
    interrupt_state_t interrupts;
    device_event_queue_t events;
    protocol_message_t message;
    protocol_device_event_t event;
    ui_view_context_t view_context;
    uint8_t wire[PROTOCOL_MAX_WIRE_FRAME];
    uint32_t valid_frames;
    uint32_t malformed_frames;
    uint32_t crc_errors;
    uint32_t overflow_frames;
    uint32_t dropped_responses;
    uint32_t dropped_events;
} protocol_context_t;

static const char *TAG = "protocol";
static protocol_context_t s_context;
static TaskHandle_t s_task;

static uint64_t uptime_ms(void)
{
    return (uint64_t)esp_timer_get_time() / UINT64_C(1000);
}

static void increment_saturating(uint32_t *counter)
{
    if (*counter != UINT32_MAX) {
        ++*counter;
    }
}

static void copy_diagnostic(protocol_error_response_t *error,
                            protocol_error_code_t code,
                            const char *diagnostic)
{
    error->code = code;
    size_t length = strlen(diagnostic);
    if (length > PROTOCOL_MAX_DIAGNOSTIC_LENGTH) {
        length = PROTOCOL_MAX_DIAGNOSTIC_LENGTH;
    }
    memcpy(error->diagnostic, diagnostic, length);
    error->diagnostic[length] = '\0';
}

static uint32_t saturating_add(uint32_t left, uint32_t right)
{
    return right > UINT32_MAX - left ? UINT32_MAX : left + right;
}

static const protocol_widget_config_t *find_widget(
    const protocol_apply_config_t *config,
    const char *widget_id)
{
    if (config == NULL || widget_id == NULL) {
        return NULL;
    }
    for (size_t i = 0U; i < config->widget_count; ++i) {
        if (strcmp(config->widgets[i].widget_id, widget_id) == 0) {
            return &config->widgets[i];
        }
    }
    return NULL;
}

static size_t active_screen_index(const widget_model_t *model)
{
    const protocol_apply_config_t *config = widget_model_config(model);
    const protocol_screen_config_t *screen = widget_model_active_screen(model);
    if (config == NULL || screen == NULL) {
        return 0U;
    }
    return (size_t)(screen - config->screens);
}

static bool show_carousel_screen(protocol_context_t *context)
{
    const protocol_apply_config_t *config = widget_model_config(&context->model);
    const protocol_screen_config_t *screen =
        widget_model_active_screen(&context->model);
    if (config == NULL || screen == NULL || config->screen_count == 0U) {
        return false;
    }
    const protocol_widget_config_t *widget = find_widget(config,
                                                          screen->widget_id);
    const template_field_state_t *fields = widget_model_widget_fields(
        &context->model, screen->widget_id);
    if (widget == NULL || fields == NULL) {
        return false;
    }
    size_t index = active_screen_index(&context->model);
    size_t previous = index == 0U ? config->screen_count - 1U : index - 1U;
    size_t next = (index + 1U) % config->screen_count;
    ui_view_context_t *view = &context->view_context;
    memset(view, 0, sizeof(*view));
    strcpy(view->screen_id, screen->screen_id);
    strcpy(view->previous_screen_id, config->screens[previous].screen_id);
    strcpy(view->previous_widget_id, config->screens[previous].widget_id);
    strcpy(view->next_screen_id, config->screens[next].screen_id);
    strcpy(view->next_widget_id, config->screens[next].widget_id);
    view->tap_action = widget->tap_action;
    return ui_runtime_show_view(widget->widget_id, widget->template_kind,
                                widget->size_class, fields, view);
}

static bool show_active_interrupt(protocol_context_t *context)
{
    const interrupt_slot_t *active = interrupt_state_active(
        &context->interrupts);
    const protocol_apply_config_t *config = widget_model_config(&context->model);
    if (active == NULL || config == NULL) {
        return false;
    }
    const protocol_widget_config_t *widget = find_widget(config,
                                                          active->widget_id);
    const template_field_state_t *fields = widget_model_widget_fields(
        &context->model, active->widget_id);
    if (widget == NULL || fields == NULL) {
        return false;
    }
    ui_view_context_t *view = &context->view_context;
    memset(view, 0, sizeof(*view));
    strcpy(view->screen_id, context->interrupts.saved_screen_id);
    view->interrupt = true;
    view->interrupt_token = active->token;
    return ui_runtime_show_view(widget->widget_id, widget->template_kind,
                                PROTOCOL_SIZE_FULL, fields, view);
}

static bool show_current_content(protocol_context_t *context)
{
    return interrupt_state_active(&context->interrupts) != NULL
               ? show_active_interrupt(context)
               : show_carousel_screen(context);
}

static void transmit(protocol_context_t *context,
                     uint32_t request_id,
                     const protocol_message_t *message)
{
    size_t wire_length = 0U;
    protocol_message_result_t result = protocol_message_encode(
        request_id, message, context->wire, sizeof(context->wire),
        &wire_length);
    if (result != PROTOCOL_MESSAGE_OK ||
        usb_link_write_frame(context->wire, wire_length,
                             pdMS_TO_TICKS(PROTOCOL_WRITE_TIMEOUT_MS)) !=
            ESP_OK) {
        increment_saturating(&context->dropped_responses);
    }
}

static void transmit_error(protocol_context_t *context,
                           uint32_t request_id,
                           protocol_error_code_t code,
                           const char *diagnostic)
{
    // Dispatch is single-threaded and no request fields are needed after an
    // error decision, so reuse the fixed decoded-message storage. Keeping a
    // second protocol_message_t on this 8 KiB task stack would cost several
    // KiB because its union includes the maximum PushData payload.
    protocol_message_t *reply = &context->message;
    memset(reply, 0, sizeof(*reply));
    reply->type = PROTOCOL_TYPE_ERROR;
    copy_diagnostic(&reply->value.error, code, diagnostic);
    transmit(context, request_id, reply);
}

static void transmit_status(protocol_context_t *context, uint32_t request_id)
{
    protocol_message_t *reply = &context->message;
    memset(reply, 0, sizeof(*reply));
    reply->type = PROTOCOL_TYPE_STATUS_RESPONSE;
    protocol_status_response_t *status = &reply->value.status;
    status->protocol_version = PROTOCOL_VERSION;
    const char *version = esp_app_get_description()->version;
    size_t version_length = strlen(version);
    if (version_length > PROTOCOL_MAX_FIRMWARE_VERSION_LENGTH) {
        version_length = PROTOCOL_MAX_FIRMWARE_VERSION_LENGTH;
    }
    memcpy(status->firmware_version, version, version_length);
    status->firmware_version[version_length] = '\0';
    if (version_length == 0U) {
        memcpy(status->firmware_version, "unknown", sizeof("unknown"));
    }
    status->uptime_ms = uptime_ms();
    status->free_heap = esp_get_free_heap_size();
    status->rotation = board_display_rotation_degrees();
    bool landscape = status->rotation == 90U || status->rotation == 270U;
    status->display_width = landscape ? BOARD_LCD_V_RES : BOARD_LCD_H_RES;
    status->display_height = landscape ? BOARD_LCD_H_RES : BOARD_LCD_V_RES;
    status->brightness = board_display_brightness();
    status->online = context->link.online;
    status->latest_revision = context->link.latest_revision;
    status->valid_frames = context->valid_frames;
    status->malformed_frames = context->malformed_frames;
    status->crc_errors = context->crc_errors;
    status->overflow_frames = context->overflow_frames;
    status->dropped_responses = context->dropped_responses;
    status->rx_dropped_bytes = usb_link_rx_dropped_bytes();
    status->dropped_events = saturating_add(
        context->dropped_events,
        device_event_queue_dropped(&context->events));
    status->event_queue_high_water =
        device_event_queue_high_water(&context->events);
    status->dropped_ui_commands = ui_runtime_dropped_commands();
    status->ui_queue_high_water = ui_runtime_queue_high_water();
    status->config_revision = widget_model_config_revision(&context->model);
    transmit(context, request_id, reply);
}

static void transmit_ack(protocol_context_t *context,
                         uint32_t request_id,
                         uint8_t acknowledged_type,
                         bool has_revision,
                         uint32_t revision)
{
    protocol_message_t *reply = &context->message;
    memset(reply, 0, sizeof(*reply));
    reply->type = PROTOCOL_TYPE_ACK;
    reply->value.ack.acknowledged_type = acknowledged_type;
    reply->value.ack.has_revision = has_revision;
    reply->value.ack.revision = revision;
    transmit(context, request_id, reply);
}

static void dispatch_time_sync(protocol_context_t *context,
                               uint32_t request_id)
{
    const protocol_time_sync_t *sync = &context->message.value.time_sync;
    if (sync->unix_seconds < PROTOCOL_MIN_UNIX_SECONDS ||
        sync->unix_seconds > PROTOCOL_MAX_UNIX_SECONDS ||
        sync->utc_offset_minutes < PROTOCOL_MIN_UTC_OFFSET_MINUTES ||
        sync->utc_offset_minutes > PROTOCOL_MAX_UTC_OFFSET_MINUTES) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INVALID_TIME,
                       "invalid time");
        return;
    }
    struct timeval time_value = {
        .tv_sec = (time_t)sync->unix_seconds,
        .tv_usec = 0,
    };
    if (settimeofday(&time_value, NULL) != 0) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INTERNAL,
                       "settimeofday failed");
        return;
    }
    if (!link_state_apply_time_sync(&context->link, sync)) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INTERNAL,
                       "time state failed");
        return;
    }
    ui_runtime_set_utc_offset_minutes(sync->utc_offset_minutes);
    transmit_ack(context, request_id, PROTOCOL_TYPE_TIME_SYNC, false, 0U);
}

static void dispatch_push_data(protocol_context_t *context,
                               uint32_t request_id)
{
    const protocol_push_data_t *push = &context->message.value.push_data;
    widget_model_update_t update;
    widget_model_push_result_t result = widget_model_apply_push(
        &context->model, push, &update);
    if (result == WIDGET_MODEL_PUSH_STALE_REVISION) {
        transmit_error(context, request_id, PROTOCOL_ERROR_STALE_REVISION,
                       "stale revision");
        return;
    }
    if (result == WIDGET_MODEL_PUSH_UNKNOWN_WIDGET) {
        transmit_error(context, request_id, PROTOCOL_ERROR_UNKNOWN_WIDGET,
                       "unknown widget");
        return;
    }
    if (result != WIDGET_MODEL_PUSH_ACCEPTED) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INVALID_PAYLOAD,
                       "invalid push data");
        return;
    }
    if (link_state_accept_push(&context->link, push) !=
        LINK_STATE_PUSH_ACCEPTED) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INTERNAL,
                       "revision state mismatch");
        return;
    }

    const interrupt_slot_t *interrupt = interrupt_state_active(
        &context->interrupts);
    const protocol_screen_config_t *screen = widget_model_active_screen(
        &context->model);
    bool visible = (interrupt != NULL &&
                    strcmp(interrupt->widget_id, push->widget_id) == 0) ||
                   (interrupt == NULL && screen != NULL &&
                    strcmp(screen->widget_id, push->widget_id) == 0);
    if (visible) {
        const protocol_apply_config_t *config = widget_model_config(
            &context->model);
        const protocol_widget_config_t *widget = find_widget(config,
                                                              push->widget_id);
        const template_field_state_t *fields = widget_model_widget_fields(
            &context->model, push->widget_id);
        if (widget != NULL && fields != NULL) {
            (void)ui_runtime_patch_view(widget->widget_id,
                                        widget->template_kind, fields,
                                        update.dirty_mask);
        }
    }
    transmit_ack(context, request_id, PROTOCOL_TYPE_PUSH_DATA, true,
                 context->link.latest_revision);
}

static protocol_error_code_t config_error_code(
    widget_model_config_result_t result)
{
    if (result == WIDGET_MODEL_CONFIG_STALE_REVISION) {
        return PROTOCOL_ERROR_STALE_REVISION;
    }
    if (result == WIDGET_MODEL_CONFIG_TOO_LARGE) {
        return PROTOCOL_ERROR_CONFIG_TOO_LARGE;
    }
    if (result == WIDGET_MODEL_CONFIG_UNKNOWN_WIDGET) {
        return PROTOCOL_ERROR_UNKNOWN_WIDGET;
    }
    if (result == WIDGET_MODEL_CONFIG_UNSUPPORTED_TEMPLATE) {
        return PROTOCOL_ERROR_UNSUPPORTED_TEMPLATE;
    }
    if (result == WIDGET_MODEL_CONFIG_UNSUPPORTED_SIZE_CLASS) {
        return PROTOCOL_ERROR_UNSUPPORTED_SIZE_CLASS;
    }
    return PROTOCOL_ERROR_INVALID_PAYLOAD;
}

static void dispatch_apply_config(protocol_context_t *context,
                                  uint32_t request_id)
{
    widget_model_config_result_t result = widget_model_apply_config(
        &context->model, &context->message.value.apply_config);
    if (result != WIDGET_MODEL_CONFIG_APPLIED &&
        result != WIDGET_MODEL_CONFIG_REPLAYED) {
        transmit_error(context, request_id, config_error_code(result),
                       "configuration rejected");
        return;
    }
    if (result == WIDGET_MODEL_CONFIG_REPLAYED) {
        transmit_ack(context, request_id, PROTOCOL_TYPE_APPLY_CONFIG, true,
                     widget_model_config_revision(&context->model));
        return;
    }
    interrupt_state_clear(&context->interrupts);
    (void)ui_runtime_set_interrupts(false, false);
    if (!show_carousel_screen(context)) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INTERNAL,
                       "UI command rejected");
        return;
    }
    transmit_ack(context, request_id, PROTOCOL_TYPE_APPLY_CONFIG, true,
                 widget_model_config_revision(&context->model));
}

static void dispatch_activate_screen(protocol_context_t *context,
                                     uint32_t request_id)
{
    const char *screen_id = context->message.value.activate_screen.screen_id;
    if (!widget_model_activate_screen(&context->model, screen_id)) {
        transmit_error(context, request_id, PROTOCOL_ERROR_UNKNOWN_SCREEN,
                       "unknown screen");
        return;
    }
    if (interrupt_state_active(&context->interrupts) != NULL) {
        (void)interrupt_state_set_saved_screen(&context->interrupts,
                                               screen_id);
    } else if (!show_carousel_screen(context)) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INTERNAL,
                       "UI command rejected");
        return;
    }
    transmit_ack(context, request_id, PROTOCOL_TYPE_ACTIVATE_SCREEN,
                 false, 0U);
}

static void dispatch_trigger_interrupt(protocol_context_t *context,
                                       uint32_t request_id)
{
    const protocol_trigger_interrupt_t *trigger =
        &context->message.value.trigger_interrupt;
    const protocol_apply_config_t *config = widget_model_config(&context->model);
    const protocol_widget_config_t *widget = find_widget(config,
                                                          trigger->widget_id);
    if (widget == NULL) {
        transmit_error(context, request_id, PROTOCOL_ERROR_UNKNOWN_WIDGET,
                       "unknown widget");
        return;
    }
    if (widget->interrupt_policy != PROTOCOL_INTERRUPT_ENABLED) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INVALID_PAYLOAD,
                       "interrupt disabled");
        return;
    }
    const protocol_screen_config_t *screen = widget_model_active_screen(
        &context->model);
    if (screen == NULL) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INTERNAL,
                       "no active carousel screen");
        return;
    }
    interrupt_trigger_result_t result = interrupt_state_trigger(
        &context->interrupts, trigger, screen->screen_id);
    if (result == INTERRUPT_TRIGGER_STALE_TOKEN) {
        transmit_error(context, request_id, PROTOCOL_ERROR_STALE_REVISION,
                       "stale interrupt token");
        return;
    }
    if (result == INTERRUPT_TRIGGER_BUSY) {
        transmit_error(context, request_id, PROTOCOL_ERROR_BUSY,
                       "interrupt queue full");
        return;
    }
    if (result == INTERRUPT_TRIGGER_INVALID_ARGUMENT) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INVALID_PAYLOAD,
                       "invalid interrupt");
        return;
    }
    if (result == INTERRUPT_TRIGGER_ACTIVATED &&
        !show_active_interrupt(context)) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INTERNAL,
                       "UI command rejected");
        return;
    }
    (void)ui_runtime_set_interrupts(true,
        interrupt_state_pending(&context->interrupts) != NULL);
    transmit_ack(context, request_id, PROTOCOL_TYPE_TRIGGER_INTERRUPT,
                 false, 0U);
}

static void dispatch_request(protocol_context_t *context,
                             const protocol_frame_t *frame)
{
    protocol_message_result_t result = protocol_message_decode(
        frame, &context->message);
    if (result == PROTOCOL_MESSAGE_ERR_VERSION) {
        transmit_error(context, frame->request_id,
                       PROTOCOL_ERROR_VERSION_MISMATCH,
                       "unsupported protocol version");
        return;
    }
    if (result == PROTOCOL_MESSAGE_ERR_UNSUPPORTED_TYPE) {
        transmit_error(context, frame->request_id,
                       PROTOCOL_ERROR_UNSUPPORTED_MESSAGE,
                       "unsupported message type");
        return;
    }
    if (result == PROTOCOL_MESSAGE_ERR_INVALID_TIME) {
        increment_saturating(&context->malformed_frames);
        transmit_error(context, frame->request_id,
                       PROTOCOL_ERROR_INVALID_TIME,
                       "invalid time");
        return;
    }
    if (result != PROTOCOL_MESSAGE_OK) {
        increment_saturating(&context->malformed_frames);
        transmit_error(context, frame->request_id,
                       PROTOCOL_ERROR_INVALID_PAYLOAD,
                       "invalid CBOR payload");
        return;
    }
    if (context->message.type != PROTOCOL_TYPE_STATUS_REQUEST &&
        context->message.type != PROTOCOL_TYPE_TIME_SYNC &&
        context->message.type != PROTOCOL_TYPE_PUSH_DATA &&
        context->message.type != PROTOCOL_TYPE_HEARTBEAT &&
        context->message.type != PROTOCOL_TYPE_APPLY_CONFIG &&
        context->message.type != PROTOCOL_TYPE_ACTIVATE_SCREEN &&
        context->message.type != PROTOCOL_TYPE_TRIGGER_INTERRUPT) {
        transmit_error(context, frame->request_id,
                       PROTOCOL_ERROR_UNSUPPORTED_MESSAGE,
                       "response type sent as request");
        return;
    }

    if (link_state_note_valid_request(&context->link, uptime_ms())) {
        ui_runtime_set_online(true);
        if (widget_model_config(&context->model) != NULL) {
            (void)show_current_content(context);
        }
        ESP_LOGI(TAG, "link online");
    }
    switch (context->message.type) {
    case PROTOCOL_TYPE_STATUS_REQUEST:
        transmit_status(context, frame->request_id);
        break;
    case PROTOCOL_TYPE_TIME_SYNC:
        dispatch_time_sync(context, frame->request_id);
        break;
    case PROTOCOL_TYPE_PUSH_DATA:
        dispatch_push_data(context, frame->request_id);
        break;
    case PROTOCOL_TYPE_APPLY_CONFIG:
        dispatch_apply_config(context, frame->request_id);
        break;
    case PROTOCOL_TYPE_ACTIVATE_SCREEN:
        dispatch_activate_screen(context, frame->request_id);
        break;
    case PROTOCOL_TYPE_TRIGGER_INTERRUPT:
        dispatch_trigger_interrupt(context, frame->request_id);
        break;
    case PROTOCOL_TYPE_HEARTBEAT: {
        protocol_message_t *reply = &context->message;
        memset(reply, 0, sizeof(*reply));
        reply->type = PROTOCOL_TYPE_HEARTBEAT_ACK;
        reply->value.heartbeat_ack.uptime_ms = uptime_ms();
        transmit(context, frame->request_id, reply);
        break;
    }
    default:
        break;
    }
}

static void frame_callback(protocol_frame_result_t result,
                           const protocol_frame_t *frame,
                           void *opaque)
{
    protocol_context_t *context = opaque;
    if (result == PROTOCOL_FRAME_ERR_CHECKSUM) {
        increment_saturating(&context->crc_errors);
        return;
    }
    if (result == PROTOCOL_FRAME_ERR_OVERLONG) {
        increment_saturating(&context->overflow_frames);
        return;
    }
    if (result != PROTOCOL_FRAME_OK || frame == NULL) {
        increment_saturating(&context->malformed_frames);
        return;
    }
    increment_saturating(&context->valid_frames);
    dispatch_request(context, frame);
}

static void transmit_device_event(protocol_context_t *context,
                                  const protocol_device_event_t *event)
{
    protocol_message_t *message = &context->message;
    memset(message, 0, sizeof(*message));
    message->type = PROTOCOL_TYPE_DEVICE_EVENT;
    message->value.device_event = *event;
    size_t wire_length = 0U;
    protocol_message_result_t result = protocol_message_encode(
        0U, message, context->wire, sizeof(context->wire), &wire_length);
    if (result != PROTOCOL_MESSAGE_OK ||
        usb_link_write_frame(context->wire, wire_length,
                             pdMS_TO_TICKS(PROTOCOL_EVENT_WRITE_TIMEOUT_MS)) !=
            ESP_OK) {
        increment_saturating(&context->dropped_events);
    }
}

static bool validate_tap_event(protocol_context_t *context,
                               const protocol_device_event_t *event)
{
    if (interrupt_state_active(&context->interrupts) != NULL) {
        return false;
    }
    const protocol_screen_config_t *screen = widget_model_active_screen(
        &context->model);
    const protocol_apply_config_t *config = widget_model_config(&context->model);
    const protocol_widget_config_t *widget = screen != NULL
        ? find_widget(config, screen->widget_id) : NULL;
    protocol_event_action_t expected = PROTOCOL_EVENT_ACTION_NONE;
    if (widget != NULL && widget->tap_action == PROTOCOL_TAP_START_PAUSE) {
        expected = PROTOCOL_EVENT_ACTION_START_PAUSE;
    } else if (widget != NULL && widget->tap_action == PROTOCOL_TAP_RESET) {
        expected = PROTOCOL_EVENT_ACTION_RESET;
    }
    return screen != NULL && widget != NULL && event->action == expected &&
           strcmp(event->screen_id, screen->screen_id) == 0 &&
           strcmp(event->widget_id, widget->widget_id) == 0;
}

static bool apply_navigation_event(protocol_context_t *context,
                                   protocol_device_event_t *event)
{
    if (interrupt_state_active(&context->interrupts) != NULL ||
        !widget_model_activate_screen(&context->model, event->screen_id)) {
        return false;
    }
    const protocol_screen_config_t *screen = widget_model_active_screen(
        &context->model);
    if (screen == NULL || strcmp(screen->widget_id, event->widget_id) != 0 ||
        !show_carousel_screen(context)) {
        return false;
    }
    strcpy(event->screen_id, screen->screen_id);
    strcpy(event->widget_id, screen->widget_id);
    return true;
}

static bool apply_dismissal_event(protocol_context_t *context,
                                  protocol_device_event_t *event)
{
    const interrupt_slot_t *active = interrupt_state_active(
        &context->interrupts);
    if (active == NULL || !event->has_interrupt_token ||
        event->interrupt_token != active->token ||
        strcmp(event->widget_id, active->widget_id) != 0) {
        return false;
    }
    interrupt_dismissal_t dismissal;
    if (!interrupt_state_dismiss(&context->interrupts, &dismissal)) {
        return false;
    }
    strcpy(event->screen_id, dismissal.saved_screen_id);
    bool shown = false;
    if (dismissal.promoted_pending) {
        shown = show_active_interrupt(context);
    } else if (dismissal.restore_saved_screen &&
               widget_model_activate_screen(&context->model,
                                             dismissal.saved_screen_id)) {
        shown = show_carousel_screen(context);
    }
    (void)ui_runtime_set_interrupts(
        interrupt_state_active(&context->interrupts) != NULL,
        interrupt_state_pending(&context->interrupts) != NULL);
    return shown;
}

static void process_device_events(protocol_context_t *context)
{
    for (size_t i = 0U; i < PROTOCOL_EVENTS_PER_POLL; ++i) {
        if (!device_event_queue_pop(&context->events, &context->event)) {
            return;
        }
        if (!context->link.online) {
            increment_saturating(&context->dropped_events);
            continue;
        }
        bool valid = false;
        if (context->event.kind == PROTOCOL_EVENT_TAP) {
            valid = validate_tap_event(context, &context->event);
        } else if (context->event.kind == PROTOCOL_EVENT_NAVIGATION) {
            valid = apply_navigation_event(context, &context->event);
        } else if (context->event.kind ==
                   PROTOCOL_EVENT_INTERRUPT_DISMISSED) {
            valid = apply_dismissal_event(context, &context->event);
        }
        if (valid) {
            transmit_device_event(context, &context->event);
        } else {
            increment_saturating(&context->dropped_events);
        }
    }
}

static void protocol_task(void *argument)
{
    protocol_context_t *context = argument;
    uint8_t chunk[PROTOCOL_READ_CHUNK_SIZE];
    for (;;) {
        size_t received = usb_link_read(
            chunk, sizeof(chunk), pdMS_TO_TICKS(PROTOCOL_READ_TIMEOUT_MS));
        if (received != 0U) {
            protocol_decoder_feed(&context->decoder, chunk, received,
                                  frame_callback, context);
        }
        process_device_events(context);
        if (link_state_poll(&context->link, uptime_ms())) {
            ui_runtime_set_online(false);
            ESP_LOGI(TAG, "link standalone after timeout");
        }
    }
}

esp_err_t protocol_task_start(void)
{
    ESP_RETURN_ON_FALSE(s_task == NULL, ESP_ERR_INVALID_STATE, TAG,
                        "protocol task already running");
    memset(&s_context, 0, sizeof(s_context));
    protocol_decoder_init(&s_context.decoder);
    link_state_init(&s_context.link);
    widget_model_init(&s_context.model);
    interrupt_state_init(&s_context.interrupts);
    device_event_queue_init(&s_context.events);
    ui_runtime_set_event_queue(&s_context.events);
    BaseType_t created = xTaskCreatePinnedToCore(
        protocol_task, "protocol", PROTOCOL_TASK_STACK_SIZE, &s_context,
        PROTOCOL_TASK_PRIORITY, &s_task, PROTOCOL_TASK_CORE);
    ESP_RETURN_ON_FALSE(created == pdPASS, ESP_ERR_NO_MEM, TAG,
                        "create protocol task");
    return ESP_OK;
}
