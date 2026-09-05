#include "protocol_task.h"

#include <limits.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdatomic.h>
#include <stdio.h>
#include <string.h>
#include <sys/time.h>
#include <time.h>

#include "board/board.h"
#include "board/display.h"
#include "core/asset_store.h"
#include "core/asset_transfer.h"
#include "core/volatile_asset_store.h"
#include "core/device_event_queue.h"
#include "core/interrupt_state.h"
#include "core/link_state.h"
#include "core/net_config.h"
#include "core/ota_policy.h"
#include "core/protocol_frame.h"
#include "core/protocol_message.h"
#include "core/scene_binding.h"
#include "core/widget_model.h"
#include "esp_app_desc.h"
#include "esp_check.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_lvgl_port.h"
#include "esp_system.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "link/asset_flash.h"
#include "link/dev_capture.h"
#include "link/link_transport.h"
#include "link/net_link.h"
#include "link/net_store.h"
#include "link/ota.h"
#include "link/usb_link.h"
#include "link/wifi_station.h"
#include "mbedtls/sha256.h"
#include "ui/carousel.h"
#include "ui/clock_screen.h"
#include "ui/font_registry.h"
#include "ui/ota_screen.h"
#include "ui/scene_view.h"
#include "ui/ui_runtime.h"

_Static_assert(PROTOCOL_ASSET_ENCODING_RAW == VOLATILE_ASSET_ENCODING_RAW,
               "raw asset encoding mirrors diverged");
_Static_assert(PROTOCOL_ASSET_ENCODING_RLE565 ==
                   VOLATILE_ASSET_ENCODING_RLE565,
               "RLE565 asset encoding mirrors diverged");
_Static_assert(PROTOCOL_VOLATILE_IMAGE_DECODED_LENGTH ==
                   VOLATILE_ASSET_FRAME_BYTES,
               "volatile decoded frame sizes diverged");

#define PROTOCOL_TASK_STACK_SIZE 8192U
#define PROTOCOL_TASK_PRIORITY 5U
#define PROTOCOL_TASK_CORE 1
#define PROTOCOL_READ_CHUNK_SIZE 512U
#define PROTOCOL_READ_TIMEOUT_MS 50U
#define PROTOCOL_WRITE_TIMEOUT_MS 200U
#define PROTOCOL_EVENT_WRITE_TIMEOUT_MS 10U
#define PROTOCOL_EVENTS_PER_POLL 2U
// The scene's binding tick retains the former template renderer's 250 ms
// cadence. This task's read timeout is 50 ms, so the loop below sees this
// deadline with plenty of margin.
#define PROTOCOL_SCENE_TICK_MS 250U

typedef struct {
    protocol_decoder_t decoder;
    protocol_decoder_t restricted_usb_decoder;
    link_state_t link;
    widget_model_t model;
    interrupt_state_t interrupts;
    device_event_queue_t events;
    protocol_message_t message;
    protocol_device_event_t event;
    // Owned here, not a static in asset_transfer.c/asset_flash.c: this is
    // the only in-flight asset transfer the device tracks, and it lives for
    // exactly as long as the rest of this task's state (see the PSRAM
    // allocation note above protocol_context_t's static instance).
    asset_transfer_t asset_transfer;
    uint32_t asset_transfer_record_index;
    uint32_t asset_transfer_blob_offset;
    // True exactly when asset_transfer_record_index names a reservation that
    // is UNCOMMITTED on flash and not yet either committed or reclaimed.
    // Distinct from asset_transfer.active: a chunk-write failure or rejected
    // chunk aborts the in-RAM transfer (asset_transfer_abort resets
    // asset_transfer to inactive) while the on-flash reservation survives --
    // this flag is what still remembers it needs reclaiming. See
    // abandon_pending_reservation().
    bool asset_reservation_pending;
    // Metadata only. This whole context is allocated from PSRAM, and the two
    // possible 329,740-byte frame buffers come from the explicit PSRAM
    // callbacks installed at start. No volatile slot or byte buffer lands in
    // file-scope .bss.
    volatile_asset_store_t volatile_assets;
    // The scene currently on the panel, retained after scene_view_show()
    // consumed it. It is kept for exactly one caller: the asset-GC teardown
    // in dispatch_asset_release(), which must destroy the renderer's objects
    // (that is what releases their font faces) before compaction moves the
    // blobs those faces were rasterized from, and then put the scene back.
    // Without a retained copy there is nothing to put back -- the decoded
    // one lives in `message`, which every subsequent Ack or Error overwrites.
    // ~6 KB, and in PSRAM for the same reason protocol_message_t's copy is.
    scene_t scene;
    char scene_card_id[PROTOCOL_MAX_CARD_ID_LENGTH + 1U];
    // A HINT, never an authority. The thing that actually decides whether a
    // scene is on the panel -- and therefore whether anything is pinning
    // font faces or holding a mapped-blob pointer -- is the renderer's own
    // screen, scene_view_screen(), which only an LVGL-lock holder may read.
    // This flag exists purely so the 250 ms tick can skip taking that lock
    // when no scene has been shown, so it is only ever set true by an
    // observed successful show and only ever cleared by the renderer's own
    // answer, never by this task predicting a queued screen change that may
    // not land. Every decision that can damage something -- the asset-GC
    // teardown, the rebuild -- reads the authority instead.
    bool scene_live;
    uint64_t scene_tick_ms;
    // uptime at the last PushData for the scene's card, so a running timer
    // binding counts down between pushes instead of freezing.
    uint64_t scene_timer_anchor_ms;
    // One slot, for rendering an integer field into text; see
    // scene_field_lookup() for why one is enough.
    char scene_field_text[24];
    uint8_t wire[PROTOCOL_MAX_WIRE_FRAME];
    uint32_t valid_frames;
    uint32_t malformed_frames;
    uint32_t crc_errors;
    uint32_t overflow_frames;
    uint32_t dropped_responses;
    uint32_t dropped_events;
    const link_transport_t *response_transport;
} protocol_context_t;

static const char *TAG = "protocol";
// Allocated from PSRAM in protocol_task_start(), not a static internal-RAM
// object: protocol_context_t is 62,128 B -- it grew from 58,792 B when
// protocol_push_scene_t put a scene_t inside protocol_message_t's union
// (core/protocol_message.h), which is exactly the kind of growth this
// allocation exists to absorb -- and it would crowd out the internal
// MALLOC_CAP_DMA headroom board_display_init() needs for its LVGL flush and
// software-rotation buffers once WiFi's static internal .bss landed (see
// docs/hardware/board-notes.md). Nothing in this struct is DMA'd -- see
// transmit()/transmit_device_event(), which hand context->wire to
// usb_link_write_frame() (copies into the USB driver's own ring buffer) and,
// from Task 8 on, esp_websocket_client_send_bin() (also copies) -- and the
// struct is only ever touched from this task, never an ISR, so PSRAM cache
// access is safe here.
static protocol_context_t *s_context;
static TaskHandle_t s_task;
static const link_transport_t *s_transport;
static bool s_owner_usb_restricted;
static atomic_bool s_ota_blocked;
static atomic_bool s_owner_state_ready;
static atomic_bool s_network_decoder_reset_requested;

static void refresh_ota_snapshots(const protocol_context_t *context)
{
    bool interrupt_live = interrupt_state_active(&context->interrupts) != NULL;
    bool progress_running = widget_model_has_running_progress(&context->model);
    atomic_store_explicit(
        &s_ota_blocked,
        ota_policy_update_deferred(interrupt_live, progress_running),
        memory_order_release);
    atomic_store_explicit(
        &s_owner_state_ready,
        context->link.online && widget_model_config(&context->model) != NULL,
        memory_order_release);
}

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

static bool fill_carousel_binding(protocol_context_t *context,
                                  carousel_binding_t *binding)
{
    const protocol_apply_config_t *config = widget_model_config(&context->model);
    const interrupt_slot_t *interrupt = interrupt_state_active(
        &context->interrupts);
    const protocol_screen_config_t *screen = widget_model_active_screen(
        &context->model);
    const char *widget_id = interrupt != NULL
        ? interrupt->widget_id
        : (screen != NULL ? screen->widget_id : NULL);
    const protocol_widget_config_t *widget = find_widget(config, widget_id);
    if (config == NULL || widget == NULL || widget_id == NULL) {
        return false;
    }

    memset(binding, 0, sizeof(*binding));
    strcpy(binding->widget_id, widget_id);
    binding->tap_action = widget->tap_action;
    binding->interrupt = interrupt != NULL;
    if (interrupt != NULL) {
        strcpy(binding->screen_id, context->interrupts.saved_screen_id);
        binding->interrupt_token = interrupt->token;
        return true;
    }
    if (screen == NULL || config->screen_count == 0U) {
        return false;
    }

    size_t index = active_screen_index(&context->model);
    size_t previous = index == 0U ? config->screen_count - 1U : index - 1U;
    size_t next = (index + 1U) % config->screen_count;
    strcpy(binding->screen_id, screen->screen_id);
    strcpy(binding->previous_screen_id, config->screens[previous].screen_id);
    strcpy(binding->previous_widget_id, config->screens[previous].widget_id);
    strcpy(binding->next_screen_id, config->screens[next].screen_id);
    strcpy(binding->next_widget_id, config->screens[next].widget_id);
    return true;
}

static bool show_carousel_fallback(protocol_context_t *context)
{
    carousel_binding_t binding;
    if (!fill_carousel_binding(context, &binding) || binding.interrupt) {
        return false;
    }
    return ui_runtime_show_card_fallback();
}

static bool show_interrupt_fallback(protocol_context_t *context)
{
    carousel_binding_t binding;
    if (!fill_carousel_binding(context, &binding) || !binding.interrupt) {
        return false;
    }
    return ui_runtime_show_card_fallback();
}

static bool show_current_content(protocol_context_t *context)
{
    return interrupt_state_active(&context->interrupts) != NULL
               ? show_interrupt_fallback(context)
               : show_carousel_fallback(context);
}

// ------------------------------------------------------------------ scenes

/* scene_binding.h's scene_field_fn: resolves `field.<name>` against the
 * pushed provider data for the widget whose id matches the live scene's card
 * id. Card ids and widget ids are the same 32-byte identifier space (see
 * PROTOCOL_MAX_CARD_ID_LENGTH) and PushData is still the only message that
 * carries provider values, so the widget model IS the field source -- there
 * is no second store to keep in step with it.
 *
 * NULL for an unknown name is not an error: scene_binding_evaluate() renders
 * the "--" placeholder for it, which is exactly the state of a provider that
 * has not reported yet. */
static const char *scene_field_lookup(void *ctx, const char *name)
{
    protocol_context_t *context = ctx;
    if (context == NULL || name == NULL) {
        return NULL;
    }
    const template_field_state_t *fields = widget_model_widget_fields(
        &context->model, context->scene_card_id);
    if (fields == NULL) {
        return NULL;
    }
    const template_field_value_t *value = template_fields_get(fields, name);
    if (value == NULL) {
        return NULL;
    }
    if (value->type == PROTOCOL_FIELD_TEXT) {
        return value->value.text;
    }
    if (value->type == PROTOCOL_FIELD_BOOLEAN) {
        return value->value.boolean ? "true" : "false";
    }
    // One scratch slot is enough because scene_binding_evaluate() copies the
    // returned string into the caller's buffer before scene_view asks for
    // the next binding; two integer-valued fields in one scene never hold
    // this at the same time.
    snprintf(context->scene_field_text, sizeof(context->scene_field_text),
             "%lld", (long long)value->value.integer);
    return context->scene_field_text;
}

/* Fills the timer half of the binding context from the same ProgressRing
 * snapshot the reference oracle draws from, counted down locally between
 * pushes exactly as its current_remaining_ms() does.
 * Without the local countdown a `timer.remaining:` binding would freeze
 * between host pushes, on a device whose whole reason for evaluating
 * bindings itself is that the face keeps moving when the link does not.
 *
 * A card that is not a progress ring leaves timer_active false, which
 * renders the "--" placeholder -- the same thing an absent field renders.
 * `timer_active` means a timer snapshot exists, not that it is running: a
 * paused pomodoro must still show its remaining time. */
static void fill_timer_bindings(const protocol_context_t *context,
                                scene_binding_context_t *binding)
{
    const template_field_state_t *fields = widget_model_widget_fields(
        &context->model, context->scene_card_id);
    if (fields == NULL ||
        fields->template_kind != PROTOCOL_TEMPLATE_PROGRESS_RING) {
        return;
    }
    const template_field_value_t *duration =
        template_fields_get(fields, "duration_seconds");
    const template_field_value_t *remaining =
        template_fields_get(fields, "remaining_seconds");
    const template_field_value_t *running =
        template_fields_get(fields, "running");
    if (duration == NULL || remaining == NULL || running == NULL ||
        duration->value.integer <= 0) {
        return;
    }
    scene_timer_snapshot_t snapshot = scene_timer_snapshot(
        duration->value.integer, remaining->value.integer,
        running->value.boolean, context->scene_timer_anchor_ms, uptime_ms());
    binding->timer_active = true;
    binding->timer_running = snapshot.running;
    binding->timer_total_ms = snapshot.total_ms;
    binding->timer_remaining_ms = snapshot.remaining_ms;
    binding->timer_remaining_pct = snapshot.remaining_pct;
    binding->timer_remaining_permille = snapshot.remaining_permille;
}

static void fill_scene_binding_context(protocol_context_t *context,
                                       scene_binding_context_t *binding)
{
    memset(binding, 0, sizeof(*binding));
    // The same clock ui/clock_screen.c reads: TimeSync calls settimeofday(),
    // so a `time:` binding and the standalone clock can never disagree.
    binding->unix_seconds = (int64_t)time(NULL);
    binding->utc_offset_minutes = context->link.utc_offset_minutes;
    binding->field = scene_field_lookup;
    binding->field_ctx = context;
    fill_timer_bindings(context, binding);
}

typedef enum {
    SHOW_SCENE_OK = 0,
    /* The scene could not be rendered exactly as specified: an unallocatable
     * object, or an asset that could not be acquired. */
    SHOW_SCENE_REFUSED,
    /* Something else owns the panel right now (in practice: the OTA
     * takeover) -- retryable, and a different answer to the host than "your
     * scene is wrong", which it is not. */
    SHOW_SCENE_BUSY,
} show_scene_result_t;

/* Binds taps to a scene only when that scene names the card that currently
 * owns the panel. PushScene deliberately accepts an arbitrary card id for
 * diagnostic scenes; borrowing the active card's action for one of those
 * would send a valid-looking event for the wrong widget. Runs with the LVGL
 * lock held by show_scene(). */
static void bind_scene_carousel(protocol_context_t *context)
{
    carousel_binding_t binding;
    if (!fill_carousel_binding(context, &binding) ||
        strcmp(binding.widget_id, context->scene_card_id) != 0) {
        carousel_unbind();
        return;
    }
    (void)carousel_bind(scene_view_screen(), &binding);
}

/* Builds `scene` onto the panel.
 *
 * Called from this task, not handed to ui_runtime's command queue, for a
 * reason that is not convenience: sizeof(scene_t) is ~6 KB, too large for
 * the bounded value queue and its fixed mailbox budget. Passing a pointer
 * through the queue instead would hand the LVGL task a pointer into
 * `message`, which the very next frame overwrites. So the handoff is
 * synchronous under lvgl_port_lock(), the mechanism CLAUDE.md names for LVGL
 * calls made outside LVGL callbacks and the one dispatch_asset_release()
 * already uses for font_registry_reset(). The LVGL task cannot be inside a
 * callback while this lock is held. */
static show_scene_result_t show_scene(protocol_context_t *context,
                                      const scene_t *scene)
{
    scene_binding_context_t binding;
    fill_scene_binding_context(context, &binding);
    lvgl_port_lock(0U);
    show_scene_result_t result = SHOW_SCENE_BUSY;
    // The OTA task owns the panel until it reboots or restores the clock.
    // ui_runtime's queue consumer drops SHOW_* commands for that reason and
    // this path does not go through that queue, so it makes the same check.
    if (!ota_screen_active_in_lvgl()) {
        if (scene_view_show(scene, &binding)) {
            bind_scene_carousel(context);
            result = SHOW_SCENE_OK;
        } else {
            result = SHOW_SCENE_REFUSED;
        }
    }
    lvgl_port_unlock();
    return result;
}

/* Re-evaluates the live scene's bindings in place, and returns whether a
 * scene is still on the panel. Both forms avoid a rebuild, but their timer
 * ownership differs deliberately: an ordinary tick preserves the scene's
 * optimistic timer snapshot, while an authoritative PushData refresh
 * replaces it whole.
 *
 * The return value is what keeps context->scene_live honest without a second
 * mechanism: this function already holds the lock the authority must be read
 * under, so it reads it while it is there. A scene retired by a screen change
 * this task never observed (a queued view command, a link-loss clock restore)
 * costs exactly one no-op refresh before the hint self-corrects. */
static bool refresh_scene_bindings(protocol_context_t *context,
                                   bool authoritative)
{
    scene_binding_context_t binding;
    fill_scene_binding_context(context, &binding);
    lvgl_port_lock(0U);
    if (authoritative) {
        scene_view_refresh_bindings(&binding);
    } else {
        scene_view_tick_bindings(&binding);
    }
    bool still_live = scene_view_screen() != NULL;
    lvgl_port_unlock();
    return still_live;
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
        context->response_transport->write_frame(
            context->wire, wire_length,
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
    status->max_protocol_version = PROTOCOL_MAX_VERSION;
    status->capabilities = PROTOCOL_CURRENT_CAPABILITIES;
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
    status->rx_dropped_bytes = s_transport->dropped_bytes();
    status->dropped_events = saturating_add(
        context->dropped_events,
        device_event_queue_dropped(&context->events));
    status->event_queue_high_water =
        device_event_queue_high_water(&context->events);
    status->dropped_ui_commands = ui_runtime_dropped_commands();
    status->ui_queue_high_water = ui_runtime_queue_high_water();
    status->config_revision = widget_model_config_revision(&context->model);
    status->latest_interrupt_token =
        interrupt_state_latest_token(&context->interrupts);
    status->ota_state = ota_state();
    status->has_last_ota_error = ota_copy_last_error(
        status->last_ota_error, sizeof(status->last_ota_error));
    status->wifi_state = wifi_station_state();
    status->wifi_rssi = wifi_station_rssi();
    wifi_station_copy_ip(status->ip, sizeof(status->ip));
    status->tier = net_store_current_tier();
    // Key 31 is omitted, not encoded empty, when the store never formatted
    // (docs/protocol/v1.md): asset_store_stats() returns
    // ASSET_STORE_ERR_ARGUMENT for a store whose asset_store_open() never
    // succeeded, in which case has_asset_store_stats stays false from this
    // function's leading memset and encode_status_payload() skips the key.
    asset_store_stats_t asset_stats;
    if (asset_store_stats(asset_flash_store(), &asset_stats) ==
        ASSET_STORE_OK) {
        status->has_asset_store_stats = true;
        status->asset_store_used_bytes = asset_stats.used_blob_bytes;
        status->asset_store_free_bytes = asset_stats.free_blob_bytes;
        status->asset_count = asset_stats.committed_count;
    }
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

static void transmit_asset_begin_ack(protocol_context_t *context,
                                     uint32_t request_id,
                                     bool already_present)
{
    protocol_message_t *reply = &context->message;
    memset(reply, 0, sizeof(*reply));
    reply->type = PROTOCOL_TYPE_ACK;
    reply->value.ack.acknowledged_type = PROTOCOL_TYPE_ASSET_BEGIN;
    reply->value.ack.has_already_present = true;
    reply->value.ack.already_present = already_present;
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

    // The scene's binding context reads its `field.` values straight out of
    // the widget model updated above, so there is nothing further to copy:
    // re-evaluating the bindings in place is the whole update. Deliberately
    // NOT a rebuild -- a rebuild would re-acquire every asset face and
    // reload the screen to change one label.
    if (context->scene_live &&
        strcmp(context->scene_card_id, push->widget_id) == 0) {
        context->scene_timer_anchor_ms = uptime_ms();
        context->scene_live = refresh_scene_bindings(context, true);
    }
    transmit_ack(context, request_id, PROTOCOL_TYPE_PUSH_DATA, true,
                 context->link.latest_revision);
}

static void dispatch_push_scene(protocol_context_t *context,
                                uint32_t request_id)
{
    const protocol_push_scene_t *push = &context->message.value.push_scene;
    // Published before the show so the first paint's `field.` and `timer.`
    // bindings resolve against THIS card rather than the outgoing one, and
    // restored on refusal so a scene that never rendered cannot redirect the
    // live scene's lookups.
    char previous_card_id[sizeof(context->scene_card_id)];
    memcpy(previous_card_id, context->scene_card_id,
           sizeof(previous_card_id));
    uint64_t previous_timer_anchor_ms = context->scene_timer_anchor_ms;
    if (strcmp(previous_card_id, push->card_id) != 0) {
        context->scene_timer_anchor_ms = uptime_ms();
    }
    memcpy(context->scene_card_id, push->card_id,
           sizeof(context->scene_card_id));

    /* ApplyConfig/ActivateScreen queue the standalone clock until a scene
     * arrives. Remove that pending fallback before loading the scene so the
     * LVGL command timer cannot replace the freshly rendered face afterward.
     * A refused scene restores the fallback request below. */
    bool fallback_discarded = ui_runtime_discard_card_fallbacks();
    show_scene_result_t result = show_scene(context, &push->scene);
    if (result != SHOW_SCENE_OK) {
        if (fallback_discarded) {
            (void)ui_runtime_show_card_fallback();
        }
        memcpy(context->scene_card_id, previous_card_id,
               sizeof(context->scene_card_id));
        context->scene_timer_anchor_ms = previous_timer_anchor_ms;
        // scene_view_show() refuses without touching the live screen (it
        // builds onto a candidate screen and loads it only once every node
        // and every asset succeeded), so whatever was on the panel -- the
        // previous scene, a template view, or the standalone clock -- is
        // still on it. This path never blanks the display.
        if (result == SHOW_SCENE_BUSY) {
            transmit_error(context, request_id, PROTOCOL_ERROR_BUSY,
                           "display unavailable");
        } else {
            transmit_error(context, request_id, PROTOCOL_ERROR_INVALID_PAYLOAD,
                           "scene could not be rendered");
        }
        return;
    }
    // Retained only after the show succeeded, and copied before
    // transmit_ack() reuses context->message for the reply.
    context->scene = push->scene;
    context->scene_live = true;
    context->scene_tick_ms = uptime_ms();
    uint32_t revision = push->revision;
    transmit_ack(context, request_id, PROTOCOL_TYPE_PUSH_SCENE, true,
                 revision);
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
    const protocol_apply_config_t *incoming =
        &context->message.value.apply_config;
    widget_model_config_result_t result = widget_model_check_config(
        &context->model, incoming);
    if (result != WIDGET_MODEL_CONFIG_APPLIED &&
        result != WIDGET_MODEL_CONFIG_REPLAYED) {
        transmit_error(context, request_id, config_error_code(result),
                       "configuration rejected");
        return;
    }
    if (board_display_rotation_degrees() != incoming->rotation &&
        board_display_set_rotation_180(incoming->rotation == 270U) != ESP_OK) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INTERNAL,
                       "display orientation rejected");
        return;
    }
    if (result == WIDGET_MODEL_CONFIG_REPLAYED) {
        if (!show_current_content(context)) {
            transmit_error(context, request_id, PROTOCOL_ERROR_INTERNAL,
                           "UI command rejected");
            return;
        }
        transmit_ack(context, request_id, PROTOCOL_TYPE_APPLY_CONFIG, true,
                     widget_model_config_revision(&context->model));
        return;
    }
    result = widget_model_apply_config(&context->model, incoming);
    if (result != WIDGET_MODEL_CONFIG_APPLIED) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INTERNAL,
                       "configuration commit failed");
        return;
    }
    interrupt_state_clear(&context->interrupts);
    if (!show_carousel_fallback(context)) {
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
    } else if (!show_carousel_fallback(context)) {
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
        !show_interrupt_fallback(context)) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INTERNAL,
                       "UI command rejected");
        return;
    }
    transmit_ack(context, request_id, PROTOCOL_TYPE_TRIGGER_INTERRUPT,
                 false, 0U);
}

// Distinguishes why an incoming NETWORK_CONFIG command was rejected, for a
// host-side diagnostic. Never includes a field value -- ssid/psk/server_url/
// token are never safe to echo back onto the wire.
static const char *network_config_error_diagnostic(net_config_error_t error)
{
    switch (error) {
        case NET_CONFIG_ERR_MISSING_SSID:
            return "networked tier requires ssid";
        case NET_CONFIG_ERR_MISSING_SERVER_URL:
            return "networked tier requires server_url";
        case NET_CONFIG_ERR_MISSING_TOKEN:
            return "networked tier requires token";
        case NET_CONFIG_ERR_INSECURE_URL:
            return "server URL must use wss://";
        case NET_CONFIG_ERR_INVALID_TIER:
            return "invalid tier";
        case NET_CONFIG_ERR_INVALID_UTC_OFFSET:
            return "utc offset out of range";
        case NET_CONFIG_OK:
        default:
            return "invalid network config";
    }
}

static void dispatch_network_config(protocol_context_t *context,
                                    uint32_t request_id)
{
    const protocol_network_config_t *config =
        &context->message.value.network_config;
    // Reject-on-write, distinct from the store's degrade-on-read rule: a
    // NETWORK_CONFIG message is the host asking to pair, and if the config
    // cannot produce the tier it asks for, the host must be told the
    // request failed rather than receiving an ACK for a config that will
    // silently boot local. Only NET_CONFIG_OK (which a factory-fresh local
    // config with every field empty satisfies) is persisted. Returning here
    // means net_store_save() is never reached on any rejection path, so a
    // rejected write cannot land a partial update over a previously-stored
    // valid config.
    net_config_error_t validation = net_config_validate(config);
    if (validation != NET_CONFIG_OK) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INVALID_PAYLOAD,
                       network_config_error_diagnostic(validation));
        return;
    }
    if (net_store_save(config) != ESP_OK) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INTERNAL,
                       "failed to persist network config");
        return;
    }
    // The new tier takes effect on the next boot, not live: net_store_save()
    // never updates net_store_current_tier()'s cache, so this ACK does not
    // change what net_config_usb_message_allowed() enforces for the rest of
    // this session.
    transmit_ack(context, request_id, PROTOCOL_TYPE_NETWORK_CONFIG, false, 0U);
}

static void dispatch_factory_reset(protocol_context_t *context,
                                   uint32_t request_id)
{
    if (net_store_erase() != ESP_OK) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INTERNAL,
                       "failed to erase network config");
        return;
    }
    transmit_ack(context, request_id, PROTOCOL_TYPE_FACTORY_RESET, false, 0U);
}

// font_registry_init()/scene_view's asset_resolver_fn: resolves a committed
// digest to read-only blob bytes. Volatile PSRAM wins before flash so an
// atomic replacement can be rendered without ever spending partition
// endurance. This is the only place the ESP-IDF-free consumers meet either
// backing store.
static bool protocol_asset_resolver(const uint8_t *digest, const void **out_ptr,
                                    uint32_t *out_len, uint8_t *out_kind)
{
    if (volatile_asset_store_find(&s_context->volatile_assets, digest,
                                  out_ptr, out_len, out_kind) ==
        VOLATILE_ASSET_STORE_OK) {
        return true;
    }
    asset_record_t record;
    if (asset_store_find(asset_flash_store(), digest, &record, NULL) !=
        ASSET_STORE_OK) {
        return false;
    }
    if (asset_flash_map(&record, out_ptr) != ESP_OK) {
        return false;
    }
    *out_len = record.length;
    *out_kind = record.kind;
    return true;
}

static void *volatile_psram_allocate(void *ctx, size_t length)
{
    (void)ctx;
    return heap_caps_malloc(length, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
}

static void volatile_psram_deallocate(void *ctx, void *ptr)
{
    (void)ctx;
    heap_caps_free(ptr);
}

static bool volatile_sha256_matches(
    void *ctx, const void *bytes, size_t length,
    const uint8_t expected[ASSET_DIGEST_BYTES])
{
    (void)ctx;
    uint8_t actual[ASSET_DIGEST_BYTES];
    if (mbedtls_sha256(bytes, length, actual, 0) != 0) {
        return false;
    }
    return memcmp(actual, expected, sizeof actual) == 0;
}

// Reclaims a reservation this context abandoned -- whether the in-RAM
// transfer over it is still marked active (a second AssetBegin superseding
// the first) or was already aborted out from under it (a rejected/failed
// AssetChunk, which resets asset_transfer to inactive but leaves the
// on-flash record UNCOMMITTED). Without this, an interrupted transfer's
// reservation is reclaimed only by AssetRelease/compaction -- and the host
// never sends AssetRelease on this path (AssetSync::reconcile returns at the
// first AssetBegin failure), so repeated interrupted retries permanently
// consume blob space. Best-effort: a mark-dead failure here is no worse than
// the pre-fix behavior (nothing ever reclaimed it), so this still clears the
// flag rather than retrying it forever on every future AssetBegin.
static void abandon_pending_reservation(protocol_context_t *context)
{
    if (!context->asset_reservation_pending) {
        return;
    }
    if (asset_store_mark_dead(asset_flash_store(),
                              context->asset_transfer_record_index) !=
        ASSET_STORE_OK) {
        ESP_LOGW(TAG,
                "failed to reclaim abandoned asset reservation (index %u)",
                (unsigned)context->asset_transfer_record_index);
    }
    context->asset_reservation_pending = false;
}

static void abort_asset_transfers(protocol_context_t *context)
{
    volatile_asset_store_abort_incoming(&context->volatile_assets);
    if (context->asset_transfer.active) {
        asset_transfer_abort(&context->asset_transfer);
    }
    abandon_pending_reservation(context);
}

static void dispatch_asset_begin(protocol_context_t *context,
                                 uint32_t request_id)
{
    const protocol_asset_begin_t *begin = &context->message.value.asset_begin;
    // A second AssetBegin while a transfer is active means the host gave up
    // on the first one; abort it before deciding how to handle this one so
    // a stale in-memory transfer can never straddle two different digests.
    // The on-flash reservation behind it (this context's own, or one left by
    // an earlier AssetChunk failure) is reclaimed the same way regardless of
    // whether the in-RAM transfer is still active -- see
    // abandon_pending_reservation().
    abort_asset_transfers(context);
    if (begin->volatile_tier) {
        volatile_asset_store_result_t result = volatile_asset_store_begin_encoded(
            &context->volatile_assets, begin->digest, (uint8_t)begin->kind,
            begin->total_length, begin->encoding,
            begin->has_decoded_length ? begin->decoded_length : 0U);
        if (result == VOLATILE_ASSET_STORE_ALREADY_PRESENT) {
            transmit_asset_begin_ack(context, request_id, true);
            return;
        }
        if (result != VOLATILE_ASSET_STORE_OK) {
            transmit_error(context, request_id,
                           result == VOLATILE_ASSET_STORE_ERR_FULL
                               ? PROTOCOL_ERROR_BUSY
                               : PROTOCOL_ERROR_INVALID_PAYLOAD,
                           "volatile asset reserve failed");
            return;
        }
        transmit_asset_begin_ack(context, request_id, false);
        return;
    }
    asset_record_t existing;
    if (asset_store_find(asset_flash_store(), begin->digest, &existing,
                         NULL) == ASSET_STORE_OK) {
        transmit_asset_begin_ack(context, request_id, true);
        return;
    }
    uint32_t index = 0U;
    uint32_t blob_offset = 0U;
    if (asset_store_reserve(asset_flash_store(), begin->digest,
                            (uint8_t)begin->kind, begin->total_length, &index,
                            &blob_offset) != ASSET_STORE_OK) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INVALID_PAYLOAD,
                       "asset store reserve failed");
        return;
    }
    if (asset_transfer_begin(&context->asset_transfer, begin->digest,
                             (uint8_t)begin->kind, begin->total_length,
                             false) != ASSET_TRANSFER_OK) {
        // The reservation above succeeded but the in-RAM transfer could not
        // start; nothing else will ever learn this index, so reclaim it now
        // rather than leaking it the same way an interrupted transfer would.
        asset_store_mark_dead(asset_flash_store(), index);
        transmit_error(context, request_id, PROTOCOL_ERROR_INVALID_PAYLOAD,
                       "asset transfer begin failed");
        return;
    }
    context->asset_transfer_record_index = index;
    context->asset_transfer_blob_offset = blob_offset;
    context->asset_reservation_pending = true;
    transmit_asset_begin_ack(context, request_id, false);
}

static void dispatch_asset_chunk(protocol_context_t *context,
                                 uint32_t request_id)
{
    const protocol_asset_chunk_t *chunk = &context->message.value.asset_chunk;
    if (context->volatile_assets.incoming_transfer.active) {
        volatile_asset_store_result_t result = volatile_asset_store_write(
            &context->volatile_assets, chunk->digest, chunk->offset,
            chunk->data, (uint32_t)chunk->data_length);
        if (result != VOLATILE_ASSET_STORE_OK) {
            transmit_error(context, request_id,
                           PROTOCOL_ERROR_INVALID_PAYLOAD,
                           "volatile asset chunk rejected");
            return;
        }
        transmit_ack(context, request_id, PROTOCOL_TYPE_ASSET_CHUNK, false,
                     0U);
        return;
    }
    asset_transfer_result_t result = asset_transfer_accept_chunk(
        &context->asset_transfer, chunk->digest, chunk->offset,
        (uint32_t)chunk->data_length);
    if (result == ASSET_TRANSFER_DUPLICATE) {
        // The lost-Ack path: the host resent a chunk we already wrote
        // because our Ack for it never arrived. Acknowledge again without
        // rewriting -- rewriting here would apply those bytes twice.
        transmit_ack(context, request_id, PROTOCOL_TYPE_ASSET_CHUNK, false,
                     0U);
        return;
    }
    if (result != ASSET_TRANSFER_OK) {
        asset_transfer_abort(&context->asset_transfer);
        transmit_error(context, request_id, PROTOCOL_ERROR_INVALID_PAYLOAD,
                       "asset chunk rejected");
        return;
    }
    if (asset_flash_write_blob(
            context->asset_transfer_blob_offset + chunk->offset, chunk->data,
            chunk->data_length) != ESP_OK) {
        asset_transfer_abort(&context->asset_transfer);
        transmit_error(context, request_id, PROTOCOL_ERROR_INTERNAL,
                       "asset blob write failed");
        return;
    }
    transmit_ack(context, request_id, PROTOCOL_TYPE_ASSET_CHUNK, false, 0U);
}

static void dispatch_asset_commit(protocol_context_t *context,
                                  uint32_t request_id)
{
    const protocol_asset_commit_t *commit =
        &context->message.value.asset_commit;
    if (context->volatile_assets.incoming_transfer.active) {
        volatile_asset_store_result_t result = volatile_asset_store_commit(
            &context->volatile_assets, commit->digest);
        if (result != VOLATILE_ASSET_STORE_OK) {
            transmit_error(context, request_id,
                           PROTOCOL_ERROR_INVALID_PAYLOAD,
                           "volatile asset commit failed");
            return;
        }
        transmit_ack(context, request_id, PROTOCOL_TYPE_ASSET_COMMIT, false,
                     0U);
        return;
    }
    if (!context->asset_transfer.active ||
        memcmp(commit->digest, context->asset_transfer.digest,
              ASSET_DIGEST_BYTES) != 0 ||
        !asset_transfer_is_complete(&context->asset_transfer)) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INVALID_PAYLOAD,
                       "asset transfer not complete");
        return;
    }
    if (asset_store_commit(asset_flash_store(),
                           context->asset_transfer_record_index) !=
        ASSET_STORE_OK) {
        transmit_error(context, request_id, PROTOCOL_ERROR_INTERNAL,
                       "asset store commit failed");
        return;
    }
    // Committed, not abandoned: this reservation must not be reclaimed by a
    // future abandon_pending_reservation() call.
    context->asset_reservation_pending = false;
    asset_transfer_abort(&context->asset_transfer);
    transmit_ack(context, request_id, PROTOCOL_TYPE_ASSET_COMMIT, false, 0U);
}

/* The mark-dead scan and the compaction, split out so the teardown sequence
 * below can wrap them: every one of its five failure exits has to be
 * followed by the same scene rebuild, and duplicating that five times is how
 * one of them ends up missing it. Returns false with *error_code and
 * *diagnostic set. */
static bool collect_released_assets(protocol_context_t *context,
                                    protocol_error_code_t *error_code,
                                    const char **diagnostic)
{
    const protocol_asset_release_t *release =
        &context->message.value.asset_release;
    const asset_store_t *store = asset_flash_store();

    // An active transfer's reserved-but-uncommitted record is, by
    // definition, absent from `release`'s digest list (the host cannot name
    // a digest it hasn't finished sending), so the mark-dead/compaction scan
    // below would exclude that slot from the compacted record array while
    // the in-RAM asset_transfer_t keeps pointing at it. A chunk landing
    // after that point writes into a blob region compaction has already
    // repacked out from under it, and a later AssetCommit would flip a
    // wiped, all-0xFF-content slot to COMMITTED -- corrupting the store for
    // every future asset_store_reserve()/asset_store_stats() call. Abort
    // (not reject-with-Busy) so this can never race: the host's resumable
    // design already handles an abort by simply re-sending AssetBegin for a
    // fresh reservation, whereas rejecting the release risks a stuck
    // transfer permanently blocking GC.
    abort_asset_transfers(context);

    for (uint32_t index = 0U; index < store->record_capacity; ++index) {
        uint8_t bytes[ASSET_RECORD_BYTES];
        if (asset_flash_io()->read(asset_flash_io()->ctx,
                                   ASSET_HEADER_BYTES +
                                       index * ASSET_RECORD_BYTES,
                                   bytes, sizeof(bytes)) != 0) {
            *error_code = PROTOCOL_ERROR_INTERNAL;
            *diagnostic = "asset store read failed";
            return false;
        }
        asset_record_t record;
        if (asset_store_record_decode(bytes, &record) != ASSET_STORE_OK) {
            // Most commonly an untouched slot: still-erased bytes decode a
            // kind of 0xFF, which is not a defined asset_kind_t, so decode()
            // correctly refuses it. That is never a committed record worth
            // pruning, so skip it rather than fail the whole release; a
            // genuinely corrupt committed record is skipped the same way and
            // is left for a future find()/stats() caller to surface.
            continue;
        }
        if (record.state != ASSET_STATE_COMMITTED) {
            continue;
        }
        bool keep = false;
        for (size_t k = 0U; k < release->digest_count; ++k) {
            if (memcmp(record.digest, release->digests[k],
                      ASSET_DIGEST_BYTES) == 0) {
                keep = true;
                break;
            }
        }
        if (!keep &&
            asset_store_mark_dead(store, index) != ASSET_STORE_OK) {
            *error_code = PROTOCOL_ERROR_INTERNAL;
            *diagnostic = "asset store mark dead failed";
            return false;
        }
    }

    // move_count can never exceed release->digest_count (plan_compaction
    // only emits a move for a committed record matching an entry in `keep`),
    // and digest_count is already bounded to PROTOCOL_MAX_ASSET_DIGESTS by
    // decode, so a buffer of that size is never truncated.
    const uint8_t *keep_ptrs[PROTOCOL_MAX_ASSET_DIGESTS];
    for (size_t i = 0U; i < release->digest_count; ++i) {
        keep_ptrs[i] = release->digests[i];
    }
    asset_move_t moves[PROTOCOL_MAX_ASSET_DIGESTS];
    size_t move_count = 0U;
    if (asset_store_plan_compaction(store, keep_ptrs, release->digest_count,
                                    moves, PROTOCOL_MAX_ASSET_DIGESTS,
                                    &move_count) != ASSET_STORE_OK) {
        *error_code = PROTOCOL_ERROR_INTERNAL;
        *diagnostic = "asset compaction planning failed";
        return false;
    }
    if (asset_flash_execute_compaction(moves, move_count) != ESP_OK) {
        *error_code = PROTOCOL_ERROR_INTERNAL;
        *diagnostic = "asset compaction failed";
        return false;
    }
    if (volatile_asset_store_release(&context->volatile_assets, keep_ptrs,
                                     release->digest_count, NULL) !=
        VOLATILE_ASSET_STORE_OK) {
        *error_code = PROTOCOL_ERROR_INTERNAL;
        *diagnostic = "volatile asset release failed";
        return false;
    }
    return true;
}

static bool scene_release_must_teardown(
    const protocol_context_t *context,
    const protocol_asset_release_t *release)
{
    const uint8_t *used[SCENE_MAX_NODES];
    size_t used_count = 0U;
    for (uint32_t i = 0U; i < context->scene.node_count; ++i) {
        const scene_node_t *node = &context->scene.nodes[i];
        if (node->kind == SCENE_NODE_IMAGE) {
            used[used_count++] = node->value.image.digest;
        } else if (node->kind == SCENE_NODE_GLYPH) {
            used[used_count++] = node->value.glyph.digest;
        } else if (node->kind == SCENE_NODE_TEXT &&
                   node->value.text.font.kind != SCENE_FONT_BAKED) {
            used[used_count++] = node->value.text.font.digest;
        }
    }
    const uint8_t *keep[PROTOCOL_MAX_ASSET_DIGESTS];
    for (size_t i = 0U; i < release->digest_count; ++i) {
        keep[i] = release->digests[i];
    }
    return volatile_asset_store_release_must_teardown(
        &context->volatile_assets, used, used_count, keep,
        release->digest_count);
}

static void dispatch_asset_release(protocol_context_t *context,
                                   uint32_t request_id)
{
    // Compaction physically moves blob bytes, which invalidates every
    // lv_font_t rasterized from an asset_flash_map() pointer. So every open
    // face must be gone before asset_flash_execute_compaction() runs, and
    // font_registry_reset() is what makes that true. It calls into
    // LVGL/tiny_ttf (lv_tiny_ttf_destroy), so it runs under lvgl_port_lock()
    // like every other LVGL mutation this task makes from outside the LVGL
    // task.
    //
    // A nonzero return from the reset is a REFUSAL, not a warning: some LVGL
    // object is still styled with an acquired face (LVGL keeps the bare
    // pointer and takes no reference of its own -- see font_registry.h's
    // ownership contract), the registry destroyed nothing, and compaction
    // therefore must not proceed. Deferring the collection costs the host a
    // retry; compacting anyway would leave a live screen drawing from moved
    // bytes.
    //
    // That refusal is a safety net and not a solution: a scene holding asset
    // faces would trip it forever, and asset garbage collection would simply
    // stop working on this device. The teardown below is the solution, and
    // it is a SEQUENCE rather than a call:
    //
    //   1. load another screen -- scene_view_destroy() refuses while its own
    //      scene is the active one, so something else has to be on the glass
    //      first, and the standalone clock is the screen this device always
    //      has;
    //   2. destroy the scene, which is what actually releases the faces: its
    //      objects drop their acquires from their LV_EVENT_DELETE handlers;
    //   3. reset the registry, which now succeeds;
    //   4. compact;
    //   5. rebuild the scene, re-acquiring every face against the moved
    //      blobs.
    //
    // Step 1 is why the retained copy of the scene exists at all: after step
    // 2 there is nothing left to rebuild from. Note step 1 usually performs
    // step 2 on its own -- lv_screen_load_anim() with auto_del deletes the
    // outgoing screen synchronously when time and delay are both 0 -- and
    // the explicit destroy is what covers the case where the clock screen
    // was already active and clock_screen_show_in_lvgl() early-returns.
    //
    // The rebuild runs on EVERY exit path after the teardown, including the
    // refusal and each store failure. The other holder of a face is the
    // DESKMATE_DEV_DIAG asset probe (link/dev_capture.c), which this task
    // cannot tear down; a release arriving while a probe render is up is
    // still correctly deferred.
    //
    // WHAT DECIDES that a scene is up is scene_view_screen(), the renderer's
    // own state, and NOT context->scene_live. The two are allowed to
    // disagree: a queued fallback can fail to land (ui_command_queue_push can
    // drop), leaving a scene alive that this task no longer believes in, and
    // show_scene() bypasses that queue entirely. Gating the teardown on the
    // belief would, in the first case,
    // skip a teardown a live scene needed and block collection until some
    // later screen change; in the second it would put a stale scene back
    // over newer content. Reading the authority under the lock we
    // already hold costs nothing and cannot be wrong.
    lvgl_port_lock(0U);
    // Three conditions, and each one earns its place:
    //  - a scene really is on the panel (the authority, see above);
    //  - it reads bytes compaction can move -- a scene of baked fonts and
    //    geometry survives compaction untouched, so tearing it down would
    //    buy a visible screen flap and nothing else;
    //  - the OTA takeover does not own the panel. Swapping it for the clock
    //    to collect assets would hide a running update, so leave it alone;
    //    font_registry_reset() then refuses if the scene pins a face and the
    //    host retries, which is the correct outcome.
    /* The end-to-end safety sequence intentionally spans this file,
     * link/asset_flash.c, ui/font_registry.c, and ui/scene_view.c. There is
     * no automated seam across all four. The pure keep/use decision is host
     * tested in volatile_asset_store; hardware must still prove the actual
     * clock -> destroy -> reset -> collect -> rebuild sequence. */
    bool torn_down = scene_view_screen() != NULL &&
                     scene_release_must_teardown(
                         context, &context->message.value.asset_release) &&
                     !ota_screen_active_in_lvgl();
    if (torn_down) {
        clock_screen_show_in_lvgl();
        (void)scene_view_destroy();
        context->scene_live = false;
    }
    uint32_t pinned_faces = font_registry_reset();
    lvgl_port_unlock();

    if (pinned_faces > 0U) {
        // font_registry.c has no ESP-IDF include and no LVGL logging config
        // of its own (see its header), so it hands the count back rather
        // than logging; this task has a working sink.
        ESP_LOGW(TAG,
                "AssetRelease deferred: %u font face(s) still in use",
                (unsigned)pinned_faces);
        // BUSY is Task 6's word for "nothing happened, retry", so it may
        // only be sent when nothing did happen. If the teardown ran and the
        // scene came back, that is still true. If the scene could NOT be
        // restored, the panel has changed and answering BUSY would assert
        // the opposite of the truth -- so say what actually happened, and
        // say it in terms the host can act on: re-push the scene.
        if (torn_down &&
            show_scene(context, &context->scene) != SHOW_SCENE_OK) {
            ESP_LOGW(TAG, "scene not rebuilt after asset collection");
            transmit_error(context, request_id, PROTOCOL_ERROR_INTERNAL,
                           "font faces in use; scene dropped, re-push it");
            return;
        }
        context->scene_live = torn_down;
        transmit_error(context, request_id, PROTOCOL_ERROR_BUSY,
                       "font faces in use");
        return;
    }

    protocol_error_code_t error_code = PROTOCOL_ERROR_INTERNAL;
    const char *diagnostic = "asset release failed";
    bool collected = collect_released_assets(context, &error_code,
                                             &diagnostic);
    if (torn_down) {
        // Re-acquires every face and re-maps every blob from the compacted
        // store. A scene naming an asset the host has just released cannot
        // be rebuilt, and is refused whole rather than drawn with a
        // substitute face: the clock stays up and this logs the loss.
        //
        // The Ack below still goes out in that case, and deliberately: the
        // release genuinely succeeded, and protocol v1's Ack for type 18
        // carries no field in which a device could report a side effect on
        // its display. Nothing in the wire tells the host its scene is gone
        // -- a host that releases an asset its own live scene uses gets
        // silence and this log line. Recorded as a gap, not papered over.
        context->scene_live =
            show_scene(context, &context->scene) == SHOW_SCENE_OK;
        if (!context->scene_live) {
            ESP_LOGW(TAG, "scene not rebuilt after asset collection");
        }
    }
    if (!collected) {
        transmit_error(context, request_id, error_code, diagnostic);
        return;
    }
    transmit_ack(context, request_id, PROTOCOL_TYPE_ASSET_RELEASE, false, 0U);
}

static void dispatch_request(protocol_context_t *context,
                             const protocol_frame_t *frame,
                             bool restricted_usb)
{
#ifdef DESKMATE_DEV_DIAG
    // Dev-only framebuffer capture (spec §3.2.3): 0x7E/0x7F sit outside
    // protocol_message_type_t's frozen 1-12 range, so this is intercepted
    // here rather than taught to protocol_message_decode -- the release
    // message tables in core/protocol_message.c never learn about it.
    if (frame->message_type == DEV_CAPTURE_REQUEST_TYPE) {
        if (frame->version != PROTOCOL_VERSION || frame->payload_length != 0U) {
            increment_saturating(&context->malformed_frames);
            transmit_error(context, frame->request_id,
                           PROTOCOL_ERROR_INVALID_PAYLOAD,
                           "invalid dev capture request");
            return;
        }
        dev_capture_handle_request(frame->request_id);
        return;
    }
    // Dev-only asset font render probe (Task 13): 0x7D sits in the same
    // dev-only range as 0x7E/0x7F, intercepted here for the same reason --
    // the release message tables in core/protocol_message.c never learn
    // about it. Payload is exactly the ASSET_DIGEST_BYTES digest of a font
    // already committed via AssetBegin/AssetChunk/AssetCommit.
    if (frame->message_type == DEV_ASSET_PROBE_REQUEST_TYPE) {
        if (frame->version != PROTOCOL_VERSION ||
            frame->payload_length != ASSET_DIGEST_BYTES) {
            increment_saturating(&context->malformed_frames);
            transmit_error(context, frame->request_id,
                           PROTOCOL_ERROR_INVALID_PAYLOAD,
                           "invalid asset probe request");
            return;
        }
        dev_capture_handle_asset_probe(frame->payload);
        return;
    }
#endif
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
    // One table, in core/protocol_message.c, for both halves of the
    // question: is this a request at all, and does this build advertise the
    // capability bit docs/protocol/v1.md gates it on? It lives there rather
    // than here so a host test can assert that every advertised bit really
    // is dispatched -- bit 8 shipped set while this function still answered
    // type 19 with "response type sent as request", and no test could see
    // it while the policy was three inline conditions in an ESP-IDF-only
    // file. A conforming host never sends a message its capabilities did
    // not offer, but the device refuses explicitly rather than trusting it.
    protocol_request_gate_t gate = protocol_message_request_gate(
        context->message.type, PROTOCOL_CURRENT_CAPABILITIES);
    if (gate == PROTOCOL_REQUEST_NOT_A_REQUEST) {
        transmit_error(context, frame->request_id,
                       PROTOCOL_ERROR_UNSUPPORTED_MESSAGE,
                       "response type sent as request");
        return;
    }
    if (gate == PROTOCOL_REQUEST_MISSING_CAPABILITY) {
        transmit_error(context, frame->request_id,
                       PROTOCOL_ERROR_UNSUPPORTED_MESSAGE,
                       "capability not supported by this build");
        return;
    }
    if (context->message.type == PROTOCOL_TYPE_ASSET_BEGIN &&
        protocol_asset_begin_request_gate(
            &context->message.value.asset_begin,
            PROTOCOL_CURRENT_CAPABILITIES) ==
            PROTOCOL_REQUEST_MISSING_CAPABILITY) {
        transmit_error(context, frame->request_id,
                       PROTOCOL_ERROR_UNSUPPORTED_MESSAGE,
                       "volatile assets unsupported by this build");
        return;
    }

    if (restricted_usb &&
        !net_config_usb_message_allowed(net_store_current_tier(),
                                        context->message.type)) {
        transmit_error(context, frame->request_id, PROTOCOL_ERROR_WRONG_TIER,
                       "device is owned over the network");
        return;
    }

    // Configurator traffic over USB must not establish or keep alive the
    // network owner's link. In local tier the owner itself is USB, so the
    // same messages still drive the ordinary link state there.
    if (!restricted_usb &&
        link_state_note_valid_request(&context->link, uptime_ms())) {
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
    case PROTOCOL_TYPE_NETWORK_CONFIG:
        dispatch_network_config(context, frame->request_id);
        break;
    case PROTOCOL_TYPE_FACTORY_RESET:
        dispatch_factory_reset(context, frame->request_id);
        break;
    case PROTOCOL_TYPE_ASSET_BEGIN:
        dispatch_asset_begin(context, frame->request_id);
        break;
    case PROTOCOL_TYPE_ASSET_CHUNK:
        dispatch_asset_chunk(context, frame->request_id);
        break;
    case PROTOCOL_TYPE_ASSET_COMMIT:
        dispatch_asset_commit(context, frame->request_id);
        break;
    case PROTOCOL_TYPE_ASSET_RELEASE:
        dispatch_asset_release(context, frame->request_id);
        break;
    case PROTOCOL_TYPE_PUSH_SCENE:
        dispatch_push_scene(context, frame->request_id);
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
        // Unreachable: the gate above admits exactly the types this switch
        // handles. It answers rather than falling through silently because
        // the two lists are maintained in different files, and the failure
        // of a type the gate calls DISPATCHABLE with no arm here is a host
        // waiting forever for a reply that is never coming -- the hardest
        // possible symptom to trace back to a missing `case`. A reply the
        // host can see is worth the four lines.
        ESP_LOGE(TAG, "no handler for dispatchable message type %u",
                 (unsigned)context->message.type);
        transmit_error(context, frame->request_id, PROTOCOL_ERROR_INTERNAL,
                       "no handler for this message type");
        break;
    }
}

static void handle_frame(protocol_frame_result_t result,
                         const protocol_frame_t *frame,
                         protocol_context_t *context,
                         const link_transport_t *response_transport,
                         bool restricted_usb)
{
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
    context->response_transport = response_transport;
    dispatch_request(context, frame, restricted_usb);
    context->response_transport = s_transport;
}

static void frame_callback(protocol_frame_result_t result,
                           const protocol_frame_t *frame,
                           void *opaque)
{
    handle_frame(result, frame, opaque, s_transport,
                 s_owner_usb_restricted);
}

static void restricted_usb_frame_callback(protocol_frame_result_t result,
                                          const protocol_frame_t *frame,
                                          void *opaque)
{
    handle_frame(result, frame, opaque, usb_link_transport(), true);
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
        s_transport->write_frame(context->wire, wire_length,
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
        !show_carousel_fallback(context)) {
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
        shown = show_interrupt_fallback(context);
    } else if (dismissal.restore_saved_screen &&
               widget_model_activate_screen(&context->model,
                                             dismissal.saved_screen_id)) {
        shown = show_carousel_fallback(context);
    }
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
        size_t received = s_transport->read(
            chunk, sizeof(chunk), pdMS_TO_TICKS(PROTOCOL_READ_TIMEOUT_MS));
        if (atomic_exchange_explicit(&s_network_decoder_reset_requested,
                                     false, memory_order_acq_rel)) {
            protocol_decoder_init(&context->decoder);
            abort_asset_transfers(context);
        }
        if (received != 0U) {
            protocol_decoder_feed(&context->decoder, chunk, received,
                                  frame_callback, context);
        }
        if (s_transport != usb_link_transport()) {
            size_t usb_received = usb_link_read(chunk, sizeof(chunk), 0U);
            if (usb_received != 0U) {
                protocol_decoder_feed(&context->restricted_usb_decoder,
                                      chunk, usb_received,
                                      restricted_usb_frame_callback, context);
            }
        }
        process_device_events(context);
        // Pumps the UI-facing half of an SNTP time sync. Must run on this
        // task (see wifi_station_poll()'s doc comment) -- the SNTP
        // notification callback runs on the unpinned lwIP tcpip task, which
        // is not safe to let touch ui_runtime's spinlocks directly.
        wifi_station_poll();
        if (link_state_poll(&context->link, uptime_ms())) {
            abort_asset_transfers(context);
            ui_runtime_set_online(false);
            ESP_LOGI(TAG, "link standalone after timeout");
        }
        // ui/scene_view.c owns no timer of its own by design -- it is
        // compiled into the host simulator, where an LVGL timer would be a
        // second clock the parity gate has to agree with. So the tick is
        // here, on the one task that already owns every value a binding
        // reads.
        if (context->scene_live &&
            uptime_ms() - context->scene_tick_ms >= PROTOCOL_SCENE_TICK_MS) {
            context->scene_tick_ms = uptime_ms();
            context->scene_live = refresh_scene_bindings(context, false);
        }
        refresh_ota_snapshots(context);
    }
}

esp_err_t protocol_task_start(void)
{
    ESP_RETURN_ON_FALSE(s_task == NULL, ESP_ERR_INVALID_STATE, TAG,
                        "protocol task already running");
    // Allocate before anything else touches the context (including
    // net_store_load() below): a failure here must be reported through this
    // function's return value rather than left to be discovered by some
    // later NULL dereference. heap_caps_calloc() already zero-fills, so no
    // separate memset() is needed. On any later failure in this function the
    // allocation is released so a caller that retries protocol_task_start()
    // (main.c does not today, but nothing here should assume that) doesn't
    // leak it.
    s_context = heap_caps_calloc(1, sizeof(*s_context), MALLOC_CAP_SPIRAM);
    ESP_RETURN_ON_FALSE(s_context != NULL, ESP_ERR_NO_MEM, TAG,
                        "allocate protocol context from PSRAM");
    volatile_asset_store_callbacks_t volatile_callbacks = {
        .allocate = volatile_psram_allocate,
        .deallocate = volatile_psram_deallocate,
        .digest_matches = volatile_sha256_matches,
        .ctx = NULL,
    };
    if (volatile_asset_store_init(&s_context->volatile_assets,
                                  &volatile_callbacks) !=
        VOLATILE_ASSET_STORE_OK) {
        heap_caps_free(s_context);
        s_context = NULL;
        return ESP_ERR_INVALID_STATE;
    }
    // The digest -> mapped-bytes lookup that `image` and asset-font scene
    // nodes read through. The same protocol_asset_resolver/asset_flash_unmap
    // pair font_registry_init() gets below, because they are the same
    // question asked by two callers -- ui/scene_view.c cannot ask it itself,
    // having no ESP-IDF include.
    //
    // Installed unconditionally and before anything can push a scene, not
    // folded into the asset_flash_init() success branch below: until this
    // call lands, scene_view_show() refuses ANY scene containing an image
    // node, whole, and says nothing about why. A resolver over a store that
    // failed to open refuses the same scene with the same outcome, but by a
    // path that is visible in the asset-store logs. Two failure modes that
    // look identical from the panel and differ entirely in diagnosability;
    // this picks the diagnosable one. lvgl-sim wires its own pair for
    // exactly the same reason (crates/lvgl-sim/csrc/sim_shim.c).
    //
    // No lvgl_port_lock(): this stores two function pointers and touches no
    // LVGL object, and it runs before the protocol task exists, so nothing
    // can be reading them yet.
    scene_view_set_asset_resolver(protocol_asset_resolver, asset_flash_unmap);
    // Asset store and font registry are independent of the network/USB
    // transport decided below, and their failure is not fatal to the rest
    // of the device: main.c's standalone-clock contract must survive a
    // missing or corrupt assets partition just as it survives a missing
    // network. A failed asset_flash_init() leaves asset_flash_store() with
    // no opened store, so every asset_store_* call downstream fails cleanly
    // (ASSET_STORE_ERR_ARGUMENT) rather than touching unopened state, and
    // StatusResponse key 31 is omitted per its own doc comment above.
    esp_err_t asset_flash_result = asset_flash_init();
    if (asset_flash_result != ESP_OK) {
        ESP_LOGW(TAG, "asset flash init failed: %s",
                esp_err_to_name(asset_flash_result));
    } else {
        // The font registry's LRU table is not glyph data but is larger
        // than this file wants to add to internal-RAM .bss (font_registry.h),
        // so it is allocated here, once, from PSRAM and handed in; the
        // registry keeps the pointer itself (font_registry.c's own static),
        // not this task's context, so nothing here needs to retain it.
        size_t font_table_bytes = font_registry_table_bytes();
        void *font_table_storage =
            heap_caps_malloc(font_table_bytes, MALLOC_CAP_SPIRAM);
        if (font_table_storage == NULL) {
            ESP_LOGW(TAG, "font registry table allocation failed");
        } else if (font_registry_init(protocol_asset_resolver, asset_flash_unmap,
                                      font_table_storage,
                                      font_table_bytes) != FONT_REGISTRY_OK) {
            heap_caps_free(font_table_storage);
            ESP_LOGW(TAG, "font registry init failed");
        }
    }
    // Populate net_store_current_tier()'s cache once, synchronously, before
    // any USB message can reach its reader gate. The loaded config itself
    // isn't needed here -- only its side effect on the tier cache -- so
    // zero it immediately after: it briefly held a PSK and a token, and
    // this stack region must not keep carrying them once its job is done.
    protocol_network_config_t boot_network_config;
    net_store_load(&boot_network_config);
    memset(&boot_network_config, 0, sizeof(boot_network_config));
    atomic_store_explicit(&s_network_decoder_reset_requested, false,
                          memory_order_relaxed);
    if (net_store_current_tier() == PROTOCOL_TIER_NETWORKED &&
        net_link_start() == ESP_OK) {
        s_transport = net_link_transport();
    } else {
        s_transport = usb_link_transport();
    }
    s_owner_usb_restricted =
        net_store_current_tier() == PROTOCOL_TIER_NETWORKED &&
        s_transport == usb_link_transport();
    s_context->response_transport = s_transport;
    protocol_decoder_init(&s_context->decoder);
    protocol_decoder_init(&s_context->restricted_usb_decoder);
    link_state_init_with_timeout(&s_context->link, s_transport->link_timeout_ms);
    widget_model_init(&s_context->model);
    interrupt_state_init(&s_context->interrupts);
    refresh_ota_snapshots(s_context);
    device_event_queue_init(&s_context->events);
    BaseType_t created = xTaskCreatePinnedToCore(
        protocol_task, "protocol", PROTOCOL_TASK_STACK_SIZE, s_context,
        PROTOCOL_TASK_PRIORITY, &s_task, PROTOCOL_TASK_CORE);
    if (created != pdPASS) {
        volatile_asset_store_destroy(&s_context->volatile_assets);
        heap_caps_free(s_context);
        s_context = NULL;
        ESP_LOGE(TAG, "create protocol task failed");
        return ESP_ERR_NO_MEM;
    }
    /* Published only after the context is certain to outlive it. carousel.c
     * keeps this pointer in a static and dereferences it from the LVGL touch
     * path, and main.c deliberately continues standalone -- display and touch
     * alive -- when this function fails. Handing the queue over before the
     * task exists would leave that static aimed at the PSRAM block freed
     * above, so the next tap would write into freed memory. Ordering it here
     * makes that unreachable by construction rather than by remembering to
     * clear the pointer on each new failure path. */
    ui_runtime_set_event_queue(&s_context->events);
    return ESP_OK;
}

bool protocol_task_is_running(void)
{
    return s_task != NULL;
}

void protocol_task_reset_network_decoder(void)
{
    atomic_store_explicit(&s_network_decoder_reset_requested, true,
                          memory_order_release);
}

bool protocol_task_ota_blocked(void)
{
    return atomic_load_explicit(&s_ota_blocked, memory_order_acquire);
}

bool protocol_task_owner_state_ready(void)
{
    return atomic_load_explicit(&s_owner_state_ready,
                                memory_order_acquire);
}
