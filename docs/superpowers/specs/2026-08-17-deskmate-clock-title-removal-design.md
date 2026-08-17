# Clock faces drop their title chips and DATE eyebrows

Date: 2026-08-17
Status: delivered — software complete and physically verified 2026-08-17, with one
residual noted below. Evidence: `docs/hardware/board-notes.md`, "Clock title removal —
partial verification 2026-08-17". All 14 clock frames byte-identical to the goldens on
hardware across two framebuffer-diff runs at both orientations; the fallback screen and
both transition directions confirmed by webcam at 90°. **Residual:** the fallback screen
at 270° was not observed (changing orientation needs the companion's settings UI), and
the fallback's bottom margin is cropped in the available camera framing. Do not describe
either as verified.

## Problem

Every clock surface on the panel labels itself. The digital clock card carries an
identity chip reading its card title, the analog clock carries the same chip in its
top-left corner, and the pre-connection fallback screen hardcodes a `CLOCK` chip. The
digital clock and the fallback screen additionally label their date module with a
`DATE` eyebrow.

None of these labels carry information. A face showing `16:00` in 96px type over a
date is not ambiguous, and a face whose only content is a twelve-hour dial is not
ambiguous either. The labels cost vertical space and visual noise for nothing.

Data cards are a different case and are out of scope: weather, RSS, calendar, and
pomodoro cards keep their identity chips, because for those the chip answers a
question the content does not.

## Scope

Three faces change.

| Face | File | Removed |
| --- | --- | --- |
| Digital clock card | `firmware/main/ui/templates/digital_clock.c` | `OBJ_TITLE` chip, `"DATE"` eyebrow |
| Analog clock card | `firmware/main/ui/templates/analog_clock.c` | `OBJ_TITLE` chip |
| Standalone fallback | `firmware/main/ui/clock_screen.c` | `"CLOCK"` chip, `"DATE"` eyebrow |

No other template is touched.

## Why the simulator matters here

`companion/crates/lvgl-sim/build.rs` compiles the firmware's own C sources — the
template renderers, `template_style.c`, the fonts, and LVGL itself — and drives them
from Rust. The simulator is not a reimplementation, so editing a template's C file
changes the on-device pixels, the companion preview, and the golden-frame tests
together. There is no second renderer to keep in sync.

`clock_screen.c` is the exception. It is board-coupled and is absent from the
simulator's source list, so the fallback screen has no golden coverage. Its chip and
eyebrow are a separate hardcoded copy of the same idea and must be changed by hand
and confirmed on the physical board.

## Layout

The grid is 8px (`DESKMATE_GRID`), the canvas margin 24px (`DESKMATE_MARGIN`), the
logical canvas 448x368. `DESKMATE_FONT_HERO` has a line height of 72px,
`DESKMATE_FONT_BODY` 36px, and `DESKMATE_FONT_CAPTION` 22px.

Content rebalances rather than leaving a hole where each removed element sat.

### Digital clock

Today the stack runs chip (`y = 16`, 32px tall) then hero (`TIME_Y = 12 * GRID`, 96)
then the module row (`MODULE_Y = 26 * GRID`, 208, `MODULE_H = 17 * GRID`, 136, bottom
edge at 344). Deleting the chip alone would leave 96px of dead canvas above the time.

The remaining hero-plus-modules stack is 248px tall and re-centers:

- `TIME_Y`: `12 * DESKMATE_GRID` (96) becomes `8 * DESKMATE_GRID` (64)
- `MODULE_Y`: `26 * DESKMATE_GRID` (208) becomes `22 * DESKMATE_GRID` (176)

This preserves the existing 40px gap between the hero's bottom edge and the module
row's top edge. The result leaves 64px above the stack and 56px below it — very
slightly top-heavy, which is what optical centering wants. Modules now end at 312
rather than 344, which also gives the bottom-aligned stale/error state label real
clearance it does not have today.

Inside the date module, the eyebrow-over-value pair becomes a value alone. The
existing centering expression

```c
const int32_t stack_top =
    (MODULE_H - eyebrow_line - DESKMATE_GRID - value_line) / 2;
```

becomes `(MODULE_H - value_line) / 2`, moving the date from y=35 to y=50 within its
136px module — centered in its surface.

The module surface itself stays. It matches the dial module beside it and preserves
the complication grammar shared with the other five faces.

The `OBJ_TITLE` entry leaves the file's local object enum, renumbering the entries
after it. This is contained: each template declares its own enum and no other
translation unit indexes `view->objects[]` by a literal, so nothing outside the file
depends on those values.

### Analog clock

Nothing moves. The dial, chapter ring, hands, and hub are all `lv_obj_center`'d, so
deleting the corner chip simply leaves the dial alone on the canvas. The block comment
above the chip explaining why the title was placed in the corner rather than centered
is deleted with it.

### Fallback clock screen

The same treatment, with the same reasoning. The stack is 192px tall (hero 72 plus a
24px gap plus a 96px module) and centers exactly:

- `CLOCK_TIME_Y`: `12 * DESKMATE_GRID` (96) becomes `11 * DESKMATE_GRID` (88)
- `CLOCK_MODULE_Y`: `24 * DESKMATE_GRID` (192) becomes `23 * DESKMATE_GRID` (184)
- date `stack_top` within the 96px module: 15 becomes `(CLOCK_MODULE_H - value_line) / 2` = 30

The "Connect deskmate app" hint keeps its position on the shared footer rail
(`LV_ALIGN_BOTTOM_MID`, `-2 * DESKMATE_GRID`). It is chrome, not part of the centered
stack, exactly as the templates' state label is.

## Config, schema, and wire: unchanged

The config schema stays at v4, the wire contract is untouched, and there is no
migration. `title` remains a field on clock cards that the host keeps compiling and
sending; the firmware's `digital_clock_patch()` and `analog_clock_patch()` simply stop
reading it. An ignored field is cheaper than a schema version, and the wire contract
has been held stable across v3 and v4 deliberately.

`title` still earns its place in the companion. `CardList.tsx` renders `cardName(card)`
as the card's name in the library, and `CardEditor.tsx` exposes the field for editing.
Its visible label changes from **Heading** to **Name** for clock cards only, so the UI
stops implying the value appears on the panel. Other card kinds keep **Heading**,
because for them it still is one.

## Tests

### Golden frames

The golden suite in `companion/crates/lvgl-sim/tests/golden` is the primary gate.
Fourteen PNGs regenerate: eight digital-clock frames (the five cases below, minus the
deleted one) and six analog-clock frames, each case at both `landscape` and `flipped`
orientations.

One case is deleted rather than regenerated. `digital-clock--empty-title` exists to
prove an empty title string does not break the chip; with no chip, it renders
byte-identical to `digital-clock--typical`. Keeping it would leave two identical
goldens in the suite pretending to assert something. Its entry in
`companion/crates/lvgl-sim/src/cases.rs` and both its PNGs are removed, taking the case
table from 56 frames to 54.

### Tabular figures

`companion/crates/lvgl-sim/tests/tabular.rs` asserts that the `tabular-1135` and
`tabular-0000` goldens have identical lit-column spans, proving the hero font's figures
are tabular. Moving the hero vertically does not change column geometry, so this should
continue to pass unchanged. Confirm it rather than assuming it.

### Host and build gates

The standard set from `CLAUDE.md`:

```sh
make -C firmware/host_tests clean test
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build
```

and from `companion/`:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

### Physical framebuffer diff

`companion/crates/device/examples/framebuffer_diff.rs` reuses the same case table and
byte-compares a real device capture against `Simulator::render`. It is the only check
that proves the firmware draws what the simulator drew rather than proving the simulator
is self-consistent, so it runs after the goldens are re-blessed. It needs a
`DESKMATE_DEV_DIAG=1` build.

The gate is that **all 14 clock frames compare identical** — eight `digital-clock--*`
and six `analog-clock--*`, at both orientations. That is what this change must prove,
and it passed on 2026-08-17 across two consecutive runs.

The summary line cannot reach `differing=0`, and this spec was wrong to expect it.
`progress-ring--running-mid-countdown` is timing-sensitive on hardware:
`progress_ring.c`'s `current_remaining_ms` keeps counting a *running* ring down from
`lv_tick_get()` after its fields are pushed, while the simulator draws the pinned
`remaining_seconds` frozen, so one of its two orientations differs depending on where the
capture lands within a second. Expect `total=54 identical=51 differing=1 excluded=2` with
the differing case being that one, and treat any *other* differing case as a real
failure. See `docs/hardware/board-notes.md`, "Clock title removal — partial verification
2026-08-17".

### Physical verification

Required for what the diff cannot reach: the fallback screen, which has no golden case,
and the transition between faces, which is a relationship rather than a frame. On the
physical board, at both 90 degrees and 270 degrees:

1. The fallback clock at boot before any host connects — no `CLOCK` chip, no `DATE`
   eyebrow, time and date centered, connection hint still on its footer rail.
2. A digital clock card — no chip, date centered in its module, dial module unmoved
   relative to the date module.
3. An analog clock card — no chip, dial unchanged.
4. The fallback-to-card transition. Because each stack centers on its own canvas and the
   fallback's is 192px against the card's 248px, the hero shifts 24px when a host first
   connects. This is accepted; confirm it reads as a screen change rather than a glitch.

Record the observed result in `docs/hardware/board-notes.md`. Prefer the webcam harness
(`tools/hwcam/`) for the panel captures, per the standing preference in `CLAUDE.md`.

## Out of scope

- Any change to non-clock templates' identity chips.
- Removing `title` from the schema, the wire, or the companion's card library.
- Changing the state/stale/error footer rail shared by all templates.
- Reintroducing or altering size classes; every card remains the single clean
  448x368 canvas.
- Deleting the `deskmate_chip()`, `deskmate_chip_set_text()`, or `deskmate_eyebrow()`
  helpers in `template_style.c`. All three keep callers: the chip helpers in
  `row_list.c`, `big_number_label.c`, `icon_badge_text.c`, and `progress_ring.c`, and
  the eyebrow helper in `progress_ring.c`.
