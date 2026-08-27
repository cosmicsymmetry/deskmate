#include <assert.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

#include "core/scene_binding.h"
#include "core/timefmt.h"

static const char *field_stub(void *ctx, const char *name)
{
    (void)ctx;
    if (strcmp(name, "temp") == 0) { return "23"; }
    return NULL;
}

/* 1787823667 == 2026-08-27T09:41:07Z; at +240 minutes that is 13:41:07
 * local, which is what the expectations below assert. Verified with
 * `date -u -r 1787823667`. */
static scene_binding_context_t fixed_context(void)
{
    scene_binding_context_t ctx;
    memset(&ctx, 0, sizeof ctx);
    ctx.unix_seconds = INT64_C(1787823667);
    ctx.utc_offset_minutes = 240;
    ctx.field = field_stub;
    return ctx;
}

static void expect(const char *text, const char *want)
{
    scene_binding_t binding;
    char out[SCENE_MAX_TEXT_BYTES + 1U];
    assert(scene_binding_parse(text, &binding) == SCENE_BINDING_OK);
    scene_binding_context_t ctx = fixed_context();
    assert(scene_binding_evaluate(&binding, &ctx, out, sizeof out) ==
           SCENE_BINDING_OK);
    assert(strcmp(out, want) == 0);
}

static void test_time_formats_use_the_utc_offset(void)
{
    expect("time:HH:mm", "13:41");
    expect("time:ss", "07");
}

static void test_field_lookup(void)
{
    expect("field.temp", "23");
}

static void test_an_absent_field_renders_the_placeholder(void)
{
    /* A missing field is normal -- a provider that has not reported yet --
     * so it renders the same placeholder the C templates use, never an
     * error and never an empty box. */
    expect("field.humidity", "--");
}

static void test_an_inactive_timer_renders_the_placeholder(void)
{
    scene_binding_t binding;
    char out[SCENE_MAX_TEXT_BYTES + 1U];
    scene_binding_context_t ctx = fixed_context();
    ctx.timer_active = false;
    const char *clock_bindings[] = {
        "timer.remaining:mm:ss",
        "timer.elapsed:mm:ss",
        "timer.total:mm:ss",
    };
    for (size_t i = 0U;
         i < sizeof clock_bindings / sizeof clock_bindings[0]; ++i) {
        assert(scene_binding_parse(clock_bindings[i], &binding) ==
               SCENE_BINDING_OK);
        assert(scene_binding_evaluate(&binding, &ctx, out, sizeof out) ==
               SCENE_BINDING_OK);
        assert(strcmp(out, "--:--") == 0);
    }
}

static void test_unknown_bindings_are_rejected_at_parse(void)
{
    scene_binding_t binding;
    /* Rejected at parse, not at evaluate: a scene carrying an unknown
     * binding is malformed and must be refused whole, so the panel keeps
     * showing the last good scene rather than a half-evaluated one. */
    assert(scene_binding_parse("shell:rm -rf /", &binding) ==
           SCENE_BINDING_ERR_UNKNOWN);
    assert(scene_binding_parse("time:%n%n", &binding) ==
           SCENE_BINDING_ERR_FORMAT);
    assert(scene_binding_parse("field.", &binding) ==
           SCENE_BINDING_ERR_FORMAT);
    assert(scene_binding_parse("timer.pctXYZ", &binding) ==
           SCENE_BINDING_ERR_UNKNOWN);
    assert(scene_binding_parse("timer.permilleXYZ", &binding) ==
           SCENE_BINDING_ERR_UNKNOWN);
    assert(scene_binding_parse("timer.statusXYZ", &binding) ==
           SCENE_BINDING_ERR_UNKNOWN);
    assert(scene_binding_parse("dateXYZ", &binding) ==
           SCENE_BINDING_ERR_UNKNOWN);
}

static void test_evaluate_never_overruns_a_short_buffer(void)
{
    scene_binding_t binding;
    char out[4];
    assert(scene_binding_parse("time:HH:mm", &binding) == SCENE_BINDING_OK);
    scene_binding_context_t ctx = fixed_context();
    assert(scene_binding_evaluate(&binding, &ctx, out, sizeof out) ==
           SCENE_BINDING_ERR_CAPACITY);
}

static void test_an_active_timer_renders_its_remaining_time(void)
{
    /* 125000 ms == 2 minutes 5 seconds. */
    scene_binding_t binding;
    char out[SCENE_MAX_TEXT_BYTES + 1U];
    assert(scene_binding_parse("timer.remaining:mm:ss", &binding) ==
           SCENE_BINDING_OK);
    scene_binding_context_t ctx = fixed_context();
    ctx.timer_active = true;
    ctx.timer_remaining_ms = 125000U;
    assert(scene_binding_evaluate(&binding, &ctx, out, sizeof out) ==
           SCENE_BINDING_OK);
    assert(strcmp(out, "02:05") == 0);
}

static void test_timer_pct(void)
{
    scene_binding_t binding;
    char out[SCENE_MAX_TEXT_BYTES + 1U];
    assert(scene_binding_parse("timer.pct", &binding) == SCENE_BINDING_OK);

    scene_binding_context_t active = fixed_context();
    active.timer_active = true;
    active.timer_remaining_pct = 42U;
    assert(scene_binding_evaluate(&binding, &active, out, sizeof out) ==
           SCENE_BINDING_OK);
    assert(strcmp(out, "42") == 0);

    /* An inactive timer.pct is not clock-shaped -- its argument is empty,
     * with no ':' -- so it renders the scalar placeholder, not "--:--". */
    scene_binding_context_t inactive = fixed_context();
    inactive.timer_active = false;
    assert(scene_binding_evaluate(&binding, &inactive, out, sizeof out) ==
           SCENE_BINDING_OK);
    assert(strcmp(out, "--") == 0);
}

static void timer_pct_is_remaining_not_elapsed(void)
{
    /* 1500s duration, 900s remaining: progress_ring.c draws 60% of a ring. */
    scene_timer_snapshot_t s = scene_timer_snapshot(1500, 900, false, 0, 0);
    assert(s.remaining_pct == 60);
    /* The inverted producer returned 40 here, and the parity gate was at zero
     * pixels the whole time, because the gate injects this value. */
}

static void a_finished_timer_reads_zero_percent(void)
{
    scene_timer_snapshot_t s = scene_timer_snapshot(1500, 0, false, 0, 0);
    assert(s.remaining_pct == 0);
}

static void a_never_started_timer_reads_one_hundred(void)
{
    scene_timer_snapshot_t s = scene_timer_snapshot(1500, 1500, false, 0, 0);
    assert(s.remaining_pct == 100);
}

static void a_running_timer_subtracts_elapsed_time(void)
{
    scene_timer_snapshot_t s =
        scene_timer_snapshot(1500, 900, true, 1000, 401000);
    assert(s.remaining_ms == 500000U);
    assert(s.remaining_pct == 33);
}

static void local_timer_actions_move_a_paused_snapshot(void)
{
    scene_timer_snapshot_t snapshot =
        scene_timer_snapshot(1500, 900, false, 0U, 1000U);

    scene_timer_apply_local_action(
        &snapshot, SCENE_TIMER_LOCAL_ACTION_START_PAUSE, 10000U);
    assert(snapshot.running);
    assert(snapshot.anchor_ms == 10000U);
    assert(snapshot.remaining_ms == 900000U);

    scene_timer_apply_local_action(
        &snapshot, SCENE_TIMER_LOCAL_ACTION_START_PAUSE, 410000U);
    assert(!snapshot.running);
    assert(snapshot.remaining_ms == 500000U);
    assert(snapshot.remaining_pct == 33U);
    assert(snapshot.remaining_permille == 333U);

    scene_timer_apply_local_action(
        &snapshot, SCENE_TIMER_LOCAL_ACTION_RESET, 500000U);
    assert(!snapshot.running);
    assert(snapshot.anchor_ms == 500000U);
    assert(snapshot.remaining_ms == snapshot.total_ms);
    assert(snapshot.remaining_pct == 100U);
    assert(snapshot.remaining_permille == 1000U);
}

static void a_non_positive_duration_returns_an_empty_snapshot(void)
{
    scene_timer_snapshot_t s = scene_timer_snapshot(0, 900, true, 0, 1000);
    assert(s.remaining_ms == 0U);
    assert(s.remaining_pct == 0);
}

static void timer_pct_truncates_instead_of_rounding(void)
{
    scene_timer_snapshot_t s = scene_timer_snapshot(1500, 901, false, 0, 0);
    assert(s.remaining_ms == 901000U);
    assert(s.remaining_pct == 60);
}

static void timer_snapshot_bounds_inputs_before_millisecond_arithmetic(void)
{
    scene_timer_snapshot_t s =
        scene_timer_snapshot(INT64_MAX, INT64_MAX, false, 0, 0);
    assert(s.remaining_ms == 86400U * 1000U);
    assert(s.remaining_pct == 100);
}

static void timer_remaining_ceils_like_format_clock(void)
{
    char out[16];
    scene_binding_t b;
    assert(scene_binding_parse("timer.remaining:mm:ss", &b) == SCENE_BINDING_OK);
    scene_binding_context_t ctx = {0};
    ctx.timer_active = true;
    /* 1ms into the 90th second: progress_ring.c still shows 00:90 -> 01:30.
     * The floor showed 01:29 almost immediately. */
    ctx.timer_remaining_ms = 89001U;
    assert(scene_binding_evaluate(&b, &ctx, out, sizeof out) == SCENE_BINDING_OK);
    assert(strcmp(out, "01:30") == 0);
}

static void timer_mm_is_total_minutes_not_a_clock_minute(void)
{
    char out[16];
    scene_binding_t b;
    assert(scene_binding_parse("timer.remaining:mm:ss", &b) == SCENE_BINDING_OK);
    scene_binding_context_t ctx = {0};
    ctx.timer_active = true;
    ctx.timer_remaining_ms = 3600U * 1000U;      /* one hour */
    assert(scene_binding_evaluate(&b, &ctx, out, sizeof out) == SCENE_BINDING_OK);
    assert(strcmp(out, "60:00") == 0);           /* was "00:00" */

    ctx.timer_remaining_ms = 86400U * 1000U;     /* the registry ceiling */
    assert(scene_binding_evaluate(&b, &ctx, out, sizeof out) == SCENE_BINDING_OK);
    assert(strcmp(out, "1440:00") == 0);
}

static void timer_remaining_clamps_like_format_clock(void)
{
    char out[16];
    scene_binding_t b;
    assert(scene_binding_parse("timer.remaining:mm:ss", &b) == SCENE_BINDING_OK);
    scene_binding_context_t ctx = {0};
    ctx.timer_active = true;

    ctx.timer_remaining_ms = 86400U * 1000U;
    assert(scene_binding_evaluate(&b, &ctx, out, sizeof out) == SCENE_BINDING_OK);
    assert(strcmp(out, "1440:00") == 0);

    ctx.timer_remaining_ms = 90000U * 1000U;
    assert(scene_binding_evaluate(&b, &ctx, out, sizeof out) == SCENE_BINDING_OK);
    assert(strcmp(out, "1440:00") == 0);
}

static void timer_remaining_rejects_an_hours_token(void)
{
    scene_binding_t b;
    assert(scene_binding_parse("timer.remaining:HH:mm:ss", &b) ==
           SCENE_BINDING_ERR_FORMAT);
    assert(scene_binding_parse("timer.remaining:hh:mm:ss", &b) ==
           SCENE_BINDING_ERR_FORMAT);
    assert(scene_binding_parse("timer.elapsed:HH:mm:ss", &b) ==
           SCENE_BINDING_ERR_FORMAT);
    assert(scene_binding_parse("timer.total:hh:mm:ss", &b) ==
           SCENE_BINDING_ERR_FORMAT);
    assert(scene_binding_parse("time:HH:mm:ss", &b) == SCENE_BINDING_OK);
}

static void timer_elapsed_is_duration_minus_remaining(void)
{
    char out[16];
    scene_binding_t b;
    assert(scene_binding_parse("timer.elapsed:mm:ss", &b) == SCENE_BINDING_OK);
    scene_binding_context_t ctx = {0};
    ctx.timer_active = true;
    ctx.timer_total_ms = 1500U * 1000U;
    ctx.timer_remaining_ms = 900U * 1000U;
    assert(scene_binding_evaluate(&b, &ctx, out, sizeof out) == SCENE_BINDING_OK);
    assert(strcmp(out, "10:00") == 0);
}

static void timer_total_formats_the_bounded_duration(void)
{
    char out[16];
    scene_binding_t binding;
    assert(scene_binding_parse("timer.total:mm:ss", &binding) ==
           SCENE_BINDING_OK);
    scene_binding_context_t ctx = {0};
    ctx.timer_active = true;
    ctx.timer_total_ms = 1500U * 1000U;
    assert(scene_binding_evaluate(&binding, &ctx, out, sizeof out) ==
           SCENE_BINDING_OK);
    assert(strcmp(out, "25:00") == 0);
}

static void timer_status_picks_the_same_word_as_status_word(void)
{
    /* The four cases progress_ring.c's status_word() distinguishes. */
    const struct {
        uint32_t total_ms, remaining_ms;
        bool running;
        const char *word;
    } cases[] = {
        { 1500000U,       0U, false, "Done"    },
        { 1500000U,       0U, true,  "Done"    }, /* zero wins over running */
        { 1500000U,  900000U, true,  "Running" },
        { 1500000U, 1500000U, false, "Ready"   },
        { 1500000U, 1499500U, false, "Ready"   }, /* both ceil to 1500s */
        { 1500000U,  900000U, false, "Paused"  },
    };
    for (size_t i = 0; i < sizeof cases / sizeof cases[0]; i++) {
        char out[16];
        scene_binding_t binding;
        assert(scene_binding_parse("timer.status", &binding) ==
               SCENE_BINDING_OK);
        scene_binding_context_t ctx = {0};
        ctx.timer_active = true;
        ctx.timer_total_ms = cases[i].total_ms;
        ctx.timer_remaining_ms = cases[i].remaining_ms;
        ctx.timer_running = cases[i].running;
        assert(scene_binding_evaluate(&binding, &ctx, out, sizeof out) ==
               SCENE_BINDING_OK);
        assert(strcmp(out, cases[i].word) == 0);
    }
}

static void an_inactive_timer_renders_the_placeholder(void)
{
    /* Consistent with timer.remaining's "--:--" shape placeholder. A status
     * with no timer must not read "Ready". */
    char out[16];
    scene_binding_t binding;
    assert(scene_binding_parse("timer.status", &binding) == SCENE_BINDING_OK);
    scene_binding_context_t ctx = {0};
    assert(scene_binding_evaluate(&binding, &ctx, out, sizeof out) ==
           SCENE_BINDING_OK);
    assert(strcmp(out, "--") == 0);
}

static void timer_permille_preserves_a_non_whole_percent(void)
{
    scene_timer_snapshot_t snapshot =
        scene_timer_snapshot(2000, 1211, false, 0, 0);
    assert(snapshot.remaining_pct == 60U);
    assert(snapshot.remaining_permille == 605U);

    char out[8];
    scene_binding_t binding;
    assert(scene_binding_parse("timer.permille", &binding) ==
           SCENE_BINDING_OK);
    scene_binding_context_t ctx = {0};
    ctx.timer_active = true;
    ctx.timer_remaining_permille = snapshot.remaining_permille;
    assert(scene_binding_evaluate(&binding, &ctx, out, sizeof out) ==
           SCENE_BINDING_OK);
    assert(strcmp(out, "605") == 0);
}

static void a_wall_clock_minute_still_wraps(void)
{
    /* The formatter split must not change time:; this instant is 21:05 UTC. */
    char out[16];
    scene_binding_t b;
    assert(scene_binding_parse("time:HH:mm", &b) == SCENE_BINDING_OK);
    scene_binding_context_t ctx = {0};
    ctx.unix_seconds = 1787000700;
    ctx.utc_offset_minutes = 0;
    assert(scene_binding_evaluate(&b, &ctx, out, sizeof out) == SCENE_BINDING_OK);
    /* Derive the expected clock fields independently from the pinned epoch. */
    time_t instant = (time_t)ctx.unix_seconds;
    struct tm expected_tm;
    assert(gmtime_r(&instant, &expected_tm) != NULL);
    char expected[16];
    snprintf(expected, sizeof expected, "%02d:%02d", expected_tm.tm_hour,
             expected_tm.tm_min);
    assert(strcmp(out, expected) == 0);
}

static void the_date_binding_matches_timefmt_date(void)
{
    char out[32];
    char expected[32];
    scene_binding_t b;
    assert(scene_binding_parse("date", &b) == SCENE_BINDING_OK);
    scene_binding_context_t ctx = {0};
    ctx.unix_seconds = 1787000700;
    ctx.utc_offset_minutes = 240;   /* the board sits at UTC+4 */
    assert(scene_binding_evaluate(&b, &ctx, out, sizeof out) == SCENE_BINDING_OK);
    /* Compare against timefmt_date() itself, never against a hardcoded
     * string: the point is that the two cannot drift. */
    struct tm tm_local;
    time_t local = (time_t)ctx.unix_seconds + ctx.utc_offset_minutes * 60;
    gmtime_r(&local, &tm_local);
    timefmt_date(expected, tm_local.tm_year + 1900, tm_local.tm_mon + 1,
                 tm_local.tm_mday, (tm_local.tm_wday + 6) % 7);
    assert(strcmp(out, expected) == 0);
}

static void the_date_binding_crosses_local_midnight_with_the_offset(void)
{
    /* 2026-08-27T20:00:00Z is 2026-08-28T00:00:00 at UTC+4: an
     * instant that is one day earlier in UTC than locally. */
    char out[32];
    char expected[32];
    scene_binding_t b;
    assert(scene_binding_parse("date", &b) == SCENE_BINDING_OK);
    scene_binding_context_t ctx = {0};
    ctx.unix_seconds = INT64_C(1787860800);
    ctx.utc_offset_minutes = 240;
    assert(scene_binding_evaluate(&b, &ctx, out, sizeof out) == SCENE_BINDING_OK);

    time_t local = (time_t)ctx.unix_seconds + ctx.utc_offset_minutes * 60;
    struct tm tm_local;
    assert(gmtime_r(&local, &tm_local) != NULL);
    timefmt_date(expected, tm_local.tm_year + 1900, tm_local.tm_mon + 1,
                 tm_local.tm_mday, (tm_local.tm_wday + 6) % 7);
    assert(strcmp(out, expected) == 0);

    time_t utc = (time_t)ctx.unix_seconds;
    struct tm tm_utc;
    assert(gmtime_r(&utc, &tm_utc) != NULL);
    timefmt_date(expected, tm_utc.tm_year + 1900, tm_utc.tm_mon + 1,
                 tm_utc.tm_mday, (tm_utc.tm_wday + 6) % 7);
    assert(strcmp(out, expected) != 0);
}

static int32_t host_trigo_sin(int32_t angle)
{
    int32_t normalised = angle % 360;
    if (normalised < 0) {
        normalised += 360;
    }
    switch (normalised) {
    case 0: return 0;
    case 90: return 32768;
    case 180: return 0;
    case 270: return -32768;
    default: assert(false); return 0;
    }
}

static int32_t host_trigo_cos(int32_t angle)
{
    return host_trigo_sin(angle + 90);
}

static void a_cardinal_angle_binding_gives_expected_hand_geometry(void)
{
    /* Quarter past three: the minute hand points at three o'clock. This core
     * suite deliberately injects a four-cardinal stub so it can isolate the
     * angle formula and fixed-point endpoint arithmetic without linking LVGL.
     * The seven-instant pixel parity matrix supplies the non-cardinal proof
     * against the real lv_trigo table used by both production render paths. */
    scene_binding_context_t ctx = {0};
    ctx.unix_seconds = INT64_C(1787800500);
    scene_line_endpoint_t endpoint;
    assert(scene_binding_line_endpoint(
        SCENE_ANGLE_BINDING_MINUTE, &ctx, 344, 244, 42,
        host_trigo_cos, host_trigo_sin, &endpoint));
    assert(endpoint.x == 386);
    assert(endpoint.y == 244);
}

static void test_zero_capacity_never_writes(void)
{
    scene_binding_t binding;
    char out[1] = { 'x' };
    assert(scene_binding_parse("field.temp", &binding) == SCENE_BINDING_OK);
    scene_binding_context_t ctx = fixed_context();
    assert(scene_binding_evaluate(&binding, &ctx, out, 0U) ==
           SCENE_BINDING_ERR_CAPACITY);
    assert(out[0] == 'x');
}

static void test_field_name_length_boundary(void)
{
    char name[SCENE_MAX_BINDING + 2U];
    memset(name, 'a', sizeof name);
    name[sizeof name - 1U] = '\0';

    char text[SCENE_MAX_BINDING + 8U];
    scene_binding_t binding;

    /* Exactly SCENE_MAX_BINDING characters is accepted. */
    snprintf(text, sizeof text, "field.%.*s", (int)SCENE_MAX_BINDING, name);
    assert(scene_binding_parse(text, &binding) == SCENE_BINDING_OK);

    /* One character over is rejected. */
    snprintf(text, sizeof text, "field.%.*s", (int)SCENE_MAX_BINDING + 1,
             name);
    assert(scene_binding_parse(text, &binding) == SCENE_BINDING_ERR_FORMAT);
}

int main(void)
{
    test_time_formats_use_the_utc_offset();
    test_field_lookup();
    test_an_absent_field_renders_the_placeholder();
    test_an_inactive_timer_renders_the_placeholder();
    test_unknown_bindings_are_rejected_at_parse();
    test_evaluate_never_overruns_a_short_buffer();
    test_an_active_timer_renders_its_remaining_time();
    test_timer_pct();
    timer_pct_is_remaining_not_elapsed();
    a_finished_timer_reads_zero_percent();
    a_never_started_timer_reads_one_hundred();
    a_running_timer_subtracts_elapsed_time();
    local_timer_actions_move_a_paused_snapshot();
    a_non_positive_duration_returns_an_empty_snapshot();
    timer_pct_truncates_instead_of_rounding();
    timer_snapshot_bounds_inputs_before_millisecond_arithmetic();
    timer_remaining_ceils_like_format_clock();
    timer_mm_is_total_minutes_not_a_clock_minute();
    timer_remaining_clamps_like_format_clock();
    timer_remaining_rejects_an_hours_token();
    timer_elapsed_is_duration_minus_remaining();
    timer_total_formats_the_bounded_duration();
    timer_status_picks_the_same_word_as_status_word();
    an_inactive_timer_renders_the_placeholder();
    timer_permille_preserves_a_non_whole_percent();
    a_wall_clock_minute_still_wraps();
    the_date_binding_matches_timefmt_date();
    the_date_binding_crosses_local_midnight_with_the_offset();
    a_cardinal_angle_binding_gives_expected_hand_geometry();
    test_zero_capacity_never_writes();
    test_field_name_length_boundary();
    return 0;
}
