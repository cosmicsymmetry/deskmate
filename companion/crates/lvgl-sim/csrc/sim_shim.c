#include "sim_shim.h"

#include <string.h>

#include "lvgl.h"
#include "core/clock_source.h"
#include "core/template_fields.h"
#include "ui/template_view.h"

#define SIM_WIDTH 448
#define SIM_HEIGHT 368

static lv_display_t *s_display;
static uint16_t s_frame[SIM_WIDTH * SIM_HEIGHT];
static uint32_t s_fake_tick;

static uint32_t sim_tick_cb(void) { return s_fake_tick; }

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
    clock_source_clear_override();
    return true;
}
