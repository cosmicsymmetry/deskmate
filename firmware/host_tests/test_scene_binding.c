#include <assert.h>
#include <stdio.h>
#include <string.h>

#include "core/scene_binding.h"

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
    assert(scene_binding_parse("timer.remaining:mm:ss", &binding) ==
           SCENE_BINDING_OK);
    scene_binding_context_t ctx = fixed_context();
    ctx.timer_active = false;
    assert(scene_binding_evaluate(&binding, &ctx, out, sizeof out) ==
           SCENE_BINDING_OK);
    assert(strcmp(out, "--:--") == 0);
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
    active.timer_pct = 42U;
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
    test_zero_capacity_never_writes();
    test_field_name_length_boundary();
    return 0;
}
