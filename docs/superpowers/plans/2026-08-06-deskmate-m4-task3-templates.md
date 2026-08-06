# M4 Task 3 — Remaining Templates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `analog-clock`, `big-number-label`, and `icon-badge-text` display templates end to end — wire contract, firmware rendering, host lowering, and app preview — so weather and JSON-feed cards render real content instead of a title over five empty rows.

**Architecture:** The scaffolding already exists and this task fills it in. `DisplayTemplate` in `config.rs` already has all six variants; firmware already reserves `PROTOCOL_CAPABILITY_EXTENDED_TEMPLATES (1 << 3)` and `PROTOCOL_ERROR_UNSUPPORTED_TEMPLATE`. The three new templates become wire values 4/5/6, gated behind the extended-templates capability bit so an older image degrades deterministically rather than rendering garbage. Firmware keeps its existing shape: one declarative field registry per template in `core/template_fields.c` (plain C, host-tested) and one view file per template in `ui/templates/`.

**Tech Stack:** ESP-IDF 5.x / C with LVGL 9 (firmware), Rust workspace (`protocol`, `app-core`, `providers`), React 19 + TypeScript with Bun test (companion webview).

## Global Constraints

- Hardware-independent firmware logic lives under `firmware/main/core/`, contains no ESP-IDF includes, and is host-testable as plain C. LVGL object construction lives under `firmware/main/ui/`.
- Guard LVGL calls made outside LVGL callbacks/timers with `lvgl_port_lock()`/`lvgl_port_unlock()`. Never mutate LVGL objects from USB or protocol callbacks.
- Keep `board_lcd_rounder_cb` registered; every invalidation area rounds outward to even pixel boundaries, including under 90°/270° software rotation.
- Treat all bytes received from the host as untrusted: bound every length and count, reject malformed or unsupported messages, and recover framing without rebooting.
- The canvas is exactly 448x368 landscape. There is one layout per template. Size classes do not exist in the card model; `SizeClass::Full` is pinned on the wire. Do not reintroduce dashboards or a status strip.
- Preserve the standalone clock on boot, host loss, malformed input, and protocol version mismatch.
- `TEMPLATE_OBJECT_CAPACITY` is `14`. A template's entries in `view->objects[]` must not exceed it. Composite artwork uses a single container object whose children are not tracked in `objects[]`.
- `template_field_patch_t.dirty_mask` is `uint16_t`, so a template registry may declare at most **16** fields.
- The weather icon vocabulary is a closed set of exactly 11 values: `sun`, `moon`, `cloud`, `cloud-sun`, `cloud-moon`, `rain`, `drizzle`, `snow`, `storm`, `fog`, `unknown`. Any unrecognised value renders `unknown`.
- Use conventional commit prefixes (`feat:`, `fix:`, `test:`, `docs:`, `chore:`). Do not rewrite shared history.

### Verification commands

```sh
make -C firmware/host_tests clean test
cd companion && cargo fmt --all --check
cd companion && cargo clippy --workspace --all-targets -- -D warnings
cd companion && cargo test --workspace
cd companion/apps/deskmate && bun test && bun run build
. "$HOME/esp/esp-idf/export.sh" && idf.py -C firmware build
```

`cargo` and `bun` are not on the default PATH. Every shell step needs:

```sh
export PATH="$HOME/.cargo/bin:$HOME/.bun/bin:$PATH"
```

---

## File Structure

**Wire contract**
- `companion/crates/protocol/src/message.rs` — `TemplateKind` gains `AnalogClock = 4`, `BigNumberLabel = 5`, `IconBadgeText = 6`; `CURRENT_CAPABILITIES` gains `CAPABILITY_EXTENDED_TEMPLATES`.
- `firmware/main/core/protocol_message.h` — the mirrored C enum values.
- `firmware/main/core/protocol_message.c` — decode acceptance for 4/5/6.

**Firmware field schemas (plain C, host-tested)**
- `firmware/main/core/template_fields.c` — three new descriptor arrays plus registry dispatch.
- `firmware/host_tests/test_template_fields.c` — schema tests.

**Firmware model**
- `firmware/main/core/widget_model.c` — accept the new template kinds in `ApplyConfig`.
- `firmware/host_tests/test_widget_model.c` — acceptance tests.

**Firmware views (LVGL)**
- `firmware/main/ui/templates/analog_clock.c` — new.
- `firmware/main/ui/templates/big_number_label.c` — new.
- `firmware/main/ui/templates/icon_badge_text.c` — new.
- `firmware/main/ui/templates/weather_icon.c` / `.h` — new; the 11-icon vector set.
- `firmware/main/ui/templates/template_internal.h` — new create/patch/tick declarations.
- `firmware/main/ui/template_view.c` — dispatcher cases.
- `firmware/main/CMakeLists.txt` — register the new sources.

**Host lowering and defaults**
- `companion/crates/app-core/src/config.rs` — `wire_config()` lowering; per-kind default templates.

**App preview**
- `companion/apps/deskmate/src/components/DevicePreview.tsx` — render the three new templates.
- `companion/apps/deskmate/src/styles.css` — preview styles.

**Docs**
- `docs/config/v3.md`, `docs/protocol/v1.md`, `docs/hardware/board-notes.md`, `CLAUDE.md`.

---

### Task 1: Extend the wire contract with three template kinds

**Files:**
- Modify: `companion/crates/protocol/src/message.rs`
- Modify: `firmware/main/core/protocol_message.h`
- Modify: `firmware/main/core/protocol_message.c`
- Test: `companion/crates/protocol/tests/fixtures.rs`, `firmware/host_tests/test_protocol.c`

**Interfaces:**
- Consumes: nothing.
- Produces: `TemplateKind::AnalogClock = 4`, `TemplateKind::BigNumberLabel = 5`, `TemplateKind::IconBadgeText = 6` (Rust); `PROTOCOL_TEMPLATE_ANALOG_CLOCK = 4`, `PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL = 5`, `PROTOCOL_TEMPLATE_ICON_BADGE_TEXT = 6` (C). `CURRENT_CAPABILITIES` becomes `CAPABILITY_CORE_WIDGETS | CAPABILITY_CONFIG_ROTATION | CAPABILITY_EXTENDED_TEMPLATES` (value `11`).

- [ ] **Step 1: Write the failing Rust round-trip test**

Append to `companion/crates/protocol/tests/fixtures.rs`:

```rust
#[test]
fn extended_template_kinds_round_trip() {
    for (value, kind) in [
        (4u8, protocol::TemplateKind::AnalogClock),
        (5u8, protocol::TemplateKind::BigNumberLabel),
        (6u8, protocol::TemplateKind::IconBadgeText),
    ] {
        assert_eq!(
            protocol::template_kind_from_wire(value).expect("kind decodes"),
            kind,
            "wire value {value} must decode to {kind:?}"
        );
        assert_eq!(kind as u8, value, "{kind:?} must encode as {value}");
    }
}

#[test]
fn current_capabilities_advertise_extended_templates() {
    assert_eq!(
        protocol::CURRENT_CAPABILITIES,
        protocol::CAPABILITY_CORE_WIDGETS
            | protocol::CAPABILITY_CONFIG_ROTATION
            | protocol::CAPABILITY_EXTENDED_TEMPLATES,
        "extended templates must be advertised once firmware renders them"
    );
}
```

- [ ] **Step 2: Run it and confirm it fails**

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cd companion && cargo test -p protocol extended_template_kinds_round_trip
```

Expected: FAIL — `no variant named AnalogClock`.

- [ ] **Step 3: Add the Rust variants and bump capabilities**

In `companion/crates/protocol/src/message.rs`, extend the `TemplateKind` enum (currently `DigitalClock = 1, ProgressRing = 2, RowList = 3`) with:

```rust
    AnalogClock = 4,
    BigNumberLabel = 5,
    IconBadgeText = 6,
```

Extend the decoder near line 345 (`1 => Ok(TemplateKind::DigitalClock)`, etc.) with:

```rust
        4 => Ok(TemplateKind::AnalogClock),
        5 => Ok(TemplateKind::BigNumberLabel),
        6 => Ok(TemplateKind::IconBadgeText),
```

Change `CURRENT_CAPABILITIES`:

```rust
pub const CURRENT_CAPABILITIES: u64 =
    CAPABILITY_CORE_WIDGETS | CAPABILITY_CONFIG_ROTATION | CAPABILITY_EXTENDED_TEMPLATES;
```

Leave `LEGACY_CAPABILITIES` untouched.

- [ ] **Step 4: Confirm the Rust tests pass**

```sh
cd companion && cargo test -p protocol
```

Expected: PASS.

- [ ] **Step 5: Write the failing C test**

Append to `firmware/host_tests/test_protocol.c` a case asserting the three new kinds decode and that an out-of-range kind still rejects:

```c
static void test_extended_template_kinds(void)
{
    ASSERT_TRUE(protocol_template_kind_valid(PROTOCOL_TEMPLATE_ANALOG_CLOCK));
    ASSERT_TRUE(protocol_template_kind_valid(PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL));
    ASSERT_TRUE(protocol_template_kind_valid(PROTOCOL_TEMPLATE_ICON_BADGE_TEXT));
    ASSERT_EQ(4, (int)PROTOCOL_TEMPLATE_ANALOG_CLOCK);
    ASSERT_EQ(5, (int)PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL);
    ASSERT_EQ(6, (int)PROTOCOL_TEMPLATE_ICON_BADGE_TEXT);
    /* Unknown kinds must still be refused, not clamped. */
    ASSERT_FALSE(protocol_template_kind_valid((protocol_template_kind_t)7));
    ASSERT_FALSE(protocol_template_kind_valid((protocol_template_kind_t)0));
}
```

Register it in that file's `main()` alongside the existing cases, matching the surrounding style.

- [ ] **Step 6: Run it and confirm it fails**

```sh
make -C firmware/host_tests clean test
```

Expected: FAIL — `PROTOCOL_TEMPLATE_ANALOG_CLOCK` undeclared.

- [ ] **Step 7: Add the C enum values and the validity helper**

In `firmware/main/core/protocol_message.h`, extend the template enum (currently ending at `PROTOCOL_TEMPLATE_ROW_LIST = 3`):

```c
    PROTOCOL_TEMPLATE_ANALOG_CLOCK = 4,
    PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL = 5,
    PROTOCOL_TEMPLATE_ICON_BADGE_TEXT = 6,
```

Declare, and implement in `protocol_message.c`:

```c
bool protocol_template_kind_valid(protocol_template_kind_t kind)
{
    return kind >= PROTOCOL_TEMPLATE_DIGITAL_CLOCK &&
           kind <= PROTOCOL_TEMPLATE_ICON_BADGE_TEXT;
}
```

Replace any existing open-coded template-range check in `protocol_message.c` with a call to this helper so there is exactly one definition of the valid range.

- [ ] **Step 8: Confirm all firmware host tests pass**

```sh
make -C firmware/host_tests clean test
```

Expected: all 9 suites OK.

- [ ] **Step 9: Commit**

```sh
git add companion/crates/protocol firmware/main/core/protocol_message.h \
        firmware/main/core/protocol_message.c firmware/host_tests/test_protocol.c
git commit -m "feat: add extended template kinds to the wire contract"
```

---

### Task 2: Add field schemas for the three templates

**Files:**
- Modify: `firmware/main/core/template_fields.c`
- Test: `firmware/host_tests/test_template_fields.c`

**Interfaces:**
- Consumes: `PROTOCOL_TEMPLATE_ANALOG_CLOCK`, `PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL`, `PROTOCOL_TEMPLATE_ICON_BADGE_TEXT` from Task 1.
- Produces: registry entries so `template_fields_registry()` and `template_fields_resolve()` accept the new kinds. Field names other tasks depend on: analog-clock `title`/`show_seconds`; big-number-label `title`/`value`/`label`; icon-badge-text `title`/`icon`/`badge`/`value`/`label`.

**Temperature bounds, corrected during execution.** These are tenths of a degree *in the
unit the user selected*, not always Celsius. An earlier draft of this plan used `-1000..1000`,
which breaks at 100.1 F because `template_fields_resolve` rejects the **entire push** when a
single field falls out of range — the card would go stale rather than mis-render one value.
`-2000..2000` covers Celsius (-90..60) and Fahrenheit (-130..135) with headroom while staying
bounded against a malicious host. Do not narrow it back.

**Why icon-badge-text accepts fields it does not render.** The weather provider pushes ten fields: `title`, `value`, `label`, `badge`, `icon`, `temperature_tenths`, `apparent_temperature_tenths`, `unit`, `stale`, `error`. The layout renders five of them. Fields absent from a registry are counted in `widget_model`'s `unknown_field_count`, which `docs/hardware/board-notes.md` instructs hardware sessions to watch as a diagnostic signal. If three fields were unknown on every weather push, that counter would climb forever and stop meaning anything. So the schema declares all ten and the view renders five. Do not "clean this up" by removing the unrendered descriptors.

- [ ] **Step 1: Write the failing schema tests**

Append to `firmware/host_tests/test_template_fields.c`:

```c
static void test_extended_template_registries(void)
{
    size_t count = 0U;
    const template_field_descriptor_t *fields = NULL;

    fields = template_fields_registry(PROTOCOL_TEMPLATE_ANALOG_CLOCK, &count);
    ASSERT_TRUE(fields != NULL);
    ASSERT_EQ(4U, count);

    fields = template_fields_registry(PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL, &count);
    ASSERT_TRUE(fields != NULL);
    ASSERT_EQ(5U, count);

    fields = template_fields_registry(PROTOCOL_TEMPLATE_ICON_BADGE_TEXT, &count);
    ASSERT_TRUE(fields != NULL);
    ASSERT_EQ(10U, count);

    /* dirty_mask is uint16_t: no registry may exceed 16 fields. */
    for (int kind = PROTOCOL_TEMPLATE_DIGITAL_CLOCK;
         kind <= PROTOCOL_TEMPLATE_ICON_BADGE_TEXT; ++kind) {
        (void)template_fields_registry((protocol_template_kind_t)kind, &count);
        ASSERT_TRUE(count <= 16U);
    }
}

static void test_weather_push_has_no_unknown_fields(void)
{
    /* Every field the weather provider emits must be declared, so
     * unknown_field_count stays a real diagnostic signal. */
    static const char *emitted[] = {
        "title", "value", "label", "badge", "icon",
        "temperature_tenths", "apparent_temperature_tenths", "unit",
        "stale", "error",
    };
    size_t count = 0U;
    const template_field_descriptor_t *fields =
        template_fields_registry(PROTOCOL_TEMPLATE_ICON_BADGE_TEXT, &count);
    ASSERT_TRUE(fields != NULL);
    for (size_t i = 0U; i < sizeof(emitted) / sizeof(emitted[0]); ++i) {
        bool found = false;
        for (size_t j = 0U; j < count; ++j) {
            if (strcmp(fields[j].name, emitted[i]) == 0) {
                found = true;
                break;
            }
        }
        ASSERT_TRUE(found);
    }
}

static void test_big_number_label_defaults_are_safe(void)
{
    template_field_state_t state;
    ASSERT_TRUE(template_fields_init(PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL, &state));
    /* No required fields: an empty push must still yield a renderable state. */
    const template_field_value_t *value = template_fields_get(&state, "value");
    ASSERT_TRUE(value != NULL);
    ASSERT_EQ(0, strcmp(value->value.text, "--"));
}
```

Register all three in that file's `main()`.

- [ ] **Step 2: Run and confirm failure**

```sh
make -C firmware/host_tests clean test
```

Expected: FAIL — registry returns `NULL` and count `0` for the new kinds.

- [ ] **Step 3: Add the three descriptor arrays**

In `firmware/main/core/template_fields.c`, after `s_row_list_fields`, using the existing `TEXT_FIELD`/`BOOL_FIELD`/`INT_FIELD` macros:

```c
static const template_field_descriptor_t s_analog_clock_fields[] = {
    TEXT_FIELD("title", false, 64U, ""),
    BOOL_FIELD("show_seconds", false, true),
    BOOL_FIELD("stale", false, false),
    TEXT_FIELD("error", false, 96U, ""),
};

static const template_field_descriptor_t s_big_number_label_fields[] = {
    TEXT_FIELD("title", false, 64U, ""),
    TEXT_FIELD("value", false, 16U, "--"),
    TEXT_FIELD("label", false, 64U, ""),
    BOOL_FIELD("stale", false, false),
    TEXT_FIELD("error", false, 96U, ""),
};

/* Declares every field the weather provider emits, including the three the
 * layout does not draw, so unknown_field_count stays zero on a weather push.
 * See this task's note in the plan before removing any of them. */
static const template_field_descriptor_t s_icon_badge_text_fields[] = {
    TEXT_FIELD("title", false, 64U, ""),
    TEXT_FIELD("icon", false, 16U, "unknown"),
    TEXT_FIELD("badge", false, 64U, ""),
    TEXT_FIELD("value", false, 16U, "--"),
    TEXT_FIELD("label", false, 64U, ""),
    TEXT_FIELD("unit", false, 16U, ""),
    INT_FIELD("temperature_tenths", false, -2000, 2000, 0),
    INT_FIELD("apparent_temperature_tenths", false, -2000, 2000, 0),
    BOOL_FIELD("stale", false, false),
    TEXT_FIELD("error", false, 96U, ""),
};
```

- [ ] **Step 4: Extend the registry dispatch**

In `template_fields_registry()`, after the `PROTOCOL_TEMPLATE_ROW_LIST` branch:

```c
    } else if (template_kind == PROTOCOL_TEMPLATE_ANALOG_CLOCK) {
        fields = s_analog_clock_fields;
        count = array_length(sizeof(s_analog_clock_fields),
                             sizeof(s_analog_clock_fields[0]));
    } else if (template_kind == PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL) {
        fields = s_big_number_label_fields;
        count = array_length(sizeof(s_big_number_label_fields),
                             sizeof(s_big_number_label_fields[0]));
    } else if (template_kind == PROTOCOL_TEMPLATE_ICON_BADGE_TEXT) {
        fields = s_icon_badge_text_fields;
        count = array_length(sizeof(s_icon_badge_text_fields),
                             sizeof(s_icon_badge_text_fields[0]));
    }
```

- [ ] **Step 5: Confirm tests pass**

```sh
make -C firmware/host_tests clean test
```

Expected: all suites OK.

- [ ] **Step 6: Commit**

```sh
git add firmware/main/core/template_fields.c firmware/host_tests/test_template_fields.c
git commit -m "feat: add field schemas for the extended templates"
```

---

### Task 3: Accept the new template kinds in the widget model

**Files:**
- Modify: `firmware/main/core/widget_model.c`
- Test: `firmware/host_tests/test_widget_model.c`

**Interfaces:**
- Consumes: Task 1's enum values, Task 2's registries.
- Produces: `ApplyConfig` carrying template 4/5/6 returns `WIDGET_MODEL_CONFIG_APPLIED` instead of `WIDGET_MODEL_CONFIG_UNSUPPORTED_TEMPLATE`.

- [ ] **Step 1: Write the failing test**

Append to `firmware/host_tests/test_widget_model.c`:

```c
static void test_extended_templates_are_accepted(void)
{
    widget_model_t model;
    widget_model_init(&model);

    const protocol_template_kind_t kinds[] = {
        PROTOCOL_TEMPLATE_ANALOG_CLOCK,
        PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL,
        PROTOCOL_TEMPLATE_ICON_BADGE_TEXT,
    };
    for (size_t i = 0U; i < sizeof(kinds) / sizeof(kinds[0]); ++i) {
        protocol_apply_config_t config;
        build_single_widget_config(&config, "card", kinds[i], (uint32_t)(i + 1U));
        ASSERT_EQ(WIDGET_MODEL_CONFIG_APPLIED,
                  widget_model_apply_config(&model, &config));
    }

    /* An out-of-range kind must still be refused. */
    protocol_apply_config_t bad;
    build_single_widget_config(&bad, "card", (protocol_template_kind_t)7, 99U);
    ASSERT_EQ(WIDGET_MODEL_CONFIG_UNSUPPORTED_TEMPLATE,
              widget_model_apply_config(&model, &bad));
}
```

If `build_single_widget_config` does not already exist in this file, add it as a small local helper that zeroes a `protocol_apply_config_t`, sets `widget_count = 1`, `screen_count = 1`, copies the id into both the widget and the screen, sets `size_class = PROTOCOL_SIZE_FULL`, and sets the given template kind and revision. Register the test in `main()`.

- [ ] **Step 2: Run and confirm failure**

```sh
make -C firmware/host_tests clean test
```

Expected: FAIL — first `ASSERT_EQ` gets `WIDGET_MODEL_CONFIG_UNSUPPORTED_TEMPLATE`.

- [ ] **Step 3: Widen the acceptance check**

In `firmware/main/core/widget_model.c`, find the template validation that currently rejects anything above `PROTOCOL_TEMPLATE_ROW_LIST` and replace the condition with the single helper from Task 1:

```c
        if (!protocol_template_kind_valid(widget->template_kind)) {
            return WIDGET_MODEL_CONFIG_UNSUPPORTED_TEMPLATE;
        }
```

- [ ] **Step 4: Confirm tests pass**

```sh
make -C firmware/host_tests clean test
```

Expected: all suites OK.

- [ ] **Step 5: Commit**

```sh
git add firmware/main/core/widget_model.c firmware/host_tests/test_widget_model.c
git commit -m "feat: accept extended template kinds in the widget model"
```

---

### Task 4: Draw the weather icon set

**Files:**
- Create: `firmware/main/ui/templates/weather_icon.h`
- Create: `firmware/main/ui/templates/weather_icon.c`
- Modify: `firmware/main/CMakeLists.txt`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `typedef enum { WEATHER_ICON_UNKNOWN = 0, WEATHER_ICON_SUN, WEATHER_ICON_MOON, WEATHER_ICON_CLOUD, WEATHER_ICON_CLOUD_SUN, WEATHER_ICON_CLOUD_MOON, WEATHER_ICON_RAIN, WEATHER_ICON_DRIZZLE, WEATHER_ICON_SNOW, WEATHER_ICON_STORM, WEATHER_ICON_FOG } weather_icon_t;`
  - `weather_icon_t weather_icon_from_name(const char *name);`
  - `void weather_icon_render(lv_obj_t *container, weather_icon_t icon, lv_color_t color);`

**Design note.** Icons are composed from LVGL primitives (`lv_obj_t` circles via full corner radius, and `lv_line`) inside a caller-owned container. They are *not* font glyphs and *not* pushed assets: asset transfer is M4 Task 6, and a bounded built-in set cannot fail to resolve. `weather_icon_render()` deletes the container's existing children before drawing, so repeated calls are idempotent. The container itself is one entry in `objects[]`; its children are untracked, which is how the icon stays within `TEMPLATE_OBJECT_CAPACITY`.

`weather_icon_from_name()` must be a pure string→enum mapping with no LVGL dependency, so Task 4's tests can run on the host.

- [ ] **Step 1: Write the failing host test for name mapping**

Create `firmware/host_tests/test_weather_icon.c`:

```c
#include <string.h>

#include "test_assert.h"
#include "ui/templates/weather_icon.h"

static void test_known_names_map(void)
{
    ASSERT_EQ(WEATHER_ICON_SUN, weather_icon_from_name("sun"));
    ASSERT_EQ(WEATHER_ICON_MOON, weather_icon_from_name("moon"));
    ASSERT_EQ(WEATHER_ICON_CLOUD, weather_icon_from_name("cloud"));
    ASSERT_EQ(WEATHER_ICON_CLOUD_SUN, weather_icon_from_name("cloud-sun"));
    ASSERT_EQ(WEATHER_ICON_CLOUD_MOON, weather_icon_from_name("cloud-moon"));
    ASSERT_EQ(WEATHER_ICON_RAIN, weather_icon_from_name("rain"));
    ASSERT_EQ(WEATHER_ICON_DRIZZLE, weather_icon_from_name("drizzle"));
    ASSERT_EQ(WEATHER_ICON_SNOW, weather_icon_from_name("snow"));
    ASSERT_EQ(WEATHER_ICON_STORM, weather_icon_from_name("storm"));
    ASSERT_EQ(WEATHER_ICON_FOG, weather_icon_from_name("fog"));
    ASSERT_EQ(WEATHER_ICON_UNKNOWN, weather_icon_from_name("unknown"));
}

static void test_unrecognised_input_falls_back(void)
{
    ASSERT_EQ(WEATHER_ICON_UNKNOWN, weather_icon_from_name(""));
    ASSERT_EQ(WEATHER_ICON_UNKNOWN, weather_icon_from_name("SUN"));
    ASSERT_EQ(WEATHER_ICON_UNKNOWN, weather_icon_from_name("sunny"));
    ASSERT_EQ(WEATHER_ICON_UNKNOWN, weather_icon_from_name("../../etc/passwd"));
    ASSERT_EQ(WEATHER_ICON_UNKNOWN, weather_icon_from_name(NULL));
}

int main(void)
{
    test_known_names_map();
    test_unrecognised_input_falls_back();
    printf("test_weather_icon: OK\n");
    return 0;
}
```

Add `test_weather_icon` to `firmware/host_tests/Makefile` following the pattern of the existing suites.

- [ ] **Step 2: Run and confirm failure**

```sh
make -C firmware/host_tests clean test
```

Expected: FAIL — `weather_icon.h` not found.

- [ ] **Step 3: Write the header**

Create `firmware/main/ui/templates/weather_icon.h`:

```c
#pragma once

/* The closed weather icon vocabulary. The host may send any string; anything
 * outside this set renders WEATHER_ICON_UNKNOWN. Keep this list in sync with
 * the weather provider in companion/crates/providers/src/weather.rs. */
typedef enum {
    WEATHER_ICON_UNKNOWN = 0,
    WEATHER_ICON_SUN,
    WEATHER_ICON_MOON,
    WEATHER_ICON_CLOUD,
    WEATHER_ICON_CLOUD_SUN,
    WEATHER_ICON_CLOUD_MOON,
    WEATHER_ICON_RAIN,
    WEATHER_ICON_DRIZZLE,
    WEATHER_ICON_SNOW,
    WEATHER_ICON_STORM,
    WEATHER_ICON_FOG,
} weather_icon_t;

/* Pure mapping, no LVGL dependency, NULL-safe. */
weather_icon_t weather_icon_from_name(const char *name);
```

Then, in a separate `#ifndef WEATHER_ICON_HOST_TEST` guard at the bottom so the host test links without LVGL:

```c
#ifndef WEATHER_ICON_HOST_TEST
#include "lvgl.h"

/* Clears `container`'s children and draws `icon` into it using LVGL
 * primitives. Idempotent. The container must already be sized. */
void weather_icon_render(lv_obj_t *container, weather_icon_t icon,
                         lv_color_t color);
#endif
```

The host test compiles with `-DWEATHER_ICON_HOST_TEST`.

- [ ] **Step 4: Implement the name mapping**

Create `firmware/main/ui/templates/weather_icon.c` starting with the pure mapping:

```c
#include "weather_icon.h"

#include <string.h>

weather_icon_t weather_icon_from_name(const char *name)
{
    if (name == NULL) {
        return WEATHER_ICON_UNKNOWN;
    }
    static const struct {
        const char *name;
        weather_icon_t icon;
    } table[] = {
        { "sun", WEATHER_ICON_SUN },
        { "moon", WEATHER_ICON_MOON },
        { "cloud", WEATHER_ICON_CLOUD },
        { "cloud-sun", WEATHER_ICON_CLOUD_SUN },
        { "cloud-moon", WEATHER_ICON_CLOUD_MOON },
        { "rain", WEATHER_ICON_RAIN },
        { "drizzle", WEATHER_ICON_DRIZZLE },
        { "snow", WEATHER_ICON_SNOW },
        { "storm", WEATHER_ICON_STORM },
        { "fog", WEATHER_ICON_FOG },
        { "unknown", WEATHER_ICON_UNKNOWN },
    };
    for (size_t i = 0U; i < sizeof(table) / sizeof(table[0]); ++i) {
        if (strcmp(name, table[i].name) == 0) {
            return table[i].icon;
        }
    }
    return WEATHER_ICON_UNKNOWN;
}
```

- [ ] **Step 5: Confirm the mapping test passes**

```sh
make -C firmware/host_tests clean test
```

Expected: `test_weather_icon: OK`, all other suites still OK.

- [ ] **Step 6: Implement the drawing, guarded from the host build**

Append to `weather_icon.c`, inside `#ifndef WEATHER_ICON_HOST_TEST`. Use these shared helpers so every icon is built from two primitives:

```c
#ifndef WEATHER_ICON_HOST_TEST
#include "lvgl.h"

#define ICON_BOX 120

static lv_obj_t *disc(lv_obj_t *parent, lv_color_t color, int16_t diameter,
                      int16_t x, int16_t y)
{
    lv_obj_t *obj = lv_obj_create(parent);
    lv_obj_remove_style_all(obj);
    lv_obj_remove_flag(obj, LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(obj, diameter, diameter);
    lv_obj_set_style_radius(obj, LV_RADIUS_CIRCLE, 0);
    lv_obj_set_style_bg_color(obj, color, 0);
    lv_obj_set_style_bg_opa(obj, LV_OPA_COVER, 0);
    lv_obj_align(obj, LV_ALIGN_CENTER, x, y);
    return obj;
}

static lv_obj_t *bar(lv_obj_t *parent, lv_color_t color, int16_t w, int16_t h,
                     int16_t x, int16_t y)
{
    lv_obj_t *obj = lv_obj_create(parent);
    lv_obj_remove_style_all(obj);
    lv_obj_remove_flag(obj, LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(obj, w, h);
    lv_obj_set_style_radius(obj, h / 2, 0);
    lv_obj_set_style_bg_color(obj, color, 0);
    lv_obj_set_style_bg_opa(obj, LV_OPA_COVER, 0);
    lv_obj_align(obj, LV_ALIGN_CENTER, x, y);
    return obj;
}

/* A cloud is three discs and a slab; every cloudy icon reuses it. */
static void draw_cloud(lv_obj_t *parent, lv_color_t color, int16_t dy)
{
    (void)disc(parent, color, 44, -18, dy);
    (void)disc(parent, color, 56, 4, dy - 8);
    (void)disc(parent, color, 40, 26, dy + 2);
    (void)bar(parent, color, 88, 26, 2, dy + 12);
}

static void draw_sun(lv_obj_t *parent, lv_color_t color, int16_t dx,
                     int16_t dy, int16_t diameter)
{
    (void)disc(parent, color, diameter, dx, dy);
}

static void draw_moon(lv_obj_t *parent, lv_color_t color, int16_t dx,
                      int16_t dy)
{
    /* Crescent: a lit disc with a background-coloured disc offset over it. */
    (void)disc(parent, color, 56, dx, dy);
    (void)disc(parent, lv_color_hex(0x000000), 48, dx + 16, dy - 8);
}

static void draw_drops(lv_obj_t *parent, lv_color_t color, int16_t count,
                       int16_t length)
{
    const int16_t xs[] = { -24, 0, 24 };
    for (int16_t i = 0; i < count && i < 3; ++i) {
        (void)bar(parent, color, 6, length, xs[i], 40);
    }
}

void weather_icon_render(lv_obj_t *container, weather_icon_t icon,
                         lv_color_t color)
{
    if (container == NULL) {
        return;
    }
    lv_obj_clean(container);
    lv_obj_set_size(container, ICON_BOX, ICON_BOX);

    switch (icon) {
    case WEATHER_ICON_SUN:
        draw_sun(container, color, 0, 0, 72);
        break;
    case WEATHER_ICON_MOON:
        draw_moon(container, color, 0, 0);
        break;
    case WEATHER_ICON_CLOUD:
        draw_cloud(container, color, 0);
        break;
    case WEATHER_ICON_CLOUD_SUN:
        draw_sun(container, color, -26, -30, 44);
        draw_cloud(container, color, 6);
        break;
    case WEATHER_ICON_CLOUD_MOON:
        draw_moon(container, color, -26, -30);
        draw_cloud(container, color, 6);
        break;
    case WEATHER_ICON_RAIN:
        draw_cloud(container, color, -12);
        draw_drops(container, color, 3, 22);
        break;
    case WEATHER_ICON_DRIZZLE:
        draw_cloud(container, color, -12);
        draw_drops(container, color, 2, 12);
        break;
    case WEATHER_ICON_SNOW:
        draw_cloud(container, color, -12);
        (void)disc(container, color, 10, -24, 42);
        (void)disc(container, color, 10, 0, 46);
        (void)disc(container, color, 10, 24, 42);
        break;
    case WEATHER_ICON_STORM:
        draw_cloud(container, color, -12);
        (void)bar(container, color, 10, 40, 0, 42);
        break;
    case WEATHER_ICON_FOG:
        draw_cloud(container, color, -16);
        (void)bar(container, color, 84, 8, -4, 30);
        (void)bar(container, color, 68, 8, 6, 46);
        break;
    case WEATHER_ICON_UNKNOWN:
    default:
        /* A hollow ring: unmistakably "no data", never a plausible-looking
         * wrong forecast. */
        (void)disc(container, color, 72, 0, 0);
        (void)disc(container, lv_color_hex(0x000000), 52, 0, 0);
        break;
    }
}
#endif
```

- [ ] **Step 7: Register the source and confirm the firmware builds**

Add `ui/templates/weather_icon.c` to the `SRCS` list in `firmware/main/CMakeLists.txt`, then:

```sh
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build
```

Expected: build succeeds.

- [ ] **Step 8: Commit**

```sh
git add firmware/main/ui/templates/weather_icon.c firmware/main/ui/templates/weather_icon.h \
        firmware/main/CMakeLists.txt firmware/host_tests/test_weather_icon.c \
        firmware/host_tests/Makefile
git commit -m "feat: add the bounded weather icon set"
```

---

### Task 5: Implement the analog-clock template view

**Files:**
- Create: `firmware/main/ui/templates/analog_clock.c`
- Modify: `firmware/main/ui/templates/template_internal.h`
- Modify: `firmware/main/ui/template_view.c`
- Modify: `firmware/main/CMakeLists.txt`

**Interfaces:**
- Consumes: Task 2's `s_analog_clock_fields` (`title`, `show_seconds`).
- Produces: `analog_clock_create(view, parent, size)`, `analog_clock_patch(view, fields, dirty_mask)`, `analog_clock_tick(view, utc_offset_minutes)` — signatures identical to the `digital_clock_*` trio.

**Layout on 448x368.** A centred circular face of diameter 300, twelve hour ticks, an hour hand of length 82, a minute hand of length 120, and a second hand of length 132 shown only when `show_seconds` is true. Title sits at the top, the shared state label at the bottom.

- [ ] **Step 1: Declare the entry points**

Add to `firmware/main/ui/templates/template_internal.h`, following the existing declarations:

```c
bool analog_clock_create(template_widget_view_t *view,
                         lv_obj_t *parent,
                         protocol_size_class_t size);
void analog_clock_patch(template_widget_view_t *view,
                        const template_field_state_t *fields,
                        uint16_t dirty_mask);
void analog_clock_tick(template_widget_view_t *view,
                       int16_t utc_offset_minutes);
```

- [ ] **Step 2: Implement the view**

Create `firmware/main/ui/templates/analog_clock.c`:

```c
#include "template_internal.h"

#include <string.h>
#include <time.h>

enum {
    OBJ_TITLE,
    OBJ_FACE,
    OBJ_HOUR,
    OBJ_MINUTE,
    OBJ_SECOND,
    OBJ_HUB,
    OBJ_STATE,
};

#define FACE_DIAMETER 300
#define HAND_HOUR_LEN 82
#define HAND_MINUTE_LEN 120
#define HAND_SECOND_LEN 132

static bool s_show_seconds = true;

static lv_obj_t *make_hand(lv_obj_t *parent, int16_t length, int16_t width,
                           lv_color_t color)
{
    lv_obj_t *hand = lv_obj_create(parent);
    lv_obj_remove_style_all(hand);
    lv_obj_remove_flag(hand, LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(hand, width, length);
    lv_obj_set_style_radius(hand, width / 2, 0);
    lv_obj_set_style_bg_color(hand, color, 0);
    lv_obj_set_style_bg_opa(hand, LV_OPA_COVER, 0);
    /* Pivot at the bottom centre so rotation sweeps around the hub. */
    lv_obj_set_style_transform_pivot_x(hand, width / 2, 0);
    lv_obj_set_style_transform_pivot_y(hand, length, 0);
    return hand;
}

static void set_hand_angle(lv_obj_t *hand, int32_t degrees)
{
    /* LVGL transform_angle is in 0.1 degree units. */
    lv_obj_set_style_transform_angle(hand, degrees * 10, 0);
}

bool analog_clock_create(template_widget_view_t *view,
                         lv_obj_t *parent,
                         protocol_size_class_t size)
{
    if (view == NULL || parent == NULL ||
        (size != PROTOCOL_SIZE_FULL && size != PROTOCOL_SIZE_STANDARD)) {
        return false;
    }
    memset(view, 0, sizeof(*view));
    view->root = lv_obj_create(parent);
    lv_obj_remove_style_all(view->root);
    lv_obj_remove_flag(view->root,
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->root, LV_PCT(100), LV_PCT(100));

    view->objects[OBJ_TITLE] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_TITLE],
                                lv_color_hex(0x8f93a8), 0);
    lv_obj_align(view->objects[OBJ_TITLE], LV_ALIGN_TOP_MID, 0, 14);

    view->objects[OBJ_FACE] = lv_obj_create(view->root);
    lv_obj_remove_style_all(view->objects[OBJ_FACE]);
    lv_obj_remove_flag(view->objects[OBJ_FACE],
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->objects[OBJ_FACE], FACE_DIAMETER, FACE_DIAMETER);
    lv_obj_set_style_radius(view->objects[OBJ_FACE], LV_RADIUS_CIRCLE, 0);
    lv_obj_set_style_border_width(view->objects[OBJ_FACE], 3, 0);
    lv_obj_set_style_border_color(view->objects[OBJ_FACE],
                                  lv_color_hex(0x3a3d4d), 0);
    lv_obj_center(view->objects[OBJ_FACE]);

    /* Twelve ticks, children of the face and therefore untracked. */
    for (int i = 0; i < 12; ++i) {
        lv_obj_t *tick = lv_obj_create(view->objects[OBJ_FACE]);
        lv_obj_remove_style_all(tick);
        lv_obj_remove_flag(tick,
                           LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
        bool major = (i % 3) == 0;
        lv_obj_set_size(tick, major ? 6 : 3, major ? 18 : 10);
        lv_obj_set_style_bg_color(
            tick, lv_color_hex(major ? 0xe6e8f0 : 0x6a6d80), 0);
        lv_obj_set_style_bg_opa(tick, LV_OPA_COVER, 0);
        lv_obj_set_style_transform_pivot_x(tick, (major ? 6 : 3) / 2, 0);
        lv_obj_set_style_transform_pivot_y(tick, FACE_DIAMETER / 2 - 8, 0);
        lv_obj_align(tick, LV_ALIGN_TOP_MID, 0, 8);
        lv_obj_set_style_transform_angle(tick, i * 300, 0);
    }

    view->objects[OBJ_HOUR] =
        make_hand(view->objects[OBJ_FACE], HAND_HOUR_LEN, 10,
                  lv_color_hex(0xffffff));
    lv_obj_align(view->objects[OBJ_HOUR], LV_ALIGN_CENTER, 0,
                 -HAND_HOUR_LEN / 2);

    view->objects[OBJ_MINUTE] =
        make_hand(view->objects[OBJ_FACE], HAND_MINUTE_LEN, 7,
                  lv_color_hex(0xffffff));
    lv_obj_align(view->objects[OBJ_MINUTE], LV_ALIGN_CENTER, 0,
                 -HAND_MINUTE_LEN / 2);

    view->objects[OBJ_SECOND] =
        make_hand(view->objects[OBJ_FACE], HAND_SECOND_LEN, 3,
                  lv_color_hex(0xf2c14e));
    lv_obj_align(view->objects[OBJ_SECOND], LV_ALIGN_CENTER, 0,
                 -HAND_SECOND_LEN / 2);

    view->objects[OBJ_HUB] = lv_obj_create(view->objects[OBJ_FACE]);
    lv_obj_remove_style_all(view->objects[OBJ_HUB]);
    lv_obj_remove_flag(view->objects[OBJ_HUB],
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->objects[OBJ_HUB], 16, 16);
    lv_obj_set_style_radius(view->objects[OBJ_HUB], LV_RADIUS_CIRCLE, 0);
    lv_obj_set_style_bg_color(view->objects[OBJ_HUB],
                              lv_color_hex(0xf2c14e), 0);
    lv_obj_set_style_bg_opa(view->objects[OBJ_HUB], LV_OPA_COVER, 0);
    lv_obj_center(view->objects[OBJ_HUB]);

    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_MID, 0, -10);
    view->state_label = view->objects[OBJ_STATE];
    return true;
}

void analog_clock_patch(template_widget_view_t *view,
                        const template_field_state_t *fields,
                        uint16_t dirty_mask)
{
    (void)dirty_mask;
    if (view == NULL || view->root == NULL || fields == NULL) {
        return;
    }
    const template_field_value_t *title =
        template_fields_get(fields, "title");
    const template_field_value_t *show =
        template_fields_get(fields, "show_seconds");
    if (title != NULL) {
        lv_label_set_text(view->objects[OBJ_TITLE], title->value.text);
    }
    if (show != NULL) {
        s_show_seconds = show->value.boolean;
        if (s_show_seconds) {
            lv_obj_remove_flag(view->objects[OBJ_SECOND], LV_OBJ_FLAG_HIDDEN);
        } else {
            lv_obj_add_flag(view->objects[OBJ_SECOND], LV_OBJ_FLAG_HIDDEN);
        }
    }
}

void analog_clock_tick(template_widget_view_t *view,
                       int16_t utc_offset_minutes)
{
    if (view == NULL || view->root == NULL) {
        return;
    }
    time_t local = time(NULL) + (time_t)utc_offset_minutes * 60;
    struct tm now;
    if (gmtime_r(&local, &now) == NULL) {
        return;
    }
    int32_t hour12 = now.tm_hour % 12;
    set_hand_angle(view->objects[OBJ_HOUR],
                   (hour12 * 30) + (now.tm_min / 2));
    set_hand_angle(view->objects[OBJ_MINUTE], now.tm_min * 6);
    if (s_show_seconds) {
        set_hand_angle(view->objects[OBJ_SECOND], now.tm_sec * 6);
    }
}
```

- [ ] **Step 3: Wire the dispatcher**

In `firmware/main/ui/template_view.c`, add `PROTOCOL_TEMPLATE_ANALOG_CLOCK` cases to `create_widget()`, `patch_widget()`, and the tick path, mirroring exactly how `PROTOCOL_TEMPLATE_DIGITAL_CLOCK` is handled (the digital clock is the only other template with a `tick` that consumes `utc_offset_minutes`).

- [ ] **Step 4: Register the source and build**

Add `ui/templates/analog_clock.c` to `firmware/main/CMakeLists.txt`, then:

```sh
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build
```

Expected: build succeeds. Note the reported binary size; it must stay well under the `0x100000` app partition.

- [ ] **Step 5: Confirm host tests still pass**

```sh
make -C firmware/host_tests clean test
```

Expected: all suites OK.

- [ ] **Step 6: Commit**

```sh
git add firmware/main/ui/templates/analog_clock.c \
        firmware/main/ui/templates/template_internal.h \
        firmware/main/ui/template_view.c firmware/main/CMakeLists.txt
git commit -m "feat: add the analog-clock template view"
```

---

### Task 6: Implement the big-number-label template view

**Files:**
- Create: `firmware/main/ui/templates/big_number_label.c`
- Modify: `firmware/main/ui/templates/template_internal.h`
- Modify: `firmware/main/ui/template_view.c`
- Modify: `firmware/main/CMakeLists.txt`

**Interfaces:**
- Consumes: Task 2's `s_big_number_label_fields` (`title`, `value`, `label`).
- Produces: `big_number_label_create(view, parent, size)`, `big_number_label_patch(view, fields, dirty_mask)`. No tick.

**Layout on 448x368.** Title top-left in muted small caps; `value` centred in `lv_font_montserrat_48` as the hero; `label` directly beneath it; the shared state label at the bottom. `value` is capped at 16 characters by the schema, so it cannot overflow the panel.

- [ ] **Step 1: Declare the entry points**

Add to `template_internal.h`:

```c
bool big_number_label_create(template_widget_view_t *view,
                             lv_obj_t *parent,
                             protocol_size_class_t size);
void big_number_label_patch(template_widget_view_t *view,
                            const template_field_state_t *fields,
                            uint16_t dirty_mask);
```

- [ ] **Step 2: Implement the view**

Create `firmware/main/ui/templates/big_number_label.c`:

```c
#include "template_internal.h"

#include <string.h>

enum {
    OBJ_TITLE,
    OBJ_VALUE,
    OBJ_LABEL,
    OBJ_STATE,
};

bool big_number_label_create(template_widget_view_t *view,
                             lv_obj_t *parent,
                             protocol_size_class_t size)
{
    if (view == NULL || parent == NULL ||
        (size != PROTOCOL_SIZE_FULL && size != PROTOCOL_SIZE_STANDARD)) {
        return false;
    }
    memset(view, 0, sizeof(*view));
    view->root = lv_obj_create(parent);
    lv_obj_remove_style_all(view->root);
    lv_obj_remove_flag(view->root,
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->root, LV_PCT(100), LV_PCT(100));

    view->objects[OBJ_TITLE] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_TITLE],
                                lv_color_hex(0x8f93a8), 0);
    lv_obj_align(view->objects[OBJ_TITLE], LV_ALIGN_TOP_LEFT, 28, 24);
    lv_label_set_text(view->objects[OBJ_TITLE], "");

    view->objects[OBJ_VALUE] = lv_label_create(view->root);
    lv_obj_set_style_text_font(view->objects[OBJ_VALUE],
                               &lv_font_montserrat_48, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_VALUE],
                                lv_color_hex(0xf2c14e), 0);
    lv_obj_align(view->objects[OBJ_VALUE], LV_ALIGN_CENTER, 0, -18);
    lv_label_set_text(view->objects[OBJ_VALUE], "--");

    view->objects[OBJ_LABEL] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_LABEL],
                                lv_color_hex(0xb3b6c7), 0);
    lv_label_set_long_mode(view->objects[OBJ_LABEL], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_LABEL], 380);
    lv_obj_set_style_text_align(view->objects[OBJ_LABEL],
                                LV_TEXT_ALIGN_CENTER, 0);
    lv_obj_align_to(view->objects[OBJ_LABEL], view->objects[OBJ_VALUE],
                    LV_ALIGN_OUT_BOTTOM_MID, 0, 16);
    lv_label_set_text(view->objects[OBJ_LABEL], "");

    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_MID, 0, -14);
    view->state_label = view->objects[OBJ_STATE];
    return true;
}

void big_number_label_patch(template_widget_view_t *view,
                            const template_field_state_t *fields,
                            uint16_t dirty_mask)
{
    (void)dirty_mask;
    if (view == NULL || view->root == NULL || fields == NULL) {
        return;
    }
    const template_field_value_t *title =
        template_fields_get(fields, "title");
    const template_field_value_t *value =
        template_fields_get(fields, "value");
    const template_field_value_t *label =
        template_fields_get(fields, "label");
    if (title != NULL) {
        lv_label_set_text(view->objects[OBJ_TITLE], title->value.text);
    }
    if (value != NULL) {
        /* The schema default is "--", so an absent value renders a stable
         * placeholder rather than stale pixels from the previous card. */
        lv_label_set_text(view->objects[OBJ_VALUE],
                          value->value.text[0] != '\0' ? value->value.text
                                                       : "--");
    }
    if (label != NULL) {
        lv_label_set_text(view->objects[OBJ_LABEL], label->value.text);
    }
    /* Re-align: the value's width changes with its content. */
    lv_obj_align(view->objects[OBJ_VALUE], LV_ALIGN_CENTER, 0, -18);
    lv_obj_align_to(view->objects[OBJ_LABEL], view->objects[OBJ_VALUE],
                    LV_ALIGN_OUT_BOTTOM_MID, 0, 16);
}
```

- [ ] **Step 3: Wire the dispatcher**

Add `PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL` cases to `create_widget()` and `patch_widget()` in `template_view.c`, mirroring `PROTOCOL_TEMPLATE_ROW_LIST` (a template with no tick).

- [ ] **Step 4: Register the source, build, and run host tests**

Add `ui/templates/big_number_label.c` to `firmware/main/CMakeLists.txt`.

```sh
. "$HOME/esp/esp-idf/export.sh" && idf.py -C firmware build
make -C firmware/host_tests clean test
```

Expected: build succeeds, all suites OK.

- [ ] **Step 5: Commit**

```sh
git add firmware/main/ui/templates/big_number_label.c \
        firmware/main/ui/templates/template_internal.h \
        firmware/main/ui/template_view.c firmware/main/CMakeLists.txt
git commit -m "feat: add the big-number-label template view"
```

---

### Task 7: Implement the icon-badge-text template view

**Files:**
- Create: `firmware/main/ui/templates/icon_badge_text.c`
- Modify: `firmware/main/ui/templates/template_internal.h`
- Modify: `firmware/main/ui/template_view.c`
- Modify: `firmware/main/CMakeLists.txt`

**Interfaces:**
- Consumes: Task 2's `s_icon_badge_text_fields`; Task 4's `weather_icon_from_name()` and `weather_icon_render()`.
- Produces: `icon_badge_text_create(view, parent, size)`, `icon_badge_text_patch(view, fields, dirty_mask)`. No tick.

**Layout on 448x368 — icon left, text block right** (the approved direction):

```
┌───────────────────────────────────┐
│ TITLE                      badge  │   title top-left, badge top-right
│                                   │
│    ▓▓▓▓                           │
│   ▓▓▓▓▓▓        21°               │   icon 120px box | value montserrat_48
│    ▓▓▓▓                           │
│                 Partly cloudy     │   label under value
│                                   │
└───────────────────────────────────┘
```

The icon container occupies one `objects[]` slot; its primitive children are untracked, keeping the view inside `TEMPLATE_OBJECT_CAPACITY`.

- [ ] **Step 1: Declare the entry points**

Add to `template_internal.h`:

```c
bool icon_badge_text_create(template_widget_view_t *view,
                            lv_obj_t *parent,
                            protocol_size_class_t size);
void icon_badge_text_patch(template_widget_view_t *view,
                           const template_field_state_t *fields,
                           uint16_t dirty_mask);
```

- [ ] **Step 2: Implement the view**

Create `firmware/main/ui/templates/icon_badge_text.c`:

```c
#include "template_internal.h"

#include <string.h>

#include "weather_icon.h"

enum {
    OBJ_TITLE,
    OBJ_BADGE,
    OBJ_ICON,
    OBJ_VALUE,
    OBJ_LABEL,
    OBJ_STATE,
};

static weather_icon_t s_current_icon = WEATHER_ICON_UNKNOWN;

bool icon_badge_text_create(template_widget_view_t *view,
                            lv_obj_t *parent,
                            protocol_size_class_t size)
{
    if (view == NULL || parent == NULL ||
        (size != PROTOCOL_SIZE_FULL && size != PROTOCOL_SIZE_STANDARD)) {
        return false;
    }
    memset(view, 0, sizeof(*view));
    view->root = lv_obj_create(parent);
    lv_obj_remove_style_all(view->root);
    lv_obj_remove_flag(view->root,
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->root, LV_PCT(100), LV_PCT(100));

    view->objects[OBJ_TITLE] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_TITLE],
                                lv_color_hex(0x8f93a8), 0);
    lv_obj_align(view->objects[OBJ_TITLE], LV_ALIGN_TOP_LEFT, 28, 22);
    lv_label_set_text(view->objects[OBJ_TITLE], "");

    view->objects[OBJ_BADGE] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_BADGE],
                                lv_color_hex(0x8f93a8), 0);
    lv_label_set_long_mode(view->objects[OBJ_BADGE], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_BADGE], 190);
    lv_obj_set_style_text_align(view->objects[OBJ_BADGE],
                                LV_TEXT_ALIGN_RIGHT, 0);
    lv_obj_align(view->objects[OBJ_BADGE], LV_ALIGN_TOP_RIGHT, -28, 22);
    lv_label_set_text(view->objects[OBJ_BADGE], "");

    view->objects[OBJ_ICON] = lv_obj_create(view->root);
    lv_obj_remove_style_all(view->objects[OBJ_ICON]);
    lv_obj_remove_flag(view->objects[OBJ_ICON],
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(view->objects[OBJ_ICON], 120, 120);
    lv_obj_align(view->objects[OBJ_ICON], LV_ALIGN_LEFT_MID, 44, 6);

    view->objects[OBJ_VALUE] = lv_label_create(view->root);
    lv_obj_set_style_text_font(view->objects[OBJ_VALUE],
                               &lv_font_montserrat_48, 0);
    lv_obj_set_style_text_color(view->objects[OBJ_VALUE],
                                lv_color_hex(0xf2c14e), 0);
    lv_obj_align(view->objects[OBJ_VALUE], LV_ALIGN_LEFT_MID, 216, -14);
    lv_label_set_text(view->objects[OBJ_VALUE], "--");

    view->objects[OBJ_LABEL] = lv_label_create(view->root);
    lv_obj_set_style_text_color(view->objects[OBJ_LABEL],
                                lv_color_hex(0xb3b6c7), 0);
    lv_label_set_long_mode(view->objects[OBJ_LABEL], LV_LABEL_LONG_DOT);
    lv_obj_set_width(view->objects[OBJ_LABEL], 200);
    lv_obj_align(view->objects[OBJ_LABEL], LV_ALIGN_LEFT_MID, 216, 40);
    lv_label_set_text(view->objects[OBJ_LABEL], "");

    view->objects[OBJ_STATE] = lv_label_create(view->root);
    lv_obj_align(view->objects[OBJ_STATE], LV_ALIGN_BOTTOM_MID, 0, -12);
    view->state_label = view->objects[OBJ_STATE];

    s_current_icon = WEATHER_ICON_UNKNOWN;
    weather_icon_render(view->objects[OBJ_ICON], s_current_icon,
                        lv_color_hex(0xe6e8f0));
    return true;
}

void icon_badge_text_patch(template_widget_view_t *view,
                           const template_field_state_t *fields,
                           uint16_t dirty_mask)
{
    (void)dirty_mask;
    if (view == NULL || view->root == NULL || fields == NULL) {
        return;
    }
    const template_field_value_t *title =
        template_fields_get(fields, "title");
    const template_field_value_t *badge =
        template_fields_get(fields, "badge");
    const template_field_value_t *value =
        template_fields_get(fields, "value");
    const template_field_value_t *label =
        template_fields_get(fields, "label");
    const template_field_value_t *icon =
        template_fields_get(fields, "icon");

    if (title != NULL) {
        lv_label_set_text(view->objects[OBJ_TITLE], title->value.text);
    }
    if (badge != NULL) {
        lv_label_set_text(view->objects[OBJ_BADGE], badge->value.text);
    }
    if (value != NULL) {
        lv_label_set_text(view->objects[OBJ_VALUE],
                          value->value.text[0] != '\0' ? value->value.text
                                                       : "--");
    }
    if (label != NULL) {
        lv_label_set_text(view->objects[OBJ_LABEL], label->value.text);
    }
    if (icon != NULL) {
        weather_icon_t next = weather_icon_from_name(icon->value.text);
        /* Only rebuild the artwork when the icon actually changes: every
         * rebuild deletes and recreates a dozen LVGL objects. */
        if (next != s_current_icon) {
            s_current_icon = next;
            weather_icon_render(view->objects[OBJ_ICON], next,
                                lv_color_hex(0xe6e8f0));
        }
    }
}
```

- [ ] **Step 3: Wire the dispatcher**

Add `PROTOCOL_TEMPLATE_ICON_BADGE_TEXT` cases to `create_widget()` and `patch_widget()` in `template_view.c`, mirroring `PROTOCOL_TEMPLATE_ROW_LIST`.

- [ ] **Step 4: Register the source, build, and run host tests**

Add `ui/templates/icon_badge_text.c` to `firmware/main/CMakeLists.txt`.

```sh
. "$HOME/esp/esp-idf/export.sh" && idf.py -C firmware build
make -C firmware/host_tests clean test
```

Expected: build succeeds, all suites OK.

- [ ] **Step 5: Commit**

```sh
git add firmware/main/ui/templates/icon_badge_text.c \
        firmware/main/ui/templates/template_internal.h \
        firmware/main/ui/template_view.c firmware/main/CMakeLists.txt
git commit -m "feat: add the icon-badge-text template view"
```

---

### Task 8: Lower the new templates on the host and repoint per-kind defaults

**Files:**
- Modify: `companion/crates/app-core/src/config.rs:1461-1467` (`wire_config`)
- Modify: `companion/crates/app-core/src/config.rs` (per-kind default template)
- Test: `companion/crates/app-core/src/config.rs` (unit tests), `companion/crates/app-core/tests/config.rs`

**Interfaces:**
- Consumes: Task 1's `TemplateKind::AnalogClock`/`BigNumberLabel`/`IconBadgeText`.
- Produces: `wire_config()` returns `Some` for all six templates. Newly added weather cards default to `DisplayTemplate::IconBadgeText { icon_asset_id: None }`; newly added json-feed cards default to `DisplayTemplate::BigNumberLabel`.

**Scope note.** Only the default for *newly added* cards changes. Do not migrate already-saved cards: the user chose "repoint defaults, leave existing saved cards alone". A saved weather card still on `row-list` keeps rendering a title over empty rows until its owner changes it, and that is intended.

**`icon_asset_id` stays out of scope.** `DisplayTemplate::IconBadgeText { icon_asset_id }` keeps its field, but rendering a pushed asset requires `CAPABILITY_ASSET_TRANSFER`, which is M4 Task 6. The firmware view built in Task 7 always renders the built-in icon selected by the `icon` field. Leave the existing `icon_asset_id` validation as it is; do not add rendering for it here.

- [ ] **Step 1: Write the failing tests**

Add to the config test module:

```rust
#[test]
fn every_display_template_lowers_to_the_wire() {
    for template in [
        DisplayTemplate::DigitalClock,
        DisplayTemplate::AnalogClock,
        DisplayTemplate::ProgressRing,
        DisplayTemplate::RowList,
        DisplayTemplate::BigNumberLabel,
        DisplayTemplate::IconBadgeText { icon_asset_id: None },
    ] {
        let card = clock_card_with_template(template.clone());
        assert!(
            card.wire_config().is_some(),
            "{template:?} must lower to a wire template"
        );
    }
}

#[test]
fn weather_and_json_feed_default_to_renderable_templates() {
    assert_eq!(
        default_template_for(CardKind::Weather),
        DisplayTemplate::IconBadgeText { icon_asset_id: None },
        "weather must not default to row-list, which renders no weather field"
    );
    assert_eq!(
        default_template_for(CardKind::JsonFeed),
        DisplayTemplate::BigNumberLabel
    );
}

#[test]
fn extended_templates_require_the_extended_capability() {
    let mut config = single_card_config(DisplayTemplate::BigNumberLabel);
    assert_eq!(
        config.required_device_capabilities()
            & protocol::CAPABILITY_EXTENDED_TEMPLATES,
        protocol::CAPABILITY_EXTENDED_TEMPLATES
    );
    config = single_card_config(DisplayTemplate::RowList);
    assert_eq!(
        config.required_device_capabilities()
            & protocol::CAPABILITY_EXTENDED_TEMPLATES,
        0,
        "core templates must not demand the extended capability"
    );
}
```

Use the file's existing card-construction helpers; add `clock_card_with_template`, `single_card_config`, and `default_template_for` only if equivalents do not already exist.

- [ ] **Step 2: Run and confirm failure**

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cd companion && cargo test -p app-core every_display_template_lowers_to_the_wire
```

Expected: FAIL — `wire_config()` returns `None`.

- [ ] **Step 3: Complete the lowering**

Replace the `return None` arm in `wire_config()`:

```rust
        let template = match self.template() {
            DisplayTemplate::DigitalClock => TemplateKind::DigitalClock,
            DisplayTemplate::ProgressRing => TemplateKind::ProgressRing,
            DisplayTemplate::RowList => TemplateKind::RowList,
            DisplayTemplate::AnalogClock => TemplateKind::AnalogClock,
            DisplayTemplate::BigNumberLabel => TemplateKind::BigNumberLabel,
            DisplayTemplate::IconBadgeText { .. } => TemplateKind::IconBadgeText,
        };
```

- [ ] **Step 4: Repoint the per-kind defaults**

Find where a newly added card picks its template (the function that currently yields `DisplayTemplate::RowList` for weather and json-feed — this is the `row-list` default introduced by the card-model final-review fix). Change weather to `DisplayTemplate::IconBadgeText { icon_asset_id: None }` and json-feed to `DisplayTemplate::BigNumberLabel`. Leave clock (`DigitalClock`), pomodoro (`ProgressRing`), calendar and rss (`RowList`) exactly as they are.

- [ ] **Step 5: Confirm the whole workspace passes**

```sh
cd companion && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
```

Expected: all green.

- [ ] **Step 6: Commit**

```sh
git add companion/crates/app-core
git commit -m "feat: lower extended templates and repoint weather and json-feed defaults"
```

---

### Task 9: Render the new templates in the app preview

**Files:**
- Modify: `companion/apps/deskmate/src/components/DevicePreview.tsx`
- Modify: `companion/apps/deskmate/src/styles.css`
- Test: `companion/apps/deskmate/src/components.test.tsx`

**Interfaces:**
- Consumes: the `DisplayTemplate` union already present in `src/lib/types.ts`.
- Produces: preview rendering for `analog-clock`, `big-number-label`, and `icon-badge-text`.

**Fidelity rule.** The preview matches *semantics and proportions*, not pixels. It stays labelled "Same data as your display · approximate pixels". Do not reimplement the LVGL vector icons in the preview: render the icon as a single glyph chosen from the same closed 11-value vocabulary, so an unknown name is visibly "no data" in the preview exactly as it is on the panel.

- [ ] **Step 1: Write the failing component tests**

Add to `companion/apps/deskmate/src/components.test.tsx`:

```tsx
test("big-number-label preview shows the value as the hero", () => {
  const html = renderPreview({
    template: { kind: "big-number-label" },
    fields: [
      { key: "title", value: { kind: "text", value: "Downloads" } },
      { key: "value", value: { kind: "text", value: "1,204" } },
      { key: "label", value: { kind: "text", value: "this week" } },
    ],
  });
  expect(html).toContain("1,204");
  expect(html).toContain("this week");
  expect(html).toContain("Downloads");
});

test("big-number-label falls back to a placeholder when value is absent", () => {
  const html = renderPreview({ template: { kind: "big-number-label" }, fields: [] });
  expect(html).toContain("--");
});

test("icon-badge-text preview shows icon, badge, value and label", () => {
  const html = renderPreview({
    template: { kind: "icon-badge-text", icon_asset_id: null },
    fields: [
      { key: "icon", value: { kind: "text", value: "cloud-sun" } },
      { key: "badge", value: { kind: "text", value: "Berlin" } },
      { key: "value", value: { kind: "text", value: "21°" } },
      { key: "label", value: { kind: "text", value: "Partly cloudy" } },
    ],
  });
  expect(html).toContain("Berlin");
  expect(html).toContain("21°");
  expect(html).toContain("Partly cloudy");
  expect(html).toContain("preview-icon--cloud-sun");
});

test("an unrecognised icon name renders the unknown icon", () => {
  const html = renderPreview({
    template: { kind: "icon-badge-text", icon_asset_id: null },
    fields: [{ key: "icon", value: { kind: "text", value: "meteor" } }],
  });
  expect(html).toContain("preview-icon--unknown");
});

test("analog-clock preview renders a face with hands", () => {
  const html = renderPreview({ template: { kind: "analog-clock" }, fields: [] });
  expect(html).toContain("preview-analog-face");
  expect(html).toContain("preview-analog-hand--hour");
  expect(html).toContain("preview-analog-hand--minute");
});
```

Use the file's existing preview render helper; add a `renderPreview` wrapper only if no equivalent exists.

- [ ] **Step 2: Run and confirm failure**

```sh
export PATH="$HOME/.bun/bin:$PATH"
cd companion/apps/deskmate && bun test components
```

Expected: FAIL — the new templates fall through to the existing default branch.

- [ ] **Step 3: Implement the three preview branches**

In `DevicePreview.tsx`, extend the template switch. Keep the closed icon vocabulary in one exported constant so it cannot drift from firmware:

```tsx
const WEATHER_ICONS = [
  "sun", "moon", "cloud", "cloud-sun", "cloud-moon",
  "rain", "drizzle", "snow", "storm", "fog", "unknown",
] as const;

const ICON_GLYPH: Record<(typeof WEATHER_ICONS)[number], string> = {
  sun: "☀", moon: "☾", cloud: "☁", "cloud-sun": "⛅", "cloud-moon": "☁",
  rain: "🌧", drizzle: "🌦", snow: "❄", storm: "⛈", fog: "🌫", unknown: "○",
};

function iconName(raw: string | undefined) {
  return WEATHER_ICONS.includes(raw as never) ? (raw as never) : "unknown";
}
```

Render `big-number-label` as title / hero value / label, falling back to `--` when `value` is empty; `icon-badge-text` as an icon block on the left with title, badge, value and label on the right, matching the firmware's proportions; and `analog-clock` as a circular face with hour and minute hands positioned from the preview clock time.

- [ ] **Step 4: Add the preview styles**

Add `.preview-bignumber`, `.preview-iconbadge`, `.preview-icon`, `.preview-analog-face`, and `.preview-analog-hand` rules to `styles.css`, using the existing preview tokens (`--panel-black`, `--emit`) so the new templates sit inside the established visual language rather than introducing new colours.

- [ ] **Step 5: Confirm the frontend passes**

```sh
cd companion/apps/deskmate && bun test && bun run build
```

Expected: all tests pass, build clean.

- [ ] **Step 6: Commit**

```sh
git add companion/apps/deskmate/src
git commit -m "feat: preview the extended templates in the settings app"
```

---

### Task 10: Update the documentation and the hardware checklist

**Files:**
- Modify: `docs/config/v3.md`
- Modify: `docs/protocol/v1.md`
- Modify: `docs/hardware/board-notes.md`
- Modify: `CLAUDE.md`

- [ ] **Step 1: Update the protocol document**

In `docs/protocol/v1.md`, extend the template-kind table with values 4, 5, and 6 and their field schemas exactly as declared in Task 2. State that these three require `CAPABILITY_EXTENDED_TEMPLATES` (bit 3), that `CURRENT_CAPABILITIES` is now `11`, and that a device without the bit rejects them with `UnsupportedTemplate` rather than rendering a substitute.

- [ ] **Step 2: Update the config contract**

In `docs/config/v3.md`, document that `analog-clock`, `big-number-label`, and `icon-badge-text` are now wire-implemented, and record the per-kind defaults: clock → `digital-clock`, pomodoro → `progress-ring`, calendar and rss → `row-list`, weather → `icon-badge-text`, json-feed → `big-number-label`. Note that the change applies to newly added cards only and does not migrate saved configurations.

- [ ] **Step 3: Replace the stale gap note in the board notes**

`docs/hardware/board-notes.md` currently records, under "Not covered by this session", that weather and json-feed render a title over five empty rows pending these templates. Replace that paragraph with a checklist for the next hardware session:

```markdown
## Extended templates (M4 Task 3) — needs physical verification

Not yet observed on the board. The next hardware session must check:

- [ ] `StatusResponse` reports capabilities 11 (core | rotation | extended templates).
- [ ] A weather card renders icon-left/text-right with a real temperature, summary
      and location, at both 90° and 270°.
- [ ] Each of the 11 icon names renders its own distinct artwork, and an
      unrecognised name renders the hollow `unknown` ring rather than blank space.
- [ ] A json-feed card renders its mapped value as the hero number, and an absent
      value renders `--` rather than stale pixels from the previous card.
- [ ] An analog-clock card tracks time, and `show_seconds` toggles the second hand.
- [ ] `unknown_field_count` stays at zero across a weather push (the schema
      declares all ten emitted fields for exactly this reason).
- [ ] Heap stays flat and `ui_queue_high_water` stays low across a full rotation
      that includes all three new templates.
```

- [ ] **Step 4: Update the current-state summary**

In `CLAUDE.md`, record that M4 Task 3 is complete in software with physical verification outstanding, and that `CURRENT_CAPABILITIES` is now `11`.

- [ ] **Step 5: Run the full verification set**

```sh
export PATH="$HOME/.cargo/bin:$HOME/.bun/bin:$PATH"
make -C firmware/host_tests clean test
cd companion && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cd apps/deskmate && bun test && bun run build
. "$HOME/esp/esp-idf/export.sh" && idf.py -C firmware build
```

Expected: everything green.

- [ ] **Step 6: Commit**

```sh
git add docs CLAUDE.md
git commit -m "docs: document the extended templates and their hardware checklist"
```

---

## Physical verification (required before marking M4 Task 3 complete)

Per CLAUDE.md, record observed results in `docs/hardware/board-notes.md`. Never claim a check that was not run on the board. This plan changes firmware, so the board must be reflashed.

- [ ] Flash and confirm `StatusResponse` reports capabilities **11**.
- [ ] Weather card at 90° and 270°: icon left, temperature, summary, location, correct proportions and no clipping.
- [ ] Walk all 11 icon names; confirm 11 distinct renderings and that an unknown name gives the hollow ring.
- [ ] JSON-feed card renders its mapped value; an absent value shows `--`.
- [ ] Analog clock tracks time; `show_seconds` shows and hides the second hand.
- [ ] Rotation through a loop containing all three new templates: flat heap, `dropped_ui_commands` 0, `unknown_field_count` 0.
- [ ] Malformed/maximal data: 16-character `value`, 64-character `label` and `badge`, and empty fields all render without overflow.

## Self-Review

**Spec coverage** — every bullet of M4 Task 3 maps to a task:

| M4 Task 3 requirement | Task |
|---|---|
| Add the three templates with explicit field schemas | 1, 2, 5, 6, 7 |
| One 448x368 layout each, no size-class matrix | 5, 6, 7 (each pins a single layout) |
| Bound formatting, truncation, glyph fallback, numeric range, icon lookup | 2 (schema bounds), 4 (closed icon set + `unknown`), 6 and 7 (`LV_LABEL_LONG_DOT`, `--` placeholder) |
| Missing fields/assets produce a stable fallback, not stale pixels | 2 (defaults `--`/`unknown`), 6, 7 |
| LVGL mutations on the UI task, even-pixel invalidation, clean-canvas layering | Global Constraints; no view touches the rounder or the interrupt layer |
| Match semantics and proportions in the preview, still labelled a preview | 9 |
| Plain-C model tests | 2, 3, 4 |
| 90°/270° physical visual/touch checks | Physical verification section |

**Placeholder scan** — no step says "add error handling" or "write tests for the above". Every code step carries the code. Two steps deliberately describe a change rather than quoting the target verbatim (Task 8 Step 4's default-template function, Task 9 Step 3's render branches) because the surrounding code is large and the implementer must read it; both name the exact old and new values.

**Type consistency** — `weather_icon_from_name`/`weather_icon_render` are declared in Task 4 and used with the same signatures in Task 7. `analog_clock_*`, `big_number_label_*`, and `icon_badge_text_*` match the `digital_clock_*` shape declared in `template_internal.h`. The Rust variant names `AnalogClock`/`BigNumberLabel`/`IconBadgeText` are identical in Tasks 1 and 8. The 11 icon names are identical in the Global Constraints, Task 4, and Task 9.
