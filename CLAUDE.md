# Deskmate Repository Instructions

This repository is shared between Claude, Codex, and human contributors. This file is
the durable handoff contract, and it is loaded into every session -- keep it to rules,
current fact, and traps that still bite. A narrative of how something came to be belongs
in `docs/history.md`, which nothing loads by default; task-specific findings belong in
the active plan or the hardware notes.

## Sources of truth

Read these before changing code:

1. `docs/superpowers/specs/2026-08-03-deskmate-design.md` - approved product and
   architecture contract.
2. `docs/superpowers/plans/2026-08-03-deskmate-roadmap.md` - milestone order and current
   milestone.
3. The current milestone plan linked from the roadmap - executable checklist and
   acceptance criteria.
4. `docs/hardware/board-notes.md` - verified board facts, component versions, and
   hardware quirks.
5. The frozen contracts: `docs/config/v10.md` (config) and `docs/protocol/v2.md` (wire).
   These state the contracts; nothing else does, including this file.

`docs/history.md` is the narrative record. It is not normative and several of its
statements are marked stale on purpose.

Explicit user direction wins over repository documents. When it changes an approved
decision or milestone scope, update the affected spec/plan in the same change instead
of letting code and documentation diverge.

## Current state

The narrative of how the project got here is `docs/history.md`, which nothing loads by
default. This section states only what is true now.

- **Config schema v10** (`docs/config/v10.md`), **protocol v2** (`docs/protocol/v2.md`,
  frozen; v1 is marked superseded and kept as the record of what flashed firmware
  speaks), `PROTOCOL_CURRENT_CAPABILITIES` **2016**. Protocol v2 is **NOT additive**: it
  removes `PushData` (type 5 is now `PushTimer`), the template/size-class/
  interrupt-policy registry, `ApplyConfig.screens`, `DeviceEvent`'s second identifier,
  the `field.*` scene binding, error codes 11-13 and capability bits 0-4. A v1 host and a
  v2 device do not half-work -- every frame is `VersionMismatch` -- so **flash the device
  first, then redeploy the server**, and expect the board to be unreachable between the
  two. Retired message-type, error-code and capability-bit numbers are never re-issued.
  The device models three things per card and nothing else: its id and tap meaning
  (`ApplyConfig`), its timer (`PushTimer`), and its face (`PushScene`).
  A `docs/config/vN.md` exists if and only if
  the store can still read vN, so the set shrinks as migrations retire; v4 is the oldest
  readable version and a pre-v4 document is a typed `UnsupportedVersion` refusal.
- **Three card kinds: clock, pomodoro, picture.** Clock and pomodoro tick on the device
  between host pushes. A picture card's face is a raster frame, frozen between pushes,
  from one of two producers: an external one POSTing a PNG, or **the server itself**. The
  device-rendered data cards (calendar, weather, json-feed, rss) went in v8 and
  manifest-based plugins in v9, and neither is coming back as a card kind.
- **The server renders weather, RSS and token faces, and therefore makes outbound HTTP
  again** (`docs/images/server-rendered-cards.md`). It did not between `e137294` and
  `feat/server-side-cards`. These are **not new card kinds**: each is an image source the
  server pushes to itself, named by an ordinary picture card, configured in
  `data-cards.json` under `DESKMATE_CONFIG_DIR` (or `DESKMATE_DATA_CARDS`). Schema stays
  v10 and the wire is untouched. Every outbound request goes through
  `crates/server/src/egress.rs` -- the same SSRF guard the plugin system used, restored
  whole -- and `EgressHttpClient` is the only HTTP client the provider layer is given.
  **A face is rastered, not composed as a scene**, because `SCENE_MAX_NODES` is 24, a
  `SceneLine` holds 8 points, and only the Caption and Body baked tiers contain letters;
  the frame then rides the picture card's existing asset path, so there is still exactly
  one *scene* renderer.
- **Every card face is a host-pushed scene**, and there is exactly **one renderer**. The
  hand-written C templates stopped shipping at stage 3a and their reference oracle was
  deleted on 2026-09-11. The settings-window preview builds the same scene
  `build_card_scene` would push, so preview and panel agree by construction. **Do not
  add a second renderer to "check" the first** -- that is what the retired parity gate
  was, and it kept three dead templates alive to have something to compare against.
- **Live deployment**: `deskmate.rodi.one` on the owner's homelab (docker-vm, reachable
  over Tailscale), behind Cloudflare -> cloudflared -> Caddy, systemd unit in
  `companion/crates/server/deploy/`. Built for linux/x86_64 in a throwaway
  `rust:1.98-bookworm` container over an rsync'd `git archive HEAD` export -- never the
  working tree. Device URL `wss://deskmate.rodi.one/v1/device/link`.
- **Milestones**: V1 and V2 are exited and tagged (`v1` at `7abd496`, `v2` at `fdf85ba`).
  V3 (server host) is in progress on `feat/v3-server-host`. Tags require explicit owner
  authorization; `m0`/`m1`/`v1`/`v2` exist and later milestones do not.
- **The legacy subtraction** (spec
  `docs/superpowers/specs/2026-09-11-deskmate-legacy-subtraction-design.md`): all three
  waves are complete and merged, Wave C on explicit owner direction 2026-09-12.
- **The fleet is UP. `dev-0005` is flashed with `v2.1.0-proto2` and linking.** Observed
  2026-09-12 20:11 UTC from the server side, immediately after a redeploy: the board
  re-linked 2 s after the restart, `firmware check` reports `current=v2.1.0-proto2`, the
  published pin matches, and no frame answered `VersionMismatch`. This supersedes the
  "fleet is down" block that stood here from 2026-09-11, and the pre-flash prose at the
  end of `docs/hardware/board-notes.md` ("not reconnected since", "what the flash session
  still owes") is stale in the same way -- see the 2026-09-12 entry appended there.
  **What this proves is the link, and only the link**: protocol v2 negotiates, the board
  runs the published image, and the two halves agree. It is *server-side* evidence. It
  says nothing about the OTA download, the capability word read by name, a face at either
  mounting, or the framebuffer matrix -- all of which remain owed below and are now, for
  the first time since the flash, actually runnable.

### Traps that still bite

Each of these cost this project real time at least once.

- **Any addition to firmware statics can break OTA downloads, unpredictably, with every
  test green.** 105 bytes of `.bss` once did, and a 15 KB *shrink* moves layout exactly
  as a growth does. IRAM is 100% full and DIRAM has ~122 KB of headroom, so this is
  layout, not capacity. **Verify an OTA download on the board after touching firmware
  statics**, and batch firmware changes so the cost is one image and one session.
- **`AssetRelease.digests` is a KEEP-set, not a delete-list**, so an empty desired set
  means "wipe every asset you hold" rather than "do nothing".
- **`AssetBegin` key 3 (`volatile`) is ALWAYS emitted, false included.** Every deployed
  decoder has `REQUIRED_BIT(3)`; the canonical omit-when-false emission rule yields to
  wire history here.
- **A flashed build is reverted within a minute** unless `DESKMATE_FIRMWARE_VERSION`
  moves to match -- the catalog pins the fleet and offers its version in either
  direction. `firmware/version.txt` pins it explicitly; do not rely on `git describe`.
- **A corrected rebuild published under the same version string as a failed one is
  refused forever**, because the refusal keys on the version string, not image content.
  OTA does not update the bootloader, so a rollback test must start from a full flash.
- **A tier round-trip costs a device identity**: returning to networked tier needs a
  plaintext token and only digests are stored.
- **An until-dismissed alert postpones firmware updates indefinitely**, because it keeps
  `interrupt_live` true.
- **Redeploy the server whenever the schema moves.** The deployed binary compiles its
  own `CURRENT_SCHEMA_VERSION` in, so a bump on the app side makes the live server
  reject every save until it is redeployed.
- **`POST /v1/devices/{id}/scene` accepts any revision, and the device does not
  self-heal from a high one.** Push just above the runtime's current counter, never an
  arbitrary large number.
- **WKWebView does not move focus to a `<button>` on mousedown; Chrome does.** The dev
  harness (`VITE_DESKMATE_MOCK=1`) runs in Chrome and cannot see this class of defect,
  and driving the app with `AXPress` fires `click` with no `mousedown` at all. If a
  report is about clicking, the only honest checks are a real pointer event in the real
  app or a unit test replaying the WebKit sequence.
- **The dev harness mocks `render_card_preview`** (`src/dev/mockPreview.ts`), so a broken
  Rust preview path renders perfectly in it. `crates/lvgl-sim/tests/preview_path.rs` is
  the honest check.
- **No caller may wait unboundedly on a thread that can block in an OS read.**
  `SerialTransport::read` can block forever once the USB device behind its fd is gone,
  which once wedged the entire Mac app and presented as "the app can't save to the
  server". A stalled session reports `Transport(Disconnected)`, never `Timeout`, because
  `is_disconnect` is what makes the runtime reconnect.
- **Reach for `sample <pid>` before reading code** when the app is wedged: the settings
  preview keeps ticking while the worker thread is dead, so the UI looks alive.
- **Green tests say nothing about whether OTA still works**, and `make -C
  firmware/host_tests sanitize` is not optional: two `core/scene_decode.c` bounds guard
  out-of-bounds *writes* that `scene_model_validate()` then reports with the same error
  code the test asserts, so the plain suite passes against a decoder with both deleted.
- **Provisioning is a cable operation by design.** In networked tier the cable is the
  configurator and the server is the owner; `WebSocketRuntimeDevice::provision` returns a
  typed unsupported-on-this-transport error. Do not add provisioning over the tunnel
  without specifying it first.
- **The device gives `esp_websocket_client_send_bin` 200 ms to deliver a reply the host
  waits 2000 ms for.** A transient stall destroys a reply the host would still accept.
  Deliberately unfixed -- no evidence it bites at realistic cadence -- but know it exists.
- **`ota_mark_running_image_valid()` is deliberately free of every dependency that can
  fail.** No network, server, config or NVS content may stand between boot and marking
  the image valid, or `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE` rolls it back.
- **The board is normally powered off.** A disconnected `dev-0005` or an idle-timeout
  close is the resting state, not a fault.
- **`face_render::text_width` does NOT account for `letter-spacing`.** Every eyebrow on a
  server-rendered face is tracked out 1.4px, so a 27-character run draws ~38px wider than
  it measures and silently overhangs the canvas -- a fixed panel has no scrollbar and no
  clipping artifact to show you. Use `svg::tracked_width`/`svg::fit_tracked` for any
  tracked run. Every fixture place name was short, so only a live response caught it.
- **`reqwest` sends no `User-Agent` unless one is set**, and a Cloudflare-fronted API
  answers that with 403 before it reads the path -- which is why a CoinGecko fetch failed
  while `curl` to the same URL worked. `egress::USER_AGENT` covers the server;
  `tools/picture-producers/claude_limits_png.py` carries the same note for urllib.

### Product rules the owner has set

These are decisions, not defaults. Changing one needs the owner, not a judgement call.

- **The window has ONE LOOP, and since schema v10 so does the document.** The
  complication tile grid *is* the loop, in loop order, and it is where the order changes.
  Adding a card is one dashed slot at the end of the grid, and appending to `cards` IS
  joining the loop. `playlists[]` and `active_playlist_id` are gone; `cards` is ordered,
  dwell is a card field, and `advance` is one document-level setting. Two tile states
  went with them and must not come back, because neither is representable: a card
  outside the loop ("not in loop" / "Add to loop"), and a loop position whose card id
  does not resolve ("Missing card"). Do not move creation or deletion into the rail: the
  rail is the face and has a fixed height budget.
- **A card that alerts but is never shown is not expressible.** That needed a card in
  the library but outside the playlist, which nothing in the window could arrange. Every
  card is in the loop, so every card is shown and every card compiles to both a widget
  and a screen.
- **A card is called the same thing on every surface, and that thing is its TEMPLATE.**
  `cardLabel()` (the template name, "Digital clock") identifies a card everywhere;
  `cardTitle()` (the owner's words, null when never typed) is a quiet second line beside
  it, never absent, because two cards can share a template. Every control acting on one
  entry names `template — title`.
- **`DESIGN.md` is the visual language** (The Modular Face); product truth is
  `PRODUCT.md`. `docs/design/companion-visual-language.md` is superseded. A label above a
  heading and a card inside a card are both out. `ui-rounded` is the numeral face because
  it is free under the Tauri CSP that blocks every font host.
- **The 2026-08-21 subtraction is permanent.** The four-complication status header, the
  pause-syncing control, the data sources panel, and the preview's label/resolution/
  caption were removed, not moved. Do not reintroduce any of them. Device ownership,
  pairing, link/Wi-Fi/IP/update state and preferences live in the one modal
  `SettingsSheet`, which is the only disclosure in the product. A config that arrives
  already paused gets a one-off "Resume sending" notice; keep that escape hatch.
- **Clock faces carry no title chip and no `DATE` eyebrow**, on any of the clock
  surfaces, and the card hero and fallback hero rails deliberately differ (8 vs 11
  grid units). Do not unify them.
- **The preview shows what a PERSON SEES, never the framebuffer the device receives.**
  It renders upright at both mountings; do not "restore fidelity" by passing
  `preferences.orientation` through. `lvgl-sim`'s own orientation support stays -- the
  hardware framebuffer diff needs both values.
- **The panel is a 448x368 landscape UI.** 90 degrees is the default (USB cable down),
  270 the flipped mounting; the companion setting owns the choice. Do not add a
  device-edge or screen gesture that changes it, and do not expose portrait. There is
  exactly one canvas and one layout per template; do not reintroduce dashboards, size
  classes or a status strip.
- **`AlertHold` is host-side bookkeeping only** and never clears the panel. Do not add a
  wire dismissal message for it.
- **`preferences.timezone` is seeded from the host's IANA zone on FIRST RUN ONLY.** An
  existing saved configuration is never rewritten, because a timezone change moves the
  clock on a physical panel and that is the owner's decision.
- **`feat/display-brightness` is a dead branch kept for reference; do not rebase it.**
  Its "schema v6" collides with the real v6. Reviving brightness is new work: a fresh
  schema bump, a wire change, a firmware change, and an on-board OTA re-verification.

### Hardware verification still owed

- V2 Task 9's tap latency.
- The BUSY/OTA-owner refusal variant (needs a pending OTA in flight).
- Stage 3b's asset-GC teardown, only partially observable on dev-0005's card set.
- The whole of protocol v2 **except the link itself**: the OTA download (mandatory --
  `.bss` moved -104 bytes, within one byte of the shift that broke OTA in `3f2aa03`),
  capabilities reading 2016 by name, a face at both mountings, a pomodoro counting down
  between pushes, and a tap reporting one card id. The link is established and observed,
  so these are runnable rather than blocked.
- A **server-rendered face on the panel**. These frames reach the device over the picture
  card's asset path, which is proven for a producer's PNG but has never carried a frame
  the server drew. Owed: one weather, one RSS and one token face on `dev-0005` at both
  mountings, plus the measured RLE565 transfer size per frame -- the flat-fill argument in
  `docs/images/server-rendered-cards.md` is reasoned, not measured.
- The framebuffer matrix has not been run since Wave A moved it to 44 rows and Wave C
  closed the four `field.*` exclusions, leaving **44 rows / 2 excluded / 42 comparable**
  (the one exclusion is the `progress-ring--running-mid-countdown` push-to-capture timing
  race, which no wire change can fix). The last observed result is **96/10/86 on
  2026-09-06**, which predates v9, Wave A and Wave C alike.

## Working agreement

- Start by reading this file, checking `git status`, and reading the roadmap, active
  plan, and relevant board notes. The worktree may contain another contributor's work;
  preserve it.
- Execute the active plan in order unless a prerequisite or new finding requires a plan
  amendment. Mark a checkbox complete only after its stated verification passes.
- Keep plans live: record material decisions, deviations, exact verification results,
  and blockers as they are discovered. Never claim hardware verification that was not
  observed on the physical board.
- **An unticked `- [ ]` in a plan is NOT evidence that work is outstanding.** Most
  executors here have never ticked a box: roughly 500 open boxes across delivered plans
  describe work that shipped, which makes
  `grep -c '^- \[ \]'` worthless as a progress signal and, worse, invites planning around
  phantom debt. Trust the commits, the plan's own prose/status headers, and
  `docs/hardware/board-notes.md` instead. If you execute a plan, tick as you go; if you
  find a plan whose boxes lie, put a STATUS header at its top saying so rather than
  back-filling ticks you did not verify.
- Write the next milestone plan at the current milestone's exit, using what was learned
  during implementation. Do not start later-milestone breadth early.
- Use conventional commit prefixes (`feat:`, `fix:`, `test:`, `docs:`, `chore:`) when
  creating commits. Do not rewrite shared history.
- Firmware and board support are from scratch. Do not copy code from `rsvpnano` or any
  unrelated project. Official ESP-IDF, component, silicon, schematic, and Waveshare
  reference material may be used for facts and API patterns.

## Engineering constraints

- Keep hardware-independent firmware logic under `firmware/main/core/`, free of
  ESP-IDF includes, and host-test it as plain C. Board-specific I/O stays under
  `firmware/main/board/`; LVGL object construction stays under `firmware/main/ui/`.
- All TCA9554 access must use the singleton `board_io_expander()`. Constructing a second
  expander handle resets the physical chip and can disturb LCD/touch reset lines.
- Guard LVGL calls made outside LVGL callbacks/timers with
  `lvgl_port_lock()`/`lvgl_port_unlock()`. Never mutate LVGL objects from USB or protocol
  callbacks; hand work to the UI/LVGL context.
- Treat all bytes received from the host as untrusted: bound lengths and counts, reject
  malformed or unsupported messages, and recover framing without rebooting.
- Keep diagnostic logs out of the machine-protocol byte stream.
- Preserve the standalone clock on boot, host loss, malformed input, and protocol
  version mismatch.

## Verification

Run the narrowest relevant checks while iterating, then the full applicable set before
handoff:

```sh
make -C firmware/host_tests clean test
make -C firmware/host_tests sanitize
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build
```

`sanitize` is not optional for firmware work: two of `core/scene_decode.c`'s bounds
guard out-of-bounds *writes* that `scene_model_validate()` then reports with the same
error code the test asserts, so the plain suite passes against a decoder with both
deleted. ASan is their only proof, and CI now runs it too.

For companion work, run formatting, linting, and workspace tests from `companion/`:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo test --workspace --doc
cd apps/deskmate && bun test && bun run check
```

Both test invocations are required: `--all-targets` adds example and integration
targets but removes doctests, so neither invocation alone covers the workspace.
Keep them as separate lines so a failure names the missing coverage directly; do
not simplify them back to one command. The workspace currently has no bench targets.

Budget for this: a cold full run is about 35 minutes on the owner's machine -- 13 for
clippy, most of the rest for `--all-targets`, whose `server/tests/ownership.rs` does
real loopback binds. `cargo` is not on the Bash tool's PATH (`export
PATH="$HOME/.cargo/bin:$PATH"`), and piping a cargo invocation into `tail` reports
`tail`'s exit status in zsh, hiding a failure as a pass -- redirect to a file and check
`$?`.

Hardware-facing changes also require the on-device checks named in the active plan and
an entry in `docs/hardware/board-notes.md` with the observed result.
