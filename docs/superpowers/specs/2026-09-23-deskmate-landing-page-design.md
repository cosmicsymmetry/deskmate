# Deskmate Landing Page — Design

**Track D.** Brainstormed with the owner on 2026-09-23. Approved before writing.

This spec covers the first shippable version of the Deskmate marketing site: what it
claims, how it is built, and what it deliberately does not say. It does not cover
content marketing, channels or launch — that is Track E, which waits on this.

## Scope and position on the board

Track D's line in `docs/roadmap.md` says research now, site waits on Track A for
sign-up and checkout. Track E waits on D "for somewhere to send people", so the first
version has to ship **before** A lands or E stays blocked indefinitely.

That constraint produced the first decision: **the page asks for an email address and
nothing else.** No price, no ship date, no deposit, no checkout. When A lands, the page
grows a real sign-up; when F lands, it grows a store. Neither is in this spec.

The site is a separate repository and deploys on its own. It touches the Deskmate tree
in exactly one place — a dev-only frame-export example — and no production code. See
"Frame production".

## Decisions taken with the owner

| Decision | Choice |
|---|---|
| What the page asks a visitor to do | Waitlist email only |
| The hook | Deskmate is an **interactive desk accessory** |
| How interactivity is demonstrated | An interactive panel in the browser, driven by pixel-honest frames |
| Visual world | Its own — not `DESIGN.md`'s Modular Face |
| Which world | **The Loop**: the page is the card loop |
| Open source / self-host claim | Omitted from the first version |
| Domain | Deferred; the owner will buy one later |

### Why the hook is not an overclaim

The owner's phrase was "interactive desk accessory", which is a category claim: every
other object of this kind shows you something, and this one answers when you touch it.
What is actually real behind it, today:

- `tap_action` is in config schema v10 (`docs/config/v10.md:152`), a closed tagged
  object whose `start-pause` and `reset` kinds are valid only on a pomodoro card
  (`:116`).
- Protocol v2 carries `Tap` events with `StartPause` and `Reset` actions
  (`docs/protocol/v2.md:262-270`), and a horizontal swipe selects the next or previous
  card (`:328-331`).
- The device applies **bounded optimistic visual feedback immediately** on a
  start/pause/reset tap, and the next complete `PushTimer` replaces it
  (`docs/protocol/v2.md:340-343`). The panel does not wait for the server.

Not real yet, and therefore staged rather than claimed: a tap reaching a plugin that
answers with a new picture (Track C1, not started) and instant on-device responses for
cards that need them (Track C2, waits on C1). Tap latency has never been measured on the
board; it is on the owed-verification list in `CLAUDE.md`. The page states the staging
openly in section 07 rather than implying all of it works now.

## The mechanic

The page has one persistent element: **the panel**, `position: sticky`, held at eye
level while sections scroll past it. Each section changes the face the panel shows. A
ring in a corner fills with scroll progress, segmented per section, and closes at the
waitlist — the same arc-proportional-to-dwell idea the product runs on.

Three rules the implementation may not trade away:

1. **Scroll is native.** No wheel interception, no scroll hijacking, no snap that fights
   a flick, no scrollytelling library. The owner accepted The Loop over two safer worlds
   knowing this is where it usually goes wrong; a page that fights the wheel is a failed
   build of this spec, not a variation on it.
2. **The page works with JavaScript off.** Sections stack, each with its own still
   frame, and the page reads top to bottom as ordinary prose. The panel is then a static
   image, not a broken control.
3. **The gesture is the product's gesture.** The panel accepts a tap (toggles the timer)
   and a horizontal drag (advances the loop), because that is exactly what the hardware
   accepts. Both have keyboard equivalents.

Section-to-face mapping is `IntersectionObserver`. The ring uses CSS
`animation-timeline: scroll()` where supported, with a `requestAnimationFrame` fallback
where not.

**The loop holds at most eight cards (`PRODUCT.md`, "Objects"). So does this page.**

## The eight sections

Copy below is direction, not final text. The voice is the one `PRODUCT.md` already
commits to and the UI already uses: plain, calm, second person, specific about
consequence, never exclaiming, never blaming the reader.

| # | Section | Panel shows | The claim |
|---|---|---|---|
| 01 | Hero | Clock | *Tap it. It answers.* A small emissive panel that clips to your monitor. Email field here as well as at the end. |
| 02 | Focus timer | 25:00 paused → the visitor taps and it counts down | The timer moves the instant your finger lands; the panel does not wait for the server, and the host reconciles afterwards. |
| 03 | The loop | Faces cycling; drag to advance | Your cards, in the order you set, each with its own dwell. |
| 04 | Server-drawn faces | Real weather, Hacker News, RSS and token frames | Drawn on the server, pushed to the panel as a finished picture. |
| 05 | Anything else | A pushed picture | One literal line of `curl`. Anything that can POST a PNG owns a card. |
| 06 | A real object | — (photographs) | Scale, clipped to a monitor, in a room. Nothing on this page is AI-generated. |
| 07 | Where it is going | Tap → a plugin answers | The three stages of interaction, named honestly: the timer answers on-device today; next a tap reaches a plugin; later, instant on-device responses. |
| 08 | Waitlist | Ring closes | A working prototype on one desk. No price yet, no ship date yet. Leave an email and you will hear when that changes. |

FAQ and footer sit **after** the ring closes, outside the loop. The FAQ follows the
reference's shape: numbered, blunt, and answering the questions a stranger actually has
— what it does, whether it is finished, what stays private, how the waitlist is used.

Section 05's `curl` line demonstrates how a picture card is fed. In this version it
links nowhere, because the repository is private (see "Open source").

## The panel component

A `<button>`, not a `<div>`, wrapping a frame stack at the device's real logical size of
448×368 (`crates/lvgl-sim/src/lib.rs:26-27`) with its true corner radius and bezel.

- **Tap / Enter / Space** toggles the timer between paused and running.
- **Horizontal drag, or ←/→** advances to the previous or next card, matching the
  device's swipe.
- Focus is always visible and never removed.
- Under `prefers-reduced-motion: reduce`, nothing plays automatically and the countdown
  does not animate; a tap still switches between the paused and running states.
- With JavaScript off, a single `<img>` of the section's face.

### Frame production

**Corrected 2026-09-23, before any plan was written.** An earlier draft of this spec
claimed `POST /v1/app/{id}/preview` would render any timer state on request. It will
not. `PreviewRequest` (`companion/crates/server/src/app_api/mod.rs:726`) carries only
`card_id`; the `duration_seconds` / `remaining_seconds` / `running` fields that
`preview_timer` (`:839`) reads come from `snapshot.card_data`, which the runtime
assembles from its own `latest_fields` (`crates/app-core/src/runtime/mod.rs:583`) —
device events and the pomodoro engine. The HTTP surface renders whatever state the
runtime is actually in. It cannot be asked for "24:37, running".

The knob exists one layer down, and it is explicit:
`lvgl_sim::scene::SceneRenderRequest` (`crates/lvgl-sim/src/scene.rs:153`) takes
`timer: Option<SceneTimer>`, and `SceneTimer` is exactly `{ total_ms, remaining_ms,
running }`. `Simulator::render_scene_png` (`:246`) returns the encoded PNG. The scene
itself comes from `app_core::preview_card_scene(&config, &card_id, &fields)`
(`crates/app-core/src/runtime/scene.rs:86`) — the same builder the server's preview
calls, so the frames stay the ones the panel would be sent.

So the exporter is **a dev-only example in `app-core`**:
`companion/crates/app-core/examples/frame_export.rs`. That crate already depends on
`lvgl-sim` for exactly this kind of tool, and its `Cargo.toml:21-24` documents the edge
as dev-only. `examples/scene_panel_check.rs` is the established pattern, down to the
`cargo run -p app-core --example ...` invocation in its module doc.

**This costs Track D one worktree and one pull request in the Deskmate repo**, against
the roadmap's expectation of a track with no code worktree. It is one file, dev-only, no
production code, no schema and no wire — but it is not zero, and the plan schedules it
as its own task with the workspace gates run.

Planned pack: 25:00 → 24:00 at one frame per second — the cadence the device itself
ticks at — plus the paused state, so roughly 61 frames for the timer. Single frames for
the other faces. The weather, Hacker News, RSS and token faces come from `bun run dump`
in `companion/faces/` and need no server and no exporter at all.

`Simulator` allows exactly one instance per process (`crates/lvgl-sim/src/lib.rs`,
`SimError::AlreadyClaimed`), so the exporter renders its frames sequentially in a single
run rather than in parallel.

**Unmeasured, and the plan measures it before committing to this approach:** the encoded
size of a 448×368 mostly-black emissive frame as WebP, and therefore the pack's total
weight. The expectation is comfortably under 1 MB with below-the-fold frames lazy
loaded. If it is not, the fallbacks in order are a lower frame rate for the ring-only
portion, a shorter countdown window, or a video element for the countdown with the
tappable states kept as frames.

### What the frames do and do not prove

They are honest about **rendering**: these are the pixels the scene renderer produces,
from the same C the device runs, and there is exactly one renderer to disagree with.
They are not photographs of a panel. `CLAUDE.md`'s standing trap — the frame store is
not the panel — applies to marketing as much as to engineering. The page may claim "this
is what the screen draws". The photographs in section 06 carry the separate claim that
the object exists and works.

## Plumbing

- **Astro**, because the page is static content plus exactly one interactive island.
  The owner asked on 2026-09-24 whether the site should be written in Rust; the answer
  is no, and it was settled rather than left open. A WASM framework would ship hundreds
  of kilobytes to swap an image source and would break the JavaScript-off requirement,
  since WASM needs JavaScript to boot. A Rust static site generator such as Zola is
  sound on its own terms but is a *new* toolchain here — the tree already runs Bun,
  TypeScript and Biome for `apps/deskmate` and `faces` — and the interactive panel and
  the waitlist function would still be JavaScript. Serving the page from the existing
  Rust server would couple marketing uptime to the homelab and make every copy edit a
  deploy, which is the coupling `DESKMATE_WEB_DIR` exists to avoid. Rust does own the
  part where it matters: the frame exporter, which links the firmware's own scene
  interpreter. Astro was kept over plain hand-authored HTML on the owner's direction.
- **Its own repository**, `~/dev/deskmate-site`, deploying independently of the server.
- **Cloudflare Pages**, which is free, static, already where the DNS lives, and keeps a
  launch-day marketing site off a home VM.
- **Email capture** to a Pages Function writing Cloudflare D1. No third party holds the
  list. Which tool eventually mails it is Track E's decision, not this spec's.
- **Cloudflare Web Analytics** — free, cookieless, no consent banner.
- A `*.pages.dev` URL serves until the owner buys a domain. Track E should not launch on
  one.

## Accessibility

`PRODUCT.md` treats accessibility as non-negotiable in the app, and the marketing site
inherits that standard rather than relaxing it:

- Visible keyboard focus on every interactive element, never removed.
- Every gesture has a keyboard equivalent.
- `prefers-reduced-motion: reduce` disables automatic playback and transitions.
- Both `prefers-color-scheme` modes are first-class. The Loop is a dark world by nature;
  the light scheme is designed, not inverted.
- Body text meets 4.5:1 in both modes.
- Colour never carries a state alone.

## What the page must not say

`PRODUCT.md` lists what is absent and must not be fabricated: users other than the
author, testimonials, install counts, and pricing. That list governs the marketing site
at least as strictly as it governs the app. Also excluded from this version:

- Any ship date, delivery window or production claim.
- Any open-source or self-host claim, per the decision below.
- Any rendered or AI-generated image presented as a photograph.

## Open source

`github.com/cosmicsymmetry/deskmate` is private as of 2026-09-23. The roadmap decides
the firmware stays open source, but today a self-host claim would link nowhere, so the
owner chose to **omit it from the first version**. Section 05 keeps its `curl`
demonstration without a link.

Making the repository public is a separate decision with a real review job attached —
the tree carries deploy scripts, hardware notes and operational detail written for a
private audience. When that happens, the open-source and self-host material is added as
a ninth block after the FAQ, not as a ninth card: the loop's eight-card limit is a
product fact the page is deliberately mirroring.

## Dependencies

| Needed | From | Blocks |
|---|---|---|
| Photographs of the real panel, not generated | The owner | Section 06 |
| A domain | The owner, later | Track E's launch, not this build |
| A worktree and a PR for the frame-export example | This track | The hero |
| Whether the repository goes public | The owner, undecided | The open-source block, which is out of scope here |

The only existing photograph in the tree is
`docs/hardware/media/2026-09-06-task7/claude-limits-on-panel.jpg` — a real panel on a
wooden desk showing Claude usage limits. It is documentation photography and is good
enough to prove the object exists, but section 06 wants better.

## Risks

1. **The Loop is the risky world and the owner chose it knowingly.** The failure mode is
   a page that feels clever rather than clear, or one that fights the scroll. The three
   rules under "The mechanic" exist to prevent exactly that and are not negotiable
   during implementation.
2. **Frame pack weight is unmeasured.** Mitigation and fallbacks are above.
3. **A sticky panel is hardest on a narrow screen.** The mobile layout is designed
   explicitly, not derived by letting the desktop one reflow: the panel shrinks and
   sticks to the top, and the ring moves with it.
4. **Section 06 collapses without real photographs**, and faking them would break the
   one claim the page makes that a competitor cannot copy.

## Out of scope

Sign-up, sign-in, accounts, checkout, pricing, the store, plugin directory pages,
documentation, and anything that mails the waitlist. Those belong to Tracks A, B, E and
F.
