#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "core/scene_model.h"

typedef enum {
    SCENE_BINDING_TIME = 1,
    SCENE_BINDING_TIMER_REMAINING = 2,
    SCENE_BINDING_TIMER_PCT = 3,
    SCENE_BINDING_FIELD = 4,
    SCENE_BINDING_TIMER_ELAPSED = 5,
    SCENE_BINDING_TIMER_TOTAL = 6,
    SCENE_BINDING_TIMER_STATUS = 7,
    SCENE_BINDING_TIMER_PERMILLE = 8,
    SCENE_BINDING_DATE = 9,
} scene_binding_kind_t;

typedef enum {
    SCENE_BINDING_OK = 0,
    SCENE_BINDING_ERR_ARGUMENT,
    SCENE_BINDING_ERR_UNKNOWN,
    SCENE_BINDING_ERR_FORMAT,
    SCENE_BINDING_ERR_CAPACITY,
} scene_binding_result_t;

/* Returns the field's current text, or NULL when the provider has not
 * reported one yet. Injected rather than called directly so this module
 * stays pure and host-testable. */
typedef const char *(*scene_field_fn)(void *ctx, const char *name);

typedef struct {
    scene_binding_kind_t kind;
    char argument[SCENE_MAX_BINDING + 1U];
} scene_binding_t;

typedef struct {
    uint32_t total_ms;
    uint32_t remaining_ms;
    uint8_t remaining_pct;
    uint16_t remaining_permille;
    uint32_t anchor_ms;
    bool running;
} scene_timer_snapshot_t;

typedef enum {
    SCENE_TIMER_LOCAL_ACTION_START_PAUSE = 1,
    SCENE_TIMER_LOCAL_ACTION_RESET = 2,
} scene_timer_local_action_t;

typedef struct {
    int64_t unix_seconds;
    int16_t utc_offset_minutes;
    bool timer_active;
    bool timer_running;
    uint32_t timer_total_ms;
    uint32_t timer_remaining_ms;
    uint8_t timer_remaining_pct;
    uint16_t timer_remaining_permille;
    scene_field_fn field;
    void *field_ctx;
} scene_binding_context_t;

typedef int32_t (*scene_trigo_fn)(int32_t degrees);

typedef struct {
    int32_t x;
    int32_t y;
} scene_line_endpoint_t;

scene_timer_snapshot_t scene_timer_snapshot(int64_t duration_seconds,
                                            int64_t remaining_seconds,
                                            bool running,
                                            uint64_t anchor_ms,
                                            uint64_t now_ms);

/* Advances an already-normalized millisecond snapshot to `now_ms`. The
 * uint32 subtraction is deliberate: LVGL's tick counter wraps, while a
 * timer is bounded to one day and therefore cannot span a full wrap. */
scene_timer_snapshot_t scene_timer_snapshot_at(
    scene_timer_snapshot_t snapshot, uint32_t now_ms);

/* Applies the same optimistic transition the ProgressRing uses. The host's
 * next full snapshot remains authoritative; this helper owns arithmetic,
 * not reconciliation policy. */
void scene_timer_apply_local_action(scene_timer_snapshot_t *snapshot,
                                    scene_timer_local_action_t action,
                                    uint32_t now_ms);

scene_binding_result_t scene_binding_parse(const char *text,
                                           scene_binding_t *out);
scene_binding_result_t scene_binding_evaluate(
    const scene_binding_t *binding, const scene_binding_context_t *context,
    char *out, size_t out_capacity);

/* Computes a clock hand's live endpoint. The caller supplies the trig
 * functions so core stays free of LVGL; scene_view.c passes LVGL's own table
 * functions, while the host test passes its pinned port. */
bool scene_binding_line_endpoint(scene_angle_binding_t binding,
                                 const scene_binding_context_t *context,
                                 int32_t pivot_x, int32_t pivot_y,
                                 int32_t length, scene_trigo_fn trigo_cos,
                                 scene_trigo_fn trigo_sin,
                                 scene_line_endpoint_t *out);
