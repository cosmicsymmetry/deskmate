#include "scene_binding.h"

#include <stdio.h>
#include <string.h>
#include <time.h>

#include "timefmt.h"

#define TIME_PREFIX "time:"
#define TIMER_REMAINING_PREFIX "timer.remaining:"
#define TIMER_PCT_TOKEN "timer.pct"
#define FIELD_PREFIX "field."

/* A `time:`/`timer.remaining:` format argument is a whitelist of these
 * characters, never a printf format string handed to a printf-family
 * function -- host bytes must never reach one of those. */
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
        if (!is_time_format_char(*p)) {
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

    /* `timer.pct` is an exact-match token with no argument -- checked
     * before the `timer.remaining:` prefix (they share a `timer.` stem)
     * via strcmp, so "timer.pctXYZ" cannot match it. */
    if (strcmp(text, TIMER_PCT_TOKEN) == 0) {
        out->kind = SCENE_BINDING_TIMER_PCT;
        out->argument[0] = '\0';
        return SCENE_BINDING_OK;
    }

    if (strncmp(text, TIMER_REMAINING_PREFIX,
                strlen(TIMER_REMAINING_PREFIX)) == 0) {
        return parse_time_like(text + strlen(TIMER_REMAINING_PREFIX),
                                SCENE_BINDING_TIMER_REMAINING, out);
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

/* Renders a validated `HhMmSs:` format string against a broken-down local
 * time. The "AA:BB" shape -- two same-letter runs joined by a single colon
 * -- is exactly what timefmt_hhmm renders ("%02d:%02d"), and it covers both
 * shapes this module actually needs (time:HH:mm, timer.remaining:mm:ss), so
 * that shape is routed through the existing helper rather than a second
 * formatter. Anything else (a lone field like "ss", or more than two
 * fields) falls through to the generic per-token renderer below. */
static scene_binding_result_t render_time_tokens(const char *format, int hour,
                                                  int minute, int second,
                                                  char *out,
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

scene_binding_result_t scene_binding_evaluate(
    const scene_binding_t *binding, const scene_binding_context_t *context,
    char *out, size_t out_capacity)
{
    if (binding == NULL || context == NULL || out == NULL) {
        return SCENE_BINDING_ERR_ARGUMENT;
    }

    switch (binding->kind) {
    case SCENE_BINDING_TIME: {
        int64_t local_seconds =
            context->unix_seconds +
            (int64_t)context->utc_offset_minutes * INT64_C(60);
        time_t local_time = (time_t)local_seconds;
        struct tm tm_local;
        if (gmtime_r(&local_time, &tm_local) == NULL) {
            return write_placeholder(out, out_capacity, binding->argument);
        }
        return render_time_tokens(binding->argument, tm_local.tm_hour,
                                   tm_local.tm_min, tm_local.tm_sec, out,
                                   out_capacity);
    }
    case SCENE_BINDING_TIMER_REMAINING: {
        if (!context->timer_active) {
            return write_placeholder(out, out_capacity, binding->argument);
        }
        uint32_t total_seconds = context->timer_remaining_ms / 1000U;
        int hour = (int)(total_seconds / 3600U);
        int minute = (int)((total_seconds % 3600U) / 60U);
        int second = (int)(total_seconds % 60U);
        return render_time_tokens(binding->argument, hour, minute, second,
                                   out, out_capacity);
    }
    case SCENE_BINDING_TIMER_PCT: {
        if (!context->timer_active) {
            return write_placeholder(out, out_capacity, binding->argument);
        }
        char piece[4];
        snprintf(piece, sizeof piece, "%u", (unsigned)context->timer_pct);
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
