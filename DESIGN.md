# Design — The Modular Face

<!-- impeccable:design-world modular-face -->

**Applies to:** `companion/apps/deskmate/src/**`, principally `src/styles.css`.
**Mode:** Operate. Compose the cards on a physical desk display, preview the result,
and explicitly save changes.
**Direction seed:** ef112502, originally pinned by the owner.
**Refinement, 2026-10-02:** the owner identified too many boxes, rounded tiles, and
purely decorative UI. Keep the display-and-loop model, but express it through open
layout, readable controls, and restrained selection. This document supersedes
`docs/design/companion-visual-language.md`.

## Principles

- The physical display is the focal point. Its preview stays black in both themes.
- Card order is one sequence, edited in one place. The card chooser is an open strip
  that wraps, with numbered positions and a blue underline on the selected card.
- Inputs and buttons have surfaces; helper text, card identities, and settings facts
  do not need enclosing boxes. Separate groups with space or a fine rule.
- Color communicates action or state. There are no decorative gradients or glows.
- Keep all existing draft, validation, ownership, keyboard, and explicit-save behavior.

## Color

| Token | Light | Dark | Purpose |
|---|---|---|---|
| `--ground` | `#F5F5F7` | `#000000` | Workspace |
| `--tile` | `#FFFFFF` | `#131316` | Inputs, controls, header, footer |
| `--tile-2` | `#F4F4F7` | `#1D1D21` | Hover and secondary controls |
| `--tile-3` | `#E6E6EB` | `#2A2A2F` | Disabled controls, scrollbars |
| `--ink` | `#1D1D1F` | `#F5F5F7` | Primary text |
| `--ink-2` | `#62626B` | `#A0A0A8` | Secondary text |
| `--ink-3` | `#6A6A72` | `#84848D` | Hints, labels, placeholders |
| `--hairline` | black at 9% | white at 10% | Dividers |
| `--stage` | `#000000` | `#000000` | Physical display |
| `--act` | `#0062E0` | `#0A84FF` | Selection, focus, primary action |
| `--live` | `#E5004C` | `#FA114F` | Running preview/timer feedback |
| `--good` | `#157B42` | `#30D158` | Healthy and saved states |
| `--warn` | `#975D00` | `#FF9F0A` | Pending or degraded states |
| `--bad` | `#D70015` | `#FF453A` | Errors and destructive actions |
| `--arc-a` / `--arc-b` | `#62626B` / `#B3B3BB` | `#A0A0A8` / `#53535C` | Neutral rotation ramp |

The selected arc and its legend swatch use `--act`, alongside stroke weight, position,
and the legend's stronger text. This is the card selected in the editor, not a claim
about which card is currently on the hardware. Other arcs use the neutral ramp.
Status words accompany semantic colors: an online panel uses `--good`, and the
Pomodoro state word uses `--live` (mixed with 4% `--ink` for text contrast) while
running and `--warn` while paused. The words carry the state; color only repeats it.
The preview badge sits on a permanently black screen, so it uses `#FFBC57` on
`#251B0B` in both themes for readable warning text.

## Type and controls

System fonts only; no font or icon library requests.

- UI: `ui-sans-serif, -apple-system, system-ui, "Segoe UI", roboto, sans-serif`.
- Times, durations, counts, and positions: `ui-rounded, "SF Pro Rounded", …`, with
  tabular numerals. Card names use the UI face, not the numeric role.
- Main loop heading: 1.5rem / 600. Editor headings: 1.15rem / 640.
- Card names: 1rem / 600. Countdown: 1.375rem / 550. Rotation total: 1.5rem / 550.
- Field labels, checkbox labels, and input values: 1rem. Labels use primary ink
  and medium weight. Necessary guidance, validation, and source status: 0.875rem
  with 1.5 line height. Secondary color belongs to supporting copy, not field labels.
- `--r-control: 7px`; `--r-tile: 14px` for notices and the settings dialog.
  The card strip has square, open edges and no filled selection box.
- Inputs are at least 40px high, primary buttons 38px, pacing controls 30px,
  rotation selections 36px, and add-menu choices 44px.
- The outline display mark and its local SVG favicon replace the glowing gradient mark.

## Composition

The desktop workspace is bounded to 1216px, including 24px side gutters. The left
column reserves 464px for the 448px display and a scrollbar; a 40px gap separates
that column from the editor, keeping 56px between the display and work region. The header and save footer align to the same content edges and are
72px high before content-driven expansion.

The rail and work column scroll independently. At heights of 800px or less, desktop
vertical spacing compresses so the display, rotation, and legend fit at 1060×740.
Below 1041px, the columns become one scrollable flow. The rail is at most 448px;
the work region is at most 640px. At 540px and below, the card strip has two columns,
and at 430px the footer stacks its message above its action.

The display is exactly 448×368 at full size. At widths of 540px and below it renders
at 224×184, a 2:1 downscale with smooth image sampling so thin firmware strokes remain
legible. Never stretch it to an arbitrary width. A neutral soft shadow belongs only
to the display: `0 12px 28px -16px`, black at 28% in light mode and 60% in dark mode.

## Card editing

The strip is the schema-v11 ordered `cards[]` loop. Drag, earlier/later buttons, and
Alt+arrow keys all change that same order. Position numbers communicate sequence and
are hidden from assistive technology to keep card names concise. Move/remove controls
appear on hover or focus, and remain visible at narrow widths or on touch devices.

A card has one identity: Clock, its picture source, or its editable timer label.
A running timer additionally shows its countdown. Accessible move/remove names retain
kind context. Dwell is edited below and visualized in the rotation; do not repeat it
on every card. The final open slot adds a card through the existing menu.

The editor uses unboxed checkbox rows and grouped settings separated by space or a
rule. Its fields hold a 40rem measure, with number inputs capped at 15rem and selects
at 20rem. Labels should explain controls without a second sentence repeating them.
Gesture guidance, alert-timing explanation, and picture-source identifiers live in
plain native disclosures. Missing sources and source validation automatically open
their disclosure; one-time credentials and actionable errors remain visible. A timed
alert hold keeps its tap-to-dismiss caveat beside the control. The selected card's
title and Remove action lead the editor.

## Rotation

A 104px ring and its legend show proportional dwell, with the compact total in the
center. Thin arcs have no glow. The heading is **Rotation**, distinguishing pacing
from the **The loop** card chooser. Timed/manual pacing and the preview transport
remain beside that heading, wrapping on narrow screens.

The calculations still come from `loopSegments`, `loopSeconds`, `loopAdvance`, and
`loopDeadline`. Manual rotation uses equal arcs and no implied duration. Playback
advances the editor preview, and reduced motion disables automatic playback with a
visible explanation. Reordering remains in the card strip.

## State and accessibility

Settings is the single native modal for account, panels, display preferences, and
connection troubleshooting. The dialog owns focus and Escape; its save bar makes
saving reachable while the page behind it is inert. Device facts use plain rows.
Consequential failures remain visible in the workspace as well as Settings.

Save remains explicit and is blocked by validation, ownership, and in-flight work.
Disabled primary actions use a neutral fill. The adjacent message explains the state;
there is no decorative hatch. Validation stays attached to the field or card that owns
it, with an unclaimed-issue fallback.

Keyboard focus is a 2px `--act` outline with a 2px offset. Selection and carets use
`--act`; thin scrollbars use `--tile-3`. Short transitions communicate interaction.
`prefers-reduced-motion: reduce` collapses transition/animation durations and disables
loop playback. Both themes remain supported, including explicit dev-harness overrides.

## Verification harness

Run `VITE_DESKMATE_MOCK=1 bun run dev` from `companion/apps/deskmate`.
The harness supplies sample data without contacting hardware. Scenarios include
`default`, `offline`, `standalone`, `unowned`, `invalid`, `firstrun`, `empty`,
`carderror`, `picture`, `signedout`, `setup`, and `nopanels`; use `?scenario=…`.
Add `&theme=dark` or `&theme=light` to pin the scheme.
