# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

<!-- Tauri v2 desktop app on macOS, rendered in a WKWebView. Web design language,
     not AppKit. Not responsive-for-mobile: the shipped device class is one
     resizable desktop window (default 1060×740, min 360×560). -->

## Stack

Existing codebase answers this: React 19 + TypeScript 7 + Vite 8, Biome for
format/lint, Bun for tests, Tauri v2 shell over a Rust runtime. No CSS framework,
no component library, no state library — a single hand-authored `styles.css` and
`useState`/`useEffect`. **No web fonts are reachable**: the Tauri CSP is
`style-src 'self'; script-src 'self'` with no font host, so any typeface must be
bundled locally or come from the system stack.

## Users

Primary user today: Rodion, the product's author, configuring his own desk display.
Confirmed direction: this ships to other people later, so the settings window is
designed for a stranger's first ten minutes as well as the author's thousandth
session. Onboarding, empty states, and error copy stay first-class rather than
being traded away for density.

The user is at a Mac, usually with the physical display in view on the same desk.
They are configuring hardware they can see — the app is never the only feedback
channel, and the panel itself is the real output.

## Product Purpose

Deskmate is a small emissive AMOLED panel (368×448 physical, driven as a 448×368
landscape UI) that clips to a monitor and shows one card at a time — a clock, a focus
timer, or a server-produced picture. This desktop app is the
**authoring and ownership surface** for that hardware: it is where a person builds
the loop of cards the panel cycles through, decides what advances when,
reviews card failures, provisions the device onto WiFi and a server, and hands
ownership between the Mac and that server.

Success is that the display shows the right thing without the person thinking about
it, and that when something is wrong they can see which card, why, and what to do —
without connecting a console.

## Picture cards

A picture card is for a non-interactive face produced outside this repository and sent
to Deskmate as a PNG. Anybody who can produce the image can add the card with one line
of `curl`; it requires no manifest, SDK, or repository access.

## Positioning

Three facts a neighbouring "smart display companion" could not truthfully copy:

1. **The preview is not a mock.** For the two on-device kinds, `render_card_preview` sends
   host-built scenes through the firmware/LVGL renderer and returns exact-pixel PNG
   frames; the retired C templates survive only in `lvgl-sim` as the reference oracle,
   not as templates shipped on the device. A framebuffer diff harness proves the
   simulator and firmware agree pixel for pixel. What the app shows is what the panel
   will show.

   A picture is already the producer's complete 448×368 frame, so the stage states that
   the pushed source owns it instead of fabricating a second rendering path.
2. **Ownership is a single implementation with two tiers.** In `local` tier the Mac
   owns the device over USB; in `networked` tier a single-tenant server owns it
   through the same `RuntimeDevice` seam and the app becomes a configurator. Where
   settings are saved is a real, visible, consequential state — not a sync toggle.
3. **Every validation issue is claimed by a surface.** `issuesForCard`,
   `cardsContainerIssues`, and `unclaimedIssues` guarantee no issue can silently
   block Save with nothing highlighted. That invariant is already enforced in code
   and must survive any redesign.

## Operating Context

- macOS. The app is tray-resident: closing the window does not stop the background
  Rust runtime, which keeps owning the device and holding timers.
- The physical display is normally within arm's reach of the Mac, and in `local`
  tier is physically cabled to it over USB.
- Provisioning is **a cable operation by design** — WiFi and server credentials
  can only be written over USB, never over the network tunnel.
- A disconnected or standalone display is routine rather than exceptional: settings
  queue and sync later.
- The user edits a **draft**; nothing reaches the device until Save. Validation runs
  on the draft, debounced 180ms, in the Rust backend over IPC.

## Capabilities and Constraints

**Objects.** The app edits three card kinds (max 8): on-device `clock` and `pomodoro`,
plus server-side `picture`. Picture content is a durable frame pushed by an external
producer; it is frozen between pushes. Clock and Pomodoro are the only faces that keep
ticking on the device while the host is absent.
The window has **one loop** (since 2026-09-06): the document's active playlist. Schema
v9 carries `playlists[]` (max 8, max 8 entries each, one active); the app exposes
exactly one, never creates another, and round-trips any extra playlist an older file
holds untouched. A new card joins the loop as it is added; a card outside the loop is
shown in the same grid, flagged, with one action to join. Each card in the loop carries
an optional dwell that inherits the loop default. Advance is `manual` or
`timed`. Alerts exist on `pomodoro` (on timer finish), with a hold that is host-side
bookkeeping and never clears the panel.

**Surfaces in the current app.** A `TopBar` wordmark and Settings button;
`SettingsSheet` as the sole disclosure of state, for device ownership, pairing,
link/WiFi/IP/update state, timezone, mounting orientation, and start-at-login; first-run
and error notices; the loop grid (complication tiles in loop order, reordered in place,
with the add-card slot and its menu of on-device kinds plus Picture); the per-card
editor, which also edits the card's dwell and picture source; `DevicePreview`;
`LoopRing` (arc ∝ dwell, with a playhead, and the pacing control in its head);
card-local failures; and save controls in both the work column and
settings sheet.

**Hard constraints that outlive any visual direction.**
- Config schema is **v9** (`docs/config/v9.md`); wire protocol is **v1**.
  A redesign of this app must not require a schema or wire change.
- Canvas is a single clean 448×368 landscape. Orientation is `landscape` or
  `landscape-flipped` only — no portrait, and orientation is owned by this settings
  app, never by a device gesture.
- Clock faces carry no title chip and no eyebrow on the device. User-visible card
  identity is card-kind-first: `cardLabel(card)` returns the card kind's display name;
  the owner's `title` is
  a quiet second line that distinguishes cards of the same kind.
- Tauri CSP blocks every external host. All fonts, images, and scripts must be local.
- Secrets (WiFi passphrase, device token, admin token) are **write-only**: accepted
  by IPC, absent from every snapshot and response, and never displayed back.
- Save is blocked when ownership tier is unknown, when validation fails, or while a
  save is in flight — and the reason must always be legible.
- Undecided: whether the app ever gains a firmware-update UI beyond read-only status.
  Protocol-v1 `last_ota_error` exists on the wire and the server exposes it, but the
  app does not yet present it as an OTA-failure explanation.

## Brand Commitments

Name: **Deskmate**. Window title "Deskmate Settings". Bundle id `io.deskmate.companion`,
version 1.0.0. No logo asset exists — the current mark is the letter "D" in a box.
No committed palette or typeface survives the decision to replace the visual world.

Voice, as already written throughout the UI and worth preserving: plain, calm,
second-person, and specific about consequence — "Saved. It will sync when your
display reconnects", "The saved file was left untouched", "Write-only. It is cleared
after submission." It states what happened and what will happen next, never
exclaims, and never blames the user.

## Evidence on Hand

- Real, working exact-pixel device previews via IPC (`render_card_preview`) for Clock and
  Pomodoro, with Picture explicitly owned by its pushed source.
- A full typed IPC contract (`src/lib/types.ts`, cross-checked against Rust
  serialization in CI) — enough to build a faithful mock backend.
- Existing tests: `tests/components.test.tsx`, `useAppState.test.ts`,
  `configDraft.test.ts`, run under Bun.
- A live deployed server at `deskmate.rodi.one`, and physical hardware on the desk.
- **Absent, and must not be fabricated:** users other than the author, testimonials,
  install counts, pricing, any App Store presence, and an app UI explanation of
  `last_ota_error`.

## Product Principles

1. **The panel is the product; this window is the instrument.** The app's job is to
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
