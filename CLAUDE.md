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
  **v10 is the only config version the store can read** (v4-v9 were retired on
  2026-09-19, once the last pre-v10 documents -- two dead identities' configs -- were
  gone); any other `schema_version` is a typed `UnsupportedVersion` refusal, the refused
  file is never overwritten on load, and the refusal stays visible in the panel's snapshot
  until an explicit successful save. The rule that made that cheap still holds for the
  next bump: a `docs/config/vN.md` exists if and only if the store can still read vN.
- **Three card kinds: clock, pomodoro, picture.** Clock and pomodoro tick on the device
  between host pushes. A picture card's face is a raster frame, frozen between pushes,
  from one of two producers: an external one POSTing a PNG, or **the server itself**. The
  device-rendered data cards (calendar, weather, json-feed, rss) went in v8 and
  manifest-based plugins in v9, and neither is coming back as a card kind.
- **The server-rendered faces -- weather, Hacker News, RSS, token -- are TypeScript, in
  `companion/faces/`, and the server runs them as a subprocess**
  (`docs/images/server-rendered-cards.md`). They were Rust until 2026-09-20
  (`crates/server/src/faces/`, `crates/providers`); the owner moved them for maintenance.
  These are **not new card kinds**: each is an image source the server pushes to itself,
  named by an ordinary picture card, configured in `data-cards.json` under
  `DESKMATE_CONFIG_DIR` (or `DESKMATE_DATA_CARDS`). Schema stays v10 and the wire is
  untouched. The split: `crates/server/src/data_cards.rs` owns the specs, the browser's
  settings contract, validation and the refresh schedule, and **names no face kind**; the
  package owns what a face fetches and how it is drawn. The seam is two verbs
  (`data_cards/faces_package.rs` <-> `faces/src/main.ts`): `describe` prints the catalog
  the add menu is built from, `render` turns `{kind, settings}` on stdin into a PNG on
  stdout that goes through `canonical_frame_from_png` like any producer's POST. Exit 2
  means "the owner must change a setting", anything else non-zero is transient, and
  either keeps the stored frame. The child's environment is cleared -- the server's holds
  the admin token. **Adding a face is a file in `faces/src/faces/` plus
  `deploy.sh --faces-only`: no Rust, no restart**, because `DESKMATE_FACES_DIR` is read
  per refresh and the catalog re-read every minute, exactly as `DESKMATE_WEB_DIR` is.
  - **The server makes no outbound GET again.** The faces fetch from their own process,
    behind their own guard (`faces/src/kit/http.ts`: private/loopback/metadata refused on
    every redirect hop, body cap, deadline, and the resolved address PINNED -- the
    request dials the validated IP with the name in `Host` and as the TLS server name,
    because Bun's `node:https` `lookup` override breaks certificate checking and its
    `fetch` would resolve the name a second time). `egress.rs` is back
    to one host, `oauth2.googleapis.com`, POST only.
  - **A face is rastered, not composed as a scene**, because `SCENE_MAX_NODES` is 24, a
    `SceneLine` holds 8 points, and only the Caption and Body baked tiers contain
    letters; the frame then rides the picture card's existing asset path, so there is
    still exactly one *scene* renderer. `@resvg/resvg-js` is the same `resvg` engine the
    Rust faces used, which is what let the port be accepted on **byte-identical SVG**:
    `faces/test/golden/` holds the Rust renderer's own output for the 19 cases that
    existed, and the owner-approved weather design moved without a pixel changing.
  - **The window's preview of a picture card is the stored frame**, decoded back to a PNG
    (`image_ingest::png_from_canonical_frame`). Nothing is rendered for it, so this is
    not a second renderer. Until 2026-09-20 every picture card previewed as a sentence on
    black, which made a freshly created face look unfinished.
- **Every card face is a host-pushed scene**, and there is exactly **one renderer**. The
  hand-written C templates stopped shipping at stage 3a and their reference oracle was
  deleted on 2026-09-11. The companion's card preview builds the same scene
  `build_card_scene` would push, so preview and panel agree by construction. **Do not
  add a second renderer to "check" the first** -- that is what the retired parity gate
  was, and it kept three dead templates alive to have something to compare against.
- **THE COMPANION IS A WEB APP, and there is no desktop app.** Every surface for a
  networked device is served by the server it configures, at
  `https://deskmate.rodi.one/`. The browser API is `crates/server/src/app_api/`, gated by
  the operator session cookie the page trades the admin token for. Nothing in the tree is
  scaffolding for a desktop app: if one is wanted it is written from scratch, and it
  talks to the server like the page does. The React source lives under `apps/deskmate/src/`: `src/lib/backend.ts` is a one-line barrel over `./backendClient`, and
  `vite.config.ts` aliases that single specifier to `src/dev/backendClient.ts` under
  `VITE_DESKMATE_MOCK=1` -- so there is exactly one seam and three implementations of it
  (HTTP, mock, and whatever comes next). Design:
  `docs/superpowers/specs/2026-09-18-deskmate-web-companion-design.md`.
  - **Gone with it, deliberately**: provisioning, factory reset, the local-ownership
    switch, autostart, and the whole local tier. They are cable operations, a browser has
    no cable, and `crates/deskmate-cli` performs all of them. They were deleted rather
    than stubbed -- a control that always fails is worse than an absent one.
  - **The SPA is served from a directory, never embedded.** `DESKMATE_WEB_DIR` points at
    a built `dist/`, read per request, so a UI change is `bun run build` plus an rsync
    with no Rust build and no restart. That is the whole point; do not "simplify" it into
    the binary.
- **Live deployment**: `deskmate.rodi.one` on the owner's homelab (docker-vm, reachable
  over Tailscale), behind Cloudflare -> cloudflared -> Caddy, systemd unit in
  `companion/crates/server/deploy/`. Built for linux/x86_64 in a throwaway
  `rust:1.98-bookworm` container over an rsync'd `git archive HEAD` export -- never the
  working tree. **The export is `companion/` AND `firmware/` now**, because the server
  compiles the panel's own LVGL for the card preview; `firmware/managed_components/` is
  gitignored and is the one part copied from the working tree, pinned by the tracked
  `firmware/dependencies.lock`. Device URL `wss://deskmate.rodi.one/v1/device/link`.
  **The faces ship separately, as a directory**: `deploy.sh` rsyncs `companion/faces/`,
  runs `bun install` and the faces suite **on the VM** (its resvg build is not the
  Mac's), and only then installs into `/var/lib/deskmate/faces`. The VM needs
  `/usr/local/bin/bun`; the script says how if it is missing.
  - **No edge auth, on the owner's decision (2026-09-18).** The app is its own gate:
    `/v1/app/*` and `/v1/manage/*` need the operator session cookie traded for
    `DESKMATE_ADMIN_TOKEN`, `/v1/device/*` and `/v1/images/*` need their own bearers, and
    `/v1/firmware/*` is unauthenticated by design. A Caddy `basic_auth` gate stood in
    front of the browser paths for a few hours and was removed as a second password for
    the same person. Publicly fetchable as a result: the JS bundle and the login form.
    That is fine against a 256-bit random admin token and the constant-time comparison,
    and **would stop being fine the moment that token became human-chosen** -- adding a
    login rate limit is the prerequisite for any such change, because the server has none.
  - **If an edge gate is ever restored it MUST be scoped by path.** The board and the
    picture producers send their own `Authorization` header, which `basic_auth` consumes,
    so a host-wide gate breaks the device link outright. That is why the removed block
    gated only `/`, `/assets/*`, `/v1/app/*` and `/v1/manage/*`. Cookie and Basic do not
    contend, so the two gates can coexist -- they just are not both wanted.
- **Milestones**: V1 and V2 are exited and tagged (`v1` at `7abd496`, `v2` at `fdf85ba`).
  V3 (server host) is in progress; **sub-projects 1-4 are merged to `main`** (`fb15060`,
  2026-09-13) and V3 is **schema- and wire-neutral** -- it adds no config field, no wire
  message and no firmware change, so merging it does not move the flash. What V3 still
  owes is the reference Google Calendar producer in `tools/picture-producers/` and the
  end-to-end gate, which needs the panel. Tags require explicit owner
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
- **The dev harness cannot see anything that needs the server.** `VITE_DESKMATE_MOCK=1`
  renders every scenario without a backend, which is what makes it useful and what makes
  it blind: it mocks the preview (`src/dev/mockPreview.ts`), so a broken Rust preview
  path renders perfectly, and until 2026-09-18 it had no signed-out state, so the window's
  sign-in dead end survived every test. `crates/lvgl-sim/tests/preview_path.rs` is the
  honest check for the preview; **driving the real build in Chrome is the honest check for
  the window**, and it is cheap now that the product is a web page. Two defects were found
  that way in one sitting, both invisible to 122 passing frontend tests.
- **For a tests-only change, "the suite passes" proves nothing -- the change IS the suite.**
  `tools/testgate/` has the two gates that carried the 2026-09-19 consolidation: production
  byte-identical above every `#[cfg(test)]`, and zero production lines lost from coverage
  (deterministic here, so the threshold is zero). Its README lists three ways that gate lied
  first; read it before trusting a number from it.
- **The DOM suite cannot see a stylesheet, so every CSS mutation survives it.** happy-dom
  never applies one: deleting a selected-state rule, swapping an error colour and breaking
  a transform origin each left all component tests green. For a CSS or markup refactor
  the check is `tools/webcheck/pixelab.py` (is build B pixel-identical to build A across
  252 states); for anything that needs the server it is `tools/webcheck/smoke.sh`. Both
  READMEs say why a naive screenshot diff lies: capture A and B in ONE browser launch,
  and drive the clock rather than merely setting it.
- **A harness must not offer what the shipped app does not have.** When the cable
  operations were removed, they were removed from `src/dev/backendClient.ts` too. A mock
  that still answers `provision_device` lets the UI be built against an affordance nobody
  can reach -- the harness would then be exercising a product that does not exist.
- **No caller may wait unboundedly on a thread that can block in an OS read.**
  `SerialTransport::read` can block forever once the USB device behind its fd is gone.
  That can wedge the whole host process: the server links `device`, and `deskmate-cli`
  opens serial ports. The protection lives in `device::session`'s worker (bounded reply wait,
  stalled-session marking, a Drop that never joins); on 2026-09-19 everything else in that
  file that only the deleted desktop host used was removed and the worker was left textually
  alone -- NOT verified on hardware. A stalled session reports
  `Transport(Disconnected)`, never `Timeout`, because `is_disconnect` is what makes the
  runtime reconnect.
- **Two assertions that look like proof and are not.** `value["key"].is_null()` on a
  `serde_json::Value` is true for a MISSING key, so it cannot pin "serialized as an
  explicit null" -- assert `value.get("key") == Some(&Value::Null)`. And a `tracing`
  macro takes its target from the enclosing module, so MOVING a `warn!` silently changes
  its target and any filter keyed on it -- pass `target:` explicitly when moving one.
  Both survived green suites during the 2026-09-19 sweep and were caught only by mutation.
- **A red CI is not background noise here -- it was, for four weeks.** From 2026-08-23
  to 2026-09-19 the firmware host tests failed on Linux only (`-std=c11` makes glibc hide
  `gmtime_r`; Apple's headers declare it regardless, so every Mac was green). The job
  died before its artifact upload and `companion`, which `needs` that artifact, was
  SKIPPED on every push: no Rust or frontend gate ran in CI through the move to the
  browser, the Tauri deletion and the engine fold. The host tests now have their own job
  so they fail loudly instead of switching the rest off. Run `gh run list` before
  believing CI agrees with your laptop; a skipped job is grey, not red.
- **Green tests say nothing about whether OTA still works**, and `make -C
  firmware/host_tests sanitize` is not optional: two `core/scene_decode.c` bounds guard
  out-of-bounds *writes* that `scene_model_validate()` then reports with the same error
  code the test asserts, so the plain suite passes against a decoder with both deleted.
- **Provisioning is a cable operation by design.** In networked tier the cable is the
  configurator and the server is the owner. The runtime has no provision or factory-reset
  path at all; `deskmate-cli` drives them through `device::session` over USB. Do not add
  provisioning over the tunnel without specifying it first.
- **The device gives `esp_websocket_client_send_bin` 200 ms to deliver a reply the host
  waits 2000 ms for.** A transient stall destroys a reply the host would still accept.
  Deliberately unfixed -- no evidence it bites at realistic cadence -- but know it exists.
- **`ota_mark_running_image_valid()` is deliberately free of every dependency that can
  fail.** No network, server, config or NVS content may stand between boot and marking
  the image valid, or `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE` rolls it back.
- **The board is normally powered off.** A disconnected `dev-0005` or an idle-timeout
  close is the resting state, not a fault.
- **`textWidth` (`companion/faces/src/kit/`) does NOT account for `letter-spacing`.**
  Every eyebrow on a server-rendered face is tracked out 1.4px, so a 27-character run
  draws ~38px wider than it measures and silently overhangs the canvas -- a fixed panel
  has no scrollbar and no clipping artifact to show you. Use `trackedWidth`/`fitTracked`
  for any tracked run. Every fixture place name was short, so only a live response
  caught it -- and the same is true of the Hacker News planner, whose lone-lead fallback
  outbid its own index until a real front page with a short top story arrived. **Build a
  face's cases from captured responses, not from invented ones.**
- **A golden SVG in `companion/faces/test/golden/` is byte-exact, and `fixed()` is why it
  can be.** Rust's `{:.2}` rounds an exact tie to even and JS `toFixed` rounds it away
  from zero; 8px-grid arithmetic produces exact ties (n/8 is representable), so a port
  using `toFixed` differs in the last digit of a few coordinates. Go through
  `kit/svg.ts`'s `fixed`, never `toFixed`, for anything that reaches the document. A
  deliberate design change is `bun run dump --update`, reviewed as a diff.
- **A second save control beside "Save to server" silently discards what it guards.** The
  server faces' fields once had their own small "Save source settings" button. Type a coin,
  press the prominent save, and the coin was gone: the face stayed blank, was never
  fetched, the panel said "Waiting for the first picture" and the window said "Saved to
  the server". Face fields now save themselves (blur, Enter, at once for a choice). **Do
  not add a control whose unsaved state the window's one save ignores.**
- **A face that cannot draw must say so in the window, because the panel cannot.** The
  device has three states for a picture card -- waiting, stale, drawn -- and none of them
  is "your coin was not found" or "the API said 429". `face_status` on `GET /v1/images`
  carries the faces package's own sentence to the editor. A new failure mode in a face is
  a `ConfigurationError` (the owner must act) or a `TransientError` (retried in a minute),
  never a log line only.
- **The server's frame store is not the panel.** On 2026-09-20 the token card was driven
  end to end "against a real server", pronounced working, and did not work on the device.
  A frame in the store proves the fetch and the drawing. It says nothing about what the
  owner typed into the live window, what the live VM's IP is allowed to fetch, or what the
  tunnel carries. Say which of those was checked, in those words.
- **Waiting for a child's pipe to close is not waiting for the child.** EOF needs every
  holder of the write end gone, and an unrelated process can inherit one: on macOS a pipe
  gains `CLOEXEC` non-atomically, so a concurrent fork elsewhere in the process races it.
  `faces_package::run` therefore waits on the exit status, gives EOF 250 ms, and takes
  what was collected. It surfaced as a flaky test, and would have been a wedged refresh.
- **An HTTP client's default `User-Agent` gets a 403 from a Cloudflare-fronted API**
  before it reads the path -- which is why a CoinGecko fetch failed while `curl` to the
  same URL worked. `companion/faces/src/kit/http.ts`'s `USER_AGENT` covers the faces;
  `tools/picture-producers/claude_limits_png.py` carries the same note for urllib.

### Product rules the owner has set

These are decisions, not defaults. Changing one needs the owner, not a judgement call.

- **A NEW DATA FACE IS A SERVER-SIDE PRODUCER, NEVER A NEW CARD KIND.** A card gets its
  face exactly two ways: on-device (clock, pomodoro, which must tick between host
  pushes), or as a raster frame a producer pushes to an image source. Anything that does
  not need device-local fast refresh is a script on the server that renders a picture.
  Weather, RSS and a token price are producers. **Do not add a card kind for one, and do
  not treat a complaint about the settings window as permission to.** This was violated
  on 2026-09-16 by `e2b103c` (schema v11, six card kinds, 42 files, an ordered two-stage
  deploy) in response to a request that was purely about what the tiles looked like; it
  was reverted the next day. The rule was already written here and in `324c981`/`e137294`
  at the time.
- **The three expensive boundaries are `CURRENT_SCHEMA_VERSION`, the wire, and firmware
  statics.** Cost in this repo is set by which boundary a change crosses, not by how many
  lines it touches. A schema bump forces a migration, a redeploy, an app rebuild and a
  strict deploy order; a wire or firmware-statics change adds a USB flash and a mandatory
  on-board OTA re-verification. **Crossing one needs explicit owner authorization**, the
  same as a tag. A change that stays inside `apps/deskmate/src/` costs `bun run build` and
  an rsync of `dist/` -- no Rust build, no restart, because the server reads
  `DESKMATE_WEB_DIR` per request. Reach for that first and say what it would cost before
  proposing more.
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
- **A CARD IS IDENTIFIED BY ITS NAME, and the tile does not classify it.** Superseded
  the template-first rule on 2026-09-18, on the owner's direction: "DIGITAL CLOCK" above
  a clock and "PICTURE" above five different pictures named the category the owner could
  already see, while the line that actually told two cards apart was the small grey one
  underneath. A tile now carries one fact -- the source name for a picture, the word
  "Clock" for a clock, the countdown and label for a pomodoro -- and no template label.
  - **The clock tile does not show the time.** A live clock in the grid was a second
    clock competing with the panel's own, and the per-second re-render of the whole card
    list that drove it is gone with it.
  - **There is no Name field.** It decided nothing: a clock is a clock, a picture is
    named by its source. `title` stays in the schema and keeps its creation value -- this
    changed what is offered, not what is stored. A new clock is created unnamed, because
    seeding a word nothing can now change is the confusion this removed. The pomodoro's
    **"Timer label" is NOT this and stays**: it is drawn on the panel face.
  - **`cardIdentity()` is the one name, and every visible surface calls it**: the tile,
    the loop list under the ring, the editor heading. A clock is "Clock", a picture is
    its source, a pomodoro is its timer label. They cannot drift, because there is one
    function; that is what replaced a rule the code had already half-abandoned.
  - **The accessible names on the move/remove controls keep the template**
    (`Remove Digital clock — Clock`). Not an oversight: it is the only context a
    listener gets, and it is not competing for space on screen.
  - **Two clocks are now indistinguishable**, on every surface, and that is the accepted
    cost of removing the Name field -- there is no longer anything to call one of them.
    Tests needing two tellable-apart cards use pomodoros, whose timer label survives.
- **`DESIGN.md` is the visual language** (The Modular Face); product truth is
  `PRODUCT.md`. `docs/design/companion-visual-language.md` is superseded. A label above a
  heading and a card inside a card are both out. `ui-rounded` is the numeral face: a system
  face, so the page fetches no fonts.
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
  the server drew. Owed: one weather, one Hacker News, one RSS and one token face on
  `dev-0005` at both mountings. The RLE565 sizes are now measured off-panel (token 28 KB /
  15 chunks, weather 32 KB / 17, Hacker News 61 KB / 33 -- the "~10 KB" once quoted was low
  by 3x), but whether the tunnel carries the Hacker News face's 33 chunks is not known.
  **On 2026-09-21 the owner reported that the token card did not work on the device**,
  after it had been verified only as far as the server's frame store. The cause was not
  observed. Two defects that produce exactly that symptom were found and fixed in the
  window and the face (a second save button that discarded the coin; a ticker refused
  silently), and neither fix has been seen on the panel.
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
cd apps/deskmate && bun test && bun run check && bun run format:check && bun run build
cd ../../faces && bun test && bun run check && bun run lint && bun run format:check
```

The faces gates need nothing from the Rust workspace and take about a second. For a
change to a face, the suite is not the real check either: `bun run dump out/` and look
at the PNGs, the `.desk.png` ones especially -- 0.4x is roughly the panel's physical
size, and type that reads fine at 448px wide can be illegible from a chair.

`bun run check` type-checks the tests as well as the app (`tsconfig.tests.json`), so a
test still passing a prop the component dropped is a type error, not a silent pass.

Both test invocations are required: `--all-targets` adds example and integration
targets but removes doctests, so neither invocation alone covers the workspace.
Keep them as separate lines so a failure names the missing coverage directly; do
not simplify them back to one command. The workspace currently has no bench targets.

`bun run build` is in the list because the companion is a deployed artifact: if it
does not build, there is nothing to ship. `format:check` is there because it was silently red for a
while, which is what an unenforced gate does.

**For a change to the window itself, none of the above is the real check.** Build
it, point `DESKMATE_WEB_DIR` at the `dist/`, and drive it in Chrome -- the
mock harness is blind to everything that needs the server, and the product is a
web page now, so this costs a minute. See the trap about the harness above.

Budget for this, re-measured 2026-09-19 at 852 Rust and 171 frontend tests: **the warm
suite is about 28 seconds**, nearly all of it `cargo test --all-targets` (23 s, all of
it test execution). The
35-minute figure this section used to quote is a *cold* run, and it is compilation,
not tests -- quoting it as the everyday cost sent one session at the wrong target.
(Counts on 2026-09-21: 721 Rust, 188 window and 192 faces tests -- the Rust faces and
`crates/providers` left for `companion/faces/`. The timing was NOT re-measured then.)

**This is the local gate suite and has nothing to do with the deploy**, which is 3.8 s
for a no-op and 23 s for a real change (`companion/crates/server/deploy/deploy.sh`).

Where the test time actually goes, so the next person does not have to find out again:

| target | seconds | tests |
|---|---|---|
| `app-core/tests/runtime.rs` (bodies in `tests/runtime/`) | 16.1 | 65 |
| `server/tests/ownership.rs` (bodies in `tests/ownership/`) | 2.2 | 30 |
| `server` unit tests (`src/lib.rs`) | 2.0 | 347 |
| everything else | about 1 or less each | |

`runtime.rs` dominates because each of its 65 tests starts, drives and stops a real
threaded runtime; that is the thing being tested, not waste. `ownership.rs` is named
here because this section used to blame it for the whole cost -- it is under 10% of it.

**A test that waits on production pacing is the failure mode to watch for.**
`server/tests/hostile_device.rs` was 14 seconds -- a third of the entire suite -- for
two tests, because each of its four reconnects paid a real `reconnect_interval` plus
`status_interval`. It is 0.18 s now: `ServerState::in_memory_with_runtime_options`
lets a test pace the device runtime, the way `app-core`'s own tests always could.
Reach for it only when the wait is incidental to what is being tested. `cargo` is not on the Bash tool's PATH (`export
PATH="$HOME/.cargo/bin:$PATH"`), and piping a cargo invocation into `tail` reports
`tail`'s exit status in zsh, hiding a failure as a pass -- redirect to a file and check
`$?`.

Hardware-facing changes also require the on-device checks named in the active plan and
an entry in `docs/hardware/board-notes.md` with the observed result.
