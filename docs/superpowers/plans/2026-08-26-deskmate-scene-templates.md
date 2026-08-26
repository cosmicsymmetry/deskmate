# Scene Templates (stage 2b) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Re-express the five remaining C templates as host-built scenes, each proven
byte-identical to the C it reproduces, so the generic interpreter has earned the right to
replace hand-written templates.

**Architecture:** Stage 2a built the chain and proved it on `DigitalClock`. This stage adds
one builder per template in `app-core`'s `scene_build.rs` and one entry per template in a
table-driven parity gate.

> **Read the amendment at the end of this file before executing anything.** It was added
> before execution began and it **supersedes the task order below**: two new node kinds are
> needed, not one, and they are folded into a single firmware task that runs **second**
> rather than last. Every "Task 6 only" note below therefore means "Task 1b". The stage's
> hardware cost is unchanged — one OTA download, one power cycle — and with 1b landed, all
> five template builders are host-only.

**Tech Stack:** Rust (`app-core`, `protocol`, `lvgl-sim`), ESP-IDF 5.x/C with LVGL 9 for
Task 6 only.

**Spec:** `docs/superpowers/specs/2026-08-22-deskmate-plugin-scene-rendering-design.md`
(§7 rollout, stage 2). Predecessor plan:
`docs/superpowers/plans/2026-08-23-deskmate-scene-renderer.md`.

## Global Constraints

- Protocol stays **v1**, additive only. The config schema is **untouched** (still v5).
- `PROTOCOL_CURRENT_CAPABILITIES` stays **491** for Tasks 1-5. Task 6 does **not** add a
  capability bit either — see its note on why a new node kind rides bit 8.
- The gate is byte-identical, **no tolerance**, at both orientations, exactly as
  `tests/scene_parity.rs` already does for `DigitalClock`.
- Text nodes are **baseline-anchored** (`baseline_y`), never box-top. `digital_clock.c:44`
  is the precedent: the C positions type by `line_height - base_line`.
- Font metrics come from `BakedFontMetrics::SHIPPED` only. Never invent a metric — a test
  re-reads `firmware/main/ui/fonts/deskmate_font_*.c` and fails on disagreement, and that
  test is the entire provenance argument.
- **Do not modify the C templates.** They are the reference the gate compares against; a
  change to make parity easier would make the gate meaningless.
- `make -C firmware/host_tests sanitize` is required for any firmware change (Task 6).

## What this stage does not do

- **It does not retire the C templates.** Stage 3 does that, and it needs Task 7's ledger
  first. A scene that reproduces a frame is not automatically a scene that can ship.
- It does not add asset transfer, plugins, or rasterization.
- It does not re-run stage 2a's hardware checks. The renderer has drawn on the panel
  (2026-08-25/26); nothing here changes the device except Task 6.

## Two findings that shape the tasks

**1. The weather icons are already rectangles.** `weather_icon.c` builds every icon from
`disc()` (an `lv_obj` with `LV_RADIUS_CIRCLE`) and `bar()` (`radius = h/2`). A cloud is
"three discs and a slab". So `SCENE_NODE_RECT` reproduces them with no new node kind. The
crescent moon is a lit disc with a **background-coloured** disc offset over it, so the card
background has to be threaded into the builder rather than hardcoded to black — the C says
so explicitly and says not to.

**2. The analog hands are rotated rectangles, and nothing can express that.** `make_hand()`
sets `transform_pivot_x/y` and `set_hand_angle()` sets `transform_rotation`. A
`SCENE_NODE_LINE` is drawn by LVGL's line path, not the transform/matrix path, so it will
not be byte-identical however carefully the endpoints are computed. This is the same class
of error that made stage 2a mis-model the dial as an arc when it was an `lv_scale`; the fix
there was a node that matches by construction, and the fix here is the same.

## The card input types

`build_digital_clock_scene` takes a `ClockCard` — a small `Copy` struct holding exactly the
card-level inputs the C template draws from, with a doc comment on each field naming the C
field it mirrors. Each task below defines its own in the same shape and the same file:
`BigNumberCard`, `RowListCard`, `IconBadgeCard`, `ProgressRingCard`, `AnalogClockCard`.
None of them exists yet; defining one is part of its task.

Two rules carried from `ClockCard`, both load-bearing:

- **Every field names the C field it mirrors**, so a reviewer can check the builder against
  the template without reading both in full.
- **A value the device resolves live is not a field.** `ClockCard` carries `local_now` for
  the date, but the time reading is a `time:` binding and so has no field. Putting a live
  value in the struct is how a scene comes to look right in the gate and go stale on the
  panel — the failure Task 5 pins for `ProgressRing`.

## File Structure

| File | Responsibility |
| --- | --- |
| `companion/crates/app-core/src/scene_build.rs` | Modify. One `build_*_scene` per template, beside `build_digital_clock_scene`. Shared helpers (module boxes, chip stacks, palette) factored as they earn it. |
| `companion/crates/app-core/tests/scene_parity.rs` | Modify. Becomes table-driven over templates instead of hard-coded to `DigitalClock`. |
| `companion/crates/protocol/src/scene.rs` | Task 6 only. Adds the rotated-rect node. |
| `firmware/main/core/scene_model.h/.c`, `scene_decode.c` | Task 6 only. Decode and validate it. |
| `firmware/main/ui/scene_view.c` | Task 6 only. Build it into an LVGL object. |
| `firmware/host_tests/test_scene_decode.c`, `test_scene_model.c` | Task 6 only. |

`scene_build.rs` is already 1,111 lines. If it passes roughly 1,800 while adding these,
split it into `scene_build/mod.rs` plus one module per template, keeping
`BakedFontMetrics`, `number_font_tier` and the shared helpers in `mod.rs`. Do not split it
pre-emptively — the shared helpers are not visible until three templates exist.

---

### Task 1: Make the parity gate table-driven

**Files:**
- Modify: `companion/crates/app-core/tests/scene_parity.rs`

**Interfaces:**
- Produces: a `ParityCase` describing one comparison, and a `cases()` table the gate walks.
  Every later task adds rows to that table and nothing else in the test file.

Today the gate hard-codes `SimTemplate::DigitalClock` in `template_request()` and
`build_digital_clock_scene` in `scene_request()`, and the instant table plus
`show_seconds` axis are baked into one test. Five more templates cannot be added without
copying it five times.

- [ ] **Step 1: Write the failing test.** Add a `cases()` function returning a
      `Vec<ParityCase>` and a test that walks it. Shape it so a case owns both halves of
      the comparison, because that is the property that must not drift:

```rust
struct ParityCase {
    /// `template--variant--orientation`, used for the PNG dump path on failure.
    name: String,
    template: RenderRequest,
    scene: SceneRenderRequest,
}

impl ParityCase {
    /// The template this case compares. Read from `self.template.template`
    /// rather than parsed back out of `name`, so a mislabelled case cannot
    /// satisfy the coverage test below while comparing something else.
    fn template_kind(&self) -> SimTemplate {
        self.template.template
    }
}
```

      Seed `cases()` with exactly the `DigitalClock` comparisons the gate makes today, so
      this task is a pure refactor with no change in coverage.

- [ ] **Step 2: Run it and confirm the count is unchanged.**
      Run: `cargo test -p app-core --test scene_parity`
      Expected: the same 28 comparisons pass. If the number moved, the refactor changed
      coverage and is wrong.

- [ ] **Step 3: Keep the four guard tests, and make them table-aware.**
      `the_instant_table_covers_what_it_claims_to`,
      `neither_half_of_the_gate_renders_a_blank_canvas`, `hiding_the_seconds_changes_both_halves`
      and `neither_half_draws_anything_in_the_state_footer_strip` exist because a gate that
      compares two blank canvases passes. They must now assert over `cases()`, not over one
      hard-coded request, or they will silently stop covering the templates added later.
      Add one more, for the same reason:

```rust
/// Every template this stage claims to reproduce must actually appear in the
/// table. Without this, deleting a task's rows leaves a green gate that proves
/// nothing about that template.
#[test]
fn the_table_covers_every_template_this_stage_claims() {
    let covered: BTreeSet<_> = cases().iter().map(|case| case.template_kind()).collect();
    assert_eq!(covered, BTreeSet::from(EXPECTED_TEMPLATES));
}
```

      `EXPECTED_TEMPLATES` starts as `[DigitalClock]` and each later task extends it. That
      makes "did you actually wire it in" a build failure rather than a review question.

- [ ] **Step 4: Gates and commit.**
      Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
      Commit: `refactor: make the scene parity gate table-driven`

---

### Task 2: `BigNumberLabel`

**Files:**
- Modify: `companion/crates/app-core/src/scene_build.rs`
- Modify: `companion/crates/app-core/tests/scene_parity.rs`

**Interfaces:**
- Produces: `build_big_number_label_scene(card: &BigNumberCard, metrics: &BakedFontMetrics) -> Scene`

First because it is the smallest (`big_number_label.c` is 133 lines) and because it is the
only template that exercises something stage 2a left completely untested.

**Why this one matters more than its size suggests:** `deskmate_number_font(text,
CONTENT_WIDTH)` steps the value's face down a tier when the string is too wide, and the
label's face follows it (`font == DESKMATE_FONT_BODY ? ... : ...`). `number_font_tier()`
already exists on the host and is **never exercised by the stage 2a gate**, which measures
exactly one string width. Cover the step-down boundary directly: a value that just fits at
`Hero`, one that just misses it, and one that misses `Display` too.

- [ ] **Step 1: Write the failing rows.** Add cases to `cases()` covering, at both
      orientations: a short value (`"7"`), a value at the step-down boundary in both
      directions, a value long enough to reach the lowest tier, and a value with a
      non-digit that forces `text_is_numeric()` false. Add `BigNumberLabel` to
      `EXPECTED_TEMPLATES`.
- [ ] **Step 2: Run and watch it fail** with the builder missing.
      Run: `cargo test -p app-core --test scene_parity`
- [ ] **Step 3: Implement `build_big_number_label_scene`** following
      `build_digital_clock_scene`'s shape: read the constants from `big_number_label.c`
      rather than re-deriving them, anchor text by baseline, and choose the tier through
      the existing `number_font_tier()`.
- [ ] **Step 4: Run to green, then dump-and-look once.** A passing byte comparison is the
      gate, but open one PNG pair anyway the first time a template lands — a builder that
      reproduces the wrong thing consistently is still wrong, and the gate cannot tell you
      that.
- [ ] **Step 5: Gates and commit.** `feat: build BigNumberLabel as a scene`

---

### Task 3: `RowList`

**Files:**
- Modify: `companion/crates/app-core/src/scene_build.rs`
- Modify: `companion/crates/app-core/tests/scene_parity.rs`

**Interfaces:**
- Produces: `build_row_list_scene(card: &RowListCard, metrics: &BakedFontMetrics) -> Scene`

Five rows, each a title and a time (`OBJ_ROW0_TITLE` … `OBJ_ROW4_TIME`). The interesting
behaviour is width-constrained text: `SceneText` carries `w`, `align` and `ellipsize`, and
`ellipsize` must map onto whatever `row_list.c` sets — read it, do not assume
`LV_LABEL_LONG_DOT`.

- [ ] **Step 1: Write the failing rows**, at both orientations: a full five rows, fewer
      than five rows, an empty list, a title exactly at the truncation boundary, and one
      past it. Add `RowList` to `EXPECTED_TEMPLATES`.
- [ ] **Step 2: Run to confirm failure.**
- [ ] **Step 3: Implement the builder.** The `.` that `LV_LABEL_LONG_DOT` appends comes
      from the `Body` tier, the only baked face carrying letters — `BakedFontMetrics`
      documents this, and `measure()` must be used rather than an estimate.
- [ ] **Step 4: Note the boundary case's status.** `row-list--truncation-boundary` is
      already excluded from the *hardware* framebuffer diff because it pins a field above
      that field's registry maximum, so a device rejects the push. That exclusion is about
      hardware; the simulator has no such ceiling, so this gate **can** and must cover it.
      Say so in a comment where the case is defined, or someone will "fix" the
      inconsistency by deleting the coverage.
- [ ] **Step 5: Gates and commit.** `feat: build RowList as a scene`

---

### Task 4: `IconBadgeText`

**Files:**
- Modify: `companion/crates/app-core/src/scene_build.rs`
- Modify: `companion/crates/app-core/tests/scene_parity.rs`

**Interfaces:**
- Produces: `build_icon_badge_text_scene(card: &IconBadgeCard, background: u32, metrics: &BakedFontMetrics) -> Scene`

Note the `background` parameter: `draw_moon()` cuts its crescent with a disc filled in the
**card background colour**. `weather_icon.c` says outright not to hardcode it back to
black. Passing it in is what keeps that true when a card's background changes.

- [ ] **Step 1: Write the failing rows** covering **all 11 icons plus the unknown-ring
      fallback**, at both orientations. That is the coverage the M4 hardware session used
      and it is the right bar here: each icon is a distinct arrangement of discs and bars,
      so one icon passing says nothing about the next. Add `IconBadgeText` to
      `EXPECTED_TEMPLATES`.
- [ ] **Step 2: Run to confirm failure.**
- [ ] **Step 3: Implement the builder**, translating `disc()` to a `SceneRect` with
      `radius` half the diameter and `bar()` to one with `radius = h/2`. `disc()` and
      `bar()` align with `LV_ALIGN_CENTER` plus an offset, while scene nodes are absolute:
      convert once, in one helper, rather than at each of the ~40 call sites.
- [ ] **Step 4: Gates and commit.** `feat: build IconBadgeText as a scene`

---

### Task 5: `ProgressRing`, and the live-update gap it exposes

**Files:**
- Modify: `companion/crates/app-core/src/scene_build.rs`
- Modify: `companion/crates/app-core/tests/scene_parity.rs`

**Interfaces:**
- Produces: `build_progress_ring_scene(card: &ProgressRingCard, metrics: &BakedFontMetrics) -> Scene`

The largest template (332 lines) and the one that cannot be fully expressed today. Build it
anyway — pixel parity at a pinned instant is achievable and is what this stage's gate asks
for — but record the gap precisely, because Task 7 depends on it and stage 3 cannot ship
this card without closing it.

**What is expressible:** the ring is an arc, and `SceneArc::end_binding` scales the sweep,
so `timer.pct` drives it live. The remaining-time label maps to `timer.remaining`.

**What is not:** `status_word()` picks a word from `running`, `remaining` and `duration`,
and the elapsed/total chips are computed per tick from `current_remaining_ms()`. A scene
carries literals or bindings — it has no conditionals and no arithmetic. So a host-built
scene can be *pixel-correct at the instant it is built* while going stale a second later.

- [ ] **Step 1: Write the failing rows**, at both orientations: paused mid-countdown, a
      running ring at zero, a full ring, and a ring at the arc-indicator hue. Mirror
      `framebuffer_diff`'s case split exactly, and for the same reason recorded there:
      `running-mid-countdown` cannot be compared deterministically because
      `current_remaining_ms()` keeps counting from `lv_tick_get()` after its fields are
      pushed, while the simulator's tick is fixed, so the label's `(remaining_ms + 999) /
      1000` ceiling flips a second whenever push-to-capture latency crosses 1000 ms. **Do
      not flip that case to `running: false` and do not merge the split cases** — the
      existing note says why. Add `ProgressRing` to `EXPECTED_TEMPLATES`.
- [ ] **Step 2: Run to confirm failure.**
- [ ] **Step 3: Implement the builder**, with the ring as an arc carrying `end_binding`,
      and the status/elapsed/total values as **literals**, with a comment naming them as
      the live-update gap and pointing at Task 7.
- [ ] **Step 4: Prove the gap rather than asserting it.** Add a host test that builds the
      scene at t and again at t+90s with the same card state, and asserts the status and
      chip text nodes are byte-identical across the two. That is the defect, pinned: a
      shipping card would show a stale word. Name it
      `progress_ring_scene_status_text_does_not_advance_with_time`.
- [ ] **Step 5: Gates and commit.** `feat: build ProgressRing as a scene, and pin its stale-text gap`

---

### Task 6: `AnalogClock`, and the rotated-rect node it requires

**Files:**
- Modify: `companion/crates/protocol/src/scene.rs`
- Modify: `firmware/main/core/scene_model.h`, `firmware/main/core/scene_model.c`,
  `firmware/main/core/scene_decode.c`
- Modify: `firmware/main/ui/scene_view.c`
- Modify: `firmware/host_tests/test_scene_decode.c`, `firmware/host_tests/test_scene_model.c`
- Modify: `companion/crates/lvgl-sim/src/cases.rs`, `companion/crates/app-core/src/scene_build.rs`,
  `companion/crates/app-core/tests/scene_parity.rs`
- Modify: `docs/protocol/v1.md`

**Interfaces:**
- Produces: `SceneNode::RotRect(SceneRotRect)` and
  `build_analog_clock_scene(card: &AnalogClockCard, metrics: &BakedFontMetrics) -> Scene`

**This is the only task in the stage that touches firmware, and therefore the only one
carrying hardware cost.** Sequenced last so the other four land and are provable without it.

```rust
/// A rectangle rotated about a pivot, which is how every clock hand is drawn:
/// `make_hand()` in `analog_clock.c` sets `transform_pivot_x/y` and
/// `set_hand_angle()` sets `transform_rotation`. A `Line` node cannot stand in
/// for this -- LVGL draws lines through a different path than the transform
/// matrix, so the anti-aliased edges differ and the gate is byte-exact.
pub struct SceneRotRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub radius: i32,
    pub fill: u32,
    pub pivot_x: i32,
    pub pivot_y: i32,
    /// Tenths of a degree, matching `lv_obj_set_style_transform_rotation`
    /// exactly rather than converting at the boundary.
    pub rotation: i32,
    /// Empty means a fixed angle. `time:hour`, `time:minute`, `time:second`
    /// drive the three hands, so a scene ticks without a re-push.
    pub rotation_binding: String,
}
```

**No new capability bit.** Bit 8 already means "this device renders scenes", and a device
that advertises it must handle every node kind in the version of the format it shipped
with. A host that sends an unknown node to an older device gets the scene refused whole,
which `docs/protocol/v1.md` already specifies. Adding a bit per node kind does not scale
and is not what bit 8 was defined to mean.

- [ ] **Step 1: Write the failing decoder tests first**, in
      `firmware/host_tests/test_scene_decode.c` and `test_scene_model.c`: a well-formed
      rotated rect decodes; an out-of-range rotation is rejected; a pivot outside the rect
      is rejected. **Bounds must be tested under ASan** — `make -C firmware/host_tests
      sanitize` is the only proof two of the existing decoder bounds have, because
      `scene_model_validate()` reports an out-of-bounds write with the same error code the
      plain test asserts.
- [ ] **Step 2: Run both suites to confirm failure.**
      Run: `make -C firmware/host_tests clean test && make -C firmware/host_tests sanitize`
- [ ] **Step 3: Implement the node** across `scene.rs`, `scene_decode.c`, `scene_model.c`
      and `scene_view.c`. In `scene_view.c` it is an `lv_obj` with the same style calls
      `make_hand()` uses — matching by construction, not by re-deriving the transform.
- [ ] **Step 4: Add fixtures to the shared corpus.** `companion/crates/protocol/tests/fixtures.rs`
      and its firmware counterpart must both carry a rotated-rect scene, or the two
      languages' encoders can drift with every test green.
- [ ] **Step 5: Update `docs/protocol/v1.md`** — the node table and the scene map section.
- [ ] **Step 6: Add the simulator case and the parity rows.** One `scene_cases()` entry for
      the node kind at both orientations, then `AnalogClock` rows covering
      `show_seconds` true and false. The false case is worth its own row: it is the M4
      defect that stayed open until V1 acceptance closed it. Add `AnalogClock` to
      `EXPECTED_TEMPLATES`.
- [ ] **Step 7: Build the firmware and record the memory delta.**
      Run: `. "$HOME/esp/esp-idf/export.sh" && idf.py -C firmware build`
      Record `.bss`, `.data` and IRAM before and after in the commit message and in
      `docs/hardware/board-notes.md`. **Assume any addition to firmware statics can break
      OTA downloads with every test green** — this repo has lost days to it twice, most
      recently to ~105 bytes of `.bss`.
- [ ] **Step 8: Gates and commit.**
      Run: `make -C firmware/host_tests clean test && make -C firmware/host_tests sanitize && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
      Commit: `feat: add the rotated-rect scene node and build AnalogClock as a scene`

- [ ] **Step 9 (owner, hardware): one OTA download on the board.** Required, not optional,
      and it is the whole hardware cost of this stage. Publish the build, move
      `firmware/version.txt` to match — a flashed build is reverted within a minute
      otherwise, because the catalog pins the fleet and offers its version in either
      direction — and power-cycle the board, since the device checks twice a day, cannot be
      asked, and the battery means a USB unplug is not a power loss. Record the result in
      `docs/hardware/board-notes.md`.

---

### Task 7: The stage 3 readiness ledger

**Files:**
- Create: `docs/scene/template-parity-ledger.md`
- Modify: `docs/superpowers/plans/2026-08-23-deskmate-scene-renderer.md` (link it)

Stage 3 retires the C templates. The question it must answer is not "does the scene draw
the same pixels" — this stage answers that — but "can the scene keep drawing the right
pixels without the host re-pushing it". Those are different, and Task 5 proves at least one
template fails the second.

- [ ] **Step 1: Write the ledger.** One row per template: which values are literals, which
      are bindings, and what goes stale if the host never pushes again. Derive the rows
      from the builders as written, not from this plan's expectations.
- [ ] **Step 2: State the options for each gap**, with the cost of each. For `ProgressRing`
      that is: add `timer.elapsed` / `timer.total` bindings and a way to select a status
      word (wire + firmware change, and another OTA check), or have the host re-push on
      every state transition (no wire change, but a push per second for a running timer,
      which §3 of the spec rejects for exactly this reason — 1,440 pushes a day that one
      network hiccup still renders wrong).
- [ ] **Step 3: Recommend one, and say why.** A ledger that lists options without a
      recommendation defers the decision to whoever reads it under time pressure.
- [ ] **Step 4: Commit.** `docs: record what each scene template still cannot do live`

---

## Exit criteria

1. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace` pass.
2. `make -C firmware/host_tests clean test` and `make -C firmware/host_tests sanitize` pass.
3. `idf.py -C firmware build` clean, with `.bss`/IRAM deltas recorded (Task 6).
4. **All six templates are byte-identical to their C originals at both orientations**, and
   `the_table_covers_every_template_this_stage_claims` proves none was quietly dropped.
5. An OTA download completes on the board on a build carrying Task 6's node.
6. The ledger exists and names every value that would go stale.

Stage 3 — retiring the C templates, the plugin manifest, and curated plugins — is planned at
this plan's exit, from the ledger.

---

## AMENDMENT (added before execution, 2026-08-26): the chips need a node, not a host-side port

Reading `template_style.c` to review Task 2 turned up a blocker this plan did not contain,
and it governs the task order rather than one task.

**`deskmate_chip()` is an `lv_label` with a background**, so its pill is content-sized:
`text_width + 2 * pad_hor`, with `pad_ver` derived from `DESKMATE_CHIP_HEIGHT` and
`letter_space` set to 1. A scene expresses that as a rect plus a text node, and the rect's
width has to be *exactly* the label's rendered width or the byte-exact gate fails.

**The host cannot compute that width today, and making it able to is the wrong move.**
`BakedFontMetrics` carries `numeric: None` for Caption and Body, because those are the
full-range faces — and `deskmate_font_18.c` and `deskmate_font_28.c` both ship
`kern_dsc = &kern_pairs` with `kern_scale = 16`, while the digits-only faces ship
`kern_dsc = NULL`. So measuring alphabetic text means reimplementing
`lv_font_get_glyph_dsc_fmt_txt()`'s advance lookup, its kern-pair lookup in 4.4 format, and
`letter_space`, on the host, and keeping that port correct against a vendored LVGL.

**This plan already knows that is the wrong answer.** Stage 2a modelled the dial as an arc,
found it was an `lv_scale`, and resolved it by adding `SCENE_NODE_SCALE` "so the pixels
match by construction rather than by porting LVGL's tick trig". Porting LVGL's *text* metrics
is the same decision, with a larger surface and a subtler failure mode: a kerning bug shows
up as a few pixels on one chip in one card, which is exactly what a byte-exact gate would
catch and exactly what nothing else would.

**And all five remaining templates keep their chips** (clock faces are the only ones that
dropped them, on 2026-08-17), so this blocks every template task, not just Task 2.

### The revised order

Both node additions are folded into one firmware task, which runs **first** rather than
last. That keeps the stage's hardware cost at what it already was — **one OTA download, one
power cycle** — because two node kinds landing in one image need one verification, not two.

| Was | Now |
| --- | --- |
| Task 1 — table-driven gate | unchanged, still first |
| Task 6 — rotated-rect node, last | **Task 1b — styled-label node AND rotated-rect node, together, second** |
| Tasks 2-5 — four host-only templates | unchanged in content, but now all five are host-only once 1b lands |
| Task 6 — AnalogClock | host-only builder work, no firmware |
| Task 7 — ledger | unchanged, still last |

### Task 1b: the styled-label and rotated-rect nodes

**Files:** as Task 6 listed, plus `firmware/main/ui/templates/template_style.c` read (not
modified) as the reference for the label's exact style calls.

```rust
/// A text label that draws its own background: `deskmate_chip()`,
/// `deskmate_eyebrow()`, and BigNumberLabel's pill are all this shape. The
/// node carries the STYLE and the device sizes the box, because the box is
/// `text_width + 2 * pad_hor` and text width depends on per-glyph advances,
/// kern pairs in 4.4 format and `letter_space` -- LVGL's own arithmetic,
/// which the host would otherwise have to reimplement and keep correct
/// against a vendored LVGL. Same reasoning as `SCENE_NODE_SCALE`.
pub struct SceneLabel {
    pub x: i32,
    pub y: i32,
    pub font: SceneFont,
    pub value: SceneValue,
    pub ink: u32,
    pub fill: u32,
    /// `LV_OPA_TRANSP` fill makes this an eyebrow rather than a chip, so one
    /// node covers both without a kind flag.
    pub fill_opacity: u8,
    pub radius: i32,
    pub pad_hor: i32,
    pub pad_ver: i32,
    pub letter_space: i32,
    /// `deskmate_chip_set_text()` HIDES a chip given an empty string rather
    /// than drawing a collapsed blob, and `icon-badge-text--empty-badge`
    /// pins that. The device must do the same, or that golden breaks.
    pub hide_when_empty: bool,
}
```

- [ ] **Step 1: Write the failing decoder tests for BOTH nodes** in
      `firmware/host_tests/test_scene_decode.c` and `test_scene_model.c`, including the
      bounds cases, and run `make -C firmware/host_tests sanitize` — two existing decoder
      bounds have no other proof, because `scene_model_validate()` reports an
      out-of-bounds write with the same error code the plain test asserts.
- [ ] **Step 2: Run both suites to confirm failure.**
- [ ] **Step 3: Implement both nodes.** In `scene_view.c`, `SceneLabel` must make the same
      style calls `deskmate_chip()` makes, in the same order, rather than approximating
      them — that is the entire point of the node. `SceneRotRect` is as the original Task 6
      specified.
- [ ] **Step 4: Fixtures for both, in both languages' corpora.**
- [ ] **Step 5: Update `docs/protocol/v1.md`** — the node table and the scene map.
- [ ] **Step 6: Simulator cases for both node kinds at both orientations.**
- [ ] **Step 7: Build, record `.bss`/`.data`/IRAM deltas, and treat any movement as the
      standing hazard it is.** Two node kinds is the largest single firmware addition in
      this stage; it is also the only one.
- [ ] **Step 8: Gates and commit.** `feat: add the styled-label and rotated-rect scene nodes`
- [ ] **Step 9 (owner, hardware): one OTA download on the board**, exactly as the original
      Task 6 Step 9 specified — publish, move `firmware/version.txt` to match, power-cycle.
      **This is the whole hardware cost of stage 2b.**

### What this changes about the template tasks

Nothing in their content, but their premise improves: with `SceneLabel` available, a chip is
one node whose pixels match by construction, so no template task needs text measurement.
`BakedFontMetrics::measure()` stays what it is — a numeric-tier measurement used for
`number_font_tier()`'s step-down decision, which is a *tier choice*, not a box size, and is
correct without kerning because the digits-only faces have none.
