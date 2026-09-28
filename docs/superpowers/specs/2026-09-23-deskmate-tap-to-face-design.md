# Tap to face — Track C1 design

Approved by the owner on 2026-09-23. Branch `track-c1-tap-to-face`, worktree
`/Users/rodion/dev/deskmate-c1`.

A tap on a picture card reaches the face that draws it, and the face answers with a new
frame. That is the whole of C1. Instant, device-drawn responses are C2 and are not
designed here.

## What this is not

- **Not a new card kind.** A tappable face is a picture card whose frame happens to
  change when you touch the panel. The rule stands: a new data face is a server-side
  producer.
- **Not a schema bump.** `CURRENT_SCHEMA_VERSION` stays 10 and `docs/config/v10.md`
  stays the only readable version.
- **Not a wire change and not a firmware image.** No message, key, error code or
  capability bit moves. `firmware/` is not touched, so no flash and no OTA
  re-verification are owed by this work.
- **Not coordinates.** `DeviceEvent` carries no point today, and putting one there is a
  wire change plus a firmware image. The seam below is shaped to accept a point later
  without changing again.

Because none of the three expensive boundaries is crossed, **C1 does not need the
schema/wire lock** and it leaves the lock free for Tracks A and B.

## Why no boundary has to move

The flashed firmware already reports the taps this design needs.

- `firmware/main/ui/carousel.c:68` emits a `Tap` event for any card whose wire tap action
  is `StartPause` or `Reset`, and nothing at all when it is `None`.
- It also calls `scene_view_apply_local_action`, whose first statement
  (`firmware/main/ui/scene_view.c:1385`) returns early unless `binding.timer_active` is
  set. A picture card has no timer, so the optimistic feedback is a no-op and the panel
  shows nothing until the new frame arrives.
- `docs/protocol/v2.md` already sanctions the case in prose: "A tap action naming a card
  with no timer is accepted by the wire and simply produces an event the host may
  ignore." Device-side validation
  (`firmware/main/core/apply_config_validation.h`) only range-checks the action.

So the host can declare "this picture reports taps" by lowering `StartPause` for it, and
the host decides what the tap means. **This is reasoned from source, not observed on the
board.** The plan's last task is a tap on `dev-0005`; until that passes, nothing here is
claimed to work on hardware.

## Architecture

```
panel                app-core runtime            server                     faces package
  |                        |                        |                            |
  tap --Tap/StartPause-->  |                        |                            |
                     picture card?                  |                            |
                           |-- CardTapSink::tapped ->|                           |
                           |                   spec for source_id                |
                           |                   wake its refresher --render{event,state}->
                           |                        |<---------- {png, state} ---|
                           |                   canonical_frame_from_png          |
                           |<-- image source accept -|                           |
  <---- asset push --------|                        |                            |
```

Each hop already exists except the sink and the envelope.

### 1. Lowering: every picture card reports taps

`CardSettings::wire_config` (`crates/app-core/src/config.rs:1048`) lowers a picture
card's `tap_action` to `TapAction::StartPause` regardless of the document's value, and
keeps today's lowering for every other card kind. Clock and pomodoro are unchanged.

Rejected alternatives, recorded so they are not re-litigated:

- *Lower only faces that declare taps.* `compile()` would then depend on the faces
  catalog, which `data_cards.rs` re-reads every minute. Config compilation would gain a
  hidden input and a catalog edit would silently re-issue `ApplyConfig`.
- *A v11 `tap_action` kind.* Protocol v2 has three tap actions and only a firmware image
  adds a fourth, so a `to-face` action would still lower to `StartPause`. The device
  cannot perceive the difference, and the cost is a migration, a strict deploy order and
  a permanently retired v10. C2 has to change the wire for coordinates anyway and is the
  right place to design the tap-action registry once, with hardware in hand. Retired wire
  numbers are never re-issued here, so a guess made before C1 has ever run on the board
  is a permanent guess.

A per-card "this card should not respond" switch is deliberately absent. If it is ever
wanted, the cheap form is a face setting in `data-cards.json`, not a card field.

**Deploy consequence.** Every existing picture card's wire `CardConfig` changes the first
time this server runs. The runtime re-sends the layout on its next full synchronize, so
the device sees an ordinary config. The plan verifies that a redeploy against the live
board does not produce `StaleRevision`.

`tap_action` in a saved document keeps its meaning — what the *device* does locally —
and for a picture that is still nothing. `docs/config/v10.md`'s lowering paragraph gains
a sentence saying so. That is a wording change to a frozen contract document, not a
schema change: no stored file, validation rule or wire byte differs because of it.

### 2. The sink: how a tap leaves `app-core`

`app-core` must not learn what a face is, exactly as `data_cards.rs` names no face kind.
The runtime already takes a server-supplied port — `ImageSourceHost`, injected through
`RuntimeHandle::start_with_image_source_host` — and the tap sink follows that precedent:

```rust
/// A tap on a card whose face the host may want to re-render.
pub trait CardTapSink: Send {
    /// Called from the runtime worker thread. MUST NOT block: the worker also
    /// drives the device link, and `SerialTransport::read` blocking forever is
    /// the failure mode this repo already protects against.
    fn tapped(&self, card_id: &str, source_id: &str);
}
```

`drain_device_events` (`crates/app-core/src/runtime/mod.rs:1053`) grows one arm: a
`Tap`/`StartPause` whose card is a `CardSettings::Picture` calls the sink with the card
id and its `source_id`, and does not reach `control_pomodoro`. `Tap`/`Reset` on a picture
is dropped and counted — no such config exists, but the device is untrusted input.

The server's implementation sends on an unbounded `tokio` sender and returns; everything
after that happens on the tokio runtime, never on the runtime worker thread.

### 3. Routing: from a tap to a render

`data_cards` resolves the `source_id` to its spec:

- No spec, or a spec whose face is not in the catalog, or a face whose descriptor does
  not declare `tap`: the tap is dropped and a diagnostic counter is bumped. An external
  producer's picture card lands here, which is the correct outcome for C1.
- Otherwise the tap is delivered to **that card's existing refresher task** (a
  `tokio::sync::mpsc` of capacity 1 per task, created with the task). It is never a new
  task, so a tap and a scheduled refresh cannot race on one card's state, and the
  existing "a replaced refresher's late verdict is ignored" behaviour keeps holding.

The refresher's loop becomes a select over its interval and its tap channel. On a tap it
renders at once with `event: {taps: n}`, then resumes its normal schedule from that
moment, so a tapped card does not also refresh a second later.

**Coalescing.** Taps arriving while a render is in flight accumulate into a count and
produce exactly one further render carrying that count. Three quick taps on Hacker News
therefore advance three pages with at most two renders. The count is capped at 32 so a
stuck finger cannot ask a face to walk a thousand pages.

### 4. The seam: `render` gains state and an event

`companion/faces/src/main.ts` keeps its two verbs. `describe` gains two optional fields
per face, and `render` gains an input and an output.

**`describe`** entry, additions marked:

```json
{
  "kind": "hackernews",
  "label": "Hacker News",
  "fields": [ ... ],
  "tap": "Tap the panel for the next stories."   // NEW, absent when the face ignores taps
}
```

The server treats `tap` as "this face handles taps" and hands the sentence to the window
verbatim. No Rust knows what any face does with a tap.

**`render` stdin** gains two optional members:

```json
{
  "kind": "hackernews",
  "settings": { ... },
  "state": { "page": 2, "stories": [ ... ] },       // NEW: what this face last returned
  "event": { "taps": 1, "point": null }             // NEW: absent for a scheduled refresh
}
```

`point` is always `null` in C1 and exists so that C2's wire change adds nothing to this
contract: a face written today keeps working, and a face that hit-tests a point works the
day the firmware ships one. `v2.md` supplies points in the logical 448x368 landscape
space at both mountings — the same space the face drew in — so a face hit-tests the raw
point and **the device never needs a region registry**.

**`render` stdout** becomes a JSON envelope:

```json
{ "png": "<base64>", "state": { "page": 3, "stories": [ ... ] } }
```

`state` may be omitted to leave the stored state unchanged, or `null` to clear it.

**Both halves deploy independently** (`deploy.sh --faces-only` ships faces without the
binary, and the binary ships without them), so each side accepts the other's older form:

- The server sniffs stdout. A body starting with the PNG magic `89 50 4E 47` is a frame
  from an older package and is used as today, with the stored state untouched. Anything
  else is parsed as the envelope.
- A newer package receiving no `state` and no `event` — an older server — renders its
  resting view, which is what every face does today.

Exit codes are unchanged: 0 is a result, 2 is `ConfigurationError` ("the owner must act"),
anything else is transient. **A failed render keeps both the stored frame and the stored
state**, so a 429 on a tap does not silently lose your place.

The base64 costs about a third more bytes through the pipe for a ~40 KB PNG, on a
subprocess call that already fetches over the network. `faces_package::run`'s exit-status
wait and 250 ms EOF grace are unchanged — the pipe-EOF trap it documents still applies.

### 5. State storage

Per `source_id`, in `face-state.json` beside `data-cards.json` under
`DESKMATE_CONFIG_DIR`:

```json
{ "version": 1, "sources": { "hn": { "page": 3, "stories": [ ... ] } } }
```

- **A separate file from `data-cards.json`**, which holds what the owner typed and must
  not churn on every tap. A corrupt or absent state file is "no state", never an error:
  faces redraw their resting view and the file is rewritten on the next render.
- **16 KB per source**, encoded. A face returning more gets a `ConfigurationError`-shaped
  log line and its state is left unchanged; the frame is still published. The cap is what
  lets Hacker News keep its fetched stories in state and page without a network round
  trip.
- Written atomically (temp file plus rename) the way `persist_specs` already writes
  specs, and only when the rendered state differs from the stored one.
- Removing a face removes its state. `face-state.json` is not deployed, not read by the
  window, and safe to delete.

### 6. The window

One line, from the descriptor's `tap` sentence, under the face's fields in the editor.
Nothing else changes: the preview is still the stored frame decoded back to a PNG, and a
tapped face previews correctly with no new code. This is `bun run build` plus an rsync of
`dist/` — no Rust build, no restart.

## Phase 1: the two faces

**A tapped view is temporary.** Both faces record when they were last tapped. A scheduled
refresh more than **10 minutes** after the last tap returns to the resting view; one
sooner keeps the tapped view. A panel on a desk settles back to what it is for, and you
never walk past page 4 of an hour-old front page.

- **Hacker News.** A tap pages to the next stories and wraps at the end. The face fetches
  five pages' worth of the front page (`MAX_STORIES` is 4, so 20 stories) and keeps them
  in its state, so a tap redraws with no network access at all — the fastest the path can
  be, and therefore the honest measurement of what a tap feels like. The eyebrow gains a
  page indicator. That is a visible design change to a face the owner has seen, so it
  goes to the owner as `bun run dump` PNGs — the `.desk.png` ones especially — before it
  ships. The lone-lead planner trap applies: the new cases come from captured front
  pages, not invented ones.
- **Weather.** A tap flips between current conditions and the coming days; tapping again
  flips back. The approved design is the resting view and does not change a pixel. The
  flipped view leads with **tomorrow in the same hero component** the approved view uses --
  eyebrow, numeral, words, glyph, range -- with the four days after it in the strip. A first
  draft spent that module on the words "Coming days" beside an icon, which is the label above
  a heading `DESIGN.md` rules out; both views now draw one hero, so they cannot drift. The
  forecast view is a new layout, so it also goes to the owner as PNGs for approval before
  it ships. `forecast_days` moves from 2 to 5 in the same Open-Meteo request. Tracked
  runs use `trackedWidth`/`fitTracked`, and every coordinate goes through `kit/svg.ts`'s
  `fixed`, never `toFixed`.

The existing goldens (32, not the 19 `CLAUDE.md` quotes) stay byte-identical; new
goldens are added per new view.

## Phase 2: the stream deck

Starts only after Phase 1 has been observed working on `dev-0005`.

A face drawing one large icon and a label, with its action in `data-cards.json`: a URL, a
method (`GET` or `POST`), an optional body and optional headers. A tap fires it through
`faces/src/kit/http.ts` — the same guard, so private, loopback and metadata addresses are
refused on every redirect hop and the resolved address is pinned — then redraws with the
outcome. A failure is a `face_status` sentence in the window, because the panel has three
states for a picture and "your webhook returned 500" is not one of them.

**One action per card.** Without a point on the wire the device can only say "this card
was tapped", so a grid of icons is C2. Several one-action cards in the loop work today,
and the grid becomes a layout change to the same face once the point exists.

## Not in this track

- **Coordinates and device-drawn press feedback** are C2, together in one firmware image
  and one flash session, per the batching trap.
- **Track G, the desk agent**: a headless daemon on the owner's machine that runs local
  actions (open a URL, focus an app, run a macro) that no server can perform. It waits on
  C1 Phase 2 and on Track A for an identity. One stance is fixed now because it shapes
  that protocol: **the server sends an action id, never a command.** The agent's own local
  config maps an id to what it does, and anything not in that file is refused — otherwise
  a stolen admin token is arbitrary code execution on the owner's machine via a card tap.
- **Bluetooth**: raised and dropped on 2026-09-23. BLE is the wrong pipe for 61 KB frames,
  and enabling the controller is the largest firmware-statics change this project could
  make on a device whose IRAM is already full. Cable-free provisioning over BLE remains a
  fair question for whichever track sells panels; it is not C1's.

## Testing

- **Faces**: golden SVGs per view, byte-exact via `fixed()`; state transitions (tap,
  wrap, the 10-minute reset) as unit cases; cases built from captured responses.
- **Server**: routing to the right refresher, the drop paths (no spec, no `tap` in the
  descriptor, external producer), coalescing under a render in flight, the 16 KB cap, the
  atomic write, a corrupt state file, and both compatibility directions of the envelope —
  against a fake faces package, as `data_cards`'s tests already do.
- **app-core**: the new `Tap` arm calls the sink for a picture and `control_pomodoro` for
  a pomodoro; a `Tap` for an unknown card is counted, not panicked on; the sink is never
  called from a blocking context.
- **Mutation probes** on the routing and coalescing code, per the repo's standing habit:
  a green suite that survives deleting a guard proves nothing.
- **The window**: built and driven in Chrome against a real server. The mock harness
  cannot see anything that needs the server, and the harness must not offer a tap
  affordance the shipped app lacks.
- **The board**, and nothing may be called working before it: a tap on a Hacker News card
  on `dev-0005` pages the panel, at both mountings; a weather card flips and flips back;
  a redeploy of the new lowering does not produce `StaleRevision`; and the observed
  tap-to-redraw latency is written into `docs/hardware/board-notes.md`. A frame in the
  server's store is not the panel — that mistake is already recorded in `CLAUDE.md` and
  cost this project a "working" token card that did not work.

## Risks

1. **Latency is unmeasured.** A Hacker News frame is 61 KB / 33 chunks and nobody has
   measured a full tap-to-redraw over the tunnel. Paging from state removes the fetch,
   which is the only part of the budget this design controls. If the result feels bad, it
   is evidence for C2 rather than a defect in C1, and the number belongs in the board
   notes either way.
2. **`StartPause` on a timerless card is reasoned, not observed.** The code path and the
   protocol document agree, but the board has the last word, and the fleet's protocol v2
   is verified only as far as the link.
3. **Every picture card's wire config changes once** on the first deploy. Expected to be
   an ordinary revision; verified against the live board rather than assumed.
