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
    uint32_t remaining_ms;
    uint8_t remaining_pct;
} scene_timer_snapshot_t;

typedef struct {
    int64_t unix_seconds;
    int16_t utc_offset_minutes;
    bool timer_active;
    uint32_t timer_remaining_ms;
    uint8_t timer_remaining_pct;
    scene_field_fn field;
    void *field_ctx;
} scene_binding_context_t;

scene_timer_snapshot_t scene_timer_snapshot(int64_t duration_seconds,
                                            int64_t remaining_seconds,
                                            bool running,
                                            uint64_t anchor_ms,
                                            uint64_t now_ms);

scene_binding_result_t scene_binding_parse(const char *text,
                                           scene_binding_t *out);
scene_binding_result_t scene_binding_evaluate(
    const scene_binding_t *binding, const scene_binding_context_t *context,
    char *out, size_t out_capacity);
