#include "scene_binding.h"

#include <stdio.h>
#include <string.h>
#include <time.h>

#include "timefmt.h"

#define TIME_PREFIX "time:"
#define TIMER_REMAINING_PREFIX "timer.remaining:"
#define TIMER_ELAPSED_PREFIX "timer.elapsed:"
#define TIMER_TOTAL_PREFIX "timer.total:"
#define TIMER_PCT_TOKEN "timer.pct"
#define TIMER_PERMILLE_TOKEN "timer.permille"
#define TIMER_STATUS_TOKEN "timer.status"
#define DATE_TOKEN "date"
#define FIELD_PREFIX "field."
#define TIMER_SECONDS_MAX INT64_C(86400)
#define CLOCK_DIAL_ROTATION 270
#define CLOCK_TRIGO_SHIFT 15

static void update_timer_ratios(scene_timer_snapshot_t *snapshot)
{
    if (snapshot->total_ms == 0U) {
        snapshot->remaining_pct = 0U;
        snapshot->remaining_permille = 0U;
        return;
    }
    snapshot->remaining_pct = (uint8_t)(
        ((uint64_t)snapshot->remaining_ms * UINT64_C(100)) /
        snapshot->total_ms);
    snapshot->remaining_permille = (uint16_t)(
        ((uint64_t)snapshot->remaining_ms * UINT64_C(1000)) /
        snapshot->total_ms);
}

/* Keep this producer and lvgl-sim's fill_scene_timer_context() on the same
 * invariant: total is clamped to 86400s, remaining is clamped to total, and
 * both remaining ratios truncate after multiplying. The shim must duplicate
 * that arithmetic because temporal parity advances sub-second millisecond
 * values that this public seconds-in producer cannot represent. */
scene_timer_snapshot_t scene_timer_snapshot(int64_t duration_seconds,
                                            int64_t remaining_seconds,
                                            bool running,
                                            uint64_t anchor_ms,
                                            uint64_t now_ms)
{
    scene_timer_snapshot_t snapshot = {0};
    if (duration_seconds <= 0) {
        return snapshot;
    }
    if (duration_seconds > TIMER_SECONDS_MAX) {
        duration_seconds = TIMER_SECONDS_MAX;
    }
    if (remaining_seconds < 0) {
        remaining_seconds = 0;
    } else if (remaining_seconds > duration_seconds) {
        remaining_seconds = duration_seconds;
    }
    int64_t total_ms = duration_seconds * INT64_C(1000);
    int64_t remaining_ms = remaining_seconds * INT64_C(1000);
    if (running) {
        uint64_t elapsed_ms = now_ms >= anchor_ms ? now_ms - anchor_ms : 0U;
        remaining_ms = elapsed_ms >= (uint64_t)remaining_ms
            ? 0
            : remaining_ms - (int64_t)elapsed_ms;
    }
    if (remaining_ms < 0) {
        remaining_ms = 0;
    } else if (remaining_ms > total_ms) {
        remaining_ms = total_ms;
    }
    snapshot.remaining_ms = (uint32_t)remaining_ms;
    snapshot.total_ms = (uint32_t)total_ms;
    snapshot.anchor_ms = (uint32_t)now_ms;
    snapshot.running = running;
    update_timer_ratios(&snapshot);
    return snapshot;
}

scene_timer_snapshot_t scene_timer_snapshot_at(
    scene_timer_snapshot_t snapshot, uint32_t now_ms)
{
    if (snapshot.running) {
        uint32_t elapsed_ms = now_ms - snapshot.anchor_ms;
        snapshot.remaining_ms = elapsed_ms >= snapshot.remaining_ms
            ? 0U
            : snapshot.remaining_ms - elapsed_ms;
    }
    snapshot.anchor_ms = now_ms;
    update_timer_ratios(&snapshot);
    return snapshot;
}

void scene_timer_apply_local_action(scene_timer_snapshot_t *snapshot,
                                    scene_timer_local_action_t action,
                                    uint32_t now_ms)
{
    if (snapshot == NULL) {
        return;
    }
    if (action == SCENE_TIMER_LOCAL_ACTION_START_PAUSE) {
        if (snapshot->running) {
            *snapshot = scene_timer_snapshot_at(*snapshot, now_ms);
        } else {
            snapshot->anchor_ms = now_ms;
        }
        snapshot->running = !snapshot->running;
    } else if (action == SCENE_TIMER_LOCAL_ACTION_RESET &&
               snapshot->total_ms > 0U) {
        snapshot->remaining_ms = snapshot->total_ms;
        snapshot->anchor_ms = now_ms;
        snapshot->running = false;
    }
    update_timer_ratios(snapshot);
}

/* A format argument is a whitelist, never a printf format string handed to
 * a printf-family function -- host bytes must never reach one of those.
 * Wall clocks admit hours; countdowns deliberately do not, because their
 * `mm` token is already the total-minute count. */
static bool is_time_format_char(char c)
{
    switch (c) {
    case 'H':
    case 'h':
    case 'M':
    case 'm':
    case 'S':
    case 's':
    case ':':
        return true;
    default:
        return false;
    }
}

static bool is_timer_format_char(char c)
{
    switch (c) {
    case 'M':
    case 'm':
    case 'S':
    case 's':
    case ':':
        return true;
    default:
        return false;
    }
}

static scene_binding_result_t store_argument(const char *argument,
                                              scene_binding_kind_t kind,
                                              scene_binding_t *out)
{
    size_t len = strlen(argument);
    if (len > SCENE_MAX_BINDING) {
        return SCENE_BINDING_ERR_FORMAT;
    }
    out->kind = kind;
    memcpy(out->argument, argument, len);
    out->argument[len] = '\0';
    return SCENE_BINDING_OK;
}

static scene_binding_result_t parse_time_like(const char *argument,
                                               scene_binding_kind_t kind,
                                               scene_binding_t *out)
{
    if (argument[0] == '\0') {
        return SCENE_BINDING_ERR_FORMAT;
    }
    for (const char *p = argument; *p != '\0'; p++) {
        bool valid = kind == SCENE_BINDING_TIMER_REMAINING ||
                kind == SCENE_BINDING_TIMER_ELAPSED ||
                kind == SCENE_BINDING_TIMER_TOTAL
            ? is_timer_format_char(*p)
            : is_time_format_char(*p);
        if (!valid) {
            return SCENE_BINDING_ERR_FORMAT;
        }
    }
    return store_argument(argument, kind, out);
}

scene_binding_result_t scene_binding_parse(const char *text,
                                           scene_binding_t *out)
{
    if (text == NULL || out == NULL) {
        return SCENE_BINDING_ERR_ARGUMENT;
    }

    /* Exact-match timer tokens carry no argument. They are checked via
     * strcmp, so a token with a trailing suffix cannot match. */
    if (strcmp(text, TIMER_PCT_TOKEN) == 0) {
        out->kind = SCENE_BINDING_TIMER_PCT;
        out->argument[0] = '\0';
        return SCENE_BINDING_OK;
    }

    if (strcmp(text, TIMER_PERMILLE_TOKEN) == 0) {
        out->kind = SCENE_BINDING_TIMER_PERMILLE;
        out->argument[0] = '\0';
        return SCENE_BINDING_OK;
    }

    if (strcmp(text, TIMER_STATUS_TOKEN) == 0) {
        out->kind = SCENE_BINDING_TIMER_STATUS;
        out->argument[0] = '\0';
        return SCENE_BINDING_OK;
    }

    if (strcmp(text, DATE_TOKEN) == 0) {
        out->kind = SCENE_BINDING_DATE;
        out->argument[0] = '\0';
        return SCENE_BINDING_OK;
    }

    if (strncmp(text, TIMER_REMAINING_PREFIX,
                strlen(TIMER_REMAINING_PREFIX)) == 0) {
        return parse_time_like(text + strlen(TIMER_REMAINING_PREFIX),
                                SCENE_BINDING_TIMER_REMAINING, out);
    }

    if (strncmp(text, TIMER_ELAPSED_PREFIX,
                strlen(TIMER_ELAPSED_PREFIX)) == 0) {
        return parse_time_like(text + strlen(TIMER_ELAPSED_PREFIX),
                               SCENE_BINDING_TIMER_ELAPSED, out);
    }

    if (strncmp(text, TIMER_TOTAL_PREFIX,
                strlen(TIMER_TOTAL_PREFIX)) == 0) {
        return parse_time_like(text + strlen(TIMER_TOTAL_PREFIX),
                               SCENE_BINDING_TIMER_TOTAL, out);
    }

    if (strncmp(text, TIME_PREFIX, strlen(TIME_PREFIX)) == 0) {
        return parse_time_like(text + strlen(TIME_PREFIX),
                                SCENE_BINDING_TIME, out);
    }

    if (strncmp(text, FIELD_PREFIX, strlen(FIELD_PREFIX)) == 0) {
        const char *argument = text + strlen(FIELD_PREFIX);
        if (argument[0] == '\0') {
            return SCENE_BINDING_ERR_FORMAT;
        }
        return store_argument(argument, SCENE_BINDING_FIELD, out);
    }

    return SCENE_BINDING_ERR_UNKNOWN;
}

static scene_binding_result_t write_bounded(char *out, size_t out_capacity,
                                            const char *value)
{
    size_t len = strlen(value);
    if (len + 1U > out_capacity) {
        return SCENE_BINDING_ERR_CAPACITY;
    }
    memcpy(out, value, len + 1U);
    return SCENE_BINDING_OK;
}

/* A value is "clock-shaped" when its format argument contains a ':' --
 * timer.remaining's mm:ss shape placeholders as "--:--", while timer.pct
 * (no argument) and an absent field. name placeholder as "--". */
static scene_binding_result_t write_placeholder(char *out, size_t out_capacity,
                                                const char *argument)
{
    bool clock_shaped = strchr(argument, ':') != NULL;
    return write_bounded(out, out_capacity, clock_shaped ? "--:--" : "--");
}

static int time_token_value(char c, int hour, int minute, int second)
{
    switch (c) {
    case 'H':
    case 'h':
        return hour;
    case 'M':
    case 'm':
        return minute;
    case 'S':
    case 's':
    default:
        return second;
    }
}

static bool is_uniform_run(const char *s, size_t len)
{
    for (size_t i = 1; i < len; i++) {
        if (s[i] != s[0]) {
            return false;
        }
    }
    return true;
}

/* Wall-clock and countdown formatters are deliberately separate: `mm` is a
 * clock-minute component in `time:` but a total-minute count in
 * `timer.remaining:`. Sharing this formatter previously made a one-hour
 * countdown wrap to 00:00.
 *
 * Renders a validated `HhMmSs:` format string against a broken-down local
 * time. The "AA:BB" shape -- two same-letter runs joined by a single colon
 * -- is exactly what timefmt_hhmm renders ("%02d:%02d"), so that shape is
 * routed through the existing helper. Anything else (a lone field like
 * "ss", or more than two fields) falls through to the generic per-token
 * renderer below. */
static scene_binding_result_t render_wall_clock_tokens(
    const char *format, int hour, int minute, int second, char *out,
    size_t out_capacity)
{
    const char *colon = strchr(format, ':');
    if (colon != NULL && strchr(colon + 1, ':') == NULL) {
        size_t left_len = (size_t)(colon - format);
        size_t right_len = strlen(colon + 1);
        if (left_len > 0U && right_len > 0U &&
            is_uniform_run(format, left_len) &&
            is_uniform_run(colon + 1, right_len)) {
            char buf[6];
            timefmt_hhmm(buf,
                         time_token_value(format[0], hour, minute, second),
                         time_token_value(colon[1], hour, minute, second));
            return write_bounded(out, out_capacity, buf);
        }
    }

    char buf[8U * SCENE_MAX_BINDING + 1U];
    size_t pos = 0;
    size_t len = strlen(format);
    size_t i = 0;
    while (i < len) {
        char c = format[i];
        if (c == ':') {
            if (pos + 1U >= sizeof buf) {
                return SCENE_BINDING_ERR_CAPACITY;
            }
            buf[pos++] = ':';
            i++;
            continue;
        }
        size_t run = 1U;
        while (i + run < len && format[i + run] == c) {
            run++;
        }
        char piece[12];
        int value = time_token_value(c, hour, minute, second);
        if (run >= 2U) {
            snprintf(piece, sizeof piece, "%02d", value);
        } else {
            snprintf(piece, sizeof piece, "%d", value);
        }
        size_t piece_len = strlen(piece);
        if (pos + piece_len >= sizeof buf) {
            return SCENE_BINDING_ERR_CAPACITY;
        }
        memcpy(buf + pos, piece, piece_len);
        pos += piece_len;
        i += run;
    }
    buf[pos] = '\0';
    return write_bounded(out, out_capacity, buf);
}

static uint32_t timer_token_value(char c, uint32_t total_seconds)
{
    switch (c) {
    case 'M':
    case 'm':
        return total_seconds / 60U;
    case 'S':
    case 's':
    default:
        return total_seconds % 60U;
    }
}

static scene_binding_result_t render_timer_tokens(const char *format,
                                                   uint32_t total_seconds,
                                                   char *out,
                                                   size_t out_capacity)
{
    char buf[8U * SCENE_MAX_BINDING + 1U];
    size_t pos = 0;
    size_t len = strlen(format);
    size_t i = 0;
    while (i < len) {
        char c = format[i];
        if (c == ':') {
            if (pos + 1U >= sizeof buf) {
                return SCENE_BINDING_ERR_CAPACITY;
            }
            buf[pos++] = ':';
            i++;
            continue;
        }
        size_t run = 1U;
        while (i + run < len && format[i + run] == c) {
            run++;
        }
        char piece[12];
        uint32_t value = timer_token_value(c, total_seconds);
        if (run >= 2U) {
            snprintf(piece, sizeof piece, "%02u", (unsigned)value);
        } else {
            snprintf(piece, sizeof piece, "%u", (unsigned)value);
        }
        size_t piece_len = strlen(piece);
        if (pos + piece_len >= sizeof buf) {
            return SCENE_BINDING_ERR_CAPACITY;
        }
        memcpy(buf + pos, piece, piece_len);
        pos += piece_len;
        i += run;
    }
    buf[pos] = '\0';
    return write_bounded(out, out_capacity, buf);
}

static uint32_t timer_seconds_ceiled(uint32_t milliseconds)
{
    uint32_t seconds =
        (uint32_t)(((uint64_t)milliseconds + 999U) / 1000U);
    if (seconds > (uint32_t)TIMER_SECONDS_MAX) {
        seconds = (uint32_t)TIMER_SECONDS_MAX;
    }
    return seconds;
}

static bool local_time(const scene_binding_context_t *context,
                       struct tm *tm_local)
{
    int64_t local_seconds =
        context->unix_seconds +
        (int64_t)context->utc_offset_minutes * INT64_C(60);
    time_t value = (time_t)local_seconds;
    return gmtime_r(&value, tm_local) != NULL;
}

scene_binding_result_t scene_binding_evaluate(
    const scene_binding_t *binding, const scene_binding_context_t *context,
    char *out, size_t out_capacity)
{
    if (binding == NULL || context == NULL || out == NULL) {
        return SCENE_BINDING_ERR_ARGUMENT;
    }

    switch (binding->kind) {
    case SCENE_BINDING_TIME: {
        struct tm tm_local;
        if (!local_time(context, &tm_local)) {
            return write_placeholder(out, out_capacity, binding->argument);
        }
        return render_wall_clock_tokens(binding->argument, tm_local.tm_hour,
                                        tm_local.tm_min, tm_local.tm_sec, out,
                                        out_capacity);
    }
    case SCENE_BINDING_DATE: {
        struct tm tm_local;
        if (!local_time(context, &tm_local)) {
            return write_placeholder(out, out_capacity, binding->argument);
        }
        char date[32];
        timefmt_date(date, tm_local.tm_year + 1900, tm_local.tm_mon + 1,
                     tm_local.tm_mday, (tm_local.tm_wday + 6) % 7);
        return write_bounded(out, out_capacity, date);
    }
    case SCENE_BINDING_TIMER_REMAINING: {
        if (!context->timer_active) {
            return write_placeholder(out, out_capacity, binding->argument);
        }
        uint32_t total_seconds =
            timer_seconds_ceiled(context->timer_remaining_ms);
        return render_timer_tokens(binding->argument, total_seconds, out,
                                   out_capacity);
    }
    case SCENE_BINDING_TIMER_ELAPSED: {
        if (!context->timer_active) {
            return write_placeholder(out, out_capacity, binding->argument);
        }
        uint32_t total_seconds =
            timer_seconds_ceiled(context->timer_total_ms);
        uint32_t remaining_seconds =
            timer_seconds_ceiled(context->timer_remaining_ms);
        uint32_t elapsed_seconds = remaining_seconds >= total_seconds
            ? 0U
            : total_seconds - remaining_seconds;
        return render_timer_tokens(binding->argument, elapsed_seconds, out,
                                   out_capacity);
    }
    case SCENE_BINDING_TIMER_TOTAL: {
        if (!context->timer_active) {
            return write_placeholder(out, out_capacity, binding->argument);
        }
        return render_timer_tokens(
            binding->argument, timer_seconds_ceiled(context->timer_total_ms),
            out, out_capacity);
    }
    case SCENE_BINDING_TIMER_STATUS:
        if (!context->timer_active) {
            return write_placeholder(out, out_capacity, binding->argument);
        }
        if (context->timer_remaining_ms == 0U) {
            return write_bounded(out, out_capacity, "Done");
        }
        if (context->timer_running) {
            return write_bounded(out, out_capacity, "Running");
        }
        if (timer_seconds_ceiled(context->timer_remaining_ms) >=
            timer_seconds_ceiled(context->timer_total_ms)) {
            return write_bounded(out, out_capacity, "Ready");
        }
        return write_bounded(out, out_capacity, "Paused");
    case SCENE_BINDING_TIMER_PCT: {
        if (!context->timer_active) {
            return write_placeholder(out, out_capacity, binding->argument);
        }
        char piece[4];
        snprintf(piece, sizeof piece, "%u",
                 (unsigned)context->timer_remaining_pct);
        return write_bounded(out, out_capacity, piece);
    }
    case SCENE_BINDING_TIMER_PERMILLE: {
        if (!context->timer_active) {
            return write_placeholder(out, out_capacity, binding->argument);
        }
        char piece[6];
        snprintf(piece, sizeof piece, "%u",
                 (unsigned)context->timer_remaining_permille);
        return write_bounded(out, out_capacity, piece);
    }
    case SCENE_BINDING_FIELD: {
        const char *value = (context->field != NULL)
            ? context->field(context->field_ctx, binding->argument)
            : NULL;
        if (value == NULL) {
            return write_placeholder(out, out_capacity, binding->argument);
        }
        return write_bounded(out, out_capacity, value);
    }
    default:
        return SCENE_BINDING_ERR_ARGUMENT;
    }
}

bool scene_binding_line_endpoint(scene_angle_binding_t binding,
                                 const scene_binding_context_t *context,
                                 int32_t pivot_x, int32_t pivot_y,
                                 int32_t length, scene_trigo_fn trigo_cos,
                                 scene_trigo_fn trigo_sin,
                                 scene_line_endpoint_t *out)
{
    if (context == NULL || trigo_cos == NULL || trigo_sin == NULL ||
        out == NULL || length < 0) {
        return false;
    }

    struct tm tm_local;
    if (!local_time(context, &tm_local)) {
        return false;
    }

    int32_t angle;
    switch (binding) {
    case SCENE_ANGLE_BINDING_HOUR:
        angle = (tm_local.tm_hour % 12) * 30 + tm_local.tm_min / 2;
        break;
    case SCENE_ANGLE_BINDING_MINUTE:
        angle = tm_local.tm_min * 6;
        break;
    case SCENE_ANGLE_BINDING_NONE:
    default:
        return false;
    }

    int32_t trigo_angle = CLOCK_DIAL_ROTATION + angle;
    out->x = pivot_x +
        ((length * trigo_cos(trigo_angle)) >> CLOCK_TRIGO_SHIFT);
    out->y = pivot_y +
        ((length * trigo_sin(trigo_angle)) >> CLOCK_TRIGO_SHIFT);
    return true;
}
