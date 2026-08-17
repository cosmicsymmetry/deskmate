# Clock Title Removal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the identity chip from all three clock faces and the `DATE` eyebrow
from the two date-bearing ones, re-centering the remaining content on each canvas.

**Architecture:** The change is four C edits plus their test and doc consequences.
`companion/crates/lvgl-sim` compiles the firmware's own template sources and drives
them from Rust, so editing `digital_clock.c` or `analog_clock.c` changes the on-device
pixels, the companion preview, and the golden-frame tests at once — there is no second
renderer to keep in sync. `clock_screen.c` is the exception: it is board-coupled, absent
from both the simulator's source list and `firmware/host_tests`, so `idf.py build` is
its only compile gate and the physical board is its only visual gate.

**Tech Stack:** ESP-IDF 5.x / C with LVGL 9 (firmware), Rust (`lvgl-sim` golden and
property tests), React + TypeScript with Bun test (companion settings UI).

## Global Constraints

Copied from `docs/superpowers/specs/2026-08-17-deskmate-clock-title-removal-design.md`
and `CLAUDE.md`. Every task's requirements implicitly include this section.

- The logical canvas is 448x368. `DESKMATE_GRID` is 8, `DESKMATE_MARGIN` is 24.
- `DESKMATE_FONT_HERO` line height is 72, `DESKMATE_FONT_BODY` 36, `DESKMATE_FONT_CAPTION` 22.
- Config schema stays at **v4**. No migration, no wire change, no fixture changes.
- `title` remains a schema and wire field on clock cards. The host keeps compiling and
  sending it; the firmware stops reading it. Do **not** remove it from the schema, the
  wire, `template_fields.c`'s registry, or the companion's card library.
- Non-clock templates keep their identity chips. Do not touch `row_list.c`,
  `big_number_label.c`, `icon_badge_text.c`, `progress_ring.c`, or `weather_icon.c`.
- Do **not** delete `deskmate_chip()`, `deskmate_chip_set_text()`, or
  `deskmate_eyebrow()` from `template_style.c`. All three keep callers: the chip helpers
  in `row_list.c`, `big_number_label.c`, `icon_badge_text.c`, and `progress_ring.c`; the
  eyebrow helper in `progress_ring.c`.
- Every commit must leave `cargo test --workspace` green. Tasks that change rendered
  pixels re-bless their own goldens in the same commit.
- Conventional commit prefixes (`feat:`, `fix:`, `test:`, `docs:`, `chore:`).
- Never claim hardware verification that was not observed on the physical board.

## File Structure

| File | Change | Responsibility |
| --- | --- | --- |
| `firmware/main/ui/templates/digital_clock.c` | Modify | Digital clock face: drop chip + eyebrow, re-center stack |
| `firmware/main/ui/templates/analog_clock.c` | Modify | Analog clock face: drop chip |
| `firmware/main/ui/clock_screen.c` | Modify | Fallback clock screen: drop chip + eyebrow, re-center stack |
| `companion/crates/lvgl-sim/tests/clock_chrome.rs` | Create | Property test: title field does not affect clock pixels |
| `companion/crates/lvgl-sim/tests/tabular.rs` | Modify | `TIME_BAND` follows the hero to its new y |
| `companion/crates/lvgl-sim/src/cases.rs` | Modify | Delete the now-degenerate `empty-title` case |
| `companion/crates/lvgl-sim/tests/golden/*.png` | Regenerate | 14 frames re-blessed, 2 deleted |
| `companion/apps/deskmate/src/components/CardEditor.tsx` | Modify | Relabel the clock card's title field |
| `companion/apps/deskmate/tests/components.test.tsx` | Modify | Pin the relabel |
| `docs/config/v4.md` | Modify | Note that clock faces do not render `title` |
| `docs/hardware/board-notes.md` | Modify | Record the physical observation |

---

### Task 1: Digital clock face

The digital clock loses its chip and its `DATE` eyebrow, and the remaining 248px
hero-plus-modules stack re-centers. `tabular.rs` hardcodes the hero's pixel band and
must follow it, or its five-glyph assertion measures a clipped hero. The
`empty-title` golden case becomes byte-identical to `typical` once the title is not
drawn, so it is deleted rather than re-blessed.

**Files:**
- Modify: `firmware/main/ui/templates/digital_clock.c` (enum at 10-19, defines at 21-31, create at 66-107, patch at 160-184)
- Create: `companion/crates/lvgl-sim/tests/clock_chrome.rs`
- Modify: `companion/crates/lvgl-sim/tests/tabular.rs:29-33`
- Modify: `companion/crates/lvgl-sim/src/cases.rs:105-113`
- Regenerate: `companion/crates/lvgl-sim/tests/golden/digital-clock--*.png`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `companion/crates/lvgl-sim/tests/clock_chrome.rs` containing
  `fn request(template: SimTemplate, title: &str, orientation: SimOrientation) -> RenderRequest`,
  `fn assert_title_is_not_rendered(sim: &mut Simulator, template: SimTemplate)`, and the
  test `clock_faces_ignore_the_title_field`, which Task 2 extends with one call.

- [ ] **Step 1: Write the failing test**

Create `companion/crates/lvgl-sim/tests/clock_chrome.rs`:

```rust
//! The clock faces draw no card title.
//!
//! `title` remains a schema and wire field — it names the card in the
//! companion's library — but no clock face renders it. See
//! `docs/superpowers/specs/2026-08-17-deskmate-clock-title-removal-design.md`.
//! Golden PNGs cannot state this: they pin one frame per case, not the
//! relationship between two frames that differ only in their title. This test
//! measures that relationship, so a chip reintroduced by accident fails here
//! rather than quietly re-blessing itself into the goldens.

use lvgl_sim::{
    LOGICAL_WIDTH, RenderRequest, SimField, SimFieldValue, SimOrientation, SimTemplate,
    Simulator,
};

/// 2025-08-12 12:00:00 UTC at +4, matching `cases.rs`'s clock instants.
const NOW: i64 = 1_755_000_000;
const OFFSET: i16 = 240;

fn request(template: SimTemplate, title: &str, orientation: SimOrientation) -> RenderRequest {
    RenderRequest {
        template,
        fields: vec![
            SimField {
                name: "title".to_string(),
                value: SimFieldValue::Text(title.to_string()),
            },
            SimField {
                name: "show_seconds".to_string(),
                value: SimFieldValue::Boolean(true),
            },
        ],
        utc_offset_minutes: OFFSET,
        now_unix_seconds: NOW,
        orientation,
    }
}

/// Renders one face three times at both orientations, varying only the title,
/// and asserts the frames are identical. A helper rather than an inner loop
/// over a template array, so Task 2 can add the analog face with one line —
/// and so this file never contains a single-element `for`, which
/// `clippy::single_element_loop` rejects under `-D warnings`.
fn assert_title_is_not_rendered(sim: &mut Simulator, template: SimTemplate) {
    for orientation in [SimOrientation::Landscape, SimOrientation::LandscapeFlipped] {
        let short = sim
            .render(&request(template, "Desk", orientation))
            .expect("short title");
        let long = sim
            .render(&request(template, "WWWWWWWWWWWWWWWW", orientation))
            .expect("long title");
        let empty = sim
            .render(&request(template, "", orientation))
            .expect("empty title");
        assert_eq!(
            short, long,
            "{template:?} at {orientation:?}: a longer title changed the frame"
        );
        assert_eq!(
            short, empty,
            "{template:?} at {orientation:?}: an empty title changed the frame"
        );
    }
}

#[test]
fn clock_faces_ignore_the_title_field() {
    let mut sim = Simulator::new().expect("simulator");
    assert_title_is_not_rendered(&mut sim, SimTemplate::DigitalClock);
}

/// The rows the title chip used to occupy: it sat at `y = 2 * DESKMATE_GRID`
/// (16) and stood `DESKMATE_CHIP_HEIGHT` (32) tall. The re-centered hero
/// starts at y = 64, so nothing may light this band.
const OLD_CHIP_BAND: std::ops::Range<usize> = 16..48;

#[test]
fn digital_clock_leaves_the_old_chip_band_dark() {
    let mut sim = Simulator::new().expect("simulator");
    let pixels = sim
        .render(&request(
            SimTemplate::DigitalClock,
            "Desk",
            SimOrientation::Landscape,
        ))
        .expect("digital clock");
    let width = LOGICAL_WIDTH as usize;
    let lit: Vec<usize> = OLD_CHIP_BAND
        .clone()
        .filter(|row| (0..width).any(|column| pixels[row * width + column] != 0))
        .collect();
    assert!(
        lit.is_empty(),
        "rows {lit:?} are lit inside the removed chip's band"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run from `companion/`:

```sh
cargo test -p lvgl-sim --test clock_chrome
```

Expected: both tests FAIL. `clock_faces_ignore_the_title_field` fails with "a longer
title changed the frame" (the chip renders its text, so the frames differ).
`digital_clock_leaves_the_old_chip_band_dark` fails listing rows in 16..48 (the chip is
a filled pill in the face hue).

- [ ] **Step 3: Remove `OBJ_TITLE` from the object enum**

In `firmware/main/ui/templates/digital_clock.c`, delete the `OBJ_TITLE,` line so the
enum reads:

```c
enum {
    OBJ_TIME,
    OBJ_SECONDS,
    OBJ_DATE,
    OBJ_DIAL,
    OBJ_HAND_HOUR,
    OBJ_HAND_MINUTE,
    OBJ_STATE,
};
```

This renumbers the entries after it. That is contained: each template declares its own
local enum and no other translation unit indexes `view->objects[]` by a literal.

- [ ] **Step 4: Re-center the stack**

Replace the layout comment and the first two defines (currently lines 21-27) with:

```c
/* The reading is anchored to the left margin rather than centred: a hero
 * that starts on the same rail as the modules below it gives the face one
 * vertical edge to hang everything from, and it leaves the top-right free
 * for the seconds. Nothing labels the face — a clock names itself — so the
 * 248px hero-plus-modules stack centres on the canvas, leaving 64 above and
 * 56 below: very slightly top-heavy, which is what optical centring wants. */
#define TIME_Y      (8 * DESKMATE_GRID)
#define MODULE_Y    (22 * DESKMATE_GRID)
#define MODULE_H    (17 * DESKMATE_GRID)
```

Leave `DATE_W`, `DIAL_X`, `DIAL_W`, and `DIAL_BOX` unchanged. This preserves the
existing 40px gap between the hero's bottom edge (64 + 72 = 136) and the module row's
top edge (176), and moves the module row's bottom edge from 344 to 312 — which also
gives the bottom-aligned state label real clearance it does not have today.

- [ ] **Step 5: Delete the chip**

Remove the chip comment and creation block (currently lines 66-71):

```c
    /* The title is the face's identity chip: a filled pill in the face hue
     * says which card you swiped to before the reading is parsed. */
    view->objects[OBJ_TITLE] = deskmate_chip(
        view->root, DESKMATE_MARGIN, 2 * DESKMATE_GRID, palette.hue,
        palette.ink);
```

Keep the `palette` declaration above it — `palette.hue` still colours the seconds
label, the dial indicator, and the minute hand.

- [ ] **Step 6: Delete the eyebrow and centre the date**

Replace the date-module block (currently lines 92-107) with:

```c
    lv_obj_t *date_module = deskmate_module(view->root, DESKMATE_MARGIN,
                                            MODULE_Y, DATE_W, MODULE_H);
    /* The date is the module's whole content — no eyebrow names it, since a
     * date needs no naming — so the value centres in the surface. */
    const int32_t value_line = lv_font_get_line_height(DESKMATE_FONT_BODY);
    const int32_t stack_top = (MODULE_H - value_line) / 2;
    view->objects[OBJ_DATE] = deskmate_label_box(
        date_module, DESKMATE_MARGIN, stack_top,
        DATE_W - 2 * DESKMATE_MARGIN, LV_TEXT_ALIGN_LEFT,
        DESKMATE_COLOR_PRIMARY, DESKMATE_FONT_BODY);
```

The `eyebrow_line` local is gone entirely — it was used only by the eyebrow call and
by the old label-box offset. Leaving it declared would trip `-Werror` on an unused
variable. `stack_top` moves from 35 to 50.

- [ ] **Step 7: Stop reading `title` in the patch function**

Replace the body of `digital_clock_patch` (currently lines 164-184) with:

```c
    (void)dirty_mask;
    if (view == NULL || view->root == NULL || fields == NULL) {
        return;
    }
    /* `title` is still a schema and wire field — it names the card in the
     * companion's library — but no clock face draws it, so nothing here
     * reads it. */
    const template_field_value_t *show = template_fields_get(
        fields, "show_seconds");
    if (show != NULL) {
        view->clock_show_seconds = show->value.boolean;
        if (view->clock_show_seconds) {
            lv_obj_remove_flag(view->objects[OBJ_SECONDS],
                               LV_OBJ_FLAG_HIDDEN);
        } else {
            lv_obj_add_flag(view->objects[OBJ_SECONDS], LV_OBJ_FLAG_HIDDEN);
        }
    }
```

- [ ] **Step 8: Run the test to verify it passes**

Run from `companion/`:

```sh
cargo test -p lvgl-sim --test clock_chrome
```

Expected: both tests PASS.

- [ ] **Step 9: Move `tabular.rs`'s hero band to the new hero position**

`TIME_BAND` is pinned to the hero's *old* y. Left alone it would measure a clipped
hero and the five-glyph assertion could merge or lose runs. In
`companion/crates/lvgl-sim/tests/tabular.rs`, replace lines 29-33:

```rust
/// Rows covering the hero time label and nothing else: the face has no title
/// chip above it and the module row starts at y = 176 (see `digital_clock.c`,
/// where the hero sits at `TIME_Y` = 64 and the HERO tier's line box is 72
/// tall). Inset 4px top and bottom so the band cannot catch a neighbour.
const TIME_BAND: std::ops::Range<usize> = 68..132;
```

- [ ] **Step 10: Run the tabular test to verify it still passes**

Run from `companion/`:

```sh
cargo test -p lvgl-sim --test tabular
```

Expected: PASS — `hero_time_label_is_tabular` finds five glyph runs per frame and
equal colon columns. If it reports a run count other than 5, the band is wrong: check
that the hero's line box is 64..136 and the band sits inside it.

- [ ] **Step 11: Delete the degenerate `empty-title` golden case**

With no chip, `digital-clock--empty-title` renders byte-identical to
`digital-clock--typical`, so it would sit in the suite asserting nothing. In
`companion/crates/lvgl-sim/src/cases.rs`, delete this whole call (currently lines
105-113):

```rust
    case(
        cases,
        "digital-clock",
        "empty-title",
        SimTemplate::DigitalClock,
        &[text("title", ""), boolean("show_seconds", true)],
        NOW,
        OFFSET,
    );
```

Its coverage is not lost — `clock_chrome.rs`'s `clock_faces_ignore_the_title_field`
asserts the empty-title case explicitly, and more directly than a pinned frame could.

- [ ] **Step 12: Re-bless the digital-clock goldens**

`tests/golden.rs` treats `BLESS` as "own the directory": it rewrites every frame and
deletes orphan PNGs whose case no longer exists, which removes the two
`digital-clock--empty-title--*.png` files automatically. Run from `companion/`:

```sh
BLESS=1 cargo test -p lvgl-sim --test golden
```

- [ ] **Step 13: Verify the golden inventory changed exactly as intended**

Run from the repository root:

```sh
git status --porcelain companion/crates/lvgl-sim/tests/golden
ls companion/crates/lvgl-sim/tests/golden | wc -l
```

Expected: exactly 8 modified `digital-clock--*.png` files, exactly 2 deleted
(`digital-clock--empty-title--landscape.png`, `digital-clock--empty-title--flipped.png`),
and **no other template's PNGs touched**. The directory count goes from 56 to 54. If
any `analog-clock`, `row-list`, `progress-ring`, `big-number-label`, or
`icon-badge-text` frame shows as modified, stop — something outside this task's scope
changed and must be understood before committing.

- [ ] **Step 14: Look at a blessed frame**

Open `companion/crates/lvgl-sim/tests/golden/digital-clock--typical--landscape.png` and
confirm by eye: no chip in the top-left, no `DATE` label above the date, the time and
module row sitting balanced on the canvas, and the date centered in its surface. The
goldens are only as good as this one look — a wrong-but-consistent layout blesses
itself happily.

- [ ] **Step 15: Run the full simulator suite green**

Run from `companion/`:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: all pass. `clippy` runs over test targets too, so an unused import in
`clock_chrome.rs` is an error, not a warning.

- [ ] **Step 16: Commit**

```bash
git add firmware/main/ui/templates/digital_clock.c \
        companion/crates/lvgl-sim/tests/clock_chrome.rs \
        companion/crates/lvgl-sim/tests/tabular.rs \
        companion/crates/lvgl-sim/src/cases.rs \
        companion/crates/lvgl-sim/tests/golden
git commit -m "feat: digital clock drops its title chip and DATE eyebrow

The chip named the card and the eyebrow named the date; neither carried
information a 96px time over a date does not already carry. The remaining
248px stack re-centers, moving the hero to y=64 and the module row to y=176.

tabular.rs's TIME_BAND followed the hero. The empty-title golden case is
deleted rather than re-blessed: with no chip it rendered byte-identical to
typical. Its coverage moves to clock_chrome.rs, which asserts title-invariance
directly instead of pinning a frame.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VkZ2n7ctwjLCWxMVZeHNCa"
```

---

### Task 2: Analog clock face

Nothing moves on this face. The dial, chapter ring, hands, and hub are all
`lv_obj_center`'d, so deleting the corner chip leaves the dial alone on the canvas.

**Files:**
- Modify: `firmware/main/ui/templates/analog_clock.c` (enum at 8-16, create at 89-96, patch at 189-212)
- Modify: `companion/crates/lvgl-sim/tests/clock_chrome.rs` (the template array from Task 1)
- Regenerate: `companion/crates/lvgl-sim/tests/golden/analog-clock--*.png`

**Interfaces:**
- Consumes: `clock_chrome.rs`'s `request(template, title, orientation)` and
  `assert_title_is_not_rendered(sim, template)` helpers and the
  `clock_faces_ignore_the_title_field` test, all created in Task 1.
- Produces: nothing later tasks depend on.

- [ ] **Step 1: Extend the failing test to the analog face**

In `companion/crates/lvgl-sim/tests/clock_chrome.rs`, add one call to
`clock_faces_ignore_the_title_field` so it reads:

```rust
#[test]
fn clock_faces_ignore_the_title_field() {
    let mut sim = Simulator::new().expect("simulator");
    assert_title_is_not_rendered(&mut sim, SimTemplate::DigitalClock);
    assert_title_is_not_rendered(&mut sim, SimTemplate::AnalogClock);
}
```

Do not add a chip-band test for this face. The analog clock's chapter ring is centered
and 320px across, so it legitimately lights rows from y=24 — a "band is dark"
assertion would be measuring the ring, not the chip.

- [ ] **Step 2: Run the test to verify it fails**

Run from `companion/`:

```sh
cargo test -p lvgl-sim --test clock_chrome
```

Expected: `clock_faces_ignore_the_title_field` FAILS with "AnalogClock at Landscape: a
longer title changed the frame". `digital_clock_leaves_the_old_chip_band_dark` still
passes (Task 1 fixed that face).

- [ ] **Step 3: Remove `OBJ_TITLE` from the object enum**

In `firmware/main/ui/templates/analog_clock.c`, delete the `OBJ_TITLE,` line so the
enum reads:

```c
enum {
    OBJ_FACE,
    OBJ_HOUR,
    OBJ_MINUTE,
    OBJ_SECOND,
    OBJ_HUB,
    OBJ_STATE,
};
```

- [ ] **Step 4: Delete the chip and its rationale comment**

Remove the block currently at lines 89-96 in full — both the comment, which exists
only to explain why the title was banished to the corner, and the chip itself:

```c
    /* The dial is centred and 320px across, so the 12 o'clock tick sits on
     * the canvas's own vertical axis: a centred title would collide with it.
     * The title takes the top-left corner instead — clear of the circle,
     * which at the chip's own band has not yet reached x = 174 — and lands
     * on the same rail as the other faces' identity chips. */
    view->objects[OBJ_TITLE] = deskmate_chip(
        view->root, DESKMATE_MARGIN, 2 * DESKMATE_GRID, palette.hue,
        palette.ink);
```

Keep the `palette` declaration — `palette.hue` still colours the twelve o'clock tick,
the second hand, and the hub.

- [ ] **Step 5: Stop reading `title` in the patch function**

Replace the body of `analog_clock_patch` (currently lines 193-211) with:

```c
    (void)dirty_mask;
    if (view == NULL || view->root == NULL || fields == NULL) {
        return;
    }
    /* `title` is still a schema and wire field — it names the card in the
     * companion's library — but no clock face draws it, so nothing here
     * reads it. */
    const template_field_value_t *show =
        template_fields_get(fields, "show_seconds");
    if (show != NULL) {
        view->clock_show_seconds = show->value.boolean;
        if (view->clock_show_seconds) {
            lv_obj_remove_flag(view->objects[OBJ_SECOND], LV_OBJ_FLAG_HIDDEN);
        } else {
            lv_obj_add_flag(view->objects[OBJ_SECOND], LV_OBJ_FLAG_HIDDEN);
        }
    }
```

- [ ] **Step 6: Run the test to verify it passes**

Run from `companion/`:

```sh
cargo test -p lvgl-sim --test clock_chrome
```

Expected: both tests PASS.

- [ ] **Step 7: Re-bless the analog-clock goldens**

Run from `companion/`:

```sh
BLESS=1 cargo test -p lvgl-sim --test golden
```

- [ ] **Step 8: Verify the golden inventory changed exactly as intended**

Run from the repository root:

```sh
git status --porcelain companion/crates/lvgl-sim/tests/golden
ls companion/crates/lvgl-sim/tests/golden | wc -l
```

Expected: exactly 6 modified `analog-clock--*.png` files, nothing deleted, no other
template touched, and the count still 54. Task 1's digital-clock frames were already
committed, so they must **not** reappear as modified here — if they do, Task 1's bless
was incomplete.

- [ ] **Step 9: Look at a blessed frame**

Open `companion/crates/lvgl-sim/tests/golden/analog-clock--typical--landscape.png` and
confirm the top-left corner is empty and the dial is unchanged — same diameter, same
position, twelve o'clock tick still in the face hue.

- [ ] **Step 10: Run the full workspace green**

Run from `companion/`:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: all pass.

- [ ] **Step 11: Commit**

```bash
git add firmware/main/ui/templates/analog_clock.c \
        companion/crates/lvgl-sim/tests/clock_chrome.rs \
        companion/crates/lvgl-sim/tests/golden
git commit -m "feat: analog clock drops its title chip

A twelve-hour dial does not need naming. Nothing moves: the dial, chapter
ring, hands and hub are all centred, so deleting the corner chip leaves the
face alone on the canvas. The comment explaining why the title lived in the
corner goes with it.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VkZ2n7ctwjLCWxMVZeHNCa"
```

---

### Task 3: Standalone fallback clock screen

The pre-connection fallback screen hardcodes its own copy of the same two labels. It is
board-coupled, so it is absent from both the simulator's source list and
`firmware/host_tests` (which covers only `core/` plus `ui_command_queue.c`). There is
no test to write — `idf.py build` is the compile gate and Task 5's physical check is
the visual one.

Note a deliberate consequence: this screen's stack is 192px (a 72px hero, a 24px gap,
a 96px module) against the digital clock card's 248px, so centering each independently
puts the fallback's hero at y=88 and the card's at y=64. The time therefore shifts 24px
when a host first connects and the card replaces the fallback. That is accepted — each
face is balanced on its own canvas rather than matched to a screen the user is leaving.
Task 5 confirms it does not read as a glitch.

**Files:**
- Modify: `firmware/main/ui/clock_screen.c` (comment and defines at 14-23, create at 192-231)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: nothing later tasks depend on.

- [ ] **Step 1: Re-center the stack and rewrite the layout comment**

In `firmware/main/ui/clock_screen.c`, replace the comment and defines currently at
lines 14-23 with:

```c
// Layout, spec §6.2's complication grammar (see templates/digital_clock.c,
// this screen's sibling face): a left-anchored hero reading with a single
// full-width module under it carrying the date. Neither is labelled -- a
// clock face names itself. This screen's stack is 192px against the card's
// 248px, so centring each on its own canvas puts this hero lower than the
// card's: 88 above and 88 below here, exactly centred. All coordinates are
// grid multiples (spec §6.2).
#define CLOCK_TIME_Y    (11 * DESKMATE_GRID)
#define CLOCK_MODULE_Y  (23 * DESKMATE_GRID)
#define CLOCK_MODULE_H  (12 * DESKMATE_GRID)
#define CLOCK_MODULE_W  (50 * DESKMATE_GRID)   /* 448 - 2*DESKMATE_MARGIN */
```

The old comment claimed `CLOCK_TIME_Y` "matches digital_clock's rail". That was true
when both were `12 * DESKMATE_GRID` and is now false, which is why the claim is gone
rather than merely re-worded.

- [ ] **Step 2: Delete the palette, the chip, and the eyebrow**

Replace the block currently at lines 192-231 with:

```c
    s_time_label = lv_label_create(scr);
    lv_obj_set_style_text_font(s_time_label, DESKMATE_FONT_HERO, 0);
    lv_obj_set_style_text_color(s_time_label, DESKMATE_COLOR_PRIMARY, 0);
    lv_label_set_text(s_time_label, "00:00");
    // Left-anchored on the margin rail, matching digital_clock's hero: one
    // vertical edge the hero and the module below both hang from.
    lv_obj_set_pos(s_time_label, DESKMATE_MARGIN, CLOCK_TIME_Y);

    // Single full-width module carrying the date. Nothing labels it -- a date
    // needs no naming -- so the value centres in the surface.
    lv_obj_t *date_module = deskmate_module(scr, DESKMATE_MARGIN,
                                            CLOCK_MODULE_Y, CLOCK_MODULE_W,
                                            CLOCK_MODULE_H);
    const int32_t value_line = lv_font_get_line_height(DESKMATE_FONT_BODY);
    const int32_t stack_top = (CLOCK_MODULE_H - value_line) / 2;
    s_date_label = deskmate_label_box(
        date_module, DESKMATE_MARGIN, stack_top,
        CLOCK_MODULE_W - 2 * DESKMATE_MARGIN, LV_TEXT_ALIGN_LEFT,
        DESKMATE_COLOR_PRIMARY, DESKMATE_FONT_BODY);
```

Three things go together here. The `deskmate_palette_t palette` declaration and its
comment are deleted: unlike the two card templates, this screen used `palette.hue`
*only* for the chip and the eyebrow, so leaving it declared trips `-Werror` on an
unused variable. The `eyebrow_line` local goes for the same reason. `stack_top` moves
from 15 to 30.

Leave everything else untouched — the `s_hint_label` block below keeps its
`LV_ALIGN_BOTTOM_MID` footer rail position, because the connection hint is chrome
rather than part of the centered stack, exactly as the templates' state label is.

- [ ] **Step 3: Verify the firmware still compiles**

This is the only automated gate on this file. Run from the repository root:

```sh
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build
```

Expected: build succeeds with no warnings. An "unused variable 'palette'" or
"unused variable 'eyebrow_line'" error means Step 2 left a declaration behind.

- [ ] **Step 4: Confirm the host tests are unaffected**

`clock_screen.c` is not in any host-test target, so this should be a no-op — run it to
prove that rather than to discover it. From the repository root:

```sh
make -C firmware/host_tests clean test
```

Expected: all pass, unchanged from before this task.

- [ ] **Step 5: Commit**

```bash
git add firmware/main/ui/clock_screen.c
git commit -m "feat: fallback clock screen drops its CLOCK chip and DATE eyebrow

The pre-connection screen hardcoded its own copy of the labels the two card
faces just lost. Its 192px stack centres exactly, putting the hero at y=88
and the date module at y=184; the date centres in its surface. The palette
lookup goes with them -- unlike the card templates, this screen used the face
hue only for the chip and the eyebrow.

The connection hint keeps its footer rail: it is chrome, not part of the
centred stack.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VkZ2n7ctwjLCWxMVZeHNCa"
```

---

### Task 4: Companion relabel and contract note

`title` still names the card in the library (`CardList.tsx` renders `cardName(card)`),
so the field stays — but calling it "Heading" now implies something appears on the
panel that does not. Only the clock branch changes; the other four occurrences belong
to card kinds whose chips still render, where "Heading" remains accurate.

**Files:**
- Modify: `companion/apps/deskmate/src/components/CardEditor.tsx:149`
- Modify: `companion/apps/deskmate/tests/components.test.tsx`
- Modify: `docs/config/v4.md:69`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: nothing later tasks depend on.

- [ ] **Step 1: Write the failing test**

The existing suite has no assertion on this label at all, so the relabel would
otherwise be unpinned. `components.test.tsx` already provides `renderCardEditor(card)`,
`clockCard(id, title)`, and `weatherCard(id)` — reuse them. Add this test inside the
same `describe` block that contains `renderCardEditor`:

```tsx
  test("names the clock card's title field rather than calling it a heading", () => {
    // The clock faces draw no title chip, so the field only names the card in
    // the library. Weather still renders its chip, so "Heading" stays right
    // there — the assertion is that the two differ, not just that one changed.
    const clockHtml = renderCardEditor(clockCard("clock-1", "Desk"));
    expect(clockHtml).toContain("<span>Name</span>");
    expect(clockHtml).not.toContain("<span>Heading</span>");

    const weatherHtml = renderCardEditor(weatherCard("weather-1"));
    expect(weatherHtml).toContain("<span>Heading</span>");
  });
```

- [ ] **Step 2: Run the test to verify it fails**

Run from `companion/apps/deskmate/`:

```sh
bun test tests/components.test.tsx
```

Expected: FAIL — the clock editor renders `<span>Heading</span>`, so the
`toContain("<span>Name</span>")` assertion fails first.

- [ ] **Step 3: Relabel the clock card's field**

In `companion/apps/deskmate/src/components/CardEditor.tsx`, line 149 sits inside the
`{card.kind === "clock" && (` branch that opens at line 147. Change only that one:

```tsx
              <span>Name</span>
```

Leave lines 251, 330, 368, and 458 as `<span>Heading</span>` — those are the pomodoro,
calendar, weather, json-feed, and rss branches, whose faces still render an identity
chip.

- [ ] **Step 4: Run the test to verify it passes**

Run from `companion/apps/deskmate/`:

```sh
bun test tests/components.test.tsx
```

Expected: PASS, and the rest of the file's tests still pass.

- [ ] **Step 5: Note the behaviour in the frozen v4 contract**

`docs/config/v4.md` line 69 lists `title` as a clock card field. It still is one — the
schema does not change — but the doc should not leave a reader expecting it on the
panel. Append a sentence to the paragraph immediately following that table:

```markdown
Clock cards still carry `title`, but no clock face renders it: the digital and analog
faces and the standalone fallback screen all draw themselves unlabelled, so the value
serves only to name the card in the companion's library. The field remains on the wire
and is accepted by firmware, which ignores it.
```

- [ ] **Step 6: Run the companion app's full gates**

Run from `companion/apps/deskmate/`:

```sh
bun test
bunx tsc --noEmit
bunx biome check src tests
```

Expected: all pass. If `biome` reports formatting on the changed lines, let it fix them
with `bunx biome check --write src tests` and re-run.

- [ ] **Step 7: Commit**

```bash
git add companion/apps/deskmate/src/components/CardEditor.tsx \
        companion/apps/deskmate/tests/components.test.tsx \
        docs/config/v4.md
git commit -m "feat: clock cards label their title field Name, not Heading

No clock face draws the title any more, so calling the field a heading
promised something the panel does not deliver. It still names the card in the
library, so the field stays -- only its label changes, and only for clock
cards. The v4 contract records that firmware accepts and ignores it.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VkZ2n7ctwjLCWxMVZeHNCa"
```

---

### Task 5: Physical verification and documentation

Two different gates run here. The framebuffer diff is automated and proves the firmware
draws what the simulator drew, byte for byte — the goldens cannot state that on their
own, since they only prove the simulator is self-consistent. The human observations
cover what no capture comparison reaches: the fallback screen, which has no golden case
at all, and the fallback-to-card transition, which is a relationship between two frames
rather than a frame.

This task cannot be completed by an agent alone: flashing, touching, and judging are
human actions, and per `CLAUDE.md` nothing may be recorded as verified that was not
observed on the physical board.

**Files:**
- Modify: `docs/hardware/board-notes.md`
- Modify: `docs/superpowers/specs/2026-08-17-deskmate-clock-title-removal-design.md` (status line)
- Modify: `CLAUDE.md` (current state)

**Interfaces:**
- Consumes: the firmware built in Tasks 1-3 and the companion built in Task 4.
- Produces: the durable record of the observation.

- [ ] **Step 1: Run the full verification set before flashing**

From the repository root:

```sh
make -C firmware/host_tests clean test
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build
```

Then from `companion/`:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: everything passes. Do not flash on a red suite.

- [ ] **Step 2: Flash a dev-diag build**

Step 6 runs the framebuffer diff, which needs the dev-only 0x7E capture message that
only a `DESKMATE_DEV_DIAG=1` build answers — against a release build every case times
out. That flag adds diagnostic message handlers without changing rendering, so the same
flash serves the visual checks below.

```sh
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware -DDESKMATE_DEV_DIAG=1 flash monitor
```

- [ ] **Step 3: Observe the fallback screen at 90°, before any host connects**

Use the webcam harness for the capture — `tools/hwcam/`, usage in
`docs/hardware/webcam-harness.md` — per the standing preference in `CLAUDE.md`. Judge
the capture by eye; there is no CV code by design.

Confirm all four: no `CLOCK` chip in the top-left, no `DATE` label above the date, the
time and date module sitting balanced on the canvas with visibly equal space above and
below, and "Connect deskmate app" still on the bottom rail.

- [ ] **Step 4: Observe both card faces at 90°**

Connect the companion and put a digital clock card and an analog clock card in the
active playlist. Capture each. Confirm: no chip on either, the digital clock's date
centered in its module with the dial module unmoved beside it, and the analog dial
unchanged.

- [ ] **Step 5: Run the framebuffer diff against the re-blessed goldens**

This is the check the goldens cannot perform on themselves. A golden proves the
simulator is self-consistent; the diff proves the *firmware* draws what the simulator
drew, byte for byte, by capturing the device's real framebuffer over the wire. Since
both come from the same C sources, a mismatch here means something environmental — a
font, a rounder callback, a rotation path — differs on the board.

Run from `companion/`:

```sh
cargo run -p device --example framebuffer_diff -- --port <serial-port>
```

Expected final line: `SUMMARY total=54 identical=52 differing=0 errored=0 excluded=2`.
The 54 replaces the previous 56 because Task 1 deleted the `empty-title` pair; the 2
exclusions are the unchanged `row-list--truncation-boundary` pair, which cannot be
pushed to hardware as written. Any `differing` count above zero means firmware and
simulator disagree — stop and report rather than re-blessing.

- [ ] **Step 6: Watch the fallback-to-card transition**

Task 3 records that the hero shifts 24px down-to-up when a card replaces the fallback,
because each stack centers on its own canvas. Trigger it (disconnect and reconnect the
companion) and confirm it reads as a screen change rather than a glitch. If it reads
badly, stop and report — do not silently re-tune the constants, since that would
un-center one of the two faces the spec centered deliberately.

- [ ] **Step 7: Repeat Steps 3, 4 and 6 at 270°**

Change the orientation in the companion's settings, which owns that choice, and repeat
the fallback observation, the two card observations, and the transition check. Both
mount orientations are part of the acceptance criteria. Step 5's framebuffer diff
already covers both orientations internally — every case runs at `landscape` and
`flipped` — so it does not need repeating.

- [ ] **Step 8: Record the observation in board-notes**

Add a dated section to `docs/hardware/board-notes.md` following the file's existing
convention. Record what was actually observed at each orientation, including any item
that did not pass. Do not write "verified" against anything not seen.

- [ ] **Step 9: Update the spec status and CLAUDE.md**

Change the spec's `Status: approved` line to `Status: delivered` with the observation
date. In `CLAUDE.md`'s "Current state", add a sentence recording that the clock faces
no longer carry title chips or `DATE` eyebrows, that schema v4 and the wire are
unchanged, and that `title` is now accepted-and-ignored by firmware — this is exactly
the kind of lasting constraint a future contributor would otherwise try to "fix" by
reintroducing a chip.

- [ ] **Step 10: Commit**

```bash
git add docs/hardware/board-notes.md CLAUDE.md \
        docs/superpowers/specs/2026-08-17-deskmate-clock-title-removal-design.md
git commit -m "docs: clock title removal verified on hardware

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VkZ2n7ctwjLCWxMVZeHNCa"
```
