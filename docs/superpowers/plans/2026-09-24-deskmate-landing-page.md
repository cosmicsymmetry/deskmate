# Deskmate Landing Page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the first version of the Deskmate marketing site — a waitlist page whose
hero is a tappable, pixel-honest replica of the panel, built in The Loop world.

**Architecture:** Two repositories. The Deskmate repo gains exactly one dev-only file, a
frame-export example that renders card faces through the firmware's own scene
interpreter. The site is a separate Astro project that replays those frames in a sticky
panel while native scroll advances the loop, and captures emails through a Cloudflare
Pages Function backed by D1.

**Tech Stack:** Rust (`app-core` example, `lvgl-sim`), Astro 5, TypeScript, Bun,
Biome, Playwright, Cloudflare Pages + D1, `cwebp`.

**Spec:** `docs/superpowers/specs/2026-09-23-deskmate-landing-page-design.md`

## Global Constraints

- **Scroll is native.** No wheel interception, no scroll hijacking, no snap that fights
  a flick, no scrollytelling library. A page that fights the wheel is a failed build of
  this spec, not a variation on it.
- **The page works with JavaScript off.** Sections stack, each with its own still frame,
  and the page reads top to bottom as ordinary prose.
- **The gesture is the product's gesture.** Tap toggles the timer; horizontal drag
  advances the loop. Both have keyboard equivalents.
- **The loop holds at most eight cards. So does this page.**
- **The page must not say:** users other than the author, testimonials, install counts,
  pricing, any ship date or delivery window, any open-source or self-host claim, or
  present any rendered or generated image as a photograph.
- **Voice:** plain, calm, second person, specific about consequence. Never exclaims,
  never blames the reader.
- **Accessibility:** visible keyboard focus never removed; every gesture has a keyboard
  equivalent; `prefers-reduced-motion: reduce` disables automatic playback and
  transitions; both `prefers-color-scheme` modes first-class; body text 4.5:1 in both;
  colour never carries a state alone.
- **Canvas:** the device's logical size is 448×368 landscape (`LOGICAL_WIDTH` /
  `LOGICAL_HEIGHT`, `companion/crates/lvgl-sim/src/lib.rs:26-27`).
- **Cargo is not on the Bash tool's PATH.** Every cargo step must begin with
  `export PATH="$HOME/.cargo/bin:$PATH"`. Never pipe a cargo invocation into `tail` —
  zsh reports `tail`'s exit status and hides the failure. Redirect to a file and check
  `$?`.

---

## File Structure

**Deskmate repo (one file, dev-only):**

- Create: `companion/crates/app-core/examples/frame_export.rs` — renders the card faces
  the site replays. Owns the frame set definition and nothing else.

**Site repo `~/dev/deskmate-site` (new):**

- `package.json`, `astro.config.mjs`, `tsconfig.json`, `biome.json`, `wrangler.toml` — project config.
- `src/lib/panel.ts` — the panel's pure state machine. No DOM, no Astro. This is where
  the logic lives so it can be unit-tested.
- `src/lib/panel.test.ts` — unit tests for the above.
- `src/components/Panel.astro` — the sticky panel's markup and its client island.
- `src/components/Ring.astro` — the loop ring.
- `src/components/Section.astro` — one loop section.
- `src/components/Waitlist.astro` — the email form.
- `src/pages/index.astro` — the eight sections, FAQ and footer.
- `src/styles/global.css` — the world. One hand-authored stylesheet, no framework.
- `public/frames/*.webp` — the exported frame pack.
- `functions/api/waitlist.ts` — the Cloudflare Pages Function.
- `schema.sql` — the D1 table.
- `tools/export-frames.sh` — regenerates the pack from a Deskmate checkout.
- `tests/site.spec.ts` — Playwright checks against the real build.

---

### Task 1: The frame exporter

**Files:**
- Create: `companion/crates/app-core/examples/frame_export.rs`
- Reference: `companion/crates/app-core/examples/scene_panel_check.rs` (the established
  dev-only example pattern), `companion/crates/app-core/Cargo.toml:21-24` (documents the
  dev-only `lvgl-sim` edge)

**Interfaces:**
- Consumes: `app_core::preview_card_scene(&AppConfig, &str, &[CardField]) -> Result<protocol::Scene, String>`
  (`crates/app-core/src/runtime/scene.rs:86`); `app_core::utc_offset_minutes(&str, DateTime<Utc>) -> Result<i16, String>`
  (`crates/app-core/src/config.rs:607`); `lvgl_sim::Simulator::new() -> Result<Simulator, SimError>`;
  `Simulator::render_scene_png(&mut self, &SceneRenderRequest) -> Result<Vec<u8>, SimError>`
  (`crates/lvgl-sim/src/scene.rs:246`); `lvgl_sim::scene::SceneTimer { total_ms: u32, remaining_ms: u32, running: bool }`
  (`:134`); `lvgl_sim::scene::SceneRenderRequest { scene, assets, utc_offset_minutes, now_unix_seconds, timer, orientation }` (`:153`).
- Produces: a directory of PNGs named `clock.png`, `focus-paused.png`, and
  `focus-000.png` … `focus-060.png`. Task 2 and `tools/export-frames.sh` depend on
  exactly these names.

- [ ] **Step 1: Create the worktree and make lvgl-sim buildable in it**

A fresh Deskmate worktree fails to build `lvgl-sim` with a misleading `cbor.h` error
until the gitignored managed components are linked in. Use literal absolute paths — the
worktree Bash guard rejects shell variables and subshells.

```bash
git -C /Users/rodion/dev/deskmate worktree add /Users/rodion/dev/deskmate-track-d -b track-d/frame-export
ln -s /Users/rodion/dev/deskmate/firmware/managed_components /Users/rodion/dev/deskmate-track-d/firmware/managed_components
```

- [ ] **Step 2: Write the verification command and watch it fail**

This example's gate is running it and inspecting its output; examples default to
`test = false`, so a `#[test]` inside one would never run and asserting otherwise would
be a lie. Run the check first so the failure is real:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate-track-d/companion
cargo run -p app-core --example frame_export -- /tmp/deskmate-frames > /tmp/fe.log 2>&1; echo "exit=$?"
```

Expected: `exit=101` or a cargo error, with `no example target named 'frame_export'` in
`/tmp/fe.log`.

- [ ] **Step 3: Write the example**

```rust
//! Dev-only frame exporter for the Deskmate landing page.
//!
//! Renders the card faces the marketing site's interactive panel replays, through
//! `preview_card_scene` -- the same scene builder the server's preview calls -- and
//! the firmware's own decoder and interpreter compiled into `lvgl-sim`. Frames are
//! PNGs on the logical 448x368 canvas.
//!
//! Run from `companion/`:
//! `cargo run -p app-core --example frame_export -- <out_dir>`
//!
//! `Simulator` allows one instance per process (`SimError::AlreadyClaimed`), so every
//! frame is rendered sequentially from a single simulator.
//!
//! What these frames prove: that this is what the scene renderer draws. They are not
//! photographs of a panel, and the site must not present them as such.

use std::fs;
use std::path::PathBuf;

use app_core::{
    AppConfig, CardAlert, CardSettings, DisplayTemplate, RefreshPolicy, WidgetTapAction,
};
use chrono::Utc;
use lvgl_sim::scene::{SceneRenderRequest, SceneTimer};
use lvgl_sim::{SimOrientation, Simulator};

/// The countdown window the site replays, in seconds. One frame per second is the
/// cadence the device itself ticks a pushed timer at.
const WINDOW_SECONDS: u32 = 60;
const DURATION_SECONDS: u32 = 25 * 60;

/// `AppConfig::default()` already carries a single clock card with id `clock`.
/// The focus card is added here so the pack has a timer face to render.
fn demo_config() -> AppConfig {
    let mut config = AppConfig::default();
    config.cards.push(CardSettings::Pomodoro {
        id: "focus".into(),
        label: "Focus".into(),
        duration_seconds: DURATION_SECONDS,
        template: DisplayTemplate::ProgressRing,
        tap_action: WidgetTapAction::StartPause,
        refresh: RefreshPolicy::DeviceLocal,
        alert: CardAlert::None,
        dwell_seconds: None,
    });
    config
}

fn render(
    sim: &mut Simulator,
    config: &AppConfig,
    card_id: &str,
    timer: Option<SceneTimer>,
) -> Vec<u8> {
    let now = Utc::now();
    let scene = app_core::preview_card_scene(config, card_id, &[])
        .unwrap_or_else(|reason| panic!("scene for {card_id:?} did not build: {reason}"));
    let utc_offset_minutes = app_core::utc_offset_minutes(&config.preferences.timezone, now)
        .unwrap_or_else(|reason| panic!("timezone did not resolve: {reason}"));
    sim.render_scene_png(&SceneRenderRequest {
        scene,
        assets: Vec::new(),
        utc_offset_minutes,
        now_unix_seconds: now.timestamp(),
        timer,
        // The site shows the upright view a person sees, at either mounting.
        orientation: SimOrientation::Landscape,
    })
    .unwrap_or_else(|error| panic!("frame for {card_id:?} did not render: {error}"))
}

fn main() {
    let out: PathBuf = std::env::args()
        .nth(1)
        .expect("usage: frame_export <out_dir>")
        .into();
    fs::create_dir_all(&out).expect("output directory");

    let config = demo_config();
    let mut sim = Simulator::new().expect("simulator");

    fs::write(out.join("clock.png"), render(&mut sim, &config, "clock", None))
        .expect("write clock frame");

    let total_ms = DURATION_SECONDS * 1_000;
    let paused = SceneTimer {
        total_ms,
        remaining_ms: total_ms,
        running: false,
    };
    fs::write(
        out.join("focus-paused.png"),
        render(&mut sim, &config, "focus", Some(paused)),
    )
    .expect("write paused frame");

    for second in 0..=WINDOW_SECONDS {
        let timer = SceneTimer {
            total_ms,
            remaining_ms: total_ms - second * 1_000,
            running: true,
        };
        let png = render(&mut sim, &config, "focus", Some(timer));
        fs::write(out.join(format!("focus-{second:03}.png")), png).expect("write frame");
    }

    println!(
        "wrote {} frames to {}",
        WINDOW_SECONDS + 3,
        out.display()
    );
}
```

- [ ] **Step 4: Fix the imports if any name is not re-exported**

`crates/app-core/src/lib.rs:17` lists what the crate re-exports at its root. If
`cargo build` reports an unresolved import for `DisplayTemplate`, `RefreshPolicy` or
`WidgetTapAction`, import it from `app_core::config::` instead. Do not add a new
`pub use` to `lib.rs` — this is a dev-only example and must not widen the crate's public
surface.

- [ ] **Step 5: Run the verification command and confirm it passes**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate-track-d/companion
cargo run -p app-core --example frame_export -- /tmp/deskmate-frames > /tmp/fe.log 2>&1; echo "exit=$?"
ls /tmp/deskmate-frames | wc -l
file /tmp/deskmate-frames/focus-000.png
cmp -s /tmp/deskmate-frames/focus-000.png /tmp/deskmate-frames/focus-030.png; echo "frames differ: $?"
```

Expected: `exit=0`; `63` files; `PNG image data, 448 x 368`; `frames differ: 1`.

A `0` from `cmp` means two countdown seconds rendered identically, which would mean the
timer is not reaching the scene — stop and fix it rather than shipping a pack that does
not animate.

- [ ] **Step 6: Run the workspace gates**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate-track-d/companion
cargo fmt --all --check > /tmp/g1.log 2>&1; echo "fmt=$?"
cargo clippy --workspace --all-targets -- -D warnings > /tmp/g2.log 2>&1; echo "clippy=$?"
cargo test --workspace --all-targets > /tmp/g3.log 2>&1; echo "test=$?"
cargo test --workspace --doc > /tmp/g4.log 2>&1; echo "doc=$?"
```

Expected: all four `=0`. `--all-targets` builds examples, so clippy covers the new file.

- [ ] **Step 7: Commit and open the PR**

```bash
cd /Users/rodion/dev/deskmate-track-d
git add companion/crates/app-core/examples/frame_export.rs
git commit -m "feat: dev-only frame exporter for the landing page

Renders the clock and a 60-second focus countdown through preview_card_scene
and lvgl-sim, so the marketing site replays the pixels the scene renderer
produces rather than a lookalike.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01RcaV4uPsAjVYN4Tnd9agVV"
git push -u origin track-d/frame-export
gh pr create --fill
```

Then confirm CI: `gh run list --limit 3`. A skipped job is grey, not red — check the
`companion` job actually ran.

---

### Task 2: Measure the pack, and decide

The spec names this as the one unmeasured assumption. A reviewer can reject the whole
hero approach on this number, which is why it is its own task.

**Files:**
- Create: `~/dev/deskmate-site/tools/export-frames.sh`

**Interfaces:**
- Consumes: Task 1's `frame_export` example and its exact output filenames.
- Produces: `public/frames/*.webp` with the same stems (`clock`, `focus-paused`,
  `focus-000` … `focus-060`). Task 4 loads them by those names.

- [ ] **Step 1: Install the encoder**

```bash
brew install webp
cwebp -version
```

- [ ] **Step 2: Write the export script**

```bash
#!/bin/bash
# usage: tools/export-frames.sh <deskmate-checkout>
# Renders the frame pack through the firmware's own scene interpreter and encodes
# it losslessly to WebP. Lossless because these are flat UI surfaces on black --
# lossy encoding puts ringing around the numerals at exactly the sizes that matter.
set -euo pipefail
TREE="${1:?usage: export-frames.sh <deskmate-checkout>}"
HERE="$(cd "$(dirname "$0")/.." && pwd)"
RAW="$(mktemp -d)"
export PATH="$HOME/.cargo/bin:$PATH"
cd "$TREE/companion"
cargo run --release -p app-core --example frame_export -- "$RAW"
mkdir -p "$HERE/public/frames"
rm -f "$HERE/public/frames"/*.webp
for png in "$RAW"/*.png; do
  cwebp -lossless -z 9 -quiet "$png" -o "$HERE/public/frames/$(basename "${png%.png}").webp"
done
du -sh "$HERE/public/frames"
ls "$HERE/public/frames" | wc -l
```

- [ ] **Step 3: Add the server-drawn faces to the pack**

These need no exporter and no server — the faces package draws them itself. Append to
`tools/export-frames.sh`, before the `du -sh` line:

```bash
# The server-rendered faces draw themselves. `dump` writes one PNG per case plus a
# 0.4x `.desk.png` preview; only the full-size ones belong in the pack.
FACES="$(mktemp -d)"
cd "$TREE/companion/faces"
bun install --frozen-lockfile
bun run dump "$FACES"
for kind in weather hackernews; do
  src="$(find "$FACES" -name "$kind*.png" ! -name "*.desk.png" | head -1)"
  [ -n "$src" ] || { echo "no dumped frame for $kind"; exit 1; }
  cwebp -lossless -z 9 -quiet "$src" -o "$HERE/public/frames/$kind.webp"
done
```

Run `bun run dump /tmp/faces-check` once by hand first and look at the filenames, so the
`find` pattern matches what `dump` actually writes rather than what this plan guessed.
Adjust the pattern if they differ; do not adjust it to match a `.desk.png`, which is a
0.4x preview and will look soft on the page.

- [ ] **Step 4: Run it and record the number**

```bash
chmod +x ~/dev/deskmate-site/tools/export-frames.sh
~/dev/deskmate-site/tools/export-frames.sh /Users/rodion/dev/deskmate-track-d
```

Expected: 63 files. **Write the measured total into the spec's "Frame production"
section**, replacing the sentence that calls it unmeasured.

- [ ] **Step 5: Apply the decision rule**

- Under 1 MB: proceed as planned, lazy-load everything below the fold.
- 1–2 MB: drop the countdown to 30 frames (`WINDOW_SECONDS = 30` in Task 1's example)
  and re-measure.
- Over 2 MB: switch the countdown to a `<video>` with `playsinline muted`, keeping
  `clock.webp` and `focus-paused.webp` as frames so the tappable states stay images.
  Task 4's state machine is unchanged either way; only its render target moves.

Record which branch was taken in the commit message.

- [ ] **Step 6: Commit**

```bash
cd ~/dev/deskmate-site
git add tools/export-frames.sh public/frames
git commit -m "feat: frame pack exported from the scene renderer"
```

---

### Task 3: Site scaffold, deployed

Deploy an empty page first. A pipeline proven on a trivial build is one that cannot be
confused later with a content bug.

**Files:**
- Create: `~/dev/deskmate-site/{package.json,astro.config.mjs,tsconfig.json,biome.json,wrangler.toml,.gitignore}`
- Create: `~/dev/deskmate-site/src/pages/index.astro`, `src/styles/global.css`

**Interfaces:**
- Produces: `bun run build` → `dist/`; `bun run check`; `bun test`. Every later task uses
  these three commands.

- [ ] **Step 1: Create the repo and config files**

```bash
mkdir -p ~/dev/deskmate-site/src/pages ~/dev/deskmate-site/src/styles
cd ~/dev/deskmate-site && git init
```

`package.json`:

```json
{
  "name": "deskmate-site",
  "type": "module",
  "private": true,
  "scripts": {
    "dev": "astro dev",
    "build": "astro build",
    "preview": "astro preview",
    "check": "astro check && tsc --noEmit",
    "lint": "biome check .",
    "format:check": "biome format .",
    "test": "bun test src",
    "e2e": "playwright test"
  },
  "dependencies": {
    "astro": "^5.0.0"
  },
  "devDependencies": {
    "@astrojs/check": "^0.9.0",
    "@biomejs/biome": "^2.0.0",
    "@playwright/test": "^1.48.0",
    "typescript": "^5.6.0",
    "wrangler": "^3.80.0"
  }
}
```

`astro.config.mjs`:

```js
import { defineConfig } from "astro/config";

// Static output. Cloudflare Pages serves dist/ and runs functions/ beside it;
// nothing on this site needs server rendering.
export default defineConfig({
  output: "static",
  build: { inlineStylesheets: "always" },
});
```

`tsconfig.json`:

```json
{
  "extends": "astro/tsconfigs/strict",
  "include": [".astro/types.d.ts", "**/*"],
  "exclude": ["dist"]
}
```

`biome.json`:

```json
{
  "$schema": "https://biomejs.dev/schemas/2.0.0/schema.json",
  "formatter": { "enabled": true, "indentStyle": "space", "indentWidth": 2 },
  "linter": { "enabled": true, "rules": { "recommended": true } },
  "files": { "includes": ["src/**", "functions/**", "tests/**"] }
}
```

`wrangler.toml`:

```toml
name = "deskmate-site"
compatibility_date = "2026-09-01"
pages_build_output_dir = "dist"

[[d1_databases]]
binding = "WAITLIST"
database_name = "deskmate-waitlist"
database_id = "PLACEHOLDER_REPLACED_IN_STEP_4"
```

`.gitignore`:

```
node_modules/
dist/
.astro/
.wrangler/
test-results/
```

- [ ] **Step 2: Write a placeholder page and build it**

`src/pages/index.astro`:

```astro
---
import "../styles/global.css";
---

<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Deskmate</title>
  </head>
  <body>
    <main><h1>Deskmate</h1></main>
  </body>
</html>
```

`src/styles/global.css`:

```css
:root { color-scheme: dark light; }
body { margin: 0; background: #000; color: #f5f5f7; font-family: system-ui, sans-serif; }
```

```bash
cd ~/dev/deskmate-site && bun install && bun run build
```

Expected: `dist/index.html` exists.

- [ ] **Step 3: Create the D1 database**

```bash
cd ~/dev/deskmate-site
bunx wrangler d1 create deskmate-waitlist
```

Copy the printed `database_id` into `wrangler.toml`, replacing the placeholder.

- [ ] **Step 4: Deploy and confirm the URL serves**

```bash
cd ~/dev/deskmate-site
bunx wrangler pages project create deskmate-site --production-branch main
bunx wrangler pages deploy dist
```

Expected: a `*.pages.dev` URL that returns the placeholder page. Record it; Task 7 and
Task 9 use it.

- [ ] **Step 5: Commit**

```bash
cd ~/dev/deskmate-site
git add -A && git commit -m "chore: Astro scaffold deployed to Cloudflare Pages"
```

---

### Task 4: The panel's state machine and component

**Files:**
- Create: `~/dev/deskmate-site/src/lib/panel.ts`
- Create: `~/dev/deskmate-site/src/lib/panel.test.ts`
- Create: `~/dev/deskmate-site/src/components/Panel.astro`

**Interfaces:**
- Consumes: `public/frames/*.webp` from Task 2.
- Produces: `PanelState`, `initialState(cardCount)`, `tap(state)`, `advance(state, delta)`,
  `tick(state)`, `frameName(state)`. Task 5 calls `advance` from its scroll observer;
  Task 9 asserts on the rendered result.

- [ ] **Step 1: Write the failing tests**

`src/lib/panel.test.ts`:

```ts
import { describe, expect, test } from "bun:test";
import { advance, frameName, initialState, tap, tick, WINDOW_SECONDS } from "./panel";

describe("panel state", () => {
  test("starts on the first card, paused, at second zero", () => {
    const s = initialState(3);
    expect(s).toEqual({ card: 0, cardCount: 3, running: false, second: 0 });
  });

  test("tap starts the timer and tapping again pauses it", () => {
    const started = tap(initialState(3));
    expect(started.running).toBe(true);
    expect(tap(started).running).toBe(false);
  });

  test("pausing keeps the second it reached", () => {
    let s = tap(initialState(3));
    s = tick(tick(s));
    expect(tap(s).second).toBe(2);
  });

  test("tick only advances while running, and stops at the window end", () => {
    expect(tick(initialState(3)).second).toBe(0);
    let s = { ...initialState(3), running: true, second: WINDOW_SECONDS };
    expect(tick(s).second).toBe(WINDOW_SECONDS);
    expect(tick(s).running).toBe(false);
  });

  test("advance wraps in both directions, like the device's swipe", () => {
    expect(advance(initialState(3), 1).card).toBe(1);
    expect(advance(initialState(3), -1).card).toBe(2);
  });

  test("advancing resets the timer, because a new card is a new face", () => {
    const running = tick(tap(initialState(3)));
    const moved = advance(running, 1);
    expect(moved.running).toBe(false);
    expect(moved.second).toBe(0);
  });

  test("frame names match the exported pack", () => {
    expect(frameName(initialState(3), ["focus", "clock", "weather"])).toBe("focus-paused");
    const running = { ...initialState(3), running: true, second: 7 };
    expect(frameName(running, ["focus", "clock", "weather"])).toBe("focus-007");
    expect(frameName(advance(initialState(3), 1), ["focus", "clock", "weather"])).toBe("clock");
  });
});
```

- [ ] **Step 2: Run the tests and watch them fail**

```bash
cd ~/dev/deskmate-site && bun test src
```

Expected: FAIL — `Cannot find module './panel'`.

- [ ] **Step 3: Write the state machine**

`src/lib/panel.ts`:

```ts
/** The exported countdown window, in seconds. Must match `WINDOW_SECONDS` in
 *  `companion/crates/app-core/examples/frame_export.rs`. */
export const WINDOW_SECONDS = 60;

export type PanelState = {
  card: number;
  cardCount: number;
  running: boolean;
  second: number;
};

export function initialState(cardCount: number): PanelState {
  return { card: 0, cardCount, running: false, second: 0 };
}

/** The device's tap: start or pause the timer, immediately, without asking anyone. */
export function tap(state: PanelState): PanelState {
  return { ...state, running: !state.running };
}

/** The device's horizontal swipe: select next or previous. Wraps, as the loop does. */
export function advance(state: PanelState, delta: number): PanelState {
  const card = (state.card + delta + state.cardCount) % state.cardCount;
  return { ...state, card, running: false, second: 0 };
}

/** One second of the replay. The pack ends at WINDOW_SECONDS, so the timer stops
 *  there rather than looping back to a time it already showed. */
export function tick(state: PanelState): PanelState {
  if (!state.running) return state;
  if (state.second >= WINDOW_SECONDS) return { ...state, running: false };
  return { ...state, second: state.second + 1 };
}

/** The frame stem for the current state. `cards[0]` is the timer; every other card
 *  is a single still. */
export function frameName(state: PanelState, cards: readonly string[]): string {
  const card = cards[state.card];
  if (card !== "focus") return card;
  if (!state.running) return "focus-paused";
  return `focus-${String(state.second).padStart(3, "0")}`;
}
```

- [ ] **Step 4: Run the tests and confirm they pass**

```bash
cd ~/dev/deskmate-site && bun test src
```

Expected: PASS, 7 tests.

- [ ] **Step 5: Write the component**

`src/components/Panel.astro`:

```astro
---
// The sticky panel. A <button>, because it is a control: tap toggles the timer,
// horizontal drag advances the loop, and both have keyboard equivalents.
// With JavaScript off the <img> below is the whole component and still correct.
const { cards } = Astro.props as { cards: readonly string[] };
---

<button class="panel" type="button" aria-live="polite"
        aria-label="Deskmate panel. Press to start or pause the focus timer. Use the left and right arrow keys to change card."
        data-cards={cards.join(",")}>
  <img class="panel__face" src="/frames/focus-paused.webp" width="448" height="368"
       alt="The Deskmate panel showing a focus timer at 25:00, paused." />
</button>

<script>
  import { advance, frameName, initialState, tap, tick } from "../lib/panel";

  const el = document.querySelector<HTMLButtonElement>(".panel");
  const face = el?.querySelector<HTMLImageElement>(".panel__face");
  if (el && face) {
    const cards = (el.dataset.cards ?? "").split(",");
    const calm = window.matchMedia("(prefers-reduced-motion: reduce)");
    let state = initialState(cards.length);
    let timer: ReturnType<typeof setInterval> | undefined;

    const paint = () => {
      face.src = `/frames/${frameName(state, cards)}.webp`;
    };

    const stop = () => {
      if (timer) clearInterval(timer);
      timer = undefined;
    };

    // Under reduced motion the countdown does not play. A tap still switches
    // between the paused and running faces -- the control stays honest, the
    // animation does not happen.
    const run = () => {
      stop();
      if (!state.running || calm.matches) return;
      timer = setInterval(() => {
        state = tick(state);
        paint();
        if (!state.running) stop();
      }, 1000);
    };

    const set = (next: typeof state) => {
      state = next;
      paint();
      run();
    };

    el.addEventListener("click", () => set(tap(state)));
    el.addEventListener("keydown", (event) => {
      if (event.key === "ArrowRight") { event.preventDefault(); set(advance(state, 1)); }
      if (event.key === "ArrowLeft") { event.preventDefault(); set(advance(state, -1)); }
    });

    // Horizontal drag, matching the device's swipe: at least 56px on the primary
    // axis and more horizontal than vertical, per docs/protocol/v2.md:328-331.
    let origin: { x: number; y: number } | null = null;
    el.addEventListener("pointerdown", (e) => { origin = { x: e.clientX, y: e.clientY }; });
    el.addEventListener("pointerup", (e) => {
      if (!origin) return;
      const dx = e.clientX - origin.x;
      const dy = e.clientY - origin.y;
      origin = null;
      if (Math.abs(dx) >= 56 && Math.abs(dx) > Math.abs(dy) * 1.5) {
        set(advance(state, dx < 0 ? 1 : -1));
      }
    });

    document.addEventListener("deskmate:card", (event) => {
      const index = (event as CustomEvent<number>).detail;
      if (index !== state.card) set({ ...state, card: index, running: false, second: 0 });
    });

    paint();
  }
</script>
```

- [ ] **Step 6: Preload the countdown so it does not stutter**

Add to `Panel.astro`, inside the `<script>`, before `paint()`:

```ts
    // 61 small images fetched one per second would visibly hitch on the first run.
    for (let s = 0; s <= 60; s++) {
      const img = new Image();
      img.src = `/frames/focus-${String(s).padStart(3, "0")}.webp`;
    }
```

- [ ] **Step 7: Build, then commit**

```bash
cd ~/dev/deskmate-site && bun run build && bun run check && bun test src
git add -A && git commit -m "feat: the panel, tappable and keyboard-equivalent"
```

---

### Task 5: The loop mechanic and the ring

**Files:**
- Create: `~/dev/deskmate-site/src/components/Ring.astro`, `src/components/Section.astro`
- Modify: `~/dev/deskmate-site/src/styles/global.css`

**Interfaces:**
- Consumes: Task 4's `deskmate:card` event contract (a `CustomEvent<number>` on `document`).
- Produces: `<Section index face>` slots used by Task 6; a ring that reads scroll progress.

- [ ] **Step 1: Write the section component**

`src/components/Section.astro`:

```astro
---
const { index, face, eyebrow } = Astro.props as { index: number; face: string; eyebrow: string };
const label = String(index + 1).padStart(2, "0");
---

<section class="loop-section" data-card={index} data-face={face} id={`card-${label}`}>
  <p class="loop-section__eyebrow">{label} · {eyebrow}</p>
  <div class="loop-section__body"><slot /></div>
  <noscript>
    <img src={`/frames/${face}.webp`} width="448" height="368" alt="" class="loop-section__still" />
  </noscript>
</section>
```

- [ ] **Step 2: Wire section visibility to the panel**

Add to `src/pages/index.astro`'s script in Task 6; for now create
`src/lib/loop.ts`:

```ts
/** Native scroll only. IntersectionObserver reports which section owns the
 *  viewport; nothing here touches the wheel, and the page scrolls exactly as
 *  the browser intends. */
export function observeSections(): void {
  const sections = document.querySelectorAll<HTMLElement>(".loop-section");
  if (!sections.length) return;
  const observer = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        const index = Number(entry.target.getAttribute("data-card"));
        document.dispatchEvent(new CustomEvent<number>("deskmate:card", { detail: index }));
      }
    },
    { rootMargin: "-45% 0px -45% 0px", threshold: 0 },
  );
  for (const section of sections) observer.observe(section);
}
```

- [ ] **Step 3: Write the ring**

`src/components/Ring.astro`:

```astro
---
const { steps } = Astro.props as { steps: number };
---

<div class="ring" aria-hidden="true" data-steps={steps}>
  <svg viewBox="0 0 48 48" width="48" height="48">
    <circle class="ring__track" cx="24" cy="24" r="20" fill="none" stroke-width="3" />
    <circle class="ring__arc" cx="24" cy="24" r="20" fill="none" stroke-width="3"
            pathLength="1" stroke-dasharray="1" stroke-dashoffset="1" />
  </svg>
</div>

<style>
  .ring__arc { transform: rotate(-90deg); transform-origin: 50% 50%; }

  /* Where the browser can drive the arc off scroll position itself, it does --
     no JavaScript, no rAF, and it stays smooth during a fling. */
  @supports (animation-timeline: scroll()) {
    .ring__arc {
      animation: ring-fill linear both;
      animation-timeline: scroll(root block);
    }
    @keyframes ring-fill { to { stroke-dashoffset: 0; } }
  }

  @media (prefers-reduced-motion: reduce) {
    .ring__arc { animation: none; }
  }
</style>

<script>
  // Fallback for browsers without scroll-driven animations.
  if (!CSS.supports("animation-timeline", "scroll()")) {
    const arc = document.querySelector<SVGCircleElement>(".ring__arc");
    if (arc) {
      let queued = false;
      const update = () => {
        queued = false;
        const max = document.documentElement.scrollHeight - window.innerHeight;
        const progress = max > 0 ? window.scrollY / max : 0;
        arc.style.strokeDashoffset = String(1 - progress);
      };
      addEventListener("scroll", () => {
        if (queued) return;
        queued = true;
        requestAnimationFrame(update);
      }, { passive: true });
      update();
    }
  }
</script>
```

The `passive: true` listener is not decoration: it is what guarantees this page cannot
block scrolling even by accident.

- [ ] **Step 4: Style the sticky stage**

Append to `src/styles/global.css`:

```css
.loop {
  display: grid;
  grid-template-columns: 1fr min(44vw, 520px);
  gap: 4rem;
  align-items: start;
}

.loop__stage {
  position: sticky;
  top: 50vh;
  translate: 0 -50%;
  grid-column: 2;
  grid-row: 1 / -1;
}

.loop-section { min-height: 92vh; display: flex; flex-direction: column; justify-content: center; }

.panel {
  display: block;
  padding: 0;
  border: 0;
  border-radius: 28px;
  background: #000;
  box-shadow: 0 0 0 10px #17171a, 0 30px 80px rgb(0 0 0 / 0.6);
  cursor: pointer;
  touch-action: pan-y;
}
.panel:focus-visible { outline: 3px solid #fff; outline-offset: 6px; }
.panel__face { display: block; width: 100%; height: auto; border-radius: 18px; }

/* The narrow layout is designed, not reflowed: the panel sticks to the top and
   the ring travels with it. */
@media (max-width: 860px) {
  .loop { grid-template-columns: 1fr; gap: 1.5rem; }
  .loop__stage { grid-column: 1; grid-row: auto; top: 0.75rem; translate: none; }
  .loop-section { min-height: 70vh; }
}

@media (prefers-reduced-motion: reduce) {
  .loop__stage { position: static; translate: none; }
}
```

- [ ] **Step 5: Build and commit**

```bash
cd ~/dev/deskmate-site && bun run build && bun run check
git add -A && git commit -m "feat: the loop mechanic and its ring, on native scroll"
```

---

### Task 6: The eight sections

**Files:**
- Modify: `~/dev/deskmate-site/src/pages/index.astro`
- Modify: `~/dev/deskmate-site/src/styles/global.css`

**Interfaces:**
- Consumes: `Panel.astro`, `Ring.astro`, `Section.astro`, `observeSections()`.
- Produces: the finished loop. Task 8 appends the FAQ and footer after it.

- [ ] **Step 1: Write the page**

The eight sections, their faces and their claims come from the spec's table. Copy is
final text here, in the voice the constraints name.

```astro
---
import Panel from "../components/Panel.astro";
import Ring from "../components/Ring.astro";
import Section from "../components/Section.astro";
import Waitlist from "../components/Waitlist.astro";
import "../styles/global.css";

const cards = ["focus", "clock", "clock", "weather", "hackernews", "clock", "focus", "clock"];
---

<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Deskmate — a desk accessory you can touch</title>
    <meta name="description" content="A small emissive panel that clips to your monitor and answers when you touch it." />
  </head>
  <body>
    <Ring steps={8} />
    <main class="loop">
      <div class="loop__stage"><Panel cards={cards} /></div>

      <Section index={0} face="focus" eyebrow="Deskmate">
        <h1>Tap it.<br />It answers.</h1>
        <p>A small emissive panel that clips to your monitor. You build a loop of cards;
           it shows one at a time, and it responds when you touch the glass.</p>
        <p class="hint">The panel on the right is live. Tap it.</p>
        <Waitlist id="hero" />
      </Section>

      <Section index={1} face="focus" eyebrow="The focus timer">
        <h2>It does not wait for the server.</h2>
        <p>Tap to start. The timer moves the instant your finger lands, and the host
           reconciles afterwards. Pull the network cable out and it keeps counting.</p>
      </Section>

      <Section index={2} face="clock" eyebrow="The loop">
        <h2>Your cards, in your order.</h2>
        <p>One loop. Each card gets a dwell, and the panel moves through them.
           Swipe the glass to go somewhere else in it.</p>
      </Section>

      <Section index={3} face="weather" eyebrow="Faces the server draws">
        <h2>Weather, a feed, a price.</h2>
        <p>Drawn on the server and pushed to the panel as a finished picture, so the
           display spends its power on being readable rather than on fetching.</p>
      </Section>

      <Section index={4} face="hackernews" eyebrow="Anything else">
        <h2>One line of curl.</h2>
        <p>A card's face is a 448×368 picture. Anything that can POST one owns a card —
           no SDK, no manifest, no plugin to install.</p>
        <pre><code>curl -T frame.png https://…/v1/images/my-source</code></pre>
      </Section>

      <Section index={5} face="clock" eyebrow="A real object">
        <h2>It exists. Here it is.</h2>
        <p>Every photograph on this page is the working prototype on a real desk.
           Nothing here is AI-generated.</p>
        <!-- Photographs are added in Task 8 once the owner supplies them. -->
      </Section>

      <Section index={6} face="focus" eyebrow="Where it is going">
        <h2>Interaction arrives in three stages.</h2>
        <ol class="stages">
          <li><strong>Now.</strong> The focus timer answers on the device itself.</li>
          <li><strong>Next.</strong> A tap reaches the card's source, which answers with a new picture.</li>
          <li><strong>Later.</strong> Cards that need it respond instantly, on the device.</li>
        </ol>
      </Section>

      <Section index={7} face="clock" eyebrow="Waitlist">
        <h2>It is a prototype on one desk.</h2>
        <p>There is no price yet and no ship date yet. Leave an email address and you
           will hear from me when that changes — once, when it does.</p>
        <Waitlist id="end" />
      </Section>
    </main>

    <script>
      import { observeSections } from "../lib/loop";
      observeSections();
    </script>
  </body>
</html>
```

- [ ] **Step 2: Confirm the section count matches the loop's limit**

```bash
cd ~/dev/deskmate-site && grep -c "<Section" src/pages/index.astro
```

Expected: `8`. The loop holds at most eight cards; if this ever prints more, a
constraint has been broken, not a layout.

- [ ] **Step 3: Build and commit**

```bash
cd ~/dev/deskmate-site && bun run build && bun run check
git add -A && git commit -m "feat: the eight sections"
```

---

### Task 7: The waitlist

**Files:**
- Create: `~/dev/deskmate-site/src/components/Waitlist.astro`
- Create: `~/dev/deskmate-site/functions/api/waitlist.ts`
- Create: `~/dev/deskmate-site/schema.sql`

**Interfaces:**
- Consumes: the `WAITLIST` D1 binding from Task 3's `wrangler.toml`.
- Produces: `POST /api/waitlist` accepting `{ email: string }`, returning 204 on success,
  400 on a malformed address, 429 when rate limited.

- [ ] **Step 1: Write the schema**

`schema.sql`:

```sql
CREATE TABLE IF NOT EXISTS waitlist (
  email TEXT PRIMARY KEY,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  source TEXT
);
```

```bash
cd ~/dev/deskmate-site
bunx wrangler d1 execute deskmate-waitlist --remote --file schema.sql
```

- [ ] **Step 2: Write the function**

`functions/api/waitlist.ts`:

```ts
interface Env {
  WAITLIST: D1Database;
}

// Deliberately permissive: the job is to reject a typo-shaped string, not to
// adjudicate RFC 5322. A rejected address the visitor meant is worse than a
// stored one that bounces.
const LOOKS_LIKE_EMAIL = /^[^@\s]+@[^@\s.]+\.[^@\s]{2,}$/;

export const onRequestPost: PagesFunction<Env> = async ({ request, env }) => {
  let email: unknown;
  let source: unknown;
  try {
    ({ email, source } = (await request.json()) as Record<string, unknown>);
  } catch {
    return new Response("malformed body", { status: 400 });
  }

  if (typeof email !== "string" || email.length > 254 || !LOOKS_LIKE_EMAIL.test(email)) {
    return new Response("that does not look like an email address", { status: 400 });
  }

  // INSERT OR IGNORE, so signing up twice is not an error the visitor has to
  // understand. They asked to be told; they will be told once.
  await env.WAITLIST.prepare(
    "INSERT OR IGNORE INTO waitlist (email, source) VALUES (?, ?)",
  )
    .bind(email.toLowerCase().trim(), typeof source === "string" ? source.slice(0, 32) : null)
    .run();

  return new Response(null, { status: 204 });
};
```

- [ ] **Step 3: Write the form**

`src/components/Waitlist.astro`:

```astro
---
const { id } = Astro.props as { id: string };
---

<form class="waitlist" data-source={id} method="post" action="/api/waitlist">
  <label class="waitlist__label" for={`email-${id}`}>Email address</label>
  <input class="waitlist__input" id={`email-${id}`} name="email" type="email"
         required autocomplete="email" placeholder="you@example.com" />
  <button class="waitlist__submit" type="submit">Notify me</button>
  <p class="waitlist__status" role="status" aria-live="polite"></p>
</form>

<script>
  for (const form of document.querySelectorAll<HTMLFormElement>(".waitlist")) {
    const status = form.querySelector<HTMLParagraphElement>(".waitlist__status");
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const input = form.querySelector<HTMLInputElement>(".waitlist__input");
      if (!input || !status) return;
      status.textContent = "Sending…";
      const response = await fetch("/api/waitlist", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ email: input.value, source: form.dataset.source }),
      }).catch(() => null);

      // Say what happened and what happens next. Never exclaim, never blame.
      if (response?.status === 204) {
        status.textContent = "Saved. You will hear from me once, when there is something to say.";
        input.value = "";
      } else if (response?.status === 400) {
        status.textContent = "That does not look like an email address.";
      } else {
        status.textContent = "That did not reach the server. Try again in a moment.";
      }
    });
  }
</script>
```

- [ ] **Step 4: Test it locally against a real D1**

```bash
cd ~/dev/deskmate-site && bun run build
bunx wrangler pages dev dist --d1 WAITLIST=deskmate-waitlist &
sleep 5
curl -s -o /dev/null -w "%{http_code}\n" -X POST localhost:8788/api/waitlist \
  -H 'Content-Type: application/json' -d '{"email":"someone@example.com","source":"hero"}'
curl -s -o /dev/null -w "%{http_code}\n" -X POST localhost:8788/api/waitlist \
  -H 'Content-Type: application/json' -d '{"email":"nope","source":"hero"}'
```

Expected: `204` then `400`. Then confirm the row landed and that a duplicate is silent:

```bash
bunx wrangler d1 execute deskmate-waitlist --local --command "SELECT count(*) FROM waitlist"
```

- [ ] **Step 5: Deploy and repeat against the live URL**

```bash
cd ~/dev/deskmate-site && bunx wrangler pages deploy dist
```

Run the same two `curl` calls against the `*.pages.dev` host. Expected: `204`, `400`.
A local pass proves the function; only the deployed pass proves the binding.

- [ ] **Step 6: Commit**

```bash
cd ~/dev/deskmate-site
git add -A && git commit -m "feat: waitlist capture into D1"
```

---

### Task 8: Photographs, FAQ, footer, metadata

**Files:**
- Modify: `~/dev/deskmate-site/src/pages/index.astro`
- Create: `~/dev/deskmate-site/public/photos/*.jpg`, `public/og.png`

**Interfaces:**
- Consumes: photographs supplied by the owner.
- Produces: the complete page.

- [ ] **Step 1: Place the photographs**

Copy the owner's photographs into `public/photos/`. If none have been supplied yet, use
`docs/hardware/media/2026-09-06-task7/claude-limits-on-panel.jpg` from the Deskmate repo
as the single honest placeholder, and **leave section 05 with one photograph rather than
a grid of three** — an empty frame is better than a generated one. Do not ship a
rendered image in this section under any circumstance.

- [ ] **Step 2: Add the photo grid to section 05**

Replace the comment in section index 5 with:

```astro
        <figure class="shots">
          <img src="/photos/on-the-desk.jpg" width="1920" height="1080" loading="lazy"
               alt="The Deskmate panel on a wooden desk, showing usage percentages." />
          <figcaption>01 · on the desk</figcaption>
        </figure>
```

Add one `<figure>` per supplied photograph, numbered in order.

- [ ] **Step 3: Add the FAQ and footer after `</main>`**

```astro
    <section class="faq">
      <h2>Before you sign up.</h2>
      <dl>
        <dt>01 · What is it?</dt>
        <dd>A small emissive panel, 448×368, that clips to a monitor and shows one card
            at a time. Tap it and the card responds.</dd>
        <dt>02 · Is it finished?</dt>
        <dd>No. It is a working prototype on one desk. The firmware, the server and the
            settings app all run today; manufacturing, packaging and shipping do not
            exist yet.</dd>
        <dt>03 · What does it cost?</dt>
        <dd>I do not know yet, which is why this page asks for an email address rather
            than a card number.</dd>
        <dt>04 · When can I get one?</dt>
        <dd>There is no date. Anyone who tells you a date for a prototype is guessing.</dd>
        <dt>05 · What happens to my email address?</dt>
        <dd>It is stored so I can tell you when there is something to tell you. It is not
            sold, not shared, and not used for anything else.</dd>
      </dl>
    </section>
    <footer class="footer">
      <p>Deskmate · built in the open by one person.</p>
    </footer>
```

- [ ] **Step 4: Add social metadata**

In `<head>`:

```astro
    <meta property="og:title" content="Deskmate — a desk accessory you can touch" />
    <meta property="og:description" content="A small emissive panel that clips to your monitor and answers when you touch it." />
    <meta property="og:image" content="/og.png" />
    <meta property="og:type" content="website" />
    <meta name="twitter:card" content="summary_large_image" />
```

Build `public/og.png` at 1200×630 from a photograph, not a render.

- [ ] **Step 5: Enable analytics**

In the Cloudflare dashboard, turn on Web Analytics for the Pages project. It is
cookieless and needs no consent banner; do not add a third-party analytics script.

- [ ] **Step 6: Build, deploy, commit**

```bash
cd ~/dev/deskmate-site && bun run build && bun run check && bunx wrangler pages deploy dist
git add -A && git commit -m "feat: photographs, FAQ, footer and social metadata"
```

---

### Task 9: Verify it in a browser

The repo's own hard lesson applies here: a DOM suite cannot see a stylesheet, and a mock
harness cannot see anything that needs the server. The real check is the real build in a
real browser.

**Files:**
- Create: `~/dev/deskmate-site/tests/site.spec.ts`, `playwright.config.ts`

- [ ] **Step 1: Write the Playwright config**

`playwright.config.ts`:

```ts
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "tests",
  webServer: { command: "bun run preview --port 4321", url: "http://localhost:4321", reuseExistingServer: true },
  use: { baseURL: "http://localhost:4321" },
});
```

- [ ] **Step 2: Write the failing checks**

`tests/site.spec.ts`:

```ts
import { expect, test } from "@playwright/test";

test("the panel changes face when tapped", async ({ page }) => {
  await page.goto("/");
  const face = page.locator(".panel__face");
  await expect(face).toHaveAttribute("src", /focus-paused/);
  await page.locator(".panel").click();
  await expect(face).toHaveAttribute("src", /focus-\d{3}/);
});

test("arrow keys change card", async ({ page }) => {
  await page.goto("/");
  await page.locator(".panel").focus();
  await page.keyboard.press("ArrowRight");
  await expect(page.locator(".panel__face")).toHaveAttribute("src", /clock/);
});

test("focus is visible on the panel", async ({ page }) => {
  await page.goto("/");
  await page.locator(".panel").focus();
  const outline = await page.locator(".panel").evaluate((el) => getComputedStyle(el).outlineStyle);
  expect(outline).not.toBe("none");
});

test("scrolling is not hijacked", async ({ page }) => {
  await page.goto("/");
  const before = await page.evaluate(() => window.scrollY);
  await page.mouse.wheel(0, 600);
  await page.waitForTimeout(200);
  const after = await page.evaluate(() => window.scrollY);
  expect(after - before).toBeGreaterThan(400);
});

test("the page still reads with JavaScript off", async ({ browser }) => {
  const context = await browser.newContext({ javaScriptEnabled: false });
  const page = await context.newPage();
  await page.goto("/");
  await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
  await expect(page.locator(".panel__face")).toBeVisible();
  await context.close();
});

test("reduced motion does not autoplay the countdown", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/");
  await page.locator(".panel").click();
  const first = await page.locator(".panel__face").getAttribute("src");
  await page.waitForTimeout(2500);
  expect(await page.locator(".panel__face").getAttribute("src")).toBe(first);
});
```

- [ ] **Step 3: Run them**

```bash
cd ~/dev/deskmate-site && bunx playwright install chromium && bun run e2e
```

Expected: 6 passed. Fix the site, not the test, for anything red — each check is a
constraint from the spec, not a preference.

- [ ] **Step 4: Check it at phone width by eye**

```bash
cd ~/dev/deskmate-site && bun run preview
```

Open it at 390px wide and scroll the whole page. The panel must stick to the top, the
ring must travel with it, and nothing may scroll horizontally. The narrow layout is
designed, not reflowed — if it looks derived, fix the `max-width: 860px` block.

- [ ] **Step 5: Commit and deploy**

```bash
cd ~/dev/deskmate-site
git add -A && git commit -m "test: browser checks for the constraints that matter"
bunx wrangler pages deploy dist
```

- [ ] **Step 6: Update the board**

In the Deskmate repo, set Track D's line in `docs/roadmap.md` to the deployed URL and
what remains (a domain, and the photographs if they were still placeholders). Commit
with a `docs:` prefix.

---

## Self-Review

**Spec coverage.** Waitlist-only → Task 7. Hook and copy → Task 6. Interactive
pixel-honest hero → Tasks 1, 2, 4. The Loop world → Task 5. Eight sections → Task 6,
with the count asserted. Native scroll, JS-off, product gestures → Tasks 4, 5, 9.
Frame production correction → Task 1. Unmeasured pack → Task 2, with a decision rule.
Server-drawn faces → Task 2, Step 3.
Accessibility → Tasks 4, 5, 9. Forbidden claims → Global Constraints and Task 8, Step 1.
Open source omitted → no task mentions it. Stack and hosting → Task 3. Photographs →
Task 8, Step 1, with the honest fallback. Mobile layout designed → Task 5, Step 4 and
Task 9, Step 4.

**Known gap, deliberate.** The spec's `prefers-color-scheme` light mode is not built out
in this plan beyond `color-scheme: dark light`. The Loop is a dark world and a designed
light scheme is a real piece of work; it is the first follow-up, not a silent omission.

**Type consistency.** `PanelState`, `initialState`, `tap`, `advance`, `tick`,
`frameName` and `WINDOW_SECONDS` are used identically in Tasks 4, 5 and 9. `WINDOW_SECONDS`
is 60 in both the Rust exporter and `panel.ts`, and Task 2's fallback branch names the
Rust constant to change. Frame stems produced by Task 1 (`clock`, `focus-paused`,
`focus-NNN`) are the stems Task 2 encodes and Task 4 requests.
