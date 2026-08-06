# Deskmate — Card Model Design

**Date:** 2026-08-06
**Status:** Approved by Rodion (pending written-spec review)
**Amends:** `docs/superpowers/specs/2026-08-03-deskmate-design.md` sections 4 and 5
**Supersedes:** `docs/config/v2.md` (becomes `docs/config/v3.md`)

## 1. Why

The v2 authoring model separates `widgets[]` from `screens[]` and carries three size
classes so that a screen can be one full-canvas widget or a 2x2 grid of tiles. In
practice:

- The companion creates exactly one screen per widget, so the two arrays are 1:1 and
  the join is ceremony the user cannot vary.
- `size` is never exposed in the companion, and the firmware has no size concept at
  all. It is host-only bookkeeping.
- `standard` already renders identically to `full` after M3's clean-canvas amendment.
- The dashboard grid lets a user express layouts a 448x368 panel at desk distance
  cannot render legibly.

The result is a settings experience with three coordinated panels maintaining a
relationship that has one legal value, and a planned M4 task (tile dashboards) whose
output would be unreadable on the target hardware.

This design replaces widgets, screens, sizes, and grids with one concept — a **card** —
and one rotation mechanic with a bounded alert mechanic beside it.

## 2. Product decisions

The device is an **ambient, contextual** display: it shows one thing at a time at full
size, rotates on a timer, and lets a bounded set of events take over the screen
temporarily. It is not an at-a-glance panel and not a dashboard. Density is not a goal;
legibility is.

Consequences, in order:

1. There is one canvas. Every template has exactly one 448x368 layout.
2. Size classes are removed from the authoring model.
3. Multi-widget grids are removed from the roadmap.
4. Rotation is a playlist with per-card dwell over a global default.
5. Alerts are a separate, closed mechanic — not a general condition language.

The Apple Watch complication approach was considered and rejected. Its purpose would
have been to avoid maintaining per-size template variants; removing the grid removes
those variants outright, so the complication host would add a concept without solving a
remaining problem.

## 3. The card

A configuration is one ordered array of 1..8 cards. A card is a closed tagged object
carrying everything about itself:

```json
{
  "kind": "calendar",
  "id": "upnext",
  "title": "Up next",
  "source": { "kind": "url", "value": "https://example.invalid/cal.ics" },
  "template": { "kind": "row-list" },
  "tap_action": { "kind": "none" },
  "refresh": { "kind": "interval", "minutes": 15 },
  "presence": { "kind": "in-rotation", "dwell_seconds": 30 },
  "alert": {
    "kind": "before-event",
    "lead_minutes": 5,
    "hold": { "kind": "seconds", "value": 60 }
  }
}
```

Card IDs are 1..32 UTF-8 bytes and unique among cards. Cards and assets remain separate
ID namespaces, as widgets and assets were in v2; the screen namespace is gone. List
order is carousel order. There is no `size`, no `layout`, and no `screens` array.

Card kinds, provider settings, templates, refresh policies, tap actions, and all string
and numeric bounds are carried forward unchanged from v2 except where stated in this
document.

### 3.1 Presence

`presence` states whether and how a card participates in the rotation.

| Variant | Meaning |
|---|---|
| `in-rotation { dwell_seconds }` | Appears in the carousel. `dwell_seconds` is `null` (inherit the global default) or `5..3600`. |
| `alert-only` | Configured and polled, never in the carousel. Exists solely to take over when its alert fires. |
| `off` | Muted. Not polled, not compiled to the wire, settings preserved. A muted pomodoro's timer does not run. |

`off` exists so that hiding a card does not require deleting it and losing its
configuration.

### 3.2 Alerts

`alert` is a closed set validated against the card's kind. Only two v1 sources have
real events, so only two triggers exist.

| Variant | Valid on | Fields |
|---|---|---|
| `none` | any kind | — |
| `on-timer-finish` | `pomodoro` only | `hold` |
| `before-event` | `calendar` only | `lead_minutes` `1..60`, `hold` |

`hold` is `until-dismissed` or `seconds { 5..600 }`.

**`hold` is host-side bookkeeping. Decided 2026-08-06.** It bounds how long the arbiter
treats an alert as outstanding, freeing the slot for the next one. It does **not** clear
the panel: under protocol v1 the device yields the interrupt overlay only on tap. There
is no host→device dismissal message, `TriggerInterrupt` carries no duration, and sending
`ActivateScreen` while an interrupt is active changes only the screen *behind* the
overlay. This is the intended, documented meaning of the field, not a deficiency to be
worked around — an alert worth interrupting you is worth acknowledging.

Verified on hardware 2026-08-06 (`docs/hardware/board-notes.md`, card-model §6): with
rotation disabled and a 60-second hold, the alert stayed until tapped; with a fast
rotation and an `until-dismissed` hold, a carousel advance did not disturb it. The card
editor's copy already states this to the user.

Adding a wire-level dismissal message was considered and rejected for v1: it costs a
protocol addition, firmware work, and a hardware re-verification to make alerts able to
disappear unacknowledged, which is the opposite of what an alert is for.

`presence` and `alert` are deliberately independent fields rather than one collapsed
variant, because the common case is a card that is both in the rotation and alerting: a
calendar you read during the loop *and* that warns you before an event. Collapsing them
would make that unrepresentable. The cost is exactly one cross-field validation rule
(section 8). The benefit beyond expressiveness is that muting a card to `off` preserves
its lead time and hold settings.

### 3.3 Carousel

```json
{ "carousel": { "advance": { "kind": "timed", "default_dwell_seconds": 20 } } }
```

`advance` is `manual` or `timed { default_dwell_seconds: 5..3600 }`. Under `timed`,
each in-rotation card holds for its own `dwell_seconds`, or the default when `null`.
Manual navigation and array order remain authoritative regardless of timed advance,
unchanged from v2.

Timed advance is driven entirely by the host, which sends `ActivateScreen` when a
card's dwell expires. The firmware has no rotation timer.

## 4. Config schema v3

```json
{
  "schema_version": 3,
  "preferences": {},
  "cards": [],
  "assets": [],
  "carousel": {},
  "updater": {}
}
```

`preferences`, `assets`, and `updater` are unchanged from v2, including the two
landscape orientations, the 65,536-byte document limit, and rejection of unknown
fields at every level.

Removed from v2: the `widgets` array, the `screens` array, the screen ID namespace,
the `size` field, the `full`/`standard`/`tile` size classes, the `single` layout
wrapper, the `dashboard { columns, rows, tiles }` layout, and
`carousel.auto_advance_seconds`.

Added: the `cards` array, `presence`, `alert`, and `carousel.advance`.

## 5. Migration

v3 recognizes every released version before deserializing its body, preserving the
existing guarantee that unknown future versions, malformed documents, and failed
migrations return a recoverable typed error without rewriting the source bytes.

v0 and v1 documents chain through the existing v2 migration first. v2 to v3:

- **Order comes from `screens[]`, not `widgets[]`.** The screen array is the carousel
  order and is what users authored.
- **Each card keeps its widget ID.** That ID is what `PushData`, `ProviderSnapshot`,
  and pomodoro runtime state already key on, so nothing downstream re-binds. Screen IDs
  are discarded.
- `size` is dropped from every card.
- `interrupt_policy: "enabled"` becomes the kind-appropriate alert with defaults:
  pomodoro to `on-timer-finish` with `until-dismissed`, calendar to `before-event` with
  5 minutes and a 60-second hold. On the other four kinds it becomes `none`, because
  those kinds had no trigger that could ever fire.
- `interrupt_policy: "disabled"` becomes `alert: none`.
- A card migrated from a screen-referenced widget gets
  `presence: in-rotation { dwell_seconds: null }`, inheriting the global default. A
  widget with no referencing screen — which v2 validation should prevent but the
  protocol permits — becomes `alert-only` when its interrupt policy was enabled and
  `off` otherwise.
- `carousel.auto_advance_seconds: N` becomes `timed { N }`; `null` becomes `manual`.

Migration cannot produce a configuration that violates the at-least-one-in-rotation
rule: v2 requires every widget to be referenced by exactly one screen, so every
migrated card is `in-rotation`. The `alert-only` and `off` outcomes above exist only to
handle a protocol-legal document that v2 validation would already have rejected.

**No persisted configuration can contain a dashboard layout.** Dashboard composition is
rejected with `requires-capability` during validation, and validation runs before
persistence, so no dashboard document has ever reached disk. Migration has no grid path
and must not grow one.

## 6. Wire and firmware impact

The protocol does not change and the firmware needs no structural change.

`ApplyConfig` remains `{0: revision, 1: widgets, 2: screens, 3: rotation}`. The host
compiles `cards[]` down to that frozen pair:

- Each `in-rotation` or `alert-only` card emits one widget map with `size_class` pinned
  to `Full`.
- Each `in-rotation` card additionally emits one screen map, in card order, whose
  screen ID **is the card ID**. The protocol treats widget and screen IDs as distinct
  namespaces, so reusing the same string across both is legal. This preserves v2's
  determinism property — compilation still invents no identifiers, so identical input
  and revision still produce identical protocol bytes.
- `alert-only` cards emit no screen. `docs/protocol/v1.md` already specifies that
  configured widgets may be interrupt-only and need not have a carousel screen.
- `off` cards emit nothing at all.

Every existing `ApplyConfig` fixture stays byte-valid, including the maximum-capacity
fixture. `WIDGET_MODEL_CONFIG_UNSUPPORTED_SIZE_CLASS` is retained: the host will never
emit a non-`Full` class again, but the firmware must still reject a malformed one.

This is the reason the redesign is affordable. `ApplyConfig` is a compilation target
rather than a serialization of the config document, so the authoring model can be
replaced while the emitted bytes stay identical.

## 7. Companion experience

### 7.1 One list

`WidgetGallery`, `ScreenArranger`, and the draft/selection synchronisation between them
collapse into a single card list with two sections:

- **In rotation** — numbered, drag- and keyboard-reorderable. Order is carousel order.
- **Alerts and muted** — unordered, because `alert-only` and `off` cards have no
  carousel position. Numbering them would be a lie.

Selecting a card opens its editor; there is no separate screen concept to arrange. The
step numbering (`1 · Widgets`, `3 · Screens`) is removed — it promised a linear flow
the layout never had.

All six card kinds are addable: clock, pomodoro, calendar, weather, JSON feed, and RSS.
Card IDs are internal and are not displayed.

### 7.2 The preview is the feedback loop

Changes reach the device through an explicit save, unchanged from M3. The in-app
preview, not the physical panel, carries the edit-see-adjust loop. That places real
requirements on the preview.

**The snapshot must carry field values.** `ProviderSnapshot` currently carries
`widget_id`, `state`, `last_success_unix_ms`, and `age_seconds` — freshness, but no
data — so the preview invents its content. That hides the three things hardest to get
right:

- whether an ICS feed parsed into anything, as opposed to merely fetching;
- whether a JSON mapping selected the intended field, which is the difficult part of
  configuring a JSON feed and which invented data will always appear to satisfy;
- whether real strings overflow or truncate at 448 px, which invented short strings can
  never reveal.

The runtime already derives exact rendered field values in order to build `PushData`.
The typed IPC snapshot is widened to include the last-good field values per card, so
the preview renders from the same values the device receives. This adds no privilege:
the webview still performs no network, filesystem, serial, or process access and only
receives values the runtime already computed.

The preview's claim changes from "layout preview, not pixel-identical" to **same data
as your display, approximate pixels**. When a card has no data yet — never fetched,
offline, or errored — the preview shows a clearly marked sample state so unconfigured
cards still preview.

### 7.3 The filmstrip

Below the device mock, a filmstrip shows the rotation as proportional-width segments,
one per in-rotation card, sized by dwell. It displays the computed loop length, doubles
as the reorder control, and can play the rotation at real dwell timing so dwell and
lead-time choices can be judged. It replaces the preview dots, which duplicated the
arranger's ordering in a second place.

### 7.4 Defects to fix in the same work

These are existing faults in the areas this design touches, not unrelated refactoring:

- The gallery caps widgets at 16 while the config contract and wire cap at 8, so the UI
  can build a configuration the backend rejects.
- Weather, JSON-feed, and RSS providers shipped in M4 Task 2 but are unreachable from
  the UI, which offers only clock, pomodoro, and calendar.
- Validation issue paths are array-index based. Today that is safe because reordering
  touches only `screens[]`. Under one reorderable `cards[]` array it breaks: dragging a
  card reassigns indices and errors attach to the wrong row. **Issues must be keyed by
  card ID.**
- First-run guidance returns true unless the configuration contains both a pomodoro and
  a calendar, so the checklist never clears for anyone who wants neither.
- Internal screen IDs are rendered to users in the arranger.

### 7.5 Gesture disclosure

Three gestures share one canvas: tap performs the card's tap action, swipe navigates,
and tap dismisses an active alert. The tap meaning during an alert takes precedence, as
the protocol already specifies. The card editor must state a card's tap behaviour
rather than leaving it undiscoverable.

## 8. Validation rules

Carried forward from v2 unchanged: all string, numeric, URL, path, and count bounds;
closed tagged variants; rejection of unknown fields; and the fixed transaction order of
merge, validate and compile, capability check, atomic persist, runtime replacement,
then queued replay.

New or changed:

1. `cards` contains 1..8 entries with unique IDs.
2. `presence: alert-only` requires `alert` to be a variant other than `none`. This is
   the single cross-field rule accepted in section 3.2.
3. `alert` variants must match the card kind: `on-timer-finish` only on `pomodoro`,
   `before-event` only on `calendar`.
4. At least one card must be `in-rotation`. An all-alert or all-muted configuration
   would leave the carousel empty and silently fall back to the standalone clock while
   reporting success.
5. `dwell_seconds` is `null` or `5..3600`; `default_dwell_seconds` is `5..3600`.
6. `lead_minutes` is `1..60`; `hold.seconds` is `5..600`.
7. Timed advance is no longer capability-gated, because this design implements it.

## 9. Testing

- Migration fixtures: representative v0, v1, and v2 documents, including a v2 document
  whose `screens[]` order differs from its `widgets[]` order, proving card order follows
  screens.
- Compilation fixtures: a card set containing `in-rotation`, `alert-only`, and `off`
  cards compiles to the expected frozen `ApplyConfig` bytes, with `off` cards absent and
  `alert-only` cards present as screenless widgets.
- Byte-stability: the existing maximum-capacity `ApplyConfig` fixture re-encodes
  identically.
- Validation: each rule in section 8, at and outside its bounds.
- Host scheduling: dwell expiry ordering, dwell inheritance from the default, and manual
  navigation overriding a pending advance — all host-side and fully testable without
  hardware.
- Alert takeover and yield: on the physical panel, observe and record whether the
  overlay itself clears when `hold` expires or only on tap (see the known limitation
  noted with the `hold` definition in section 3.2), and confirm the carousel position
  the device is showing once the overlay does clear matches the host's arbiter state.
  This must be observed, not assumed — do not write a test that asserts a specific
  yield behavior the codebase has not decided.
- Frontend: reorder-then-validate keeps issues attached to the correct card; loading,
  empty, error, stale, offline, and no-data preview states.
- Firmware: unchanged structurally; existing host suites must pass without modification,
  which is itself the evidence that the wire did not move.

## 10. Impact on the M4 plan

| Task | Effect |
|---|---|
| new task before 3 | Schema v3, migration, compile-to-frozen-wire, IPC field values |
| 3 — remaining templates | Shrinks. Size-class compatibility work is removed; each template gets one 448x368 layout. Still owes big-number, icon-badge-text, and analog clock. |
| 4 — tile dashboards | Replaced. Becomes host-side timed rotation and alerts over the existing interrupt path. No firmware layout work, no grid placement rules, no grid physical checks. |
| 8 — settings experience | Grows. Single card list, filmstrip preview, six addable kinds, and the section 7.4 defects. |

Documents that must move in the same change: `docs/config/v2.md` to `docs/config/v3.md`;
design spec sections 4 and 5, which currently specify size classes and dashboard grids;
the M4 plan; and the CLAUDE.md current-state and clean-canvas bullets.

The fixture `companion/crates/app-core/tests/fixtures/future-v3.json` proves that
unknown future versions fail safely. Shipping v3 requires renaming it to
`future-v4.json`, or the test begins asserting the opposite of its intent.

## 11. Out of scope

Unchanged from the M4 plan: arbitrary plugins or scripts, shell execution, cloud
accounts or sync, mobile clients, remote device control, and unbounded user-supplied
HTML.

Specific to this design: multi-widget layouts of any kind, portrait orientation,
on-device rotation gestures, complication or slot hosts, named modes or scenes, a
general condition language for alerts, alert triggers on weather, JSON-feed, or RSS
cards, and live-apply of edits to the device.

## 12. Decision log

| Decision | Choice | Why |
|---|---|---|
| Device role | Ambient and contextual | One thing at a time, rotating, with events able to take over; density was never achievable at 448x368 |
| Multi-widget screens | Removed | The panel cannot render four legible tiles at desk distance; removing them also removes per-size template variants |
| Size classes | Removed | `standard` already aliased `full`, firmware has no size concept, and nothing renders at `tile` without a grid |
| Widgets and screens | Merged into `cards[]` | The join was 1:1 and unauthorable; merging collapses three UI panels into one list |
| Contextual behaviour | Timed playlist plus bounded alerts | Smaller change than eligibility-and-priority, and reuses the proven interrupt arbitration |
| Alert triggers | Closed set, timer and calendar only | A general condition rule would reintroduce an evaluator over feed-controlled data, and alerts seize the whole screen |
| Presence and alert | Two fields, one cross-field rule | A card is commonly both in rotation and alerting; collapsing them would make that unrepresentable and would lose settings on mute |
| Dwell time | Global default with per-card override | Most users have one intent; per-card-only would be eight decisions and would hide loop length |
| Applying edits | Explicit save retained | The in-app preview carries the iteration loop; live-apply would put keystroke-rate revision churn through the atomic replay path |
| Preview data | Real last-good field values over IPC | A preview with different data is an illustration; the gap sits exactly on JSON mappings and text overflow |
| Wire format | Unchanged | `ApplyConfig` is a compilation target, not a mirror of config, so the authoring model can be replaced without moving bytes |
