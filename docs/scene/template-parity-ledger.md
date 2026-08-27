# Scene template parity and stage 3 readiness

Date: 2026-08-27

This ledger is the input to stage 3, where the six hand-written C templates are retired.
Stage 2b proved that a host-built scene and its C template draw the same framebuffer at a
pinned instant. Retirement asks a different question: after that scene has been pushed,
can the device keep it correct as local time and timer state advance?

The inventory below comes from the six builders in
[`scene_build.rs`](../../companion/crates/app-core/src/scene_build.rs), as they exist at
stage 2b exit. A value called a literal is frozen into `PushScene`; only another scene push
can change it. A binding is re-evaluated in place by the device. Static geometry and style
are omitted from the inventory unless a changing value selected them at build time.

## What the shipped bindings actually do

The device refreshes a live scene every 250 ms and immediately after matching `PushData`.
The closed binding set is not an expression language:

| Binding | Used by these builders | Device source and effect |
| --- | --- | --- |
| `time:HH:mm` | `DigitalClock` | Formats the device wall clock plus its current UTC offset into the hero reading. |
| `time:ss` | `DigitalClock`, when seconds are shown | Formats seconds from the same device clock. |
| `time:hour`, `time:minute`, `time:second` | `AnalogClock` | Rotation-only bindings. They recompute the three hand rotations from the device clock in the same whole-degree steps as `analog_clock_tick()`. The second-hand node is absent when `show_seconds` is false. |
| `timer.remaining:mm:ss` | `ProgressRing` | Formats the locally counted-down `remaining_ms` from the matching progress card's latest `PushData` snapshot. This is intended to keep advancing while the link is down. Its ceiling and total-minute behavior were fixed in Task 1 (`20304d9`) to match the C face. |
| `timer.pct` | `ProgressRing` | Scales the indicator arc's declared sweep. Task 1 (`20304d9`) fixed the device producer to derive remaining percentage, matching the parity fixture and C face. |
| `field.*` | **None of the six** | Resolves a field from the matching card's widget-model state. It is refreshed on `PushData`, but is not autonomous: without a host update there is no new value to evaluate. |

`time:*` reads `time(NULL)` plus the device's stored UTC offset. Timer bindings read the
device's mirrored progress snapshot and subtract monotonic uptime while `running` is true.
`field.*` avoids rebuilding a scene for a simple host field update; it does not make a data
source live on the device.

## Template ledger

| Template | Literals and build-time choices | Bindings | What becomes wrong without another scene push | Stage 3 disposition |
| --- | --- | --- | --- | --- |
| `DigitalClock` | The formatted date; both small-dial hand endpoints computed from `ClockCard.local_now`; the existence of the seconds node from `show_seconds`. | `time:HH:mm`; optional `time:ss`. | The minute hand is wrong at the next minute, the hour hand advances only in the pushed geometry, and the date is wrong after local midnight. A UTC-offset change also updates the text bindings but not the literal date or dial. | **Blocked.** Text timekeeping is live; the date and dial are not. |
| `AnalogClock` | Chapter ring, ticks, hub, and whether the second-hand node exists. | `time:hour`, `time:minute`, and optional `time:second`. | No autonomous clock value goes stale. Changing `show_seconds` is a card/config change and correctly requires a new scene. | **Timekeeping-ready.** It still shares the omitted data-state footer described below. |
| `BigNumberLabel` | Title, value (including the `--` fallback), label, chosen value/label font tiers, and layout derived from those tiers. | None. | It remains the last known card value. A provider or card-data change requires a rebuild because the value can cross a font-tier boundary and move both value and label; substituting `field.*` only for the text would be incorrect. | **Ready with event-driven scene pushes** for new host data. There is no device-local value that should advance by itself. |
| `IconBadgeText` | Title, badge, value, label, selected icon geometry, chosen font tiers, and tier-dependent vertical layout. | None. | It remains the last known provider result. Icon changes can replace the node list, and value changes can change tier and layout, so a text-only field refresh is insufficient. | **Ready with event-driven scene pushes** for new host data. There is no autonomous gap. |
| `RowList` | Title; count; every row title/time; which row modules exist; and the empty-state module. | None. | It remains the last known list. A data update can add/remove rows, change the count, or switch the entire empty-state branch, so it requires a rebuilt scene. | **Ready with event-driven scene pushes** for new host data. There is no autonomous gap. |
| `ProgressRing` | Label; TOTAL, ELAPSED, and STATUS text; indicator and STATUS colours selected from `running`; empty-chip values when duration is zero. | `timer.remaining:mm:ss` for the large countdown; `timer.pct` for the indicator sweep. | ELAPSED is wrong after one second. STATUS cannot change among Ready/Running/Paused/Done. A start/pause transition leaves the indicator and status colour wrong. TOTAL remains correct only while duration is unchanged. Local tap feedback reaches only the C template. | **Blocked on the literal chips, state-dependent colours, and local tap ownership.** The three original timer-binding defects were fixed in Task 1 (`20304d9`); Task 2 closes the remaining literal-chip vocabulary gap. |

### The shared data-state footer is absent

All six C templates create `OBJ_STATE`. `template_view.c` uses it to show an error in the
error colour, `Stale` in the stale colour, or nothing in the OK state. Every builder omits
that node because stage 2b's 106 parity rows intentionally exercise the OK state. A scene
therefore cannot currently reproduce a later stale/error transition.

This is not a reason for a periodic device binding. Stale/error is a new host-owned fact,
just like a provider result. Stage 3 should rebuild and push the scene when that fact
changes, adding the footer as an ordinary literal node. The alternative is a semantic
data-state binding that selects text, colour, and visibility; `field.error` plus
`field.stale` alone cannot express that conditional. The host-push option adds no idle
traffic and matches the existing C path, which also changes the footer only when host data
state is applied.

## Blocking gaps and choices

### DigitalClock: date and dial

**Option A — extend native time bindings.** Add a closed date binding that emits exactly
the shipped `date_text()` form, and add a live geometry binding for the two `SceneLine`
hands. The line solution must preserve the LVGL line draw path; replacing the hands with
`SceneRotRect` would trade away the pixel proof. This is a wire and firmware change and
requires another OTA/download check and temporal tests around minute, midnight, and UTC
offset boundaries.

**Option B — push a new scene on every minute boundary.** No wire or firmware change, but
it means 1,440 pushes per device per day. Section 3 of the
[scene-rendering design](../superpowers/specs/2026-08-22-deskmate-plugin-scene-rendering-design.md#3-render-negotiation)
rejects that clock strategy: one delayed or lost push leaves the face wrong, precisely
when standalone rendering is supposed to protect it.

**Recommendation:** Option A. Use narrow semantic bindings, not a general expression
engine: one date token and a line-hand rotation/geometry binding whose firmware evaluator
uses the same integer clock arithmetic as the C face. `AnalogClock` proves the device
clock can drive hands correctly; it does not prove that a transformed rectangle can
replace `DigitalClock`'s line pixels.

### ProgressRing: existing binding defects fixed in Task 1

Task 1 (`20304d9`) fixed and tested three mismatches in the bindings already named by
the builder. The history remains here because it explains why injected parity inputs
could not prove producer semantics:

1. `progress_ring.c` draws **remaining** percentage. The parity request also supplies
   `remaining_seconds * 100 / duration_seconds`. On the device,
   `fill_timer_bindings()` set `timer_pct` to
   `(total_ms - remaining_ms) * 100 / total_ms`, which is **elapsed** percentage. A native
   scene therefore grew where the C ring shrank; the two percentages agreed only at the
   halfway point (apart from integer-rounding coincidences). **Fixed:** the producer now
   computes remaining percentage and its C fields name that meaning explicitly.
2. `progress_ring.c` displays `(remaining_ms + 999) / 1000`, a ceiling that holds the
   current second until it has fully elapsed. `timer.remaining:mm:ss` formatted
   `remaining_ms / 1000`, a floor, so it could show the next second almost immediately.
   **Fixed:** the binding applies the same ceiling as the C face.
3. `progress_ring.c` formats total minutes and supports `1440:00` at the 86,400-second
   bound. The generic binding formatter treats `mm` as the minute component of an
   hours/minutes/seconds clock, modulo 60. At one hour it rendered `00:00` instead of
   `60:00`. **Fixed:** wall-clock and countdown formatters are separate, countdown `mm`
   means total minutes, and the countdown is clamped to the same ceiling as the C face.

The simulator-to-simulator gate injects a pinned timer context and does not run the
device's `fill_timer_bindings()` clock-forward path. Its zero-pixel result is therefore
compatible with all three historical defects. Task 1 added direct producer and formatter
tests, including a running clock-forward snapshot, so those semantics no longer rely on
the injected parity gate.

There is also a state-ownership gap. On a tap, `carousel.c` sends the event and calls
`template_view_apply_local_action()` for optimistic start/pause/reset feedback. That path
updates only the C `ProgressRing` view. A live scene has no corresponding local action,
and its timer context continues to read the unchanged widget-model snapshot until the
host answers with `PushData`. With the link down, the scene never reflects the tap. Stage
3 must route that local transition into the scene timer context and refresh its bindings;
otherwise retiring the C template also retires its offline/optimistic behavior.

After those corrections, the remaining choices are:

**Option A — extend the closed timer vocabulary.** Add direct
`timer.elapsed:mm:ss` and `timer.total:mm:ss` bindings. Add a semantic `timer.status`
binding that returns Ready/Running/Paused/Done; the status word is a conditional, so two
numeric bindings do not solve it. Preserve the C colour transition with a narrow
timer-running style selector for the indicator arc and STATUS value, rather than adding a
general conditional/expression form to `SceneValue`. This is an additive wire and firmware
change. The same firmware work must apply local start/pause/reset to the timer state those
bindings read. It also needs temporal and disconnected-tap tests plus another OTA/download
check.

**Option B — rebuild and push the scene for every timer update.** This needs no wire
change, but ELAPSED makes it a push every second while running: up to 86,400 pushes per
day, worse than the 1,440-per-day clock strategy §3 already rejects. A network hiccup
freezes the face. Pushing only on start/pause/reset transitions fixes the words and colours
but does not fix ELAPSED.

**Recommendation:** Option A, after correcting the existing bindings. Keep the extension
semantic and small: timer total, elapsed, status, and running-dependent colour are domain
facts the device already owns. Do not add arithmetic or conditionals to `SceneValue`.
That preserves standalone timer behavior and keeps untrusted scene input away from a
general evaluator.

The temporal parity tests in
[`scene_parity.rs`](../../companion/crates/app-core/tests/scene_parity.rs) replace the
former literal-chip divergence regression: they advance the C template and an already
decoded scene through the same interval, without a scene re-push, and prove that elapsed
time, completion status, and start/pause colours remain byte-identical.

### Host-owned data templates

For `BigNumberLabel`, `IconBadgeText`, and `RowList`, the alternatives are either a new
scene when host data changes or a collection of field, conditional-layout, and
dynamic-node bindings. The latter would recreate a layout engine on the device and still
could not fetch new provider data without the host. Rebuild and push on each provider,
config, or data-state change. This is event-driven invalidation, not a periodic rendering
approximation; if no new host fact arrives, the last scene remains correct as last-good
data.

`AnalogClock` needs no additional time binding. Re-push it only for configuration or
data-state changes.

## What the evidence proves, and what remains open

- Stage 2b compares the C template and scene through the same host simulator and LVGL.
  All six templates, 106 rows, and both logical orientations are byte-identical. That is
  conclusive for the compared inputs and catches different C-template/scene-builder
  behavior; it is structurally blind to a defect in code or assumptions shared by both
  halves, and it does not advance a scene through time.
- The flipped simulator framebuffer is an exact reversal of the landscape framebuffer.
  Neither that gate nor the device's pre-flush diagnostic capture can prove physical 270°
  panel geometry. Real 270° geometry remains a panel observation.
- Stage 2a observed a `DigitalClock` scene with ticking seconds over the shipping path on
  the panel. It did not prove the literal date, every dial state, all six stage 2b
  templates, or byte-exact target rendering. The device-vs-simulator harness exists but
  its byte-exact physical run was deferred.
- The asset-GC teardown/rebuild sequence has no automated end-to-end test. Stage 3's use
  of runtime assets must exercise teardown, font-registry reset, compaction, retained
  scene rebuild, and the `BUSY`/OTA-owner paths before relying on it.
- `number_font_tier()` is used by both numeric templates, but only `BigNumberLabel` has
  rows deliberately straddling the HERO and DISPLAY step-down boundaries.
  `IconBadgeText` proves selected examples, not an independent boundary matrix.
- The shared stale/error footer is outside the current parity matrix and needs explicit
  stage 3 cases when it is added to builders.

## Stage 3 entry decision

Do not retire `DigitalClock` or `ProgressRing` yet. Bundle their narrow live-binding work
and the three existing timer corrections into one additive firmware image and one hardware
verification cycle. Retire `AnalogClock`, `BigNumberLabel`, `IconBadgeText`, and `RowList`
only after the host scene policy rebuilds on provider/config/data-state changes and the
shared footer has parity coverage.

The observation that would change this recommendation is a product decision to remove
the small dial/date from `DigitalClock` or the ELAPSED/STATUS modules and running colour
from `ProgressRing`. With the shipped faces preserved, periodic scene pushes are not an
acceptable substitute for native evaluation.
