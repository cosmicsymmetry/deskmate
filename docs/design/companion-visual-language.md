# Deskmate companion — visual language

**Date:** 2026-08-06
**Status:** Proposed. Authored during an unattended session; needs Rodion's approval.
**Applies to:** `companion/apps/deskmate/src/**` and `src/styles.css`.

## Thesis: the lit panel

Deskmate is a small emissive rectangle clipped to the top of a monitor. In the room it is
the only thing giving off light. The settings window should behave like the surround: matte,
quiet, unlit — so the one thing that glows is the preview of the panel itself.

This inverts the usual settings-app hierarchy, where the chrome is bright and the preview is
an inset thumbnail. Here light comes *from* the device outward and everything else recedes.
The inversion is derived from a real material property of the hardware — an AMOLED's unlit
pixels emit nothing, which is why the device reads as a floating pane of light — not from a
mood board.

**Consequence, and it is a hard rule:** `--panel-black` is used **only** inside the device
preview screen. No other surface in the application may use it. If the chrome ever goes as
dark as the panel, the idea is dead.

## Color

Chrome is strictly achromatic warm graphite. Chroma is reserved for emission and state.

| Token | Value | Role |
|---|---|---|
| `--vellum` | `#E8E6E1` | Warm paper. Light-mode surround. Slightly warm so it does not compete with a bright monitor. |
| `--graphite` | `#1C1B1A` | Warm near-black. Dark-mode surround, and the preview bezel in both modes. |
| `--panel-black` | `#050505` | True AMOLED black. **Preview screen only.** |
| `--emit` | `#F2C14E` | Warm amber. The *lit* state: active card, playhead, preview type. |
| `--signal` | `#4E86F2` | Cool blue. Interactive affordances only: focus rings, selection, links. |
| `--alarm` | `#C4553D` | Muted brick. Alert and error state only. |

Derived neutrals: `--ink`, `--muted`, `--line` (hairline, never heavier than 1px).

**Why two chromatic roles rather than one accent.** Keeping "emissive" (amber) and
"interactive" (blue) separate is what stops this from collapsing into the single-accent dark
theme that every dashboard ships. Amber is never clickable; blue never glows.

## Type

No web fonts. The Tauri CSP blocks external hosts and nothing is bundled, so personality has
to come from *how* the system stack is used. That constraint is honest and the design leans
into it rather than smuggling in a display face.

| Role | Stack | Use |
|---|---|---|
| Body / UI | `ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif` | Everything prose. |
| **Numerals** | `ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace` with `font-variant-numeric: tabular-nums` | **Every duration, time and count**: dwell seconds, lead minutes, hold, loop length, the preview clock, card indices. |
| Label | Body stack, `0.75rem`, `letter-spacing: 0.08em`, uppercase, `--muted` | Section names only. Never decorative. |

**Why tabular monospace numerals.** This product is about time. A ribbon whose numbers jitter
as they tick is a worse instrument than one whose digits hold their columns. This is the
deliberate pairing, and it is the reason to reach past a single system face.

## Layout

Two columns on wide, stacked below ~900px. Left is the loop (the card list). Right is the
panel (preview, ribbon, preferences). The preview column is sticky — it is what you are
editing toward.

```
┌──────────────────────────────────────────────────────┐
│ Deskmate            ● Connected · /dev/cu.usb…       │  quiet status bar
├───────────────────────────┬──────────────────────────┤
│ IN ROTATION               │      ┌──────────────┐    │
│ ╭───────────────────────╮ │      │  09:41       │    │  the only lit thing
│ │⠿ 1  Desk         20s ›│ │      │  Wednesday   │    │
│ ╰───────────────────────╯ │      └──────────────┘    │
│ ╭───────────────────────╮ │         ▔▔▔▔▔ USB        │
│ │⠿ 2  Up next      45s ›│ │                          │
│ ╰───────────────────────╯ │  ╔══════════════════╗    │  SIGNATURE
│                           │  ║▓▓▓▓│░░░░░░░│▒▒▒▒▒║    │  loop ribbon
│ ALERTS AND MUTED          │  ╚══▲═══════════════╝    │  width = dwell
│ ╭───────────────────────╮ │    playhead   1 min 5 s  │
│ │   Focus      alert  ›│ │                          │
│ ╰───────────────────────╯ │  Timezone    Mounting    │
└───────────────────────────┴──────────────────────────┘
```

## Signature: the loop ribbon

One horizontal bar beneath the device mock. **Each in-rotation card is a segment whose width
is proportional to its dwell time.** A playhead sweeps left to right in real time while
playing and wraps at the end. Dragging a segment reorders the rotation. Total loop length is
printed at the right end in tabular numerals.

This is the one place to spend boldness. It earns its place because width encodes duration —
true information the user cannot otherwise see — and because it replaces two separate
controls that previously showed the same ordering twice (the preview dots and the screen
arranger).

Cards that are `alert-only` or `off` never appear in the ribbon. They have no position in the
loop, and giving them one would be a lie.

## Restraint

Everything that is not the preview or the ribbon stays quiet: hairline borders, no gradients,
no shadows except a single soft amber glow beneath the preview screen (the emission itself),
6px radii, generous whitespace.

**Removed on purpose:** the previous `html` radial teal gradient, the drop shadows on panels,
and the numbered wizard step labels (`1 · Widgets`, `3 · Screens`) — they promised a linear
flow the layout never had. Numbering survives in exactly one place, the rotation list, where
order genuinely carries information.

## Quality floor

- Responsive down to a narrow window; the two columns stack, the ribbon scrolls horizontally
  inside its own container and never makes the page scroll sideways.
- Visible keyboard focus everywhere, using `--signal`, never removed.
- `prefers-reduced-motion: reduce` disables the playhead sweep and all transitions.
- Both `prefers-color-scheme` modes are first-class; dark is not an afterthought.
- Contrast: body text meets 4.5:1 in both modes; `--emit` on `--panel-black` is checked for
  the preview's own type.

## Self-critique against generic defaults

Current AI-generated design clusters around three looks. This direction was checked against
each:

- *Cream background, high-contrast serif display, terracotta accent* — no serif display face,
  no terracotta, and the surround is a cooler warm-grey than the usual cream.
- *Near-black with a single acid accent* — the default mode is light, dark is a peer rather
  than the premise, there are two chromatic roles deliberately kept apart, and true black is
  quarantined to the preview.
- *Broadsheet hairlines, zero radius, dense columns* — soft radii, two columns, generous
  space.

The parts that are specific to this brief rather than transferable to any settings app: the
unlit-chrome/emissive-preview inversion, the quarantine of `--panel-black`, the amber/blue
split between emission and interaction, and the width-encodes-dwell ribbon.
