# Scene-Native Rendering (stage 3a) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every one of the six card faces correct *without a re-push*, so the
hand-written C templates can stop shipping.

**Architecture:** Stage 2b proved a host-built scene draws the same pixels as its C
template at a pinned instant. It also proved that is not enough: two faces go wrong
seconds later, because a scene carries literals and a closed binding set with no
conditionals and no arithmetic. This stage closes that gap by **extending the closed
binding vocabulary**, not by adding an expression language and not by re-pushing on a
timer. The device already owns the clock and the timer; the bindings let a scene read
what the device already knows.

**Tech Stack:** ESP-IDF 5.x/C with LVGL 9 (`firmware/main/core`, `firmware/main/ui`),
Rust (`protocol`, `app-core`, `lvgl-sim`, `server`).

**Spec:** `docs/superpowers/specs/2026-08-22-deskmate-plugin-scene-rendering-design.md`
(§2 bindings, §3 render negotiation, §7 rollout stage 3).
Predecessor plans: `docs/superpowers/plans/2026-08-23-deskmate-scene-renderer.md` (2a),
`docs/superpowers/plans/2026-08-26-deskmate-scene-templates.md` (2b).
**Read `docs/scene/template-parity-ledger.md` before starting.** It is the derived
inventory this plan acts on, and it disagrees with the spec's expectations in three
places.

---

## Scope: why this is stage 3a and not stage 3

The spec's §7 names stage 3 as "Plugin manifest, curated plugins, clock and pomodoro as
scenes". The ledger showed the third of those is a plan on its own: **two** templates are
blocked rather than one, three device-path binding defects sit underneath them, and
retiring any C template also retires the optimistic offline tap behaviour that only the C
view has.

So this plan is **stage 3a — scene-native rendering**, and the plugin manifest and curated
plugins become **stage 3b**, planned at this plan's exit. That split is not a scope
reduction: the manifest is an *authoring* model whose entire value depends on scenes being
able to render a real card correctly, which is exactly what is not yet true. Building the
authoring model first would mean authoring plugins that cannot keep time.

## What this stage does not do

- No expression language. `SceneValue` gains no conditional and no arithmetic. Every
  addition here is a **named domain fact the device already owns**.
- No rasterization fallback and no SVG (stage 4).
- No public uploads or sandbox (stage 5).
- No change to the 448x368 canvas, the card model, or the playlist model.
- **The §8 draw-buffer spike stays out.** §9 says why: it is a memory-layout change, and
  folding one into a feature change is how this repository lost days twice.

Two of §6's mandatory hardware gates are **deliberately not in this stage**, and saying so
here is cheaper than someone rediscovering the omission at the exit gate:

- *Glyph-cache-miss timing at 96 px.* That gate belongs to runtime **asset** fonts. Every
  face here draws from the baked fonts the simulator compiles, so there is no cache miss
  to time. It becomes owed when a plugin ships a font.
- *`board_lcd_rounder_cb` at both orientations with **image** nodes present.* No node in
  the six templates is an image. Gate A does exercise the rounder at both orientations
  with arcs, clipped rects and transformed rects, which is new coverage — but it does not
  discharge the image-node form of that gate.

---

## Global Constraints

- Protocol stays **v1, additive only**. Config schema stays **v5**.
- `PROTOCOL_CURRENT_CAPABILITIES` stays **491**. No new capability bit: bit 8 already
  means "this device renders scenes", and `docs/protocol/v1.md` already specifies that an
  unknown node or binding gets the scene refused whole. A bit per feature does not scale.
  Pin the numeric value in a test, as the spec's §4 checklist item requires — bit 7 sat
  defined-but-dark for most of V2 and a conforming host could not have provisioned the
  device.
- **Every new bound must be tested under ASan.** `make -C firmware/host_tests sanitize` is
  not optional: two existing decoder bounds have no other proof, because
  `scene_model_validate()` reports an out-of-bounds *write* with the same error code the
  plain test asserts.
- **Nothing may be added to file-scope state.** `scene_view.c` allocates per-scene node
  state from the LVGL heap specifically because ~105 bytes of `.bss` once broke OTA
  downloads outright with every test green. Report `.bss`, DIRAM `.text`, `.data`, IRAM
  and DIRAM total as **same-tree** before/after deltas for every firmware task.
- Fixtures live in **both** languages' corpora (`companion/crates/protocol/tests/fixtures.rs`
  and the firmware counterpart, plus `firmware/main/core/protocol_message.c`'s encoder
  arms). `push_scene_min.bin` must stay **byte-identical** — that is the standing proof
  that omitting an optional key preserves v1 meaning.
- Keep hardware-independent firmware logic under `firmware/main/core/`, free of ESP-IDF
  includes, and host-test it as plain C.
- **Preserve the standalone clock** (`ui/clock_screen.c`) on boot, host loss, malformed
  input and protocol version mismatch. It is separate from the six templates and is not
  retired by this plan — it is what makes Task 8 survivable.

---

## Two findings that shape the tasks

**1. The parity gate is blind to binding *semantics*, and that is where the bugs are.**
The gate injects a pinned `SceneTimer` rather than deriving it, so the device's own
producer never runs. All three timer defects live in that hole and the gate is at zero
pixels with every one of them present. **A gate that supplies a binding's input can prove
how a value is drawn and never what it means.** Task 1 exists because of this, and every
task here that adds a binding must add a test that runs the *producer*, not just the
formatter.

**2. Deleting the C templates deletes the parity oracle.** `lvgl-sim` compiles the real
firmware C, which is what makes the 106-row gate byte-exact rather than a golden
comparison. Removing those files from the firmware build would leave the gate comparing a
scene against nothing. Task 8 resolves this deliberately rather than by accident.

---

## The binding vocabulary this stage adds

One table, because every task below refers to it and the wire cost is the sum of it.

| Binding | Kind | Replaces | Read by |
| --- | --- | --- | --- |
| `date` | new `SCENE_BINDING_DATE` | `DigitalClock`'s literal date line | `timefmt_date()` |
| `time:angle:hour` / `:minute` | extends `SCENE_BINDING_TIME` | `DigitalClock`'s two literal `SceneLine` hands | device clock |
| `timer.elapsed:mm:ss` | extends `SCENE_BINDING_TIMER_*` | `ProgressRing`'s ELAPSED literal | timer snapshot |
| `timer.total:mm:ss` | extends `SCENE_BINDING_TIMER_*` | `ProgressRing`'s TOTAL literal | timer snapshot |
| `timer.status` | new `SCENE_BINDING_TIMER_STATUS` | `ProgressRing`'s STATUS literal | timer snapshot |

Plus one **style** selector, which is not a `SceneValue` and must not become one:
`running_color` on `SceneArc` and `SceneText`, chosen between two literal colours by the
device's `timer_running` flag. `progress_ring.c` changes the arc indicator *and* the
STATUS text colour on that flag; a value binding cannot express a colour.

**`timer.status` is a conditional and that is the point.** Two more numeric bindings do
not solve it — `status_word()` picks between four words from three inputs. It is added as
a **named domain fact** the device already computes, not as a generic conditional. If a
future face needs a different word set, it gets its own named binding or it does not ship;
that ceiling is deliberate.

---

## File Structure

| File | Responsibility in this stage |
| --- | --- |
| `firmware/main/core/scene_binding.h/.c` | The closed binding set. Gains `date`, `time:angle:*`, `timer.elapsed`, `timer.total`, `timer.status`. Pure C, host-tested. |
| `firmware/main/core/scene_model.h/.c` | `SceneLine`'s optional pivot/length/angle-binding fields; `running_color` on arc and text; validation for all of it. |
| `firmware/main/core/scene_decode.c` | CBOR arms and bounds for the above. |
| `firmware/main/core/timefmt.c` | Already owns `timefmt_date()`. Read, not modified — the `date` binding calls it so the two cannot drift. |
| `firmware/main/ui/scene_view.c` | Applies `running_color`; recomputes a bound line's endpoints; re-evaluates on the 250 ms refresh. |
| `firmware/main/link/protocol_task.c` | `fill_timer_bindings()` — the producer where all three defects live. Gains elapsed/total/status/running. |
| `firmware/main/ui/carousel.c` | Routes a local tap into the scene timer context, not only into the C view. |
| `companion/crates/protocol/src/scene.rs` | The Rust mirror of every one of the above. |
| `companion/crates/app-core/src/scene_build.rs` | Six builders swap literals for the new bindings; all six gain the state footer. |
| `companion/crates/app-core/tests/scene_parity.rs` | The 106-row gate, plus footer rows and *temporal* rows. |
| `companion/crates/lvgl-sim/src/{cases.rs,scene.rs}` | Scene cases for each new binding; a tick-advancing harness for temporal tests. |
| `companion/crates/server/src/runtime.rs` (+ `runtime_device.rs`) | The scene push policy: which cards, and on what event. |
| `docs/protocol/v1.md` | The binding table and the node table. |

---

## Task ordering, and why the hardware cost is two checks

Tasks 1-5 are firmware and **all land in one image**. That is the lesson stage 2b paid
for: four needs found separately cost one OTA because each was deferred as it was found
rather than fixed in place. Do the same here — do not verify between them.

**This stage needs two hardware gates, not one, and the reason is diagnosability.**

- **Gate A (Task 6)** proves the new bindings draw correctly on the panel at both
  orientations. The C templates are still present, so a failure is isolated to the
  bindings.
- **Gate B (Task 9)** proves the device still works with the six C templates *removed* —
  which is a large memory-layout change, the exact class that has twice cost this
  repository days, both times with every test green.

Merging them would put a new-rendering failure and a memory-layout failure in the same
image, and memory-layout failures present as "OTA download fails" or "boot crash-loop",
which look nothing like "the dial is wrong". The whole reason `3f2aa03` took a
three-build bisect on the board is that it was entangled with a feature. **Do not merge
these two gates.**

| Task | Kind | Hardware |
| --- | --- | --- |
| 1-5 | firmware + host | — |
| 6 | **Gate A**: one image, one OTA, panel at both orientations | ✅ |
| 7-8 | host, then firmware deletion | — |
| 9 | **Gate B**: second image, OTA + regression | ✅ |
| 10 | docs / stage 3b handoff | — |

---

### Task 1: The three timer-binding defects

**Files:**
- Modify: `firmware/main/link/protocol_task.c` (`fill_timer_bindings`, ~line 345)
- Modify: `firmware/main/core/scene_binding.h`, `firmware/main/core/scene_binding.c`
- Modify: `firmware/host_tests/test_scene_binding.c`
- Modify: `docs/protocol/v1.md`

**Interfaces:**
- Produces: a `scene_binding_context_t` whose timer fields mean what
  `progress_ring.c` means by them, and a `timer.remaining` formatter that agrees with
  `format_clock()`.

These are bugs, not gaps, and they are first because every later timer binding inherits
them. The ledger found all three; `docs/scene/template-parity-ledger.md` §"ProgressRing:
existing binding defects first" is the source.

**Defect 1 — `timer.pct` is inverted.** `protocol_task.c:361` computes
`(total_ms - remaining_ms) * 100 / total_ms`, which is **elapsed** percent.
`progress_ring.c`'s arc uses `remaining_ms * 1000 / duration_ms`, and `SceneArc`'s
`end_binding` *scales the declared sweep*, so a full ring scaled by elapsed percent grows
while the C ring shrinks.

> **Decision: `timer.pct` keeps its wire token and means REMAINING percent.** The
> alternative — renaming it `timer.remaining_pct` — buys clarity at the cost of a wire
> change the additive-only rule discourages, and the token is already in the v1 fixture
> corpus. Nothing shipping depends on the wrong meaning, so fixing the producer is safe.
> **Pay for the clarity where the bug actually lived**: rename the C struct field to
> `timer_remaining_pct` so the producer cannot be misread, and state the meaning
> explicitly in `docs/protocol/v1.md`. The name is what caused this.

**Defect 2 — floor where the C ceils.** `scene_binding.c:237` uses
`timer_remaining_ms / 1000`. `progress_ring.c` renders `(remaining_ms + 999) / 1000`, a
ceiling that holds the current second until it has fully elapsed.

**Defect 3 — `mm` wraps at 60.** `scene_binding.c:238-240` treats `mm` as the minute
component of an hours/minutes/seconds clock. `format_clock()` prints **total minutes**,
supporting `1440:00` at the 86,400-second bound. A one-hour timer renders `00:00`.

> **Decision: `mm` in a `timer.*` argument means total minutes; `mm` in a `time:`
> argument keeps its clock meaning.** They are different domains — a wall clock has hours,
> a countdown does not — and `render_time_tokens()` is currently shared between them,
> which is how they came to disagree. Split the formatter rather than adding a flag, and
> put the reason in a comment at the split.

- [x] **Step 1: Write the failing producer tests.** These must run
      `fill_timer_bindings()`'s arithmetic, not the formatter, because that is the hole the
      parity gate leaves. Extract the computation into a pure `core/` function so it is
      host-testable without ESP-IDF — `protocol_task.c` is under `link/` and cannot be
      host-tested as it stands. Add to `firmware/host_tests/test_scene_binding.c`:

```c
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
```

- [x] **Step 2: Write the failing formatter tests.**

```c
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

static void a_wall_clock_minute_still_wraps(void)
{
    /* The split must not change time:. 13:05 stays 13:05. */
    char out[16];
    scene_binding_t b;
    assert(scene_binding_parse("time:HH:mm", &b) == SCENE_BINDING_OK);
    scene_binding_context_t ctx = {0};
    ctx.unix_seconds = 1787000700;  /* verify against gmtime_r in the test */
    ctx.utc_offset_minutes = 0;
    assert(scene_binding_evaluate(&b, &ctx, out, sizeof out) == SCENE_BINDING_OK);
    /* assert the exact HH:mm this instant produces; compute it in the test
     * with gmtime_r rather than hardcoding a guess. */
}
```

- [x] **Step 3: Run both suites to confirm failure.**
      Run: `make -C firmware/host_tests clean test && make -C firmware/host_tests sanitize`
      Expected: the five new tests fail; everything else passes.

- [x] **Step 4: Implement.** Extract `scene_timer_snapshot()` into `core/`, fix the
      percentage to remaining, rename the field to `timer_remaining_pct`, split the
      countdown formatter from the wall-clock formatter, and apply the ceiling.

- [x] **Step 5: Run to green, including ASan.**

- [x] **Step 6: Update `docs/protocol/v1.md`** — state that `timer.pct` is **remaining**
      percent, and that `mm` is total minutes in a `timer.*` argument and a clock minute in
      a `time:` argument. Both sentences exist because their absence caused a defect.

- [x] **Step 7: Commit.** `fix: make the timer bindings mean what progress_ring.c means`

---

### Task 2: `ProgressRing`'s live vocabulary

**Files:**
- Modify: `firmware/main/core/scene_binding.h/.c`, `firmware/main/core/scene_model.h/.c`,
  `firmware/main/core/scene_decode.c`, `firmware/main/ui/scene_view.c`
- Modify: `firmware/main/link/protocol_task.c`
- Modify: `companion/crates/protocol/src/scene.rs`
- Modify: `firmware/host_tests/test_scene_binding.c`, `test_scene_decode.c`, `test_scene_model.c`
- Modify: `companion/crates/protocol/tests/fixtures.rs`, `firmware/main/core/protocol_message.c`
- Modify: `docs/protocol/v1.md`

**Interfaces:**
- Consumes: Task 1's `scene_timer_snapshot()` and the renamed
  `scene_binding_context_t.timer_remaining_pct`.
- Produces: `timer.elapsed:<fmt>`, `timer.total:<fmt>`, `timer.status`, and a
  `running_color: Option<u32>` on `SceneArc` and `SceneText`.
- Adds to `scene_binding_context_t`: `uint32_t timer_total_ms` and `bool timer_running`.
  Note `timer_active` already exists and means something different — "a timer snapshot is
  present at all" — so a paused timer is `active` and not `running`. Do not conflate them;
  `timer.status` distinguishes `Ready`/`Paused` using exactly that difference.

- [x] **Step 1: Write the failing binding tests.**

```c
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

static void timer_status_picks_the_same_word_as_status_word(void)
{
    /* The four cases progress_ring.c's status_word() distinguishes. */
    const struct { int64_t total, remaining; bool running; const char *word; } cases[] = {
        { 1500, 0,    false, "Done"    },
        { 1500, 0,    true,  "Done"    },  /* remaining <= 0 wins over running */
        { 1500, 900,  true,  "Running" },
        { 1500, 1500, false, "Ready"   },
        { 1500, 900,  false, "Paused"  },
    };
    for (size_t i = 0; i < sizeof cases / sizeof cases[0]; i++) { /* assert each */ }
}

static void an_inactive_timer_renders_the_placeholder(void)
{
    /* Consistent with timer.remaining's "--:--" shape placeholder. A status
     * with no timer must not read "Ready". */
}
```

- [x] **Step 2: Write the failing decoder/model tests** for `running_color` on both node
      kinds, including the rejecting bound, and run them **under ASan** —
      `scene_model_validate()` reports an out-of-bounds write with the same error code the
      plain test asserts, so ASan is the only proof.

- [x] **Step 3: Run both suites to confirm failure.**

- [x] **Step 4: Implement.** `running_color` is an **optional colour**, absent meaning "no
      running variant", so every existing scene keeps its meaning and
      `push_scene_min.bin` stays byte-identical. In `scene_view.c` it selects between
      `color` and `running_color` on `context->timer_running` at build **and** at each
      refresh — a start/pause transition must repaint without a push.

> **Why a style selector rather than a value binding.** `SceneValue` resolves to *text*.
> `progress_ring.c` changes the arc indicator colour and the STATUS text colour on
> `running`. Encoding that as text would mean the device parsing a colour out of a
> binding result, which is a general evaluator wearing a disguise. One optional colour
> field per affected node is smaller, is validated as a colour, and cannot grow.

- [x] **Step 5: Fixtures in both corpora**, plus `protocol_message.c`'s encoder arms —
      Task 1b of stage 2b found that file missing from its task list and it will be missing
      again. Confirm `push_scene_min.bin` is unchanged, hash included.

- [x] **Step 6: Update `docs/protocol/v1.md`** — the binding table and both node tables.

- [x] **Step 7: Swap `build_progress_ring_scene`'s literals for the new bindings.**
      Without this the stage ships bindings nothing uses: the parity gate would never
      evaluate one and Gate A would have nothing to observe. TOTAL becomes
      `timer.total:mm:ss`, ELAPSED becomes `timer.elapsed:mm:ss`, STATUS becomes
      `timer.status`, and the indicator arc and STATUS text carry `running_color`. The
      comment naming these as the live-update gap goes away with them — delete it rather
      than leaving a comment that describes a fixed defect.
      **The existing 8 parity rows must still pass byte-identically.** A binding evaluated
      at the pinned instant must produce exactly the literal it replaced; if a row moves,
      the binding disagrees with the C and that is the defect this task exists to prevent.

- [x] **Step 8: Prove it temporally, which the byte gate cannot.**
      Build a tick-advancing harness in `companion/crates/lvgl-sim/src/scene.rs` that
      renders a scene, advances the simulated clock and timer, and re-renders **without a
      re-push** — the C side advancing through the same interval. Then assert the two
      still agree. This is the whole point of the stage and no existing test can express
      it: every one of the 106 rows renders a single instant.
      Cover at minimum: a running timer crossing a second boundary (ELAPSED advances), a
      timer reaching zero (STATUS becomes `Done`), and a start/pause transition (the
      indicator and STATUS colour follow `running`).
      **Then delete `progress_ring_scene_status_text_does_not_advance_with_time`.** It
      pins the gap this task closes; leaving it green would mean the gap is still open.
      The ledger says to replace it with a test proving the two advance together — that is
      Step 8, so the replacement must exist before the deletion.

- [x] **Step 9: Gates and commit.**
      Run: `make -C firmware/host_tests clean test && make -C firmware/host_tests sanitize`
      then from `companion/`: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
      Commit: `feat: give a scene the timer facts progress_ring.c computes`

---

### Task 3: `DigitalClock`'s live vocabulary

**Files:**
- Modify: `firmware/main/core/scene_binding.h`, `firmware/main/core/scene_binding.c`
- Modify: `firmware/main/core/scene_model.h`, `firmware/main/core/scene_model.c`,
  `firmware/main/core/scene_decode.c`, `firmware/main/ui/scene_view.c`
- Modify: `companion/crates/protocol/src/scene.rs`
- Modify: `companion/crates/app-core/src/scene_build.rs`,
  `companion/crates/app-core/tests/scene_parity.rs`, `companion/crates/lvgl-sim/src/scene.rs`
- Modify: `firmware/host_tests/test_scene_binding.c`, `test_scene_decode.c`, `test_scene_model.c`
- Modify: `companion/crates/protocol/tests/fixtures.rs`, `firmware/main/core/protocol_message.c`
- Modify: `docs/protocol/v1.md`
- **Read, not modified:** `firmware/main/core/timefmt.c` — the `date` binding calls
  `timefmt_date()`, which is the point; do not reimplement its format.

**Interfaces:**
- Produces: the `date` binding, and `SceneLine`'s optional
  `{pivot_x, pivot_y, length, angle_binding}`.

The ledger's first surprise: `DigitalClock` is blocked too. Its date line and both
small-dial hand endpoints are literals computed from the pushed instant, so the face is
wrong at the next minute and after local midnight even though its hero reading is live.

**The date binding calls `timefmt_date()` rather than reimplementing its format.** That is
the same by-construction principle that produced `SCENE_NODE_SCALE` and `SceneLabel`: the
C's format is `"%s, %s %d"` with an unpadded day and a **Monday-based** weekday index
(`(tm_wday + 6) % 7`), and a second implementation of that would drift silently.

**The hands stay `SceneLine`.** Do **not** convert them to `SceneRotRect`. LVGL draws
lines through a different path than the transform matrix, so the anti-aliased edges
differ and the byte-exact gate would be traded away for convenience. Instead a bound line
carries a pivot, a length and an angle binding, and the device computes the endpoint with
**LVGL's own trig table** — the same `(length * trigo_cos(rot + angle)) >> TRIGO_SHIFT`
the host port uses, which `the_trig_port_reproduces_lvgls_table` already pins.

- [x] **Step 1: Write the failing binding tests.**

```c
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
    timefmt_date(expected, sizeof expected, &tm_local);
    assert(strcmp(out, expected) == 0);
}

static void the_date_binding_crosses_local_midnight_with_the_offset(void)
{
    /* An instant that is one day earlier in UTC than at UTC+4. This is the
     * defect being fixed: a literal date is wrong after local midnight. */
}

static void an_angle_binding_gives_lvgls_own_hand_geometry(void)
{
    /* Quarter past three: the minute hand points at three o'clock. Assert the
     * computed endpoint equals the host port's, which is pinned against
     * LVGL's table. */
}
```

- [x] **Step 2: Write the failing decoder/model tests.** A bound line's `length` must be
      canvas-bounded and its pivot on-canvas; a line carrying **both** an explicit
      `xs`/`ys` and an `angle_binding` is **rejected**, because two sources for one
      geometry is ambiguity the device should not resolve silently. Run under ASan.

- [x] **Step 3: Run both suites to confirm failure.**

- [x] **Step 4: Implement**, re-evaluating bound lines on the 250 ms refresh.

- [x] **Step 5: Fixtures in both corpora; `push_scene_min.bin` unchanged.**

- [x] **Step 6: Update `docs/protocol/v1.md`.**

- [x] **Step 7: Swap `build_digital_clock_scene`'s literals for the new bindings.**
      The date text node becomes `SceneValue::Binding("date")`, and both dial hands become
      bound `SceneLine`s carrying pivot, length and `time:angle:hour` / `time:angle:minute`.
      **All 28 existing `DigitalClock` parity rows must still pass byte-identically** at
      every one of the 7 instants — the bindings evaluated at a pinned instant must equal
      the literals they replace, and 7 instants across both `show_seconds` settings is a
      strong check that the device's trig and `timefmt_date()` agree with the host port.

- [x] **Step 8: Prove it temporally**, reusing Task 2 Step 8's harness. Cover a minute
      boundary (the minute hand steps and the reading changes) and **local midnight with a
      non-zero UTC offset** (the date line changes). Local midnight is the case a literal
      gets wrong for up to a day, and the board sits at UTC+4, so a UTC-only test would
      pass while the shipped face was wrong.

- [x] **Step 9: Gates and commit.**
      Run: `make -C firmware/host_tests clean test && make -C firmware/host_tests sanitize`
      then from `companion/`: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
      Commit: `feat: let a scene draw the date and the dial live`

---

### Task 4: A tap must reach the scene, not only the C view

**Files:**
- Modify: `firmware/main/ui/carousel.c`, `firmware/main/ui/scene_view.c`
- Modify: `firmware/main/link/protocol_task.c`
- Modify: `firmware/host_tests/` (a new pure-core test for the transition arithmetic)

The ledger's state-ownership gap, and the one item here that is not about pixels.

On a tap, `carousel.c` sends the event **and** calls
`template_view_apply_local_action()` so the C `ProgressRing` shows optimistic start/pause
feedback immediately. A live scene has no equivalent: its timer context reads the
unchanged widget-model snapshot until the host answers with `PushData`. **With the link
down, a scene never reflects the tap at all.**

So retiring the C template would also retire its offline behaviour — which is the repo's
standalone rule, not a nicety. Board notes already record a related UX finding from
2026-08-15: a sticky unreconciled optimistic red flash on tapping a completed pomodoro.
Read it before designing the reconciliation, so this does not reintroduce it.

- [x] **Step 1: Write the failing test** for the local transition applied to the timer
      snapshot: start from paused with 900 s left, apply START_PAUSE, assert the snapshot
      is running and anchored to now; apply it again, assert it is paused at the elapsed
      remainder; apply RESET, assert it is back at duration and not running. This is the
      same arithmetic `progress_ring_local_action()` does, so extract and share it rather
      than writing a second copy.

- [x] **Step 2: Run to confirm failure.**

- [x] **Step 3: Implement**, routing the local action into the scene binding context and
      refreshing the bound nodes. **The host's `PushData` remains authoritative**: a later
      snapshot overwrites the local one. The optimistic value is a prediction, not a
      second source of truth.

- [x] **Step 4: Gates and commit.** `feat: let a tap move a scene's timer while the link is down`

---

### Task 5: The state footer, in all six builders

**Files:**
- Modify: `companion/crates/app-core/src/scene_build.rs`
- Modify: `companion/crates/app-core/tests/scene_parity.rs`
- Modify: `companion/crates/lvgl-sim/src/cases.rs`

Every C template creates `OBJ_STATE`; `template_view.c` shows an error in the error
colour, `Stale` in the stale colour, or nothing when OK. **No builder emits it**, because
all 106 parity rows deliberately exercise the OK state. So a scene cannot currently
reproduce a stale or error transition, and retiring the C would lose that signal.

This is host-only. It is **not** a binding: stale/error is a host-owned fact like a
provider result, so the host rebuilds and pushes when it changes. That adds no idle
traffic and matches the C path, which also changes the footer only when host data state
is applied.

> **Read `build_digital_clock_scene`'s doc comment before writing the fixtures.** It
> records an expensive trap: `digital_clock.c` creates the state label and never sets its
> text, and `LV_WIDGETS_HAS_DEFAULT_VALUE 1` makes the default the literal string
> `"Text"`. A harness that does not drive the C through `template_view`'s state-update
> path renders the word "Text" bottom-centre and costs a long pixel hunt.

- [x] **Step 1: Write the failing parity rows** — for each of the six templates, one stale
      row and one error row, at both orientations. That is 24 new rows on top of 106.
- [x] **Step 2: Run to confirm failure.**
- [x] **Step 3: Implement the footer** in a single shared helper, since all six use the
      same `template_view.c` logic. Six copies is how the two near-identical-but-not pill
      pairs happened in stage 2b.
- [x] **Step 4: Run to green**, and dump-and-look at one pair — a footer that reproduces
      the wrong thing consistently is still wrong and the gate cannot tell you.
- [x] **Step 5: Gates and commit.** `feat: give every scene the state footer its C template draws`

---

### Task 6: GATE A — one image, one OTA, and the panel at both orientations

**Files:** `firmware/version.txt`, `docs/hardware/board-notes.md`

This is the spec's stage 3 gate: **physical verification at both orientations**. It is
also the first time any of the renderer's node capabilities are seen on hardware — stage
2b's check was a *download*, and arc opacity, the external pivot and both clips have never
drawn on the panel.

- [x] **Step 1: Build and record the memory deltas** on a **same-tree** before/after.
      Baseline at the time of writing (commit `05500ef`): `.bss` **102,624**, DIRAM `.text`
      **93,635**, `.data` **23,128**, IRAM **16,384/16,384 with 0 remaining**, DIRAM total
      **219,387**. Re-measure the baseline on the actual tree rather than quoting these —
      a fresh total means nothing without the before/after.
- [x] **Step 2: Publish.** Three steps, all required: put `<version>.bin` in
      `$DESKMATE_FIRMWARE_DIR` on the VM, set `DESKMATE_FIRMWARE_VERSION` to that string,
      restart `deskmate-server.service`. Move `firmware/version.txt` to match — the catalog
      offers its version in **either** direction, so a device on a different string is
      offered a change within the minute. **Reach the VM over Tailscale
      (`100.93.166.123`)**, not the `~/.ssh/config` host, whose pinned LAN address times
      out from any other network. Then verify end-to-end: `GET
      https://deskmate.rodi.one/v1/firmware/<version>.bin` returns 200 and an md5 matching
      the local build.
- [x] **Step 3: Power-cycle** (owner action). The device checks at boot and twice a day,
      cannot be asked, and **a USB unplug is link loss, not power loss** — the board has a
      battery.
- [x] **Step 4: Observe on the panel, at 90° and 270°.** Use `tools/hwcam/`
      (`docs/hardware/webcam-harness.md`); prefer it for panel observations. Observe, per
      orientation:
      - a `DigitalClock` scene ticking **across a minute boundary**, with the date line and
        both dial hands correct — the dial is the literal being replaced, so a still frame
        proves nothing; watch the minute hand move.
      - a `ProgressRing` scene running down, with ELAPSED advancing, STATUS changing
        Ready→Running→Paused→Done, and the indicator colour following `running`.
      - **a tap with the link down**, showing the timer move anyway (Task 4).
      - an arc drawn at `LV_OPA_20`, an external-pivot tick, and a clipped rect — none has
        ever drawn on hardware.
- [x] **Step 5: Record in `docs/hardware/board-notes.md`.** Quote the observed values
      rather than paraphrasing, and state explicitly what was *not* observed. Never
      describe as verified anything not seen on the physical board.

---

### Task 7: The host decides scene or widget, per device

**Files:**
- Modify: `companion/crates/server/src/runtime.rs`, `companion/crates/server/src/runtime_device.rs`
- Modify: `companion/crates/app-core/src/state.rs` (`DeviceCapability`, already carries
  `SceneRender` as of `05500ef`) and the `RuntimeDevice` seam the server and the Mac app
  share — `push_scene` already exists on it (`runtime_device.rs:677`)
- Test: `companion/crates/server/tests/`

**Interfaces:**
- Produces: a per-card render decision resolved from the device's capability bits.

§3's render negotiation, in the only form this stage needs. Full negotiation — assets,
node types, rasterization — is stage 4. Here the question is binary and unavoidable: after
Task 8 the firmware has no C templates, so the host **must** send scenes to a
scene-capable device and must not send them to one without bit 8.

- [x] **Step 1: Write the failing tests.** A device advertising bit 8 receives
      `PushScene`; a device without it receives the legacy `ApplyConfig` widget path; the
      decision is per `(scene, device)` and re-resolved on reconnect, because a device can
      come back on different firmware after an OTA.
- [x] **Step 2: Run to confirm failure.**
- [x] **Step 3: Implement**, pushing on **event**, never on a timer: provider result,
      config change, data-state (stale/error) change, and playlist advance. The ledger is
      explicit that `BigNumberLabel`, `IconBadgeText`, `RowList` and `AnalogClock` need
      nothing else — if no new host fact arrives, the last scene stays correct as last-good
      data. **Do not add a periodic push.** §3 rejects the 1,440-per-day clock strategy for
      exactly the reason that applies here.
- [x] **Step 4: The refuse path must be typed and visible.** §3: a card that would freeze
      says so in its editor; it does not silently display a stopped clock. The V1 playlists
      plan fixed the validation-mislabeling defect precisely so a failure is never
      presented as something else, and that rule governs here.
- [x] **Step 5: Gates and commit.** `feat: send scenes to devices that can draw them`

---

### Task 8: Retire the C templates, and keep the oracle

**Files:**
- Modify: `firmware/main/CMakeLists.txt`, `firmware/main/ui/template_view.c`, `carousel.c`
- Move: `firmware/main/ui/templates/*.c` → a reference location the simulator compiles
- Modify: `companion/crates/lvgl-sim/` build glue

**The gate that proves the scenes correct compiles the C it compares against.** Deleting
those files outright would leave `scene_parity.rs` comparing a scene against nothing,
silently — the 106 rows would still "pass" while proving less and less.

> **Decision: the templates stop *shipping* but survive as the parity oracle.** Move them
> to a clearly-named reference directory that only `lvgl-sim` compiles, and drop them from
> the ESP-IDF build. The alternative — freezing the rendered framebuffers as goldens — is
> cheaper but weaker: a frozen golden cannot tell you whether an LVGL upgrade changed the
> scene, the C, or both, and this repo vendors LVGL. Keeping a live oracle preserves the
> ability to re-derive the answer.
>
> Record in the moved directory's own README that this code **does not ship** and exists
> only to be compared against, or someone will eventually "fix a bug" in a file no device
> runs.

- [x] **Step 1: Move the templates and rewire `lvgl-sim`.** Confirm the 106 + 24 rows
      still pass **unchanged** — the move must be provably a no-op before anything is
      removed from the firmware. Run the parity suite before and after and diff the counts.
- [x] **Step 2: Write the failing test** that the firmware no longer registers any C
      template and that a card with no scene falls back to the standalone clock rather than
      a blank screen. That fallback is the repo's standalone rule and it is what makes this
      task survivable.
- [x] **Step 3: Remove the templates from the ESP-IDF build** and delete the now-dead
      registry/dispatch paths.
- [x] **Step 4: Retarget `framebuffer_diff.rs` to the scene cases.** §6 requires this
      and it is easy to miss: the device-vs-simulator hardware diff currently drives
      **template** cases, and after this task the device has no templates to drive. Point
      it at `scene_cases()` instead. Carry the two existing exclusions forward with their
      reasons intact — `progress-ring--running-mid-countdown` (the running label's ceiling
      flips a second when push-to-capture latency crosses 1000 ms, so it is golden-only)
      and `row-list--truncation-boundary` (it pins a field above the registry maximum, so
      a device rejects the push). Both reasons still hold; deleting either exclusion
      without deleting its reason is how a flaky case gets re-introduced.
- [x] **Step 5: Record the memory delta**, which will be **large and negative** — the
      first big *shrink* this project has measured. Treat it with the same suspicion as a
      growth: the hazard is layout movement, and a shrink moves layout just as a growth
      does. `.bss`, DIRAM `.text`, `.data`, IRAM and DIRAM total, same-tree.
- [x] **Step 6: Gates and commit.** `feat: stop shipping the hand-written C templates`

---

### Task 9: GATE B — the second image

**Files:** `firmware/version.txt`, `docs/hardware/board-notes.md`

Separate from Gate A because a memory-layout failure and a rendering failure look nothing
alike, and this repository has twice spent days on the former with every test green.

- [x] **Step 1: Build, publish and power-cycle**, exactly as Task 6 Step 2.
      Done in `045f432`; owner power-cycled 2026-08-28 08:27Z.
- [x] **Step 2: Verify the OTA download completes.** PASSED on the first attempt —
      `live1` -> `live2` in 66 s, link then continuous for 6.5 min. Evidence is the
      server journal (this session had no admin token, so `ota_state` and
      `last_ota_error` were **not** read; rollback-window survival across a second boot
      is **not** settled). **The rollback-survival item closed 2026-09-06:** the device
      ran `v2.0.0-live2` across every subsequent reboot — including that session's USB
      RTS reset — and the templates-removed successor `v2.0.0-raster1` then installed
      first-try over OTA and survived its own rollback window (board-notes, "Stage 4
      Task 7 Phase B"). Recorded in `docs/hardware/board-notes.md`. This is the check the layout hazard
      demands: `firmware_version`, `ota_state: idle`, `last_ota_error: null`, link
      connected, and survival of the rollback window. **If the download fails, do not
      retry blindly** — the documented failure mode is *deterministic*, and `3f2aa03` was
      bisected across three builds from an identical base.
- [x] **Step 3: Re-observe one card per template at both orientations**, confirming the
      removal changed nothing visible. All six render correctly at 270 and 90 (a test
      config added the missing `analog-clock` and `big-number-label` cards). **It did NOT
      change nothing visible** — see the transition-flash defect in board-notes: the
      standalone clock now shows for ~220-250 ms at every carousel transition, because
      `SHOW_CARD_FALLBACK` replaced the local `SHOW_VIEW` render.
- [x] **Step 4: Confirm the standalone clock still appears** on host loss — pull the link
      and watch. It is the only face left when a scene is absent. Confirmed: the device
      retains and locally ticks the last scene for **~39 s** (measured at 60 fps), then
      loads `clock_screen.c` with its "Connect deskmate app" hint.
- [x] **Step 5: Record in `docs/hardware/board-notes.md`.**

---

### Task 10: Close the stage and hand off to 3b

**Files:** this plan, `CLAUDE.md`, `docs/scene/template-parity-ledger.md`,
`docs/superpowers/plans/2026-08-28-deskmate-plugin-manifest.md` (create)

- [x] **Step 1: Update the ledger** to say which gaps closed and how, rather than leaving
      a document that describes a world that no longer exists.
- [x] **Step 2: Update `CLAUDE.md`** — six templates retired, the binding vocabulary as
      shipped, and what remains unproven.
- [x] **Step 3: Write the stage 3b plan** (plugin manifest and curated plugins) from what
      this stage taught, per the working agreement: write the next plan at the current
      one's exit, using what was learned during implementation.
- [ ] **Step 4: Commit.** `docs: close stage 3a and plan the plugin manifest`

---

## Exit criteria

1. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace` pass.
2. `make -C firmware/host_tests clean test` and `make -C firmware/host_tests sanitize` pass.
3. `idf.py -C firmware build` clean, with same-tree memory deltas recorded for **both**
   images.
4. All six templates remain byte-identical to their (now non-shipping) C originals at both
   orientations, **and** the stale and error states are covered — **132 rows**.
   (This plan said 130, from a 106-row baseline. Task 2 added two rows for the
   zero-duration `ProgressRing` state, which the C renders as empty chips rather than
   `--`, so the real baseline is 108 and 108 + 24 = 132. No coverage was dropped.)
5. **A test proves each new binding's producer**, not only its formatter. The three timer
   defects existed because the gate injects binding inputs; a stage that adds five
   bindings and does not close that hole will simply add more.
6. **Every binding added is actually emitted by a builder**, and a temporal test proves the
   scene and the C advance together without a re-push. A binding no builder emits is dead
   wire surface that the byte-exact gate will happily report green forever.
7. Gate A observed: the clock crossing a minute boundary with a live date and dial, the
   timer advancing with a changing status word and colour, and a tap moving the timer with
   the link down — at both orientations.
8. Gate B observed: OTA download completes with the C templates removed, nothing visibly
   changed, and the standalone clock still appears on host loss.
   **Observed 2026-08-28, with one qualification and one item owed.** The download passed
   on the first attempt (`live1` -> `live2` in 66 s, `ota_state idle`, `last_ota_error
   null`); all six templates render at both orientations; the standalone clock appears on
   host loss after a ~39 s retained-scene window. **"Nothing visibly changed" is FALSE:**
   the standalone clock now flashes for ~220-250 ms at every carousel transition — see
   board-notes. Still owed: rollback survival across a second boot (the device has not
   rebooted since installing live2). **Closed 2026-09-06:** live2 survived every
   subsequent reboot through the stage-4 hardware session, and its templates-removed
   successor `v2.0.0-raster1` installed first-try over OTA and survived the rollback
   window (board-notes, "Stage 4 Task 7 Phase B"). Nothing from Gate B remains owed.
9. `docs/protocol/v1.md` documents every new binding, including the two sentences whose
   absence caused Task 1's defects.

Stage 3b — the plugin manifest and curated plugins — is planned at this plan's exit.

---

## Risks

**Memory layout, as always, and this time in both directions.** Tasks 1-5 grow the
firmware; Task 8 shrinks it substantially. Both move layout. Flat internal RAM has
predicted a clean download twice (2026-08-26, 2026-08-27) — two data points, not a law,
against a failure mode that is deterministic when it bites.

**The binding vocabulary is a ratchet.** Every binding added here is a permanent
firmware-side surface that a future device must keep evaluating. Five is a deliberate
ceiling: they are the exact set the ledger derived from the two blocked faces, and
`timer.status` is the only conditional among them. **If a task finds itself wanting a
sixth, stop and re-read §2** — "anything computed happens on the server" is the rule, and
the pressure to add "just one more" is precisely how a closed set becomes an expression
language.

**Task 4 is the one place this stage can regress behaviour rather than add it.** The
optimistic local action is a prediction; the host's `PushData` is authoritative. Board
notes already record a sticky unreconciled optimistic flash from 2026-08-15. Getting the
reconciliation wrong turns a working offline face into a lying one, which is worse than
the gap being closed.
