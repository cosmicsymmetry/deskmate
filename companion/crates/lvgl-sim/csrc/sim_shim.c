#include "sim_shim.h"

#include <stdlib.h>
#include <string.h>

#include "lvgl.h"
#include "core/asset_store.h"
#include "core/clock_source.h"
#include "core/scene_binding.h"
#include "core/scene_model.h"
#include "core/template_fields.h"
#include "ui/font_registry.h"
#include "ui/scene_view.h"
#include "ui/template_view.h"

#define SIM_WIDTH 448
#define SIM_HEIGHT 368

static lv_display_t *s_display;
static uint16_t s_frame[SIM_WIDTH * SIM_HEIGHT];
static uint32_t s_fake_tick;

static uint32_t sim_tick_cb(void) { return s_fake_tick; }

/* Shared by sim_render and sim_render_asset_font: copies the just-flushed
 * frame out, applying the 180° flip that models the 270° mount. */
static void copy_frame_out(bool orientation_flipped, uint16_t *out_pixels)
{
    if (orientation_flipped) {
        /* The firmware's "flipped" mount is a 180° rotation of the logical
         * canvas (LV_DISPLAY_ROTATION_90 vs 270 on the physical panel). */
        for (size_t index = 0; index < (size_t)SIM_WIDTH * SIM_HEIGHT; ++index) {
            out_pixels[index] =
                s_frame[(size_t)SIM_WIDTH * SIM_HEIGHT - 1 - index];
        }
    } else {
        memcpy(out_pixels, s_frame, sizeof(s_frame));
    }
}

static void sim_flush_cb(lv_display_t *display, const lv_area_t *area,
                         uint8_t *px_map)
{
    /* FULL render mode: px_map is the whole frame. */
    (void)area;
    memcpy(s_frame, px_map, sizeof(s_frame));
    lv_display_flush_ready(display);
}

bool sim_init(void)
{
    if (s_display != NULL) {
        return true;
    }
    lv_init();
    /* Templates that capture lv_tick_get() while being built or patched
     * (e.g. progress_ring's progress_anchor_ms) read s_fake_tick *before*
     * sim_render's own "round up to the next 1000ms boundary" runs. After
     * render N, s_fake_tick is left at boundary_N + 160 (four +40 ticks past
     * the boundary), so render N+1 captures its anchor at boundary_N + 160
     * and then rounds to boundary_N + 1000 — a fixed 840-tick offset between
     * anchor and boundary on every render but the first. s_fake_tick starts
     * at 0, not "boundary + 160", so the first render's anchor/boundary
     * offset was 1000, not 840: an order-dependence exception for whichever
     * case happened to run first. Starting one boundary in at 160 makes the
     * first render match the steady state instead. */
    s_fake_tick = 160U;
    lv_tick_set_cb(sim_tick_cb);
    s_display = lv_display_create(SIM_WIDTH, SIM_HEIGHT);
    if (s_display == NULL) {
        return false;
    }
    static uint16_t draw_buffer[SIM_WIDTH * SIM_HEIGHT];
    lv_display_set_buffers(s_display, draw_buffer, NULL,
                           sizeof(draw_buffer),
                           LV_DISPLAY_RENDER_MODE_FULL);
    lv_display_set_flush_cb(s_display, sim_flush_cb);
    lv_display_set_color_format(s_display, LV_COLOR_FORMAT_RGB565);
    return true;
}

static bool build_fields(int template_kind, const sim_field_t *fields,
                         size_t field_count, template_field_state_t *state)
{
    if (!template_fields_init((protocol_template_kind_t)template_kind, state)) {
        return false;
    }
    /* Overwrite defaults directly: the registry validated shape lives in
     * template_fields_resolve, but the sim pins values without a PushData
     * round-trip. TEXT fields stay bounded because template_field_value_t
     * text is a fixed buffer (see the strncpy below), but INTEGER fields
     * have no such backstop here — keeping each face's displayed value
     * inside its documented range is that face's own display-layer
     * responsibility (see progress_ring.c's patch-entry clamp for an
     * example), not something this shim or the registry enforces. */
    size_t registry_count = 0;
    const template_field_descriptor_t *registry = template_fields_registry(
        (protocol_template_kind_t)template_kind, &registry_count);
    for (size_t input = 0; input < field_count; ++input) {
        for (size_t slot = 0; slot < registry_count; ++slot) {
            if (strcmp(registry[slot].name, fields[input].name) != 0) {
                continue;
            }
            template_field_value_t *value = &state->values[slot];
            if (fields[input].type == 0) {
                strncpy(value->value.text, fields[input].text,
                        PROTOCOL_MAX_FIELD_TEXT_LENGTH);
                value->value.text[PROTOCOL_MAX_FIELD_TEXT_LENGTH] = '\0';
            } else if (fields[input].type == 1) {
                value->value.integer = fields[input].integer;
            } else {
                value->value.boolean = fields[input].boolean;
            }
            break;
        }
    }
    return true;
}

bool sim_render(int template_kind, const sim_field_t *fields,
                size_t field_count, int16_t utc_offset_minutes,
                int64_t now_unix_seconds, bool orientation_flipped,
                uint16_t *out_pixels)
{
    if (!sim_init() || out_pixels == NULL) {
        return false;
    }
    clock_source_set_override(now_unix_seconds);
    template_field_state_t state;
    if (!build_fields(template_kind, fields, field_count, &state)) {
        clock_source_clear_override();
        return false;
    }
    template_view_set_utc_offset_minutes(utc_offset_minutes);
    if (!template_view_show((protocol_template_kind_t)template_kind,
                            PROTOCOL_SIZE_FULL, &state)) {
        clock_source_clear_override();
        return false;
    }
    /* Advance fake time so LVGL runs its refresh timer.
     *
     * Start every render on a 1000ms boundary. The tick counter is process
     * global and monotonic (LVGL timers must never see it move backwards),
     * so without this the phase a render begins at depends on how many
     * renders preceded it — and the progress ring, whose arc value is
     * derived from elapsed ticks since its patch, would then produce
     * different pixels purely because a case was added earlier in the
     * table. Pinning the phase makes every golden independent of case
     * ordering. */
    s_fake_tick = (s_fake_tick / 1000U + 1U) * 1000U;
    for (int cycle = 0; cycle < 4; ++cycle) {
        s_fake_tick += 40;
        lv_timer_handler();
    }
    copy_frame_out(orientation_flipped, out_pixels);
    clock_source_clear_override();
    return true;
}

/* ---------------------------------------------------------------------
 * Task 12: RAM-backed asset store shim.
 *
 * The spec's parity obligation (see the module doc in cases.rs and the
 * task-12 brief) is that the simulator resolves the same digest to the
 * same bytes the device would. On device that path is
 * link/asset_flash.c's esp_partition-backed asset_flash_io_t feeding
 * core/asset_store.c, with link/protocol_task.c's protocol_asset_resolver
 * bridging asset_store_find()+asset_flash_map() into
 * font_registry_init()'s asset_resolver_fn (see font_registry.h's own
 * comment on why that indirection exists: ui/font_registry.c carries no
 * ESP-IDF include specifically so it can compile here).
 *
 * This shim reuses the *same* core/asset_store.c unmodified (see build.rs),
 * backed by a plain heap buffer instead of esp_partition_*, exactly like
 * firmware/host_tests/test_asset_store.c's fake flash. That is the whole
 * point: asset_store_format/open/reserve/commit/find behave identically to
 * the device's, so a digest registered here resolves through the identical
 * on-flash record format the device would decode -- only the I/O
 * (RAM vs NOR flash) differs, not the format or the lookup.
 * --------------------------------------------------------------------- */

/* 64 KiB, matching test_asset_store.c's FAKE_SIZE -- ample for the handful
 * of small test fonts this shim ever registers (the committed golden asset
 * is ~4 KiB; see assets.rs). */
#define SIM_ASSET_FLASH_BYTES (64U * 1024U)
#define SIM_ASSET_RECORD_CAPACITY 8U

static uint8_t s_asset_flash[SIM_ASSET_FLASH_BYTES];
static asset_store_t s_asset_store;
static bool s_asset_store_ready;

static int sim_asset_io_read(void *ctx, uint32_t offset, void *out, size_t length)
{
    (void)ctx;
    if ((size_t)offset + length > SIM_ASSET_FLASH_BYTES) {
        return -1;
    }
    memcpy(out, s_asset_flash + offset, length);
    return 0;
}

static int sim_asset_io_write(void *ctx, uint32_t offset, const void *data,
                              size_t length)
{
    (void)ctx;
    if ((size_t)offset + length > SIM_ASSET_FLASH_BYTES) {
        return -1;
    }
    /* NOR semantics: writes only clear bits. Matches
     * firmware/main/link/asset_flash.c's io_write and
     * test_asset_store.c's fake_write -- core/asset_store.c's commit-bit
     * trick depends on this being faithfully modelled, not a plain
     * memcpy. */
    const uint8_t *src = data;
    for (size_t i = 0; i < length; ++i) {
        s_asset_flash[offset + i] &= src[i];
    }
    return 0;
}

static int sim_asset_io_erase(void *ctx, uint32_t offset, size_t length)
{
    (void)ctx;
    if ((size_t)offset + length > SIM_ASSET_FLASH_BYTES) {
        return -1;
    }
    memset(s_asset_flash + offset, 0xFF, length);
    return 0;
}

static const asset_flash_io_t s_sim_asset_io = {
    .read = sim_asset_io_read,
    .write = sim_asset_io_write,
    .erase = sim_asset_io_erase,
    .ctx = NULL,
};

static bool ensure_asset_store(void)
{
    if (s_asset_store_ready) {
        return true;
    }
    if (asset_store_format(&s_sim_asset_io, SIM_ASSET_FLASH_BYTES,
                           SIM_ASSET_RECORD_CAPACITY) != ASSET_STORE_OK) {
        return false;
    }
    if (asset_store_open(&s_asset_store, &s_sim_asset_io, SIM_ASSET_FLASH_BYTES) !=
        ASSET_STORE_OK) {
        return false;
    }
    s_asset_store_ready = true;
    return true;
}

/* Inserts a committed record plus its blob for `digest`, if not already
 * present. A second registration of the same digest is a harmless no-op
 * (not an error) so a golden case that registers its font on every render
 * -- once per orientation -- does not have to track whether this is the
 * first call. */
bool sim_asset_register(const uint8_t *digest, const uint8_t *bytes, uint32_t len,
                        uint8_t kind)
{
    if (digest == NULL || bytes == NULL || len == 0U) {
        return false;
    }
    if (!ensure_asset_store()) {
        return false;
    }

    asset_record_t existing;
    if (asset_store_find(&s_asset_store, digest, &existing, NULL) == ASSET_STORE_OK) {
        return true;
    }

    uint32_t index = 0U;
    uint32_t blob_offset = 0U;
    if (asset_store_reserve(&s_asset_store, digest, kind, len, &index,
                            &blob_offset) != ASSET_STORE_OK) {
        return false;
    }
    if (sim_asset_io_write(NULL, s_asset_store.blob_region_offset + blob_offset,
                           bytes, len) != 0) {
        return false;
    }
    if (asset_store_commit(&s_asset_store, index) != ASSET_STORE_OK) {
        return false;
    }
    return true;
}

/* font_registry_init()'s asset_resolver_fn. Mirrors
 * link/protocol_task.c's protocol_asset_resolver exactly, except the
 * mapped pointer comes straight out of the RAM buffer above instead of
 * link/asset_flash.c's esp_partition_mmap -- there is no flash to map on
 * the host, and s_asset_flash already IS the "mapped" bytes. */
static bool sim_asset_resolver(const uint8_t *digest, const void **out_ptr,
                               uint32_t *out_len, uint8_t *out_kind)
{
    if (!s_asset_store_ready) {
        return false;
    }
    asset_record_t record;
    if (asset_store_find(&s_asset_store, digest, &record, NULL) != ASSET_STORE_OK) {
        return false;
    }
    *out_ptr = s_asset_flash + s_asset_store.blob_region_offset + record.offset;
    *out_len = record.length;
    *out_kind = record.kind;
    return true;
}

/* font_registry_init()'s asset_release_fn. link/asset_flash.c's counterpart
 * (asset_flash_unmap) gives back an esp_partition_mmap() handle; the
 * resolver above never created one -- it handed back a direct pointer into
 * s_asset_flash, which is not a mapping and needs no unmapping. Still wired
 * in (rather than passing NULL) so the sim exercises the same
 * acquire/evict/release call pattern real firmware does. */
static void sim_asset_release(const void *ptr)
{
    (void)ptr;
}

static void *s_font_registry_table;
static bool s_font_registry_ready;

static bool ensure_font_registry(void)
{
    if (s_font_registry_ready) {
        return true;
    }
    size_t table_bytes = font_registry_table_bytes();
    s_font_registry_table = malloc(table_bytes);
    if (s_font_registry_table == NULL) {
        return false;
    }
    if (font_registry_init(sim_asset_resolver, sim_asset_release,
                           s_font_registry_table, table_bytes) !=
        FONT_REGISTRY_OK) {
        free(s_font_registry_table);
        s_font_registry_table = NULL;
        return false;
    }
    s_font_registry_ready = true;
    return true;
}

/* Drops a screen's hold on its registry face when that screen is deleted --
 * the mirror of dev_capture.c's release_probe_font(). */
static void release_registry_font(lv_event_t *event)
{
    font_registry_release((lv_font_t *)lv_event_get_user_data(event));
}

bool sim_render_asset_font(const uint8_t *digest, const uint8_t *ttf_bytes,
                           uint32_t ttf_len, int32_t pixel_size, const char *text,
                           bool orientation_flipped, uint16_t *out_pixels)
{
    if (!sim_init() || out_pixels == NULL || digest == NULL || ttf_bytes == NULL ||
        text == NULL) {
        return false;
    }
    if (!sim_asset_register(digest, ttf_bytes, ttf_len, (uint8_t)ASSET_KIND_FONT)) {
        return false;
    }
    if (!ensure_font_registry()) {
        return false;
    }

    lv_font_t *font = font_registry_acquire(digest, pixel_size);
    if (font == NULL) {
        return false;
    }
    font_registry_warm(font, text);

    /* A bare screen + centred label, deliberately not template_view.c's
     * card chrome: this golden pins raw font rendering parity, not any
     * one template's layout. Colours match template_style.c's canvas/
     * primary-text convention (DESKMATE_COLOR_CANVAS / _PRIMARY) so the
     * PNG still reads as a plausible Deskmate frame. */
    lv_obj_t *screen = lv_obj_create(NULL);
    if (screen == NULL) {
        font_registry_release(font);
        return false;
    }
    lv_obj_remove_flag(screen, LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_style_bg_color(screen, lv_color_hex(0x000000), 0);
    lv_obj_set_style_bg_opa(screen, LV_OPA_COVER, 0);

    lv_obj_t *label = lv_label_create(screen);
    lv_obj_remove_style_all(label);
    lv_obj_set_style_text_font(label, font, 0);
    lv_obj_set_style_text_color(label, lv_color_hex(0xf5f5f7), 0);
    lv_label_set_text(label, text);
    lv_obj_center(label);

    /* Hand this render's acquire to the screen, mirroring the device probe
     * (firmware/main/link/dev_capture.c). LVGL stores the bare lv_font_t* in
     * the label's style and takes no reference of its own, so releasing
     * inline would leave the face unpinned -- and therefore evictable -- for
     * as long as this screen stays the active one. font_registry.h's
     * ownership contract calls for the release to happen when the objects
     * styled with the face are destroyed, which for a screen is its delete
     * event. The auto_del below is what fires it, on the next render. */
    lv_obj_add_event_cb(screen, release_registry_font, LV_EVENT_DELETE, font);

    /* auto_del=true deletes whatever screen was previously active (a prior
     * template render or asset-font render), exactly like
     * template_view_show's own lv_screen_load_anim call. */
    lv_screen_load_anim(screen, LV_SCREEN_LOAD_ANIM_NONE, 0U, 0U, true);

    s_fake_tick = (s_fake_tick / 1000U + 1U) * 1000U;
    for (int cycle = 0; cycle < 4; ++cycle) {
        s_fake_tick += 40;
        lv_timer_handler();
    }

    copy_frame_out(orientation_flipped, out_pixels);
    return true;
}

/* ---------------------------------------------------------------------
 * Task 8 (stage 2a): scene rendering.
 *
 * The whole of the simulator's scene support. It adds no rendering logic
 * of its own -- ui/scene_view.c does all of it, compiled from the
 * firmware tree by build.rs -- only the three things the host has no
 * equivalent of: somewhere to put a decoded scene_t, a `field.` lookup,
 * and the LVGL binary image wrapper an `image` node reads.
 * --------------------------------------------------------------------- */

/* The decoded scene. A file-scope static rather than a local because
 * sizeof(scene_t) is ~6 KB -- the same reason link/protocol_task.c
 * PSRAM-allocates its copy rather than putting one on a task stack (see
 * scene_model.h's scene_decode() comment). Nothing here is reentrant
 * anyway: src/lib.rs's SIMULATOR_CLAIMED already pins the whole shim to
 * one caller at a time. */
static scene_t s_scene;

typedef struct {
    const sim_scene_field_t *fields;
    size_t count;
} sim_scene_field_table_t;

/* scene_binding.h's scene_field_fn. Returning NULL for an unknown name is
 * not an error: scene_binding_evaluate() renders the "--" placeholder for
 * it, which is exactly what the device does for a provider that has not
 * reported yet, and a golden that pins that state is worth having. */
static const char *sim_scene_field_lookup(void *ctx, const char *name)
{
    const sim_scene_field_table_t *table = (const sim_scene_field_table_t *)ctx;
    if (table == NULL || name == NULL) {
        return NULL;
    }
    for (size_t i = 0; i < table->count; ++i) {
        if (table->fields[i].name != NULL &&
            strcmp(table->fields[i].name, name) == 0) {
            return table->fields[i].value;
        }
    }
    return NULL;
}

size_t sim_build_rgb565_image(int32_t width, int32_t height,
                              const uint16_t *pixels, uint8_t *out,
                              size_t out_capacity)
{
    if (width <= 0 || height <= 0 || pixels == NULL || out == NULL) {
        return 0U;
    }
    size_t stride = (size_t)width * sizeof(uint16_t);
    size_t data_bytes = stride * (size_t)height;
    size_t total = sizeof(lv_image_header_t) + data_bytes;
    if (out_capacity < total) {
        return 0U;
    }

    lv_image_header_t header;
    lv_memzero(&header, sizeof header);
    header.magic = LV_IMAGE_HEADER_MAGIC;
    header.cf = LV_COLOR_FORMAT_RGB565;
    header.flags = 0U;
    header.w = (uint32_t)width;
    header.h = (uint32_t)height;
    header.stride = (uint32_t)stride;

    memcpy(out, &header, sizeof header);
    memcpy(out + sizeof header, pixels, data_bytes);
    return total;
}

/* Maps scene_decode()'s scene_model_result_t onto the SIM_SCENE_ERR_DECODE_*
 * subrange, in the same order the two enums are declared in -- see
 * sim_scene_result_t's own comment in sim_shim.h for why this is not folded
 * into one code. */
static sim_scene_result_t map_decode_result(scene_model_result_t result)
{
    switch (result) {
    case SCENE_MODEL_OK:
        return SIM_SCENE_OK;
    case SCENE_MODEL_ERR_ARGUMENT:
        return SIM_SCENE_ERR_DECODE_ARGUMENT;
    case SCENE_MODEL_ERR_NODE_COUNT:
        return SIM_SCENE_ERR_DECODE_NODE_COUNT;
    case SCENE_MODEL_ERR_NODE_KIND:
        return SIM_SCENE_ERR_DECODE_NODE_KIND;
    case SCENE_MODEL_ERR_GEOMETRY:
        return SIM_SCENE_ERR_DECODE_GEOMETRY;
    case SCENE_MODEL_ERR_TEXT:
        return SIM_SCENE_ERR_DECODE_TEXT;
    case SCENE_MODEL_ERR_FONT:
        return SIM_SCENE_ERR_DECODE_FONT;
    default:
        return SIM_SCENE_ERR_DECODE_ARGUMENT;
    }
}

/* Advances the fake tick by one boundary-plus-four-ticks phase, shared by
 * every renderer that must run LVGL's refresh timer at a golden-stable
 * instant -- see sim_init()'s comment for why the phase itself matters. */
static void advance_fake_tick_phase(void)
{
    s_fake_tick = (s_fake_tick / 1000U + 1U) * 1000U;
    for (int cycle = 0; cycle < 4; ++cycle) {
        s_fake_tick += 40;
        lv_timer_handler();
    }
}

sim_scene_result_t sim_render_scene(const uint8_t *payload, size_t payload_length,
                                    int16_t utc_offset_minutes, int64_t now_unix_seconds,
                                    bool timer_active, uint32_t timer_remaining_ms,
                                    uint8_t timer_pct, const sim_scene_field_t *fields,
                                    size_t field_count, bool orientation_flipped,
                                    uint16_t *out_pixels)
{
    if (payload == NULL || payload_length == 0U || out_pixels == NULL) {
        return SIM_SCENE_ERR_ARGUMENT;
    }
    if (!sim_init()) {
        return SIM_SCENE_ERR_SETUP;
    }
    /* Both stores are wired unconditionally, not only when a scene happens
     * to name an asset: whether this scene has an image or an asset-font
     * node is the payload's business, and a renderer that silently refused
     * one because the host had not pre-armed the right lookup would be a
     * confusing failure to debug. */
    if (!ensure_asset_store() || !ensure_font_registry()) {
        return SIM_SCENE_ERR_SETUP;
    }
    /* scene_view.c refuses any scene containing an `image` node until this
     * is called (see scene_view_set_asset_resolver's own comment). It reads
     * the same RAM-backed store font_registry_init() was given above, which
     * mirrors the device: link/protocol_task.c hands its one
     * asset_flash-backed protocol_asset_resolver to both. */
    scene_view_set_asset_resolver(sim_asset_resolver, sim_asset_release);

    scene_model_result_t decode_result =
        scene_decode(payload, payload_length, &s_scene);
    if (decode_result != SCENE_MODEL_OK) {
        return map_decode_result(decode_result);
    }

    sim_scene_field_table_t table = {
        .fields = fields,
        .count = (fields == NULL) ? 0U : field_count,
    };
    scene_binding_context_t context = {
        .unix_seconds = now_unix_seconds,
        .utc_offset_minutes = utc_offset_minutes,
        .timer_active = timer_active,
        .timer_remaining_ms = timer_remaining_ms,
        .timer_pct = timer_pct,
        .field = sim_scene_field_lookup,
        .field_ctx = &table,
    };
    /* Every binding is evaluated inside this call, so `context` and the
     * table it points at only have to outlive it. */
    if (!scene_view_show(&s_scene, &context)) {
        return SIM_SCENE_ERR_SHOW;
    }

    /* The same fixed tick phase sim_render pins, for the same reason: the
     * counter is process-global and monotonic, so without this a scene
     * golden would depend on how many renders ran before it. */
    advance_fake_tick_phase();
    copy_frame_out(orientation_flipped, out_pixels);
    return SIM_SCENE_OK;
}
