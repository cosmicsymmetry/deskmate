# Scene Renderer Implementation Plan (Plugins, Stage 2a)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The device renders a declarative display list pushed from the host, and
reproduces the shipped `DigitalClock` template from one — pixel for pixel.

**Architecture:** The host emits a scene: absolutely-positioned nodes on the 448×368
canvas, with a closed set of bindings the device evaluates locally so a clock still ticks
with the link down. A generic C interpreter under `firmware/main/ui/` draws it with LVGL,
compiles into `lvgl-sim` unchanged, and is proved correct by reproducing an existing
hand-written template byte-identically.

**Tech Stack:** ESP-IDF 5.x / C11, LVGL 9, TinyCBOR, Rust (`protocol`, `app-core`,
`lvgl-sim`, `device`).

**Spec:** `docs/superpowers/specs/2026-08-22-deskmate-plugin-scene-rendering-design.md`

## Scope: this is stage 2a, not all of stage 2

The spec's stage 2 is "scene renderer + wire, then re-express all six templates and
framebuffer-diff them." Decomposed, that is over twenty tasks. This plan covers **2a**:
the format, the wire, the interpreter, the bindings, and **one** template — `DigitalClock`
— reproduced pixel for pixel. That single template exercises the entire chain (text at
three tiers, a rect module, a scale dial, two line hands, a live binding) and is a complete,
gated, shippable slice.

**Stage 2b** re-expresses the remaining five templates and retires the C ones. Its plan is
written at this plan's exit, per the repo's working agreement.

## Two decisions that shape everything below

**1. Text nodes are baseline-anchored.** `digital_clock.c:44` positions type by
`baseline_offset(font) = lv_font_get_line_height(font) - font->base_line`, not by box top.
A scene that carried a box-top Y could not reproduce the shipped pixels. Scene `text` nodes
therefore carry the **baseline Y**, and the host computes it from the same metrics.

**2. Scene fonts reference a baked tier, not an asset — for now.** The six templates use
fonts baked by `lv_font_conv`; a runtime TTF is rasterized by `stb_truetype`. Their metrics
need not agree, so building the parity gate on runtime fonts would make it unwinnable for
reasons unrelated to the interpreter. The `font` field therefore has **two forms** — a baked
tier (`caption`/`body`/`display`/`hero` = 18/28/56/96) and an asset reference
(digest + pixel size) — and **every parity case in this plan uses a baked tier.** Runtime
fonts are supported by the format and exercised by stage 3.

## What this plan does not cover

The spec's **§3 render negotiation** — where the server rasterizes a scene the device
cannot draw and pushes pixels instead — has **no task here**, and cannot: it needs the
`resvg` raster path and the volatile asset tier, both of which are stage 4. Until then a
scene the device cannot render is refused, not degraded. That is the correct behaviour for
this stage but it is not §3, and nothing in this plan should be read as implementing it.

## Global Constraints

- `firmware/main/core/` and `firmware/main/ui/scene_*.c` must stay **free of ESP-IDF
  includes** and compile as plain C11 under `-Wall -Wextra -Werror -std=c11`.
  `lvgl-sim` compiles these sources on the host; an `#include "esp_*.h"` silently breaks
  the parity harness, which is this plan's entire gate.
- **No new file-scope non-const objects in firmware.** DIRAM `.bss` is **102,608** and IRAM
  is **16,384 / 16,384 with 0 remaining**. ~105 bytes of static internal DRAM once broke
  OTA downloads on this board entirely (`3f2aa03`), diagnosed only by bisecting three
  builds on hardware with every test green. Anything landing in IRAM is a hard failure.
- All bytes from the host are untrusted: bound every count and length, reject malformed
  input, recover without rebooting. Four overflow bugs were caught in stage 1's modules.
- The wire stays **protocol v1**, additive only. `PROTOCOL_CURRENT_CAPABILITIES` goes from
  **235** to **491** (bit 8, `+256`). Pin the literal in a test in both languages — bit 7
  sat defined-but-unset for most of V2 and the constant read 75 instead of 203.
- Guard LVGL calls made outside LVGL callbacks with `lvgl_port_lock()`/`lvgl_port_unlock()`.
- **The standalone clock must still survive** boot, host loss, malformed input, and version
  mismatch. A scene failing to decode must fall back, never blank the panel.
- Conventional commit prefixes. Commit every file you changed for a task; leave alone
  changes that are not yours.

## File Structure

| File | Responsibility |
| --- | --- |
| `firmware/main/core/scene_model.h/.c` | Scene struct, node union, bounds. No ESP-IDF, no LVGL. Host-tested. |
| `firmware/main/core/scene_decode.c` | CBOR → `scene_t`, hostile-input hardened. Host-tested. |
| `firmware/main/core/scene_binding.h/.c` | The closed binding set and its evaluation. No ESP-IDF, no LVGL. Host-tested. |
| `firmware/main/ui/scene_view.h/.c` | `scene_t` → LVGL objects. No ESP-IDF. Compiled into `lvgl-sim`. |
| `firmware/main/core/protocol_message.h/.c` | Message 19 `PushScene`, capability bit 8. |
| `firmware/main/link/protocol_task.c` | Dispatch, binding value updates from `PushData`, font lifetime. |
| `firmware/main/ui/font_registry.c` | The deferred use-after-free fix — stage 2 is its first real consumer. |
| `companion/crates/protocol/src/scene.rs` | Host scene types, encode/decode, shared fixtures. |
| `companion/crates/app-core/src/scene_build.rs` | Template → scene, including baseline and tier selection. |
| `companion/crates/lvgl-sim/src/cases.rs` | Scene golden cases (one per node kind, both orientations). |
| `companion/crates/app-core/tests/scene_parity.rs` | **The parity gate.** Delivered here, not in `cases.rs` — see Task 9. |

Splitting decode from the model keeps the hostile-input surface in one reviewable file, and
keeps `scene_model.c` free of TinyCBOR so the simulator and host tests can construct scenes
directly.

## A third decision: scenes fit one envelope

The spec allows chunked scenes. This plan does **not** implement chunking. A scene is
capped at `SCENE_MAX_NODES` 24 and must encode inside the existing 2034-byte payload, which
`DigitalClock`'s seven nodes clear by a wide margin. Chunking is stage 2b's problem if a
template needs it, and inventing a second chunk protocol now — alongside the asset one —
would be work with no consumer.

---

### Task 1: Scene model

**Files:**
- Create: `firmware/main/core/scene_model.h`, `firmware/main/core/scene_model.c`
- Create: `firmware/host_tests/test_scene_model.c`
- Modify: `firmware/host_tests/Makefile`, `firmware/main/CMakeLists.txt`, `.gitignore`

**Interfaces:**
- Consumes: nothing.
- Produces: `scene_t`, `scene_node_t`, `scene_node_kind_t`, `scene_font_ref_t`,
  `scene_value_t`, `SCENE_MAX_NODES` (24), `SCENE_MAX_TEXT_BYTES` (128),
  `SCENE_MAX_LINE_POINTS` (8), `scene_model_validate()`.

This file holds types and bounds only — no CBOR (Task 3) and no LVGL (Task 5). That split
is what lets host tests and the simulator build scenes directly.

- [x] **Step 1: Write the failing test**

Create `firmware/host_tests/test_scene_model.c`:

```c
#include <assert.h>
#include <string.h>

#include "core/scene_model.h"

static scene_t minimal_scene(void)
{
    scene_t scene;
    memset(&scene, 0, sizeof scene);
    scene.node_count = 1U;
    scene.nodes[0].kind = SCENE_NODE_RECT;
    scene.nodes[0].value.rect.w = 100;
    scene.nodes[0].value.rect.h = 50;
    return scene;
}

static void test_a_minimal_scene_validates(void)
{
    scene_t scene = minimal_scene();
    assert(scene_model_validate(&scene) == SCENE_MODEL_OK);
}

static void test_too_many_nodes_is_rejected(void)
{
    scene_t scene = minimal_scene();
    scene.node_count = SCENE_MAX_NODES + 1U;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_NODE_COUNT);
}

static void test_an_unknown_node_kind_is_rejected(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = (scene_node_kind_t)99;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_NODE_KIND);
}

static void test_a_node_outside_the_canvas_is_rejected(void)
{
    /* The canvas is the only coordinate space that exists. A node placed
     * outside it is not clipped silently -- a host that computes a layout
     * off-canvas has a bug, and hiding it makes that bug invisible. */
    scene_t scene = minimal_scene();
    scene.nodes[0].value.rect.x = SCENE_CANVAS_WIDTH;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_an_unterminated_text_value_is_rejected(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_TEXT;
    scene.nodes[0].value.text.font.kind = SCENE_FONT_BAKED;
    scene.nodes[0].value.text.font.baked = SCENE_FONT_BODY;
    memset(scene.nodes[0].value.text.value.literal, 'a',
           SCENE_MAX_TEXT_BYTES + 1U);
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_TEXT);
}

static void test_a_line_with_too_many_points_is_rejected(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_LINE;
    scene.nodes[0].value.line.point_count = SCENE_MAX_LINE_POINTS + 1U;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_an_unknown_baked_font_tier_is_rejected(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_TEXT;
    scene.nodes[0].value.text.font.kind = SCENE_FONT_BAKED;
    scene.nodes[0].value.text.font.baked = (scene_font_tier_t)42;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_FONT);
}

int main(void)
{
    test_a_minimal_scene_validates();
    test_too_many_nodes_is_rejected();
    test_an_unknown_node_kind_is_rejected();
    test_a_node_outside_the_canvas_is_rejected();
    test_an_unterminated_text_value_is_rejected();
    test_a_line_with_too_many_points_is_rejected();
    test_an_unknown_baked_font_tier_is_rejected();
    return 0;
}
```

- [x] **Step 2: Run to confirm it fails**

```sh
make -C firmware/host_tests test_scene_model
```

Expected: FAIL — `core/scene_model.h: No such file or directory`.

- [x] **Step 3: Write the header**

Create `firmware/main/core/scene_model.h`:

```c
#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "core/asset_store.h"   /* ASSET_DIGEST_BYTES */

#define SCENE_CANVAS_WIDTH   448
#define SCENE_CANVAS_HEIGHT  368
#define SCENE_MAX_NODES      24U
#define SCENE_MAX_TEXT_BYTES 128U
#define SCENE_MAX_LINE_POINTS 8U
#define SCENE_MAX_GLYPH_NAME 32U
#define SCENE_MAX_BINDING    48U

typedef enum {
    SCENE_NODE_RECT = 1,
    SCENE_NODE_ARC = 2,
    SCENE_NODE_LINE = 3,
    SCENE_NODE_TEXT = 4,
    SCENE_NODE_IMAGE = 5,
    SCENE_NODE_GLYPH = 6,
} scene_node_kind_t;

/* The four faces baked by tools/genfonts.sh. Kept as a tier rather than a
 * pixel size because the shipped templates address them by role, and because
 * a tier resolves to the identical lv_font_t the C templates use -- which is
 * what makes the stage-2a parity gate winnable. */
typedef enum {
    SCENE_FONT_CAPTION = 1,  /* deskmate_font_18 */
    SCENE_FONT_BODY = 2,     /* deskmate_font_28 */
    SCENE_FONT_DISPLAY = 3,  /* deskmate_font_56 */
    SCENE_FONT_HERO = 4,     /* deskmate_font_96 */
} scene_font_tier_t;

typedef enum {
    SCENE_FONT_BAKED = 1,
    SCENE_FONT_ASSET = 2,
} scene_font_kind_t;

typedef struct {
    scene_font_kind_t kind;
    scene_font_tier_t baked;              /* when kind == SCENE_FONT_BAKED */
    uint8_t digest[ASSET_DIGEST_BYTES];   /* when kind == SCENE_FONT_ASSET */
    int32_t pixel_size;                   /* when kind == SCENE_FONT_ASSET */
} scene_font_ref_t;

/* A text node's string is either a literal or a binding the device evaluates
 * locally. The binding form is what keeps a clock ticking with the link down. */
typedef enum {
    SCENE_VALUE_LITERAL = 1,
    SCENE_VALUE_BINDING = 2,
} scene_value_kind_t;

typedef struct {
    scene_value_kind_t kind;
    char literal[SCENE_MAX_TEXT_BYTES + 1U];
    char binding[SCENE_MAX_BINDING + 1U];
} scene_value_t;

typedef enum {
    SCENE_ALIGN_LEFT = 1,
    SCENE_ALIGN_CENTER = 2,
    SCENE_ALIGN_RIGHT = 3,
} scene_align_t;

typedef struct { int32_t x, y, w, h, radius; uint32_t fill; uint8_t opacity; }
    scene_rect_t;
typedef struct { int32_t cx, cy, r, start_deg, end_deg, width; uint32_t color;
                 bool rounded; char end_binding[SCENE_MAX_BINDING + 1U]; }
    scene_arc_t;
typedef struct { int32_t xs[SCENE_MAX_LINE_POINTS], ys[SCENE_MAX_LINE_POINTS];
                 uint32_t point_count; int32_t width; uint32_t color; }
    scene_line_t;
/* `baseline_y` is the type's baseline, not the box top. The shipped templates
 * position by baseline (see digital_clock.c's baseline_offset()), so a
 * box-anchored node could not reproduce their pixels. */
typedef struct { int32_t x, baseline_y, w; scene_align_t align;
                 scene_font_ref_t font; uint32_t color; scene_value_t value;
                 bool ellipsize; }
    scene_text_t;
typedef struct { int32_t x, y, w, h; uint8_t digest[ASSET_DIGEST_BYTES];
                 bool recolor; uint32_t color; }
    scene_image_t;
typedef struct { int32_t x, baseline_y, size; uint8_t digest[ASSET_DIGEST_BYTES];
                 char name[SCENE_MAX_GLYPH_NAME + 1U]; uint32_t color; }
    scene_glyph_t;

typedef struct {
    scene_node_kind_t kind;
    union {
        scene_rect_t rect;
        scene_arc_t arc;
        scene_line_t line;
        scene_text_t text;
        scene_image_t image;
        scene_glyph_t glyph;
    } value;
} scene_node_t;

typedef struct {
    uint32_t revision;
    uint32_t background;
    uint32_t node_count;
    scene_node_t nodes[SCENE_MAX_NODES];
} scene_t;

typedef enum {
    SCENE_MODEL_OK = 0,
    SCENE_MODEL_ERR_ARGUMENT,
    SCENE_MODEL_ERR_NODE_COUNT,
    SCENE_MODEL_ERR_NODE_KIND,
    SCENE_MODEL_ERR_GEOMETRY,
    SCENE_MODEL_ERR_TEXT,
    SCENE_MODEL_ERR_FONT,
} scene_model_result_t;

scene_model_result_t scene_model_validate(const scene_t *scene);
```

- [x] **Step 4: Implement `scene_model_validate`**

Required behaviour, all covered by Step 1's tests:

- `node_count > SCENE_MAX_NODES` → `ERR_NODE_COUNT`.
- Any `kind` outside `scene_node_kind_t` → `ERR_NODE_KIND`.
- Every node's geometry must lie inside `0..SCENE_CANVAS_WIDTH` ×
  `0..SCENE_CANVAS_HEIGHT`. Reject rather than clip: a host computing an
  off-canvas layout has a bug and silent clipping hides it. Arithmetic must be
  overflow-safe — compute `x > WIDTH - w` rather than `x + w > WIDTH`.
- `line.point_count` outside `2..SCENE_MAX_LINE_POINTS` → `ERR_GEOMETRY`.
- A text node's `literal` must be NUL-terminated within `SCENE_MAX_TEXT_BYTES`,
  and its `binding` within `SCENE_MAX_BINDING`, else `ERR_TEXT`. Use `memchr`
  over the buffer; do not call `strlen` on a possibly-unterminated array.
- A `SCENE_FONT_BAKED` ref whose tier is outside `scene_font_tier_t` → `ERR_FONT`.
  A `SCENE_FONT_ASSET` ref with `pixel_size` outside `8..200` → `ERR_FONT`.

- [x] **Step 5: Register in both build systems and `.gitignore`**

Add to `firmware/host_tests/Makefile`'s `test:` dependency list, its run list, and:

```make
test_scene_model: test_scene_model.c ../main/core/scene_model.c \
	../main/core/asset_store.c
	$(CC) $(CFLAGS) -I../main -o $@ $^
```

Add `"core/scene_model.c"` to `SRCS` in `firmware/main/CMakeLists.txt`, and
`firmware/host_tests/test_scene_model` to `.gitignore` — that file lists every host-test
binary individually, and stage 1 left two out.

- [x] **Step 6: Run and commit**

```sh
make -C firmware/host_tests clean test
```

```bash
git add firmware/main/core/scene_model.h firmware/main/core/scene_model.c \
        firmware/host_tests/test_scene_model.c firmware/host_tests/Makefile \
        firmware/main/CMakeLists.txt .gitignore
git commit -m "feat: add the scene model and its bounds"
```

---

### Task 2: Bindings

**Files:**
- Create: `firmware/main/core/scene_binding.h`, `firmware/main/core/scene_binding.c`
- Create: `firmware/host_tests/test_scene_binding.c`
- Modify: `firmware/host_tests/Makefile`, `firmware/main/CMakeLists.txt`, `.gitignore`

**Interfaces:**
- Consumes: Task 1's `SCENE_MAX_BINDING`, `SCENE_MAX_TEXT_BYTES`.
- Produces: `scene_binding_kind_t`, `scene_binding_t`, `scene_binding_context_t`,
  `scene_field_fn`, `scene_binding_parse()`, `scene_binding_evaluate()`.

Bindings are **the only thing the device computes**. Everything else is decided by the
host. That is what keeps this surface small enough to audit and is why the set is closed
rather than an expression language. Evaluation is a pure function over an explicit context
so it is fully host-testable — no clock, no LVGL, no globals.

- [x] **Step 1: Write the failing test**

Create `firmware/host_tests/test_scene_binding.c`:

```c
#include <assert.h>
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

int main(void)
{
    test_time_formats_use_the_utc_offset();
    test_field_lookup();
    test_an_absent_field_renders_the_placeholder();
    test_an_inactive_timer_renders_the_placeholder();
    test_unknown_bindings_are_rejected_at_parse();
    test_evaluate_never_overruns_a_short_buffer();
    return 0;
}
```

- [x] **Step 2: Run to confirm failure**

```sh
make -C firmware/host_tests test_scene_binding
```

- [x] **Step 3: Write the header**

```c
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
    int64_t unix_seconds;
    int16_t utc_offset_minutes;
    bool timer_active;
    uint32_t timer_remaining_ms;
    uint8_t timer_pct;
    scene_field_fn field;
    void *field_ctx;
} scene_binding_context_t;

scene_binding_result_t scene_binding_parse(const char *text,
                                           scene_binding_t *out);
scene_binding_result_t scene_binding_evaluate(
    const scene_binding_t *binding, const scene_binding_context_t *context,
    char *out, size_t out_capacity);
```

- [x] **Step 4: Implement**

- `scene_binding_parse` recognises exactly four prefixes: `time:`, `timer.remaining:`,
  `timer.pct`, `field.`. Anything else is `ERR_UNKNOWN`.
- A `time:` or `timer.remaining:` argument must consist **only** of the characters
  `HhMmSs:` — this is a whitelist, not a `printf` format. Anything else is `ERR_FORMAT`.
  Never pass a host-supplied string to a `printf`-family format argument.
- `field.` with an empty name is `ERR_FORMAT`; the name is bounded by `SCENE_MAX_BINDING`.
- `scene_binding_evaluate` renders local time from `unix_seconds + utc_offset_minutes * 60`
  using the existing `core/timefmt.c` helpers rather than a second implementation.
- An absent field, or any timer binding with `timer_active == false`, renders the same
  placeholder the C templates use: `--` for a scalar, `--:--` for a clock-shaped value.
- Every write is bounded by `out_capacity`; a value that will not fit returns
  `ERR_CAPACITY` and writes nothing.

- [x] **Step 5: Register, run, commit**

Makefile rule mirrors Task 1's, adding `../main/core/timefmt.c`. Add
`"core/scene_binding.c"` to `SRCS` and the binary to `.gitignore`.

```sh
make -C firmware/host_tests clean test
```

```bash
git add firmware/main/core/scene_binding.h firmware/main/core/scene_binding.c \
        firmware/host_tests/test_scene_binding.c firmware/host_tests/Makefile \
        firmware/main/CMakeLists.txt .gitignore
git commit -m "feat: add the closed scene binding set"
```

---

## AMENDMENT (added during execution, 2026-08-23): Task 1b, the scale node

**This section was not in the plan as written.** It was added by controller ruling during
execution, and the two corrections above (the overview's "arc dial", and Task 9's "arc
angle convention") belong to it.

The plan rested on a factual error about the template it exists to reproduce.
`digital_clock.c:103` builds the dial with `lv_scale_create` — a scale **widget** whose 13
ticks LVGL generates internally, with minor ticks at width 2 / length 6 / `TERTIARY` and
major ticks at width 3 / length 11 / `LV_OPA_70`. It is not an arc and not a set of lines.
Only the two hands are `lv_line`. The six node kinds in Task 1 therefore cannot express the
dial, which made **Task 9 — the byte-identical parity gate this plan exists for —
unwinnable as written**.

Two alternatives were rejected. Emitting 13 line nodes would require reproducing LVGL's own
radial tick-endpoint math and rounding exactly: thirteen independent chances to be off by
one pixel, failing only at Task 9 across a wide search space. Descoping the dial would not
save the work, since stage 2b cannot retire the C `DigitalClock` without it either.

**Task 1b adds `SCENE_NODE_SCALE`**, carrying only the fields `digital_clock.c` varies, so
`scene_view` issues the same `lv_scale_*` calls and the pixels match by construction. This
is consistent with the model as it already is: `SCENE_NODE_ARC` already maps to `lv_arc`,
so this is a widget-mapped display list, not a primitive rasterizer. `sizeof(scene_node_t)`
must not grow. **Corrected after measurement:** the union's largest member is
`scene_text_t` at **252 bytes** — it carries the 128-byte literal and the 49-byte binding
inline — not `scene_line_t` at 76, as this amendment first claimed. `scene_node_t` is
**256 bytes** and unchanged; `scene_scale_t` is 24, so it fits far more comfortably than
the original rationale argued. The wire envelope is unaffected either way, because the
2034-byte bound applies to the CBOR encoding, not to the decoded struct.

Note the consequence, which is pre-existing rather than introduced here: `scene_t` is
`SCENE_MAX_NODES` × 256 ≈ **6 KB in RAM**. On a board with 0 bytes of IRAM headroom, where
~105 bytes of static DRAM once broke OTA outright, that is not a structure that can sit on
a stack or in `.bss` by accident. Task 3 decodes into one and must place it deliberately —
`protocol_task.c` already PSRAM-allocates its context for exactly this reason.

Task 1b runs **before Task 3**, so the wire tasks pick the node up naturally and Task 7
emits it. Its brief is `.superpowers/sdd/2026-08-23-deskmate-scene-renderer/task-1b-brief.md`.

A consequence worth recording: the concern that raised this — that `scene_line_t` has no
opacity field — is **dissolved**, not deferred. The `LV_OPA_70` lives on the scale's major
ticks and the two hands are full-opacity lines, so no opacity field is added to
`scene_line_t`.

---

### Task 3: Scene decoder

**Files:**
- Create: `firmware/main/core/scene_decode.c` (declaration goes in `scene_model.h`)
- Create: `firmware/host_tests/test_scene_decode.c`
- Modify: `firmware/main/core/scene_model.h`, `firmware/host_tests/Makefile`,
  `firmware/main/CMakeLists.txt`, `.gitignore`

**Interfaces:**
- Consumes: Task 1's `scene_t`; Task 2's `scene_binding_parse()`.
- Produces: `scene_decode(const uint8_t *payload, size_t length, scene_t *out)`
  → `scene_model_result_t`.

Every byte here comes from an untrusted host. Mirror the existing decoders in
`core/protocol_message.c` exactly — `read_key` for strictly-increasing integer keys,
`skip_value` for unknown ones, required-key bitmasks checked after the loop. Do not
invent a second decoding style.

**CBOR shape.** Scene: `{0: revision, 1: background, 2: [node, …]}`. Node:
`{0: kind, 1: <kind-specific map>}`. Keep every map's keys in deterministic order.

- [x] **Step 1: Write the failing test**

Create `firmware/host_tests/test_scene_decode.c` with hand-built CBOR, following
`test_protocol.c`'s fixture-builder style. Cover at minimum:

```c
static void test_a_valid_text_scene_roundtrips(void);
static void test_more_nodes_than_the_cap_is_rejected(void);
static void test_a_node_with_an_unknown_kind_is_rejected(void);
static void test_an_unknown_integer_key_is_skipped_not_rejected(void);
static void test_a_duplicate_key_is_rejected(void);
static void test_an_unterminated_utf8_text_is_rejected(void);
static void test_a_text_longer_than_the_cap_is_rejected(void);
static void test_an_unknown_binding_is_rejected_by_the_decoder(void);
static void test_trailing_bytes_after_the_scene_are_rejected(void);
static void test_recursion_is_bounded(void);
```

`test_an_unknown_binding_is_rejected_by_the_decoder` is the important one: the decoder must
call `scene_binding_parse()` while decoding, so a scene carrying a binding the device
cannot evaluate is refused **whole**. Accepting it and failing at draw time would leave the
panel half-rendered.

- [x] **Step 2: Run to confirm failure, then implement**

Add `scene_decode()`'s declaration to `scene_model.h` and implement it in a **separate**
`scene_decode.c`, so `scene_model.c` stays free of TinyCBOR and the simulator can build
scenes directly.

- [x] **Step 3: Register, run, commit**

```make
test_scene_decode: test_scene_decode.c ../main/core/scene_decode.c \
	../main/core/scene_model.c ../main/core/scene_binding.c \
	../main/core/timefmt.c ../main/core/asset_store.c $(CBOR_SRCS)
	$(CC) $(CFLAGS) -I$(CBOR_DIR) -I../main -o $@ $^
```

```sh
make -C firmware/host_tests clean test
```

```bash
git add firmware/main/core/scene_decode.c firmware/main/core/scene_model.h \
        firmware/host_tests/test_scene_decode.c firmware/host_tests/Makefile \
        firmware/main/CMakeLists.txt .gitignore
git commit -m "feat: decode a scene from untrusted CBOR"
```

---

### Task 4: Wire — message 19 and capability bit 8

**Files:**
- Modify: `firmware/main/core/protocol_message.h`, `firmware/main/core/protocol_message.c`
- Modify: `firmware/host_tests/test_protocol.c`
- Modify: `companion/crates/protocol/src/message.rs`, `companion/crates/protocol/src/lib.rs`
- Create: `companion/crates/protocol/src/scene.rs`
- Modify: `companion/crates/protocol/examples/generate-fixtures.rs`,
  `companion/crates/protocol/tests/fixtures.rs`, `protocol/fixtures/v1/`
- Modify: `docs/protocol/v1.md`

**Interfaces:**
- Consumes: Task 3's `scene_decode()`.
- Produces: `PROTOCOL_TYPE_PUSH_SCENE` (19), `protocol_push_scene_t`,
  `PROTOCOL_CAPABILITY_SCENE_RENDER` (`1 << 8`), `Message::PushScene`, Rust `Scene` types.

`PushScene` payload: `{0: card_id, 1: revision, 2: scene}` where `scene` is the map Task 3
decodes. Answered with `Ack{acknowledged_type: 19, revision}` — revision is **required**
here, like `PushData` and `ApplyConfig`, so extend that list rather than rewriting the
validator.

> **`PROTOCOL_CURRENT_CAPABILITIES` becomes `491`** (235 + 256). Pin the literal in a test
> on **both** sides. Bit 7 sat defined-but-unset for most of V2 and the constant read 75
> instead of 203; a conforming host could not have provisioned the device.

- [x] **Step 1: Failing tests, both languages**

C, in `test_protocol.c`: `test_push_scene_roundtrips`,
`test_push_scene_rejects_a_scene_over_the_node_cap`,
`test_ack_for_push_scene_requires_a_revision`, and
`test_current_capabilities_is_491` asserting the literal.

Rust, in `message.rs`: the mirror set plus `current_capabilities_is_491`.

- [x] **Step 2: Run both to confirm failure**

```sh
make -C firmware/host_tests test_protocol
export PATH="$HOME/.cargo/bin:$PATH" && cd companion && cargo test -p protocol
```

- [x] **Step 3: Implement both sides**

Mirror `decode_asset_begin`'s structure on the C side and `AssetBegin`'s on the Rust side.
Bounds must match exactly across languages: 24 nodes, 128-byte text, 48-byte binding,
8 line points.

- [x] **Step 4: Add fixtures to the shared corpus — do not skip this**

`protocol/fixtures/v1/` is read by **both** `companion/crates/protocol/tests/fixtures.rs`
and `firmware/host_tests/test_protocol.c`, whose `assert_valid_fixture()` re-encodes each
decoded message and `memcmp`s it against the file. That round-trip is the only mechanism
proving the two implementations agree byte for byte — in stage 1 it immediately exposed a
firmware defect both unit suites had passed over.

Add `push_scene.bin` and `ack_scene.bin`. Give the scene fixture **every node kind**, a
binding, a multi-byte CBOR coordinate, and a non-trivial colour. An all-minimal fixture
passes even when the encoders disagree.

- [x] **Step 5: Update `docs/protocol/v1.md`**

Add row 19 to the registry, document the payload and the scene map, note the bit-8 gate,
and state that the current capability value is `491`.

- [x] **Step 6: Run every gate and commit**

```sh
make -C firmware/host_tests clean test
cd companion && cargo fmt --all --check \
  && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

If firmware's `memcmp` fails, the encoders genuinely disagree — report it, do not adjust
the fixture to make it pass.

---

### Task 5: The scene interpreter

**Files:**
- Create: `firmware/main/ui/scene_view.h`, `firmware/main/ui/scene_view.c`
- Modify: `firmware/main/CMakeLists.txt`

**Interfaces:**
- Consumes: Task 1's `scene_t`; Task 2's `scene_binding_evaluate()`; Task 6's font handles.
- Produces: `scene_view_show(const scene_t *, const scene_binding_context_t *)`,
  `scene_view_refresh_bindings(const scene_binding_context_t *)`,
  `scene_view_destroy()`, `scene_view_active()`, `scene_view_screen()`.

**This file must contain no ESP-IDF include.** Task 8 compiles it into `lvgl-sim`, and
that is what the entire parity gate rests on. It may include `lvgl.h` and `core/scene_*.h`
and nothing else from the platform.

Follow `firmware/main/ui/template_view.c` for lifecycle shape — it is the existing
equivalent and its `show`/`destroy`/`active`/`screen` surface is the one the rest of the UI
already knows how to drive.

- [x] **Step 1: Implement the six node renderers**

- `rect` → `lv_obj_t` with radius, `bg_color`, `bg_opa`; scrollbars and borders removed.
- `arc` → `lv_arc` with start/end angle, width, rounded caps.
- `line` → `lv_line` over the point array.
- `text` → `lv_label`. **Position by baseline**: set the label's Y to
  `baseline_y - (lv_font_get_line_height(font) - font->base_line)`, the inverse of
  `digital_clock.c:44`'s `baseline_offset()`. Getting this wrong is invisible until the
  parity diff fails, so put the derivation in a comment.
- `image` → `lv_image` from the mapped asset bytes.
- `glyph` → `lv_label` with an icon-font face and the resolved codepoint.

Resolve a `SCENE_FONT_BAKED` tier to the identical `lv_font_t *` the C templates use —
`DESKMATE_FONT_CAPTION`/`BODY`/`DISPLAY`/`HERO` from `ui/templates/template_internal.h`.
That identity is what makes the parity gate winnable.

- [x] **Step 2: Implement binding refresh without rebuilding**

`scene_view_refresh_bindings()` re-evaluates only nodes whose value is a binding and
updates those labels (and any arc with an `end_binding`) in place. This is the scene
model's replacement for `template_view_patch()` — a clock tick must not rebuild the screen,
and a `PushData` field update must not either.

- [x] **Step 3: Build and commit**

```sh
. "$HOME/esp/esp-idf/export.sh" && idf.py -C firmware build && idf.py -C firmware size
```

Report DIRAM `.bss` and IRAM; neither may move meaningfully. There is no host test for
this file — Task 9 is its test, and it is a stronger one than any unit test would be.

---

### Task 6: Fix the font lifetime use-after-free

**Files:**
- Modify: `firmware/main/ui/font_registry.h`, `firmware/main/ui/font_registry.c`
- Modify: `firmware/main/link/protocol_task.c`
- Modify: `firmware/host_tests/` as needed

**Interfaces:**
- Consumes: stage 1's `font_registry_acquire/release/reset`.
- Produces: whatever mechanism you choose, documented in the header.

Stage 1 documented this and deliberately did not fix it, because its only caller was a
dev-only probe. **Stage 2 makes it live**: `scene_view` acquires fonts for real, in release
builds. The hazard, verified in stage 1's review:

> `font_registry_release()` only decrements `ref_count`; the LVGL label keeps the bare
> `lv_font_t *` in its style. So `ref_count` reaches 0 while a label still points at the
> face, and then **either** `font_registry_reset()` **or** ordinary `claim_slot()` LRU
> eviction destroys it. The pinned-face counter cannot see this — `ref_count` is already 0.
> With 8 slots, eviction is ordinary, not exotic.

Two viable fixes; pick one and justify it in your report:

- **Hold for the lifetime of the object.** `scene_view` keeps its acquire until it destroys
  the label, so `ref_count > 0` protects the face and eviction skips it. Simple, but a
  scene using 9+ distinct faces then fails to render rather than thrashing.
- **Invalidate on destroy.** The registry notifies `scene_view`, which rebuilds affected
  labels with a replacement face before the old one is freed. No cap, materially more
  machinery.

Whichever you choose, `font_registry_reset()` must remain safe to call before compaction —
compaction moves the mapped bytes, so a face surviving it reads moved memory.

- [x] **Steps: test, implement, build, commit** following the established pattern. Add a
      host test for whatever part is host-testable and say plainly which part is not.

---

### Task 7: Host-side scene builder

**Files:**
- Create: `companion/crates/app-core/src/scene_build.rs`
- Modify: `companion/crates/app-core/src/lib.rs`

**Interfaces:**
- Consumes: Task 4's Rust `Scene` types.
- Produces: `build_digital_clock_scene(&ClockCard, &Metrics) -> Scene`.

The host does **all** layout. This is where `digital_clock.c`'s geometry moves to Rust:
`TIME_Y = 8 * GRID`, `MODULE_Y = 22 * GRID`, `MODULE_H = 17 * GRID`, `DATE_W = 28 * GRID`,
`DIAL_X = 33 * GRID`, `DIAL_W = 20 * GRID`, `DIAL_BOX = 14 * GRID`, `GRID = 8`, plus the
dial's `DIAL_RANGE 720`, `HAND_HOUR_LEN 28`, `HAND_MINUTE_LEN 42`. Read the C file and port
the numbers exactly — do not re-derive them.

Two things the host must reproduce that are easy to miss:

1. **Baseline offsets.** Emit `baseline_y`, computed from the same font metrics the device
   uses. The metrics must come from the baked font data, not from a guess.
2. **Tier selection.** `template_internal.h:136`'s `deskmate_number_font()` picks `HERO` or
   `DISPLAY` depending on whether the string fits. The host now makes that choice and emits
   a fixed tier. Port the same rule.

- [x] **Steps: failing test, implement, gates, commit.** The test asserts the emitted
      scene's node count, kinds, and a few exact coordinates against the C constants.

---

### Task 8: Simulator scene rendering

**Files:**
- Modify: `companion/crates/lvgl-sim/build.rs`, `companion/crates/lvgl-sim/csrc/sim_shim.c`,
  `companion/crates/lvgl-sim/src/cases.rs`, `companion/crates/lvgl-sim/src/lib.rs`

**Interfaces:**
- Consumes: Task 5's `scene_view_show()`; Task 7's builder.
- Produces: `render_scene_png(&Scene) -> Vec<u8>`, plus scene golden cases.

Add `core/scene_model.c`, `core/scene_decode.c`, `core/scene_binding.c` and
`ui/scene_view.c` to `build.rs`'s source list — the same firmware sources, as it already
does for the templates. If any of them fails to compile on the host, an ESP-IDF include has
crept in and Task 5's constraint was violated.

Add one golden per node kind at both orientations, and **look at the PNGs** before
committing. A golden nobody looked at pins whatever bug shipped with it.

---

### Task 9: The parity gate

**Files:**
- Create: `companion/crates/app-core/tests/scene_parity.rs` **(delivered here, not in
  `cases.rs` as originally planned — see the note below)**
- Modify: `companion/crates/device/examples/framebuffer_diff.rs` if needed

**This task is the reason the plan exists.** Everything before it is unproven.

- [x] **Step 1: Add the paired case**

For a fixed time and a fixed `show_seconds`, render **both**: the shipped
`DigitalClock` C template via the existing path, and Task 7's scene via `scene_view`.
Assert the two framebuffers are **byte-identical**.

- [x] **Step 2: Run it and expect it to fail first**

It will not pass on the first attempt. The likely culprits, in order: baseline arithmetic
off by the `line_height`/`base_line` derivation; the dial's scale rotation and tick counts (LVGL
measures from 3 o'clock, the template may not); hand endpoints rounding differently;
a colour taken from the wrong palette entry.

Fix until identical. **Do not** relax the assertion to "close enough" — a fuzzy pixel gate
is worth nothing, and the whole argument for the scene renderer is that it can replace the
C templates exactly.

- [x] **Step 3: Extend to both orientations, then commit**

**Delivered, in `companion/crates/app-core/tests/scene_parity.rs`, not
`lvgl-sim/src/cases.rs` as this section's Files list says.** That file's own header
records the reason: nominating `cases.rs` would mean `lvgl-sim` naming `app-core`,
closing a cycle onto the existing `app-core -> device -> lvgl-sim` chain. Cargo
would tolerate it — `device -> lvgl-sim` is a dev-dependency, so the normal-dependency
graph stays acyclic — but it buys nothing: the gate needs neither a committed golden
nor a hardware push, so it renders both sides in-process and compares them directly,
leaving `cases.rs`'s `golden_cases()` at its pinned shape. A judgement call, not a
correction of the brief. The gate covers `INSTANTS` (7 pinned instants) × seconds
shown/hidden × two orientations = 28 comparisons, of which 14 are independent — the
flipped-orientation half is `sim_shim.c`'s `copy_frame_out` reversing the finished
buffer index-by-index, so `flipped(A) == flipped(B)` iff `A == B` and carries no
additional information. Real 270° geometry is provable only on hardware.

---

### Task 10: Device wiring

**Files:**
- Modify: `firmware/main/link/protocol_task.c`, `firmware/main/ui/ui_runtime.c`
- Also modified during execution: `firmware/main/core/protocol_message.h/.c`,
  `firmware/host_tests/test_protocol.c` — the capability gate moved into `core/` so a
  host test could reach it; see below.

- [x] Dispatch message 19: decode, validate, gate on capability bit 8, hand to
      `scene_view_show()` under `lvgl_port_lock()`, `Ack` with the revision.
- [x] On `PushData`, update the field values the binding context reads and call
      `scene_view_refresh_bindings()` — do not rebuild the scene.
- [x] On the clock tick, refresh time bindings the same way.
- [x] **Preserve the standalone clock.** A scene that fails to decode or validate must
      leave the previous scene up, or fall back to the clock — never blank the panel. Add
      a test for the malformed-scene path.
- [x] Build, report `.bss`/IRAM, commit.

**Delivered 2026-08-24** (`0e96c50`, `e82d636`). Notes that outlive the task:

- The capability gate is `protocol_message_request_gate()` in `core/protocol_message.c`,
  not three inline conditions in `protocol_task.c`. It moved so a host test can assert
  the invariant that broke here: *every capability bit advertised in
  `PROTOCOL_CURRENT_CAPABILITIES` has its message types dispatchable.* Bit 8 had shipped
  set while dispatch answered type 19 with "response type sent as request", the mirror of
  V2's bit 7 sitting defined-but-dark. The gate also implements the bit-7 gating on
  NetworkConfig/FactoryReset that `docs/protocol/v1.md` already documented and the device
  did not enforce.
- **The asset-GC teardown is a sequence:** load the clock → `scene_view_destroy()` →
  `font_registry_reset()` → compact → rebuild from the retained `scene_t`. The rebuild
  runs on every exit after the teardown, including the `BUSY` refusal, so a release that
  changes nothing leaves the panel unchanged too. It is skipped entirely while the OTA
  screen owns the panel, which correctly leaves the release deferred.
- The scene handoff is **synchronous under `lvgl_port_lock()`**, not via `ui_runtime`'s
  command queue: `ui_command_t` is a pair of internal-DRAM statics and `sizeof(scene_t)`
  is ~6 KB, so queueing a scene by value would add ~12 KB of `.bss`, and queueing a
  pointer would hand the LVGL task a pointer into `message` that the next frame
  overwrites.
- The binding tick lives in the protocol task loop at 250 ms (`template_view.c`'s
  cadence). `scene_view.c` owns no timer by design — it compiles into the simulator.
- **`.bss` moved, for the first time this plan: 102,608 → 102,624 (+16).** The map file
  attributes all of it to `ui/scene_view.c`'s three file-scope pointers (12 bytes,
  16 after alignment), which existed since Task 4 but were garbage-collected while
  nothing called into the file. `protocol_task.c`'s own `.bss` is byte-identical, `.data`
  is unchanged at 23,128, and IRAM is unchanged at 16,384/16,384 with 0 remaining. Task
  11's OTA-download check on the board is therefore **required, not optional.**

---

## AMENDMENT (added during execution, 2026-08-25): Task 10b, the host-side push path

Task 11's second check -- *the parity case on the panel* -- is blocked by something no
task in this plan owned. The device can receive and render a scene; **nothing can send
one.** `RuntimeDevice` has no `push_scene`, `device::Session` has no `push_scene`, and
`build_digital_clock_scene`'s only caller is the parity test. This was found on the board
on 2026-08-25 and is the reason the panel check did not run that session.

The cable is not a way around it. `net_config_usb_message_allowed()` admits only
status / heartbeat / ack / error / event / `NetworkConfig` / `FactoryReset` over USB in
networked tier, so a `PushScene` from `deskmate-cli` is refused with `WRONG_TIER` --
correctly, because the server owns the display. Flipping the device to local tier to
dodge that costs a device identity, since only digests are stored. The scene has to come
from the server.

### Task 10b: `push_scene` through the one ownership seam

**Files:**
- Modify: `companion/crates/device/src/session.rs`
- Modify: `companion/crates/app-core/src/runtime.rs`
- Modify: `companion/crates/server/src/runtime_device.rs`, `companion/crates/server/src/admin.rs`
- Modify: every other `RuntimeDevice` implementation, test doubles included

- [x] **Step 1: `Session::push_scene(&self, push: PushScene) -> Result<Ack, DeviceError>`**,
      shaped exactly like `activate_screen` / `asset_begin`: a plain request/reply that
      rejects anything but the matching `Ack`. Firmware acks `PushScene` *with* the
      revision, so the match arm must require it and check it equals the one sent -- that
      is the only host-side proof the device accepted the revision it was given. No
      capability check here; `Session` is a thin transport, and the device already gates
      on bit 8 in `protocol_message_request_gate()`.
- [x] **Step 2: `RuntimeDevice::push_scene(&mut self, push: PushScene)`.** Both real
      implementations delegate to the session. This goes on the trait, not on the
      WebSocket type alone: ownership has one implementation, not two that must agree, and
      Task 10a exists because that rule was nearly broken once already.
- [x] **Step 3: `RuntimeHandle::push_scene`, as a runtime command.** The route must not
      open a `device::Session` of its own -- that would put two processes on one cable,
      which is exactly what Task 10a refused. It enqueues a command the way
      `activate_screen` does, and the single owning worker sends it.
- [x] **Step 4: `POST /v1/devices/{id}/scene`**, behind the admin token with the other
      three routes. The body names the template and its inputs, not a serialised scene:
      `{"card_id": string, "revision": u32, "template": "digital_clock",
      "show_seconds": bool, "local_now": "YYYY-MM-DDTHH:MM:SS"}`. The server calls
      `build_digital_clock_scene(&ClockCard { .. }, &BakedFontMetrics::SHIPPED)` -- the
      same builder the parity gate compares against the C template, so the panel is shown
      the scene the golden was rendered from and there is no second JSON representation to
      drift. An unknown template is a typed 400.
- [x] **Step 5: gates.** `cargo fmt --all --check`, `cargo clippy --workspace
      --all-targets -- -D warnings`, `cargo test --workspace`. No firmware change, so
      `.bss` cannot move and this task needs no reflash.

**Delivered 2026-08-25** (`4d81b22`). Two notes that outlive the task:

- `DeviceClient::expected_response` in `device/src/lib.rs` also had to learn that
  `PushScene` is answered by an `Ack`. Without it the session rejects the message
  locally as `InvalidRequest`, before any transport is touched -- a second, quieter
  place the wire contract is written down.
- **The capability precheck immediately caught a fixture describing an impossible
  device.** The server's shared `support::sample_status()` advertised bits 0, 1 and 3 --
  no networking bit -- though every peer in those tests reached the server over WSS,
  which requires it. It now advertises `CURRENT_CAPABILITIES`, with the reason recorded
  in place. This is the same class of defect as bit 7 sitting defined-but-dark for most
  of V2: a capability claim nothing was checking.

**This is deliberately not render negotiation.** The spec's §3 resolves scene-vs-raster
per card per revision, and stage 3 is where the shipped clock card starts drawing itself
as a scene. This task adds the *path* and one dev-facing trigger for it, and decides no
policy. What it buys is Task 11's panel check, and every later stage's hardware check.

---


## AMENDMENT (added during execution, 2026-08-25): Task 11a, and Task 11 split in two

Task 11's panel check was written as one line -- "push the scene, confirm the panel
matches the simulator golden at both orientations" -- and reading the code to run it
showed it is two checks with different costs, different strengths, and no reason to
couple them.

**What the transports allow, established by reading the firmware rather than assumed:**

- `s_owner_usb_restricted` (`protocol_task.c`) is set at boot to *networked tier*, and
  every USB frame is then dispatched with `restricted_usb`. So `PushScene`, `ApplyConfig`,
  `PushData` and `TimeSync` over the cable are refused with `WRONG_TIER` while the device
  is networked. Driving the panel from USB requires local tier, and a tier round trip
  costs a device identity.
- **The dev-only 0x7E framebuffer capture is not gated by any of that.** `dispatch_request`
  intercepts `DEV_CAPTURE_REQUEST_TYPE` under `#ifdef DESKMATE_DEV_DIAG` and returns,
  *before* the decode, the capability gate and the tier gate. Capture works over USB in
  either tier. That is what makes a byte-exact panel check possible at all.

So Task 11's second line becomes:

- [ ] **The scene on the panel, over the shipping path.** Device on `v2.0.0-scene1` in
      networked tier, server carrying Task 10b: `POST /v1/devices/{id}/scene` and look at
      the panel. Proves server -> WSS -> `scene_decode` -> `scene_model_validate` ->
      `scene_view` -> LVGL end to end, on the release image the fleet actually runs. **No
      flash and no tier change**, so it costs a deploy and a minute. This is an observation,
      not a pixel claim -- record it as one.
- [ ] **Byte-exact parity on the panel, both orientations** (Task 11a's harness). This is
      the one that answers the question the parity gate provably cannot: the gate's flipped
      half is `sim_shim.c` reversing a finished buffer, so **real 270-degree geometry is
      only ever proven here**. Costs a `DESKMATE_DEV_DIAG=1` flash, a trip through local
      tier, and therefore a device identity; a release flash and a re-provision restore it.

Run them in that order. The first is nearly free and is worth having before committing to
the second.

### Task 11a: the scene panel check harness

**Files:**
- Add: `companion/crates/app-core/examples/scene_panel_check.rs`
- Modify: `companion/crates/device/src/` -- extract the 0x7E capture plumbing from
  `examples/framebuffer_diff.rs` into the crate so both callers share it

`build_digital_clock_scene` lives in `app-core`, and `app-core -> device -> lvgl-sim`, so
the harness **cannot** be another case inside `framebuffer_diff.rs`; the dependency edge
points the wrong way. It goes in `app-core/examples/`, beside `alert_replay_check.rs`,
which is the precedent for a hardware check that needs the whole stack.

- [ ] **Step 1: share the capture plumbing.** Move the 0x7E request / 0x7F chunk
      reassembly, the CRC check and the RGB565 frame constants out of
      `examples/framebuffer_diff.rs` and into the `device` crate. `framebuffer_diff.rs`
      keeps working through the shared code -- a second hand-rolled copy of a reassembler
      is how two harnesses come to disagree about what the device sent. The dev-only
      message ids stay out of the `protocol` crate for the reason its comment already
      gives: they are not part of the release wire contract.
- [ ] **Step 2: the check itself.** For each orientation in {90, 270} and each of the
      parity gate's cases, apply the rotation, push `Message::PushScene` carrying
      `build_digital_clock_scene(&ClockCard { .. }, &BakedFontMetrics::SHIPPED)`, time-sync
      last (`framebuffer_diff.rs` explains why: `dispatch_time_sync` enqueues the redraw and
      the UI queue drains FIFO), settle, capture, and byte-compare against
      `Simulator::render_scene` of the identical `SceneRenderRequest`. Reuse
      `tests/scene_parity.rs`'s `Difference`/`diff` reporting shape rather than a bare
      "differs" -- a pixel count and a bounding box localise a failure; a boolean does not.
- [ ] **Step 3: the asset-free scene cases too, if they cost nothing.** `cases::scene_cases()`
      is one scene per `scene_node_kind_t` at both orientations. Those needing no registered
      asset can go through the same loop for free. Any case that needs one is **excluded with
      a stated reason**, the way `framebuffer_diff.rs` already excludes
      `row-list--truncation-boundary` and `progress-ring--running-mid-countdown` -- an
      exclusion with a reason is evidence; a silently skipped case is not.
- [ ] **Step 4: gates.** `cargo fmt --all --check`, `cargo clippy --workspace
      --all-targets -- -D warnings`, `cargo test --workspace`. The example must compile in
      CI even though only hardware can run it.

**This harness is not a substitute for the first check.** It runs against a
`DESKMATE_DEV_DIAG=1` image over a cable, which is neither the image nor the path the
product ships. It proves pixels; the shipping-path observation proves the path.

---


### Task 11: Hardware verification (owner)

Minimal, per the owner's standing direction that hardware checks are kept small:

- [x] **An OTA download on the physical board**, from a build carrying this stage's
      `.bss`. This is the only check that catches the failure mode this codebase has proven
      no test can see, and stage 1's attempt was left **inconclusive** on a weak hotel link
      (RSSI -72 → -93, no download request reaching the server). Run it on a stable network.
      **Passed 2026-08-25** on `v2.0.0-scene1` (built at `55d1a97`) over
      `deskmate.rodi.one`: the first attempt failed with `download: ESP_FAIL` and the
      second succeeded with the identical image at the same signal, installed, rebooted
      and marked itself valid. Recorded as transient rather than the `.bss` hazard,
      because that hazard is deterministic -- see `docs/hardware/board-notes.md`,
      "Scene renderer OTA + boot -- verified 2026-08-25".
- [ ] **The scene on the panel, over the shipping path** (see the Task 11a amendment
      above, which splits this line in two and states what each half proves). Needs Task
      10b: the 2026-08-25 session found that no host can send a `PushScene` at all.
- [ ] **Byte-exact parity on the panel, both orientations.** Needs Task 11a's harness.
- [ ] Record both in `docs/hardware/board-notes.md`, stating plainly what was observed and
      what was not.

Everything else stage 1 deferred — glyph-cache timing, asset persistence, stack high-water
— stays deferred unless one of the above surfaces something.

## Exit criteria

1. `make -C firmware/host_tests clean test` passes.
2. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace` pass.
3. `idf.py -C firmware build` clean; `.bss` and IRAM deltas recorded.
4. **The `DigitalClock` scene is byte-identical to the C template at both orientations.**
5. An OTA download completes on the board.

Stage 2b — the remaining five templates, and retiring the C ones — is planned at this
plan's exit.
