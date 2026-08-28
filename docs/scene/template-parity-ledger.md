# Scene template parity, and what stage 3a settled

Date: 2026-08-27. **Updated 2026-08-28 at stage 3a exit.**

This ledger was written as the input to stage 3, where the six hand-written C templates
are retired. Stage 2b proved that a host-built scene and its C template draw the same
framebuffer at a pinned instant. Retirement asked a different question: after that scene
has been pushed, can the device keep it correct as local time and timer state advance?

**Stage 3a answered it, and the answer was yes for all six.** The document below keeps its
original derivation — the reasoning is why the vocabulary is the size it is — with each
gap marked resolved and the evidence that closed it named. What remains open is listed in
[Stage 3b entry decision](#stage-3b-entry-decision) at the end.

## What stage 3a changed

| Gap this ledger identified | How it closed | Where |
| --- | --- | --- |
| Three `ProgressRing` binding defects the byte-exact gate could not see | Producer fixed and tested directly, not through injected inputs | `20304d9`..`abea638` |
| `ProgressRing`'s TOTAL/ELAPSED/STATUS literals and its running colour | `timer.total`, `timer.elapsed`, `timer.status`, `timer.permille`, and the `running_color` style selector | `abea638`..`d143d51` |
| `DigitalClock`'s literal date and both small-dial hand endpoints | `date` and `time:angle:hour` / `time:angle:minute` | Task 3 |
| A tap reaching only the C view, so a scene could not move with the link down | The local action is applied to the scene timer context and its bindings refreshed | Task 4 |
| The shared stale/error footer missing from every builder | `push_state_footer()`, reached by all six through `finish_scene()` | Task 5 |
| Retiring the C templates would delete the parity oracle | Templates moved to `companion/crates/lvgl-sim/reference-oracle/`, with a build guard against their return | `7d47ab6` |

The parity matrix grew from 108 rows to **132**: 24 new rows cover the stale and error
states the original 108 never exercised. All 132 are byte-identical at both orientations
with no tolerance.

**The gate's second blindness closed at its source.** `SceneTimer` went from
`{ remaining_ms, pct }` to `{ total_ms, remaining_ms, running }` during Task 2, so
percentages are now derived in C rather than injected by the test. That change was not
requested by the plan; it is what makes the timer rows evidence about meaning rather than
only about drawing.

## What the shipped bindings actually do

The device refreshes a live scene every 250 ms and immediately after matching `PushData`.
The closed binding set is not an expression language. This is the complete vocabulary as
shipped; `docs/protocol/v1.md` is its normative grammar.

| Binding | Used by these builders | Device source and effect |
| --- | --- | --- |
| `time:HH:mm` | `DigitalClock` | Formats the device wall clock plus its current UTC offset into the hero reading. |
| `time:ss` | `DigitalClock`, when seconds are shown | Formats seconds from the same device clock. |
| `date` | `DigitalClock` | Applies the UTC offset, breaks down local time, and delegates to `timefmt_date()` — the same `Mon, Aug 3` form the C face drew as a literal. |
| `time:angle:hour`, `time:angle:minute` | `DigitalClock`'s small dial | Rotates the two `SceneLine` hands. `hour = (h % 12) * 30 + m / 2`, `minute = m * 6`, keeping the LVGL line draw path rather than trading it for a rot-rect. |
| `time:hour`, `time:minute`, `time:second` | `AnalogClock` | Rotation-only bindings recomputing the three hand rotations from the device clock in the same whole-degree steps as `analog_clock_tick()`. The second-hand node is absent when `show_seconds` is false. |
| `timer.remaining:mm:ss` | `ProgressRing` | Formats the locally counted-down `remaining_ms` from the matching progress card's latest `PushData` snapshot, with the C face's ceiling and total-minute behaviour. Keeps advancing while the link is down. |
| `timer.elapsed:mm:ss`, `timer.total:mm:ss` | `ProgressRing` | The ELAPSED and TOTAL chips, formerly literals. Elapsed is total minus the *displayed* remaining time, so the two chips cannot disagree by a rounding step. |
| `timer.status` | `ProgressRing` | `Done`, `Running`, `Ready` or `Paused` — or `--` with no active snapshot. A conditional, added as a named domain fact rather than as a generic one. |
| `timer.permille` | `ProgressRing` | Scales the indicator arc's declared sweep at 0..1000. **Not `timer.pct`:** at r=195 the circumference is ~1225 px, so one percent is 12.25 px of arc and percent quantisation is visibly insufficient. |
| `timer.pct` | **None of the six** | Kept as wire surface; the remaining percentage at 0..100. |
| `field.*` | **None of the six** | Resolves a field from the matching card's widget-model state. Refreshed on `PushData`, but not autonomous: without a host update there is no new value to evaluate. |

`running_color` on `SceneArc` (key 10) and `SceneText` (key 8) is a **style selector, not a
value binding** — the device swaps between two literal colours on its `timer_running` flag.
`progress_ring.c` changes the arc indicator *and* the STATUS text colour on that flag, and
a `SceneValue` resolves to text, so a value binding could not have expressed it.

`time:*` reads `time(NULL)` plus the device's stored UTC offset. Timer bindings read the
device's mirrored progress snapshot and subtract monotonic uptime while `running` is true.

## Template ledger

| Template | Literals and build-time choices | Bindings | Stage 3a disposition |
| --- | --- | --- | --- |
| `DigitalClock` | The existence of the seconds node from `show_seconds`. | `time:HH:mm`; optional `time:ss`; `date`; `time:angle:hour`; `time:angle:minute`. | **Retired.** The date and both hands are live; nothing goes stale without a re-push except a config change. |
| `AnalogClock` | Chapter ring, ticks, hub, and whether the second-hand node exists. | `time:hour`, `time:minute`, optional `time:second`. | **Retired.** Changing `show_seconds` is a config change and correctly requires a new scene. |
| `BigNumberLabel` | Title, value (including the `--` fallback), label, chosen value/label font tiers, and layout derived from those tiers. | None. | **Retired, event-driven.** A value change can cross a font-tier boundary and move both value and label, so a text-only `field.*` refresh would be incorrect; the host rebuilds. |
| `IconBadgeText` | Title, badge, value, label, selected icon geometry, chosen font tiers, and tier-dependent vertical layout. | None. | **Retired, event-driven.** Icon changes replace the node list and value changes can change tier and layout. |
| `RowList` | Title; count; every row title/time; which row modules exist; and the empty-state module. | None. | **Retired, event-driven.** A data update can add or remove rows, change the count, or switch the entire empty-state branch. |
| `ProgressRing` | Label; the empty-chip values when duration is zero. | `timer.remaining:mm:ss`, `timer.elapsed:mm:ss`, `timer.total:mm:ss`, `timer.status`, `timer.permille`; `running_color` on the indicator arc and STATUS. | **Retired.** The chips, the status word, the two colours and the local tap all reach the scene. |

### The shared data-state footer — resolved

All six C templates create `OBJ_STATE`; `template_view.c` used it to show an error in the
error colour, `Stale` in the stale colour, or nothing in the OK state. Every builder
omitted that node, because stage 2b's rows all exercised the OK state.

`push_state_footer()` now adds it as an ordinary literal node, reached by all six builders
through the single `finish_scene()` path, and 24 parity rows cover it. It stayed a literal
deliberately: stale/error is a new host-owned fact like a provider result, so the host
rebuilds and pushes when it changes. `field.error` plus `field.stale` could not have
expressed it — the footer selects text, colour *and* visibility together.

## The blocking gaps, and how they were decided

### DigitalClock: date and dial — Option A shipped

Two options were on the table. **Option A** extended the native time bindings; **Option B**
pushed a new scene every minute boundary — 1,440 pushes per device per day, which §3 of the
[scene-rendering design](../superpowers/specs/2026-08-22-deskmate-plugin-scene-rendering-design.md#3-render-negotiation)
rejects, because one delayed or lost push leaves the face wrong at exactly the moment
standalone rendering is supposed to protect it.

Option A shipped: one `date` token emitting exactly the `date_text()` form, and an angle
binding on the two `SceneLine` hands. The line draw path was preserved rather than swapped
for a `SceneRotRect`, which would have traded away the pixel proof.

### ProgressRing: the three defects the gate could not see

These are kept in full, because they are the clearest evidence for why an injected-input
gate proves less than it appears to. All three were present while the byte-exact gate read
zero differing pixels.

1. `progress_ring.c` draws **remaining** percentage, and the parity request supplied
   `remaining_seconds * 100 / duration_seconds`. On the device, `fill_timer_bindings()`
   set `timer_pct` to `(total_ms - remaining_ms) * 100 / total_ms` — **elapsed**
   percentage. A native scene would have grown where the C ring shrank; the two agreed
   only at the halfway point. **Fixed:** the producer computes remaining percentage and
   its C fields name that meaning explicitly.
2. `progress_ring.c` displays `(remaining_ms + 999) / 1000`, a ceiling that holds the
   current second until it has fully elapsed. `timer.remaining:mm:ss` formatted
   `remaining_ms / 1000`, a floor, so it could show the next second almost immediately.
   **Fixed:** the binding applies the same ceiling.
3. `progress_ring.c` formats total minutes and supports `1440:00` at the 86,400-second
   bound. The generic formatter treated `mm` as the minute component of a clock, modulo
   60, so at one hour it rendered `00:00` instead of `60:00`. **Fixed:** wall-clock and
   countdown formatters are separate, countdown `mm` means total minutes, and the
   countdown is clamped to the same ceiling as the C face.

**The state-ownership gap is also closed.** On a tap, `carousel.c` sent the event and
called `template_view_apply_local_action()` for optimistic start/pause/reset feedback —
a path that updated only the C view. A live scene had no equivalent and read an unchanged
widget-model snapshot until the host answered with `PushData`, so with the link down the
scene never reflected the tap. Task 4 routes that local transition into the scene timer
context and refreshes its bindings, so retiring the C template did not retire its offline
behaviour.

Option B — rebuilding and pushing on every timer update — was rejected: ELAPSED makes it a
push every second while running, up to 86,400 per day, worse than the clock strategy §3
already rejects. Pushing only on transitions would fix the words and colours and still
leave ELAPSED frozen.

### Host-owned data templates

For `BigNumberLabel`, `IconBadgeText` and `RowList` the alternative to event-driven pushes
was a collection of field, conditional-layout and dynamic-node bindings — a layout engine
on the device that still could not fetch provider data without the host. They rebuild and
push on each provider, config or data-state change. This is event-driven invalidation, not
a periodic approximation: if no new host fact arrives, the last scene remains correct as
last-good data. `AnalogClock` needs no additional binding for the same reason.

## What the evidence proves, and what remains open

- The byte-exact gate compares the C template and the scene through the same host
  simulator and the same LVGL. All six templates, **132 rows**, both logical orientations,
  zero differing pixels, no tolerance. It is conclusive for the compared inputs and
  catches divergence between the two implementations. It remains structurally blind to a
  defect in code or assumptions the two halves **share**.
- The flipped simulator framebuffer is an exact reversal of the landscape framebuffer, and
  the device's pre-flush 0x7E capture carries the same reversal. **Neither can prove
  physical 270° panel geometry** — that is only ever a panel observation.
- `field.*` and `timer.pct` are wire surface no builder emits. A binding no builder emits
  is surface the byte-exact gate reports green on forever; both are retained deliberately
  (`field.*` for plugin-authored scenes in stage 3b) rather than by omission.
- The asset-GC teardown/rebuild sequence still has **no automated end-to-end test**.
  Stage 3b's runtime assets must exercise teardown, font-registry reset, compaction,
  retained scene rebuild, and the `BUSY`/OTA-owner paths before anything relies on it.
- `number_font_tier()` is used by both numeric templates, but only `BigNumberLabel` has
  rows deliberately straddling the HERO and DISPLAY step-down boundaries.
  `IconBadgeText` proves selected examples, not an independent boundary matrix.

## Stage 3b entry decision

The vocabulary is now a **ratchet**: every binding here is a permanent firmware-side
surface a future device must keep evaluating, and the whole set is nine tokens plus one
style selector. Stage 3b authors plugins against that set. If a curated plugin wants a
tenth, the answer is a host-side rebuild-and-push, not a new token — "anything computed
happens on the server" is §2's rule, and the pressure to add just one more is exactly how
a closed set becomes an expression language.

Two things stage 3b inherits and must not lose:

1. **`field.*` is the plugin data path.** It is the one binding designed for values a
   plugin supplies, and it is the only member of the vocabulary with no builder and
   therefore no pixel coverage. A plugin that binds it is exercising an untested arm.
2. **A gate that supplies a binding's input proves how a value is drawn, never what it
   means.** Stage 3a's three defects lived in exactly that hole for a whole stage. Any
   plugin-facing test that pins its own inputs inherits the same blindness.
