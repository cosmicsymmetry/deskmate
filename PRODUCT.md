# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

<!-- Browser SPA served by the Rust server. Wide-screen-first, with narrow-width
     layouts for the same card-authoring work rather than a separate mobile UI. -->

## Stack

Existing codebase answers this: React 19 + TypeScript 7 + Vite 8, Biome for
format/lint, Bun for tests, and a Rust server that serves the built SPA from
`DESKMATE_WEB_DIR` and exposes its API over same-origin HTTP and SSE. No CSS
framework, no component library, no state library — a single hand-authored
`styles.css` and `useState`/`useEffect`. The server sets no CSP header. System
typefaces and the small inline-SVG icon set are deliberate: they require no asset
fetches and keep the visual language consistent.

## Users

Primary user today: Rodion, the product's author, configuring his own desk display.
Confirmed direction: this ships to other people later, so the companion is
designed for a stranger's first ten minutes as well as the author's thousandth
session. Onboarding, empty states, and error copy stay first-class rather than
being traded away for density.

The user is in a browser, usually with the physical display in view on the same desk.
They are configuring hardware they can see — the app is never the only feedback
channel, and the panel itself is the real output.

## Product Purpose

Deskmate is a small emissive AMOLED panel (368×448 physical, driven as a 448×368
landscape UI) that clips to a monitor and shows one card at a time — a clock, a focus
timer, or a server-produced picture. The web companion at
`https://deskmate.rodi.one/` is the
**authoring and ownership surface** for that hardware: it is where a person builds
the loop of cards the panel cycles through, decides what advances when,
reviews card failures, configures picture sources, and sees whether the server and
display accepted the result.

Success is that the display shows the right thing without the person thinking about
it, and that when something is wrong they can see which card, why, and what to do —
without connecting a console.

## Picture cards

A picture card is a complete, non-interactive frame. An external producer can push a
PNG, or the server can render one of its own data-backed faces. An external producer
needs only one line of `curl`; it requires no manifest, SDK, or repository access.

## Positioning

Two facts a neighbouring "smart display companion" could not truthfully copy:

1. **The preview is not a mock.** For the two on-device kinds, `render_card_preview`
   builds the same scene the server pushes to the panel and renders it through LVGL as
   an exact-pixel PNG. What the companion shows is what the panel will show.

   A picture is already the producer's complete 448×368 frame, so the stage states that
   the pushed source owns it instead of fabricating a second rendering path.
2. **Every validation issue is claimed by a surface.** `issuesForCard`,
   `cardsContainerIssues`, and `unclaimedIssues` guarantee no issue can silently
   block Save with nothing highlighted. That invariant is already enforced in code
   and must survive any redesign.

## Operating Context

- The server keeps the display updated whether or not the browser page is open.
- The physical display is normally within arm's reach of the browser running the
  companion.
- Provisioning and factory reset are **cable operations by design** and belong to
  `deskmate-cli`; the web companion does not expose them.
- A disconnected or standalone display is routine rather than exceptional: settings
  queue and sync later.
- The user edits a **draft**; nothing reaches the device until Save. Validation runs
  on the draft, debounced 180ms, through the Rust server's validation endpoint.

## Capabilities and Constraints

**Objects.** The app edits three card kinds (max 8): on-device `clock` and `pomodoro`,
plus server-side `picture`. Picture content is a durable frame pushed by an external
producer or rendered by the server; it is frozen between pushes. Clock and Pomodoro
are the only faces that keep ticking on the device between host pushes.
The companion has **one loop**: schema v10's ordered `cards[]` is the loop, with no
separate playlist. A new card joins the loop as it is added. Each card carries an
optional dwell that inherits the loop default. Advance is `manual` or `timed`. Alerts
exist on `pomodoro` (on timer finish), with a hold that is host-side bookkeeping and
never clears the panel.

**Surfaces in the current app.** A `TopBar` wordmark and Settings button;
`SettingsSheet` as the sole disclosure of state, for operator sign-in, device selection
and ownership, link/WiFi/IP/update state, timezone, mounting orientation, and panel brightness; first-run
and error notices; the loop grid (complication tiles in loop order, reordered in place,
with the add-card slot and its menu of on-device kinds, Picture, and server-listed
faces); the per-card editor, which also edits the card's dwell and picture source;
`DevicePreview`; `LoopRing` (arc ∝ dwell, with a playhead, and the pacing control in
its head); card-local failures; and save controls in both the work column and settings
sheet.

**Hard constraints that outlive any visual direction.**
- Config schema is **v11** (`docs/config/v11.md`); wire protocol is **v2**.
  A redesign of this app must not require a schema or wire change.
- Canvas is a single clean 448×368 landscape. Orientation is `landscape` or
  `landscape-flipped` only — no portrait, and orientation is owned by this settings
  app, never by a device gesture.
- Clock faces carry no title chip and no eyebrow on the device. `cardIdentity()` is the
  one visible name on the tile, ring legend, and editor: Clock for a clock, the source
  name for a picture, and the editable timer label for a Pomodoro.
- The companion fetches no fonts or icon libraries. `ui-rounded` uses the platform's
  rounded system face, and the seven icons are inline SVGs on one consistent grid.
- The admin token is **write-only**: it is traded for an `HttpOnly` operator session
  cookie, absent from snapshots and responses, and never displayed back.
- Save is blocked when ownership tier is unknown, when validation fails, or while a
  save is in flight — and the reason must always be legible.
- Undecided: whether the app ever gains a firmware-update UI beyond read-only status.
  Protocol-v2 `last_ota_error` exists on the wire and the server exposes it, but the
  app does not yet present it as an OTA-failure explanation.

## Brand Commitments

Name: **Deskmate**. Page title "Deskmate Settings", version 1.0.0. No logo asset
exists — the current mark is the letter "D" in a box. The committed palette and type
roles are defined in `DESIGN.md`.

Voice, as already written throughout the UI and worth preserving: plain, calm,
second-person, and specific about consequence — "Saved. It will sync when your
display reconnects", "The saved file was left untouched", "Write-only. It is cleared
after submission." It states what happened and what will happen next, never
exclaims, and never blames the user.

## Evidence on Hand

- Real, working exact-pixel device previews through `POST /v1/app/{id}/preview` for
  Clock and Pomodoro, with Picture explicitly owned by its pushed source.
- A full typed HTTP contract (`src/lib/types.ts`, cross-checked against Rust
  serialization in CI) — enough to build a faithful mock backend.
- Existing tests: `tests/components.test.tsx`, `useAppState.test.ts`,
  `configDraft.test.ts`, run under Bun.
- A live deployed server at `deskmate.rodi.one`, and physical hardware on the desk.
- **Absent, and must not be fabricated:** users other than the author, testimonials,
  install counts, pricing, and a companion UI explanation of `last_ota_error`.

## Product Principles

1. **The panel is the product; this companion is the instrument.** The app's job is to
   make the physical display right, then get out of the way.
2. **Show state, not reassurance.** Connection, ownership, card failures, and
   save destination are consequential facts. Name them precisely, including when
   they are bad.
3. **Every problem is attached to the thing that causes it.** An issue belongs next
   to the control that fixes it, never only in a banner at the top.
4. **Nothing reaches the hardware without an explicit Save**, and the app is always
   honest about where that save lands and whether it arrived.
5. **Degraded is a first-class state.** Disconnected, standalone, stale, and paused
   are normal operating modes, and the UI must stay useful and calm inside them.

## Accessibility & Inclusion

Established in code and non-negotiable: visible keyboard focus on every interactive
element (never removed), `role="alert"` / `aria-live` on validation, save, and
connection changes, `aria-invalid` on every failing field, drag-reorder that has a
full keyboard equivalent (⌥↑/⌥↓), `prefers-reduced-motion: reduce` disabling
automatic playback and transitions, and both `prefers-color-scheme` modes as
first-class peers rather than one being an afterthought. Body text meets 4.5:1 in
both modes.
