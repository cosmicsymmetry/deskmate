# Design — The Modular Face

<!-- impeccable:design-world modular-face -->

**Applies to:** `companion/apps/deskmate/src/**`, principally `src/styles.css`.
**Status:** built and in the tree. Supersedes `docs/design/companion-visual-language.md`
("the lit panel"), which described a proposal that was never approved and is now
recorded there as superseded.
**Direction seed:** ef112502. The world was pinned by the owner, not dealt by the roll.

## Thesis

Deskmate's hardware is a small emissive rounded panel that shows one glanceable thing
at a time. That is, structurally, a watch face clipped to a monitor. So the app that
composes it is built from the parts a watch face is built from: **complications** on a
grid, each owning exactly one fact; **rings** wherever something has a real proportion;
**huge rounded numerals**.

It refuses the arrangement this category always ships — a left source list, a stack of
soft grey cards, one blue primary button — because a face has no navigation. Everything
is already on it.

The feeling the owner asked for, being organised and visibly productive, has a
mechanism rather than a mood: watchOS earned "closing your rings" by making completion
**visible, proportional and physical**. The rotation gets the same treatment. The loop
is a ring, each card's dwell is an arc of it, and it closes.

## Colour

Two grounds, five roles. Both schemes are first-class; the light one is designed, not
inverted.

| Token | Light | Dark | Role |
|---|---|---|---|
| `--ground` | `#ECECF0` | `#000000` | The window. Cool grey, deliberately not cream. |
| `--tile` | `#FFFFFF` | `#131316` | Anything you can touch: tiles, inputs, controls. |
| `--tile-2` / `--tile-3` | `#F4F4F7` / `#E6E6EB` | `#1D1D21` / `#2A2A2F` | Nested control surfaces and hover. |
| `--ink` / `--ink-2` / `--ink-3` | `#1D1D1F` / `#62626B` / `#6A6A72` | `#F5F5F7` / `#A0A0A8` / `#84848D` | Primary, secondary, label. |
| `--stage` | `#000000` | `#000000` | **Black in both schemes.** |

The five chromatic roles, each with **exactly one meaning** and no decorative use:

| Token | Light | Dark | Means |
|---|---|---|---|
| `--live` | `#E5004C` | `#FA114F` | On the panel right now; a running timer. |
| `--good` | `#157B42` | `#30D158` | Reachable, owned, fresh, healthy. |
| `--act` | `#0062E0` | `#0A84FF` | Interactive only: focus, selection, the primary action. |
| `--warn` | `#975D00` | `#FF9F0A` | Stale, pending, paused, needs a look. |
| `--bad` | `#D70015` | `#FF453A` | Fault, refused, destructive. |

**Every one of these was measured, not eyeballed.** `--ink-3` is the label role at
0.625rem uppercase, which is small text and therefore owes 4.5:1 — it is checked against
both `--ground` and `--tile` in each scheme, and `--good` and `--warn` are checked the
same way because they colour a state word at 0.9rem. The worst small-text pair
in the built system is 4.52:1. The light values are darker than the platform's own
because macOS system green and orange do not clear 4.5:1 on white at this size.

**Colour is never the sole carrier of a state.** Every state prints a word that says the
same thing its colour does ("Connected", "stale", "Ownership unavailable"), so the window
survives a greyscale screenshot and a colour-blind reader alike.

`--arc-a` / `--arc-b` are a **ramp, not states**. Only the loop ring uses them, and only
to keep neighbouring arcs tellable apart. Which arc is live is said by luminance and
stroke weight, never by hue.

### Two rules that outrank convenience

1. **`--stage` is black in both schemes.** The preview shows an AMOLED panel whose unlit
   pixels emit nothing. Drawing it on a light ground in light mode would be a picture of
   a different device.
2. **The stage renders at an integer scale, always.** `.stage__frame` reserves the box and
   `.stage__screen` is exactly 448×368; below 540px it takes `transform: scale(0.5)`, a
   whole fraction, so `image-rendering: pixelated` never resamples unevenly. The caption
   under it claims exact pixels, and the exact-pixel preview is a product constraint —
   a `max-width` that rendered a 448px source at 452px made that caption a lie.
3. **The bloom under the stage is scheme-aware.** On black it is the panel's own light
   falling on the desk (`--stage-bloom`, tinted with `--live`). On a light ground that
   same glow reads as a lavender haze, so light mode gets a neutral, tighter shadow. It
   is the only shadow in the design; nothing else is elevated by shadow.

## Type

Scale: `h1` 1.5rem, `h2` 1.15rem, body 0.78–0.8rem, `.tile-label` 0.625rem. The two rail
heroes — the ring total and a card tile's live value — carry 1.75rem, because the thesis
promises huge numerals and a design whose ceiling is 23px has not delivered them.

No web fonts, and this is a deliberate choice rather than a constraint absorbed. The
Tauri CSP is `style-src 'self'` with no font host, and the pinned world's real typeface
is Apple's own.

| Role | Stack | Use |
|---|---|---|
| UI | `ui-sans-serif, -apple-system, system-ui, …` | All prose and controls. |
| **Numerals** | `--round`: `ui-rounded, "SF Pro Rounded", …` with `tabular-nums` | **Every time, duration, count and index.** |
| Label | UI stack, `0.625rem`, `700`, `0.085em`, uppercase, `--ink-3` | `.tile-label`. |

`ui-rounded` resolves to **SF Pro Rounded** on macOS, and this is a deliberate exception
to the usual rule that an own-world page sources and self-hosts its display face. The
reason is *not* that the CSP forbids alternatives — `default-src 'self'` would happily
serve a bundled `woff2` from the app's own assets. The reason is that SF Pro Rounded
**is the pinned world's own face**, on the only platform this app ships to, and no
redistributable equivalent is a better likeness of it than the real thing. There is also
no separate display face here: this is a numeral role, not headline lettering.
Numerals are tabular because this product is about time: digits that jitter as they tick
make a worse instrument than digits that hold their columns.

**`.tile-label` names a reading, never a heading.** A label stacked above an `h2` is
banned — the heading carries its own weight. The label survives only where it is the
only text naming a value ("LINK", "ADD A CARD", "WORKDAY").

## Composition

```
┌─────────────────────────────────────────────────────────────┐
│ ● Deskmate                                     ⚙ Settings   │  chrome
├──────────────────────────┬──────────────────────────────────┤
│ ┌──────────────────────┐ │  The loop      drag · ⌥ ← →  3/8 │
│ │                      │ │  ┌────────┐┌────────┐┌────────┐  │
│ │        17:24         │ │  │ 17:24  ││  18°   ││   3    │  │  live tiles,
│ │   Wednesday 19 Aug   │ │  │ Desk   ││Outside ││Headline│  │  in loop order
│ │                      │ │  └────────┘└────────┘└────────┘  │
│ └──────────────────────┘ │  ┌ + Add a card ┐                 │  the slot
│  THE LOOP [Timed]Manual  │  ─────────────────────────────   │
│           50 s   ▷ Play  │  Digital clock         Remove    │
│         ╭─────╮          │  Name  [ Desk               ]    │
│        │ 2:30 │          │  Show seconds  [ ]               │
│         ╰─────╯          │  Stays on the panel for [     ]  │
│  ● Desk            50s   │                                  │
│  ● Outside         50s   │                                  │
├──────────────────────────┴──────────────────────────────────┤
│ Everything is up to date                    [Save to server] │  save bar
└─────────────────────────────────────────────────────────────┘
```

- **The rail is the face.** What the panel shows, what the loop looks like, whether the
  data behind it is fresh. It stays put while the work column scrolls, because every
  edit in that column is aimed at it.
- **The work column is one surface.** Sections are bare regions separated by space and a
  hairline. **There is no panel-card layer** — a card inside a card is always wrong, and
  only the interactive atoms (tiles, inputs, controls) are elevated off the ground.
- **The panel identifies itself.** No label above it, no resolution beside it, no caption
  under it. It is the one object in the window that looks exactly like the thing it
  represents, so anything printed around it described what you could already see.
- **Ownership and provisioning are behind the one door.** Pairing is a first-run task and
  a troubleshooting task; it is not something you look at while arranging cards. It lives
  in a modal `<dialog>` reached from the chrome, with the device's link, Wi-Fi, address
  and update state beside it. When one of those turns bad the button carries a dot and
  the work column carries the sentence explaining it — so the sheet is a place to *go*,
  never a place where news can hide.
- **The grid is the loop.** (2026-09-06, on owner direction.) There is no card library
  and no playlist anywhere in the window: the complication tiles in the work column *are*
  the loop, in loop order, and they are where the order changes — drag a tile, use the
  earlier / later buttons that appear on hover, or ⌥ ← → on a focused tile. Adding a card
  is one dashed slot at the end of the grid; the new card joins the loop at once. A tile
  owns one fact, so dwell is never printed on it: the ring draws the proportion and the
  card's editor edits it ("Stays on the panel for"). A card an older file left outside
  the loop trails the others in the same grid, flagged `not in loop` (and `alerts` when
  it still can), with one action, "Add to loop" — nothing plays that is not listed, and
  nothing listed is silently inert. Schema v6 still carries `playlists[]` underneath;
  the window exposes exactly one and never creates another.
- **Exactly one disclosure for state, and one menu.** No tabs, no accordions, no drawer
  for anything you touch more than twice. Everything the design is about — cards, the
  loop, the panel — is on one surface, focused by luminance. This is what keeps the
  redesign from being slower than what it replaced. The add-card slot's menu
  (2026-09-06) is the one menu: it holds choices, never state, so nothing can hide in
  it — and it exists because a plugin registry that grows must never reshape the
  window. When that list is long enough to want search, it becomes a picker window.
- **One card, one name — and the name says what the card is.** A card is identified by
  its template on every surface: loop tile, ring legend, editor heading, the add-card
  menu. The owner's own title rides beside it in quiet type, and is dropped
  rather than repeated when none was typed. Two names for one object is the failure this
  rule prevents; a label that teaches a first-time reader nothing ("Outside", "Desk") is
  the failure it prevents second.
- **A status bar is not information.** Four permanent complications reading Connected /
  Server / -54 dBm / Live answered questions asked twice in a device's life and charged
  every other session for it. State that is nominal 99% of the time is noise; state worth
  a sentence gets one, in the column you are already reading.

Breakpoints: two columns above 940px; below it the regions flow at natural height and
`.face` scrolls. At 430px the card grid drops to two columns and hover-revealed controls
become permanently visible.

## Signature: the loop ring

`src/components/LoopRing.tsx`. A 124px ring paired side by side with its legend, one arc
per card in the loop, sweep proportional to that card's resolved dwell, the
on-panel entry at full luminance with a marker riding its start, and the loop total in
the centre at 1.75rem.

The centre prints a compact `1:50`, not `formatDuration`'s "1 min 50 s": the prose form
is right in a sentence and wrong inside a ring, where it wraps at hero size. The full
phrasing still reaches assistive technology through the ring's `aria-label`.

It replaces the previous filmstrip and keeps the same truth — a card's share of the loop,
which no other control shows — in the form the rest of this world is built from. All the
arithmetic still comes from the pure helpers in `src/lib/configDraft.ts`
(`filmstripSegments`, `loopSeconds`, `filmstripAdvance`, `filmstripDeadline`), so the ring
and any other consumer of loop length cannot drift apart.

Two decisions worth keeping:

- **A manual loop gets equal arcs.** With no dwell there is no proportion to encode, and
  a ring implying one would be inventing it.
- **The ring displays; the legend beside it selects.** Reordering lives on the grid in
  the work column (2026-09-06; it used to be here), so one order has exactly one place
  to change it. Dragging arcs around a circle still has no keyboard equivalent worth
  shipping.
- **Arc colour spreads across the ramp**, `index / (count - 1)`, never `index % 4`. A
  loop holds up to eight cards, and two arcs sharing a colour would break the only
  mapping there is from an arc back to its name.
- **The whole ring, its legend and its transport fit above the save bar at 1060×740**,
  the app's real default window. The transport lives in the loop's head row for exactly
  this reason, and so does the pacing control (Timed / Manual and the default dwell),
  which replaced the "TIMED LOOP" label rather than adding to the row. Anything added
  here has to pay for itself out of that budget.

## Icons

`src/components/Icon.tsx`. One 16px grid, one 1.75 stroke, round caps and joins, drawn.
No icon library is reachable under the CSP, and the habit this replaces — borrowing `×`,
`↑`, `✓`, `⠿` as icons — hands their weight and alignment to whatever font resolves.

## Motion

Two curves, both without overshoot: `--ease: cubic-bezier(0.32,0.72,0,1)` and
`--spring: cubic-bezier(0.16,1,0.3,1)`. Things settle; they do not bounce. An overshoot
on a press or a ring reads as tacky rather than physical.

`prefers-reduced-motion: reduce` collapses every duration to 0.001ms and stops the loop
ring's automatic playback outright, with the reason stated in the UI rather than the
control silently doing nothing.

## Browser surfaces

The parts nobody draws still carry the design: `::selection` is tinted from `--act`,
`caret-color` is `--act`, scrollbars are thin and coloured from `--tile-3`, focus rings
are a 2px `--act` outline with a 2px offset and are never removed.

## States

Every one of these has a designed treatment, and each is reachable in the dev harness
(`VITE_DESKMATE_MOCK=1 bun run dev`, then `?scenario=…`): `default`, `offline`,
`standalone`, `local`, `unowned`, `invalid`, `firstrun`, `empty`, `carderror`. Add
`&theme=dark` or `&theme=light` to pin the scheme.

A **barred primary action** is drawn as barred — a diagonal hatch on `Save` says a
condition is holding it — rather than merely faded. Quiet buttons keep plain reduced
opacity, because four hatched controls at once is noise instead of a signal.

## What this design will not do

- Reintroduce a kicker or eyebrow above a heading.
- Put a card inside a card.
- Use `--live`, `--good`, `--act`, `--warn` or `--bad` decoratively, or let any of them
  carry a state that no word also carries.
- Give the ring a proportion that is not real.
- Hide a state behind a tab, accordion or drawer. The settings sheet is the one modal in
  the product and it holds a *task*, not news: anything the sheet knows that is going
  wrong is also said in the open, on the button and in the work column.
- Print a fact the user cannot act on, or that the thing beside it already shows.
- Identify a card by the owner's title alone. The title disambiguates; it does not name.
- Add a permanent status band. State earns its place by being abnormal.
- Add a shadow anywhere except under the stage.
