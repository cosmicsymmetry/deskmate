# Deskmate Repository Instructions

This repository is shared between Claude, Codex, and human contributors. This file is
the durable handoff contract. Keep it concise and update it when a lasting rule,
constraint, or project-state fact changes; task-specific findings belong in the active
plan or the hardware notes instead.

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

Explicit user direction wins over repository documents. When it changes an approved
decision or milestone scope, update the affected spec/plan in the same change instead
of letting code and documentation diverge.

## Current state

- M0 is complete and tagged `m0`.
- M2 (template engine and first widgets) is complete. Its physical exit includes clean
  90°/270° widgets and gestures, protocol/config/data stress, corrected full-power
  replay, and a flat-heap 30-minute mixed soak. The user waived nine repetitions after
  one observed corrected power-cycle replay; do not describe ten M2 cycles as observed.
  No `m2` tag exists because tags require explicit authorization.
- M3 (Tauri v2 companion app) is complete. Its plan is
  `docs/superpowers/plans/2026-08-04-deskmate-m3-companion-app.md`. One long-lived Rust
  runtime must own time, config, all widget data, interaction state, and interrupts so
  power-reset replay cannot reproduce M2's multi-process ownership hole. Tasks 1-7 are
  complete: the strict config/store contract, bounded single-owner runtime, pinned Tauri
  shell and lifecycle, typed IPC, accessible settings experience, and end-to-end
  provider/timer/persistence integration are implemented and tested. Task 8's macOS
  builds, audits, physical-board UI/replay, host sleep/wake, and morning tray-resident
  soak pass. After the settings-owned orientation/clean-canvas amendment passed its
  focused hardware regression, the user explicitly waived repeating the unchanged soak
  and authorized starting M4. No `m3` tag exists because tags require explicit
  authorization. M4 Task 1 is complete: config schema v2, lossless M3 migration,
  deterministic capability-gated compilation, the additive protocol-v1 handshake,
  cross-language contracts, and compatibility fixtures pass. M4 Task 2B replaced the
  widget/screen authoring model with the card model: schema v3 (`cards[]`, `presence`,
  `alert`, `carousel.advance`), lossless v0/v1/v2 migration, host-driven timed rotation,
  and bounded alert triggers compiling to the unchanged wire contract. M4 Task 4 is now
  timed rotation and alerts (delivered by Task 2B) rather than tile dashboards, which
  were cancelled, not deferred. M4's plan is
  `docs/superpowers/plans/2026-08-05-deskmate-m4-v1-completion.md` and its frozen schema-v3
  config contract is `docs/config/v3.md`. The V1 playlists plan
  (`docs/superpowers/plans/2026-08-11-deskmate-v1-playlists.md`) is delivered: schema v4
  replaces per-card `presence` and the global carousel with the card library and named
  playlists, one active, while preserving the wire and firmware. Schema v4 was
  superseded by **v5** on 2026-08-22 (`0c7c467`, "reshape config assets for runtime
  fonts"); the current frozen contract is `docs/config/v5.md`. A server built before
  that commit rejects every save with `schema version 5 is not supported; expected 4`,
  so redeploy the server whenever the schema moves — the app and the server carry
  independent copies of `CURRENT_SCHEMA_VERSION`. That plan also **FIXED** the
  validation-mislabeling defect: a validation failure preserves genuine last-good
  state, remains typed, and is never presented as "your last working settings".
  M4 Task 3 (the `AnalogClock`,
  `BigNumberLabel`, and `IconBadgeText` templates; wire kinds 4-6) is complete —
  `CURRENT_CAPABILITIES` is now `11` (core widgets | config rotation | extended
  templates) — and was physically verified on 2026-08-11, recorded in
  `docs/hardware/board-notes.md` under "Extended templates (M4 Task 3) — verified
  2026-08-11". Capabilities 11, the weather layout at both orientations, all 11 icons
  plus the unknown-ring fallback, the json-feed hero/`--`/truncation/boolean cases,
  the one-line ellipsize, and a byte-flat heap with `ui_queue_high_water` 2 all pass.
  Two items did not close: the `show_seconds`-false analog clock showed an
  undiagnosed wrong time (untested hypothesis: the test config pinned UTC while the
  board sits at UTC+4; the hour/minute angles provably do not depend on the flag),
  and `unknown_field_count` is **not observable on hardware** — it is absent from
  `StatusResponse` and `widget_model_unknown_field_count()` has no callers, so that
  checklist item cannot be closed as written; a host test covers the weather field
  set instead. Do not describe either as verified. The card model
  was physically verified on 2026-08-06:
  eight of the nine checks pass, recorded in `docs/hardware/board-notes.md`. `AlertHold`
  is decided: `hold` is host-side bookkeeping only and never clears the panel, which
  yields the overlay on tap alone. Do not add a wire dismissal message for it. Of the
  two carried defects, the validation-mislabeling defect is fixed as described above;
  the only host-side mechanism capable of losing an alert that fired while unpowered
  was root-caused to a bounded hold expiring from schedule time before delivery and
  fixed by starting that countdown at delivery, with regression tests for bounded and
  until-dismissed reconnect paths. Hardware re-verification passed on 2026-08-15 via
  `companion/crates/app-core/examples/alert_replay_check.rs`: a bounded-hold pomodoro
  completing while the link was down was delivered on reconnect. Note the board has a
  battery, so USB unplug is link loss, not power loss — the true power-loss variant
  was not separately run, and the 2026-08-06 observation was likely also link-loss.
  Three UX findings from that session are recorded in board-notes (takeover face
  indistinguishable from the completed card; sticky unreconciled optimistic red flash
  on tapping a completed pomodoro; host silently ignores stale-token dismissals).
- **The companion app's visual language is now `DESIGN.md` (The Modular Face), at the
  repository root.** Delivered 2026-08-19 on explicit owner direction to replace the
  previous world rather than refine it; the owner pinned the reference ("somewhat
  resemble Apple Watch", a tool that conveys being organised and productive).
  `docs/design/companion-visual-language.md` ("the lit panel") is **superseded** and
  marked so in-file — its `--panel-black` quarantine rule is retired, because the
  successor deliberately uses a true-black ground in both colour schemes. The card
  library is complication tiles, the filmstrip is replaced by `LoopRing.tsx` (arc =
  dwell, same `configDraft` helpers), and `Filmstrip.tsx` is deleted. Product truth
  lives in `PRODUCT.md` at the root. Schema v4 and protocol v1 are untouched: this was
  a presentation change only. Two rules worth not relearning: a label above a heading
  and a card inside a card are both out, and `ui-rounded` (SF Pro Rounded) is the
  numeral face because it is free under the Tauri CSP that blocks every font host.
  A **dev-only browser harness** now exists: `VITE_DESKMATE_MOCK=1 bun run dev` aliases
  the Tauri IPC bridge at `src/dev/`, so every device, provider, ownership and
  validation state renders without hardware (`?scenario=…`, `?theme=…`). It is absent
  from production builds.
- **A subtraction pass followed on owner feedback (2026-08-21), and its rules are
  durable.** The four-complication status header, the pause-syncing control, the data
  sources panel, and the preview's label/resolution/caption were **removed, not moved
  around** — the owner's objection was that the window read as knobs made for their own
  sake. Do not reintroduce any of them. What replaced them: chrome is now a wordmark and
  one Settings button (`TopBar.tsx`); device ownership, pairing and the device's link /
  Wi-Fi / IP / update state live in a modal `<dialog>` (`SettingsSheet.tsx`), which is
  the **only** disclosure in the product; protocol mismatch, runtime and command errors
  are notices in the work column; the one useful thing the sources panel said survives as
  a `stale` flag on the affected card tile plus an inline message and Refresh in that
  card's editor (`lib/providers.ts`). `setPushingPaused` has no everyday control any
  more, so a config that arrives already paused gets a one-off "Resume sending" notice —
  keep that escape hatch. `DeviceHeader.tsx` and `ProviderStatus.tsx` are deleted.
  Preferences (timezone, mounting, start-at-login) moved into the sheet too, under a
  "Display" section; because they are draft state, the sheet renders the **same**
  `SaveBar.tsx` the window does (`variant="sheet"`), since a modal that can strand an
  edit behind itself is a trap. The `Clean 448 x 368 canvas` caption is gone from the
  card editor.
- **A card is called the same thing on every surface, and that thing is its TEMPLATE.**
  The library led each tile with its template ("Digital clock") while the ring legend,
  the playlist row and the editor heading used the owner's title ("Desk"), so one card
  appeared to have two names. The owner's rule, given explicitly after seeing the
  opposite resolution and rejecting it: *a card should say what it is* — "Outside" and
  "Desk" teach a first-time reader nothing where "Weather" and "Digital clock" do. So
  `cardLabel()` (the template name) identifies a card in the library tile, ring legend,
  playlist row, editor heading, picker and error notices, and `cardTitle()` (the owner's
  words, null when never typed) is a quiet second line beside it — never absent, because
  two cards can share a template and the title is then the only thing telling them
  apart. For the same reason every control acting on one entry (move up/down, remove,
  dwell) names `template — title`. Two regression tests pin this. `cardName()` still
  exists for the old "title with a kind fallback" shape; prefer `cardLabel`/`cardTitle`
  for anything user-visible.
- **V2's exit gate is OPEN, but no longer blocked.** Task 11 Step 11 failed on hardware
  on 2026-08-19 and **passed on a re-run the same day** after two real defects were
  fixed; what remains open is the gate's own unobserved items, not a blocker. The first
  physical session found and fixed three firmware defects on paths that had been marked
  complete on software grounds. Do not describe V2 as software-complete.
  - `62e5aea` — `CONFIG_MBEDTLS_INTERNAL_MEM_ALLOC` confined TLS to internal DRAM, which
    LVGL and WiFi had already spent, so `mbedtls_ssl_setup()` failed before any socket
    work. **The networked link had never once worked on hardware.** Fixed by
    `CONFIG_MBEDTLS_EXTERNAL_MEM_ALLOC=y`. Note `free_heap` reads ~8 MB throughout —
    that is PSRAM, so it is not a TLS health signal on this board.
  - `77f0e52` — passing the same partition as `.staging` and `.final` left
    `handle->partition.final` unassigned in `esp_https_ota_begin()`; the OTA download
    panicked LoadProhibited before writing a byte. Leave `.final` NULL.
  - `0ad1a51` — the hardware AES accelerator's DMA buffers must come from internal RAM,
    so **two concurrent TLS sessions cannot coexist**: a download alongside the live WSS
    link dies on `esp-aes: Failed to allocate memory`. `install_update()` now suspends
    the link for the duration and resumes it on every failure path.
  - **Closed: the OTA deferral, which needed two fixes, not one.** `3f83911` — the boot
    check waited on `wait_for_wifi()`, i.e. the radio, while the deferral gate reads
    host-pushed state; it now also waits on `protocol_task_owner_state_ready()` (link
    online **and** widget config present) for at most 60 s. **That wait is bounded and
    fails open on purpose** — a device whose owner can never become ready must stay
    updatable, or a bad config strands it. `12e5f4d` — the server built a
    `RuntimeHandle` per WebSocket and shut it down on close, so a link drop discarded a
    running pomodoro; the runtime is now per-device and long-lived, with sockets
    attaching as replaceable transports pinned by a generation, and a reconnect replays
    time, layout, fields, active screen and interrupts. Neither fix works alone: the
    first gives the guard a link to wait for, the second a running timer to see.
    Verified on the board 2026-08-19 — `ota` stayed `idle` for a full five-minute timer
    with an update genuinely available, and installed within a second of the blocking
    state clearing. **`connected` in `GET /v1/devices/{id}` now means "a socket is
    live", not "a runtime exists"** — a retained runtime returns a `snapshot` while
    disconnected, which is intended.
  - Verified and passing: the server owns the device, providers run server-side, real
    weather renders with the Mac quit, a tap travels up and state comes back down,
    pulling USB changes nothing, the Mac app holds the cable without taking the display
    (`reconnects: 0`), OTA installs and reboots, and **rollback works unattended** —
    otadata marks the broken image `ABORTED` and the previous slot `VALID`.
  - Still owed: Task 8's widening-backoff observation on a shipping build, and Task 9's
    tap latency. (The rotation-stall anomaly was retracted; there is no defect.) The OTA
    observability gap is **partly** closed: additive protocol-v1 `StatusResponse` key 30
    carries bounded `last_ota_error`, `GET /v1/devices/{id}` exposes it under
    `snapshot.device`, and it was verified on the board.
  - **`3f2aa03` was a REGRESSION that broke OTA downloads; `5699f1d` fixes it and the
    fix is verified on the board.** Do not flash anything between those two commits. Bisected on the board 2026-08-20 to **~105 bytes of static internal DRAM**
    (`s_last_error` and its `portMUX_TYPE`) — not to any of its logic. Three builds from
    the identical base decide it: the control downloads and installs, the same base plus
    those statics fails, and it fails whether or not a critical section touches them.
    Also ruled out: the custom HTTP event handler (refuted from IDF sources *and* by
    removing it and still failing), the image, the server, the tunnel, the network.
  - **This is layout, not capacity, and it is a standing hazard.** `idf.py size` reports
    **122 KB of static DIRAM headroom** (219307/341760 as of `5699f1d`; it reads
    219387 as of `300d91f`, and the ~80-byte difference predates the scene nodes —
    do not read a fresh total as evidence about a recent change without a
    before/after on the SAME tree) and **IRAM 100% full**, so 105
    bytes cannot be exhausting a budget; shifting `.bss` moves the runtime heap and
    something on the TLS/AES path — which `0ad1a51` showed needs DMA-capable *internal*
    RAM — stops finding what it needs. Same class as V1's boot crash-loop, which was
    also a memory-layout shift. **Assume any addition to firmware statics can break OTA
    downloads, unpredictably, with every test green**, and verify the download on the
    board after touching firmware statics. `5699f1d` keeps the reason out of `.bss`: stage,
    `esp_err_t` and HTTP status pack into two lock-free 32-bit atomic halves and the
    string is formatted on read into the PSRAM-resident status buffer, taking `ota.c`'s
    `.bss` from 9 to **17** bytes rather than ~114. Note a 64-bit atomic is **not** an
    option here — GCC emits `__atomic_load_8`/`__atomic_store_8` and ESP-IDF backs those
    with a global `portMUX`. Two lessons stand regardless:
    green tests say nothing about whether OTA still works, and `last_ota_error` is what
    made its own regression diagnosable, which argues for the field rather than against
    it.
  - Traps learned on the board, all still true: **a flashed build is reverted within a
    minute** unless `DESKMATE_FIRMWARE_VERSION` is moved to match, because the catalog
    pins the fleet and offers its version in either direction — a downgrade path exists
    by design. **A tier round-trip costs a device identity**, since returning to
    networked needs a plaintext token and only digests are stored. **An
    until-dismissed alert postpones firmware updates indefinitely**, because it keeps
    `interrupt_live` true. `firmware/version.txt` now pins the version explicitly; do
    not rely on `git describe`, which serves a stale cached string from a dirty tree.
  Plan `docs/superpowers/plans/2026-08-18-deskmate-v2-networked-device.md`, branch
  `feat/v2-networked-device`. The device joins WiFi, dials out over WSS to a single-tenant
  server, and that server owns it through the same `RuntimeDevice` seam the Mac app uses —
  so ownership has one implementation, not two that must agree. Providers run server-side,
  which is what makes the display work with the Mac quit. The wire stays **protocol v1**
  and the config schema stays **v4**; V2 is additive only. New message types are 13
  (`NETWORK_CONFIG`) and 14 (`FACTORY_RESET`); `CURRENT_CAPABILITIES` is now **203**
  (core widgets | config rotation | extended templates | firmware update | networking).
  It read 75 for most of V2 because bit 7 was defined in Task 1 and never switched on;
  the whole-branch review caught it. `docs/protocol/v1.md` gates NetworkConfig and
  FactoryReset on that bit, so a conforming host could not have provisioned the device.
- **The scene renderer is delivered on `feat/v2-networked-device`, software-complete and
  unverified on hardware.** Plan
  `docs/superpowers/plans/2026-08-23-deskmate-scene-renderer.md`, spec
  `docs/superpowers/specs/2026-08-22-deskmate-plugin-scene-rendering-design.md` (stage 2a of
  five). A host now pushes a **declarative display list** — absolutely-positioned nodes on the
  448x368 canvas — instead of a template being hand-written in C and compiled into firmware.
  The chain is `app-core`'s builder -> `protocol::encode_scene_payload` -> CBOR over **message
  19 (`PushScene`)** -> `core/scene_decode.c` -> `core/scene_model.c` validation ->
  `ui/scene_view.c` into LVGL objects. **`PROTOCOL_CURRENT_CAPABILITIES` is now 491** (bit 8,
  `+256`); the wire stays protocol v1, additive only, and the config schema is untouched.
  - **The gate that justifies the whole thing passed**: the host-built scene reproduces the
    shipped `DigitalClock` C template **byte-identically**, with no tolerance —
    `companion/crates/app-core/tests/scene_parity.rs`. It is **7 instants x seconds
    shown/hidden x two orientations = 28 comparisons, of which 14 are independent**. Do not
    quote 28 as a count of evidence: the flipped half is `sim_shim.c`'s `copy_frame_out`
    reversing the finished buffer index-by-index, so `flipped(A) == flipped(B)` iff `A == B`.
    **Real 270-degree geometry is provable only on hardware.**
  - The gate exercises the **real codec**, not a parallel one: the simulator encodes to CBOR and
    decodes with the firmware's own `scene_decode()` rather than marshalling over FFI. That is
    deliberate — a device-vs-simulator comparison is structurally blind to defects in code the
    two SHARE, which is how a full-circle arc that drew nothing survived until a reading review.
  - **Text nodes are baseline-anchored** (`baseline_y`, not a box top), because
    `digital_clock.c:44` positions type by `line_height - base_line`. **The dial is an
    `lv_scale`, not an arc** — the plan mis-modelled it, and `SCENE_NODE_SCALE` was added by
    amendment so the pixels match by construction rather than by porting LVGL's tick trig.
  - **`.bss` moved 102,608 -> 102,624**; `.data` 23,128 and IRAM 16,384/16,384-with-0-remaining
    are unchanged. The +16 is `scene_view.c`'s three existing pointers becoming reachable, not
    new state (verified in `deskmate.map`). **An OTA-download check on the board is therefore
    required before this is trusted** — this repo has twice lost days to memory-layout shifts
    with every test green.
  - **The OTA download passed on the board on 2026-08-25** with the moved `.bss`, on
    `v2.0.0-scene1`. It failed once and succeeded on a retry of the identical image at the
    same signal; recorded as transient, because the documented layout-shift failure mode is
    *deterministic*. **The renderer itself has still never drawn on the panel.**
  - **A host can now push a scene** (Task 10b, `4d81b22`): `Session::push_scene` ->
    `RuntimeDevice::push_scene` -> a `RuntimeHandle` command -> `POST /v1/devices/{id}/scene`.
    The route names a template and its inputs and calls `build_digital_clock_scene`, so there
    is no second JSON representation of a scene to drift. It never opens a `device::Session`
    of its own. This is **not** render negotiation — the spec's §3 policy is stage 3 work.
  - **A full-turn `SceneArc` is byte-identical to LVGL's circular border**, at the drawn
    object's unadjusted radius and border width — proven by the parity gate on `RowList`'s
    count ring (`OBJ_COUNT`), which has a 2px border and no fill where no scene node has a
    border field. Nothing was tuned to make it true. Also: **a bordered `lv_obj` insets its
    content origin by the border width**, so text inside one sits that far in.
  - **`protocol::validate_message` is public and is the one place the wire's bounds live.**
    Call it before sending rather than restating a rule the protocol already states.
  - **The dev-only 0x7E capture is PRE-FLUSH, and therefore cannot prove panel rotation.**
    `dev_capture.c` re-renders the object tree with `lv_snapshot_take_to_draw_buf()` into an
    offscreen buffer outside the CO5300 flush pipeline; the 90/270 software rotation and
    `board_lcd_rounder_cb`'s even-pixel rounding both happen at flush time. The raw capture is
    byte-identical at either rotation and the host reverses it exactly as `sim_shim.c` does —
    so `framebuffer_diff` and `scene_panel_check` carry the SAME `flipped(A) == flipped(B)`
    blindness as the host parity gate. **The physical 270-degree transform is only ever proven
    by looking at the panel.** This was mis-stated in a plan amendment on 2026-08-25 and
    corrected the same day; the firmware comment had said it all along.
  - `companion/crates/app-core/examples/scene_panel_check.rs` byte-compares the device's
    interpreter against the simulator. It needs **local tier** (in networked tier
    `s_owner_usb_restricted` refuses `ApplyConfig`/`PushScene`/`TimeSync` over the cable with
    `WRONG_TIER`) and a `DESKMATE_DEV_DIAG=1` build, so running it costs a device identity.
    Since `lvgl-sim` compiles the same firmware C *and* links the same baked fonts, what it
    can still catch is target codegen, ESP-IDF's LVGL configuration, and memory behaviour —
    not a defect in code the two share. Its `text` node case cannot run at all: it binds
    `field.status`, and device `PushData` retains only fields some built-in template
    registers, none of which registers `status`.
  - **The renderer has drawn on the panel (2026-08-25/26), and stage 2a's exit criteria are
    all met.** The server pushed a scene over the tunnel on the release image and the panel
    showed a ticking seconds field with no artifacts -- conclusive, because the saved config
    has `show_seconds: false`, so the C template cannot draw seconds. **Not** confirmed: the
    date line, and 90 degrees. Task 11's byte-exact half is **deferred, not owed** (see the
    plan for the three reasons); the harness is written and runnable.
  - **That check found a host-side defect no test here could see** (`95ed9eb`): `SocketPeer`
    took the pending waiter *before* comparing request ids, so one late reply failed the
    NEXT request, whose late reply failed the one after -- a cascade ending only when
    traffic stopped. The same shape was latent in `DeviceClient` and `DeviceSession`, i.e.
    over the cable too. **Decode first, correlate second**: screening on the request id
    before decoding lets an undecodable frame skip the check that closes the link, which
    `hostile_device.rs` catches. After the fix: 20/20 pushes at 5 s, `dropped_responses` 0.
  - **Sharp edge, deliberately unfixed:** the device gives `esp_websocket_client_send_bin`
    `PROTOCOL_WRITE_TIMEOUT_MS` = **200 ms** to deliver a reply the host waits **2000 ms**
    for, so a transient stall destroys a reply the host would still accept. No evidence it
    bites at realistic cadence. If it ever needs fixing, add a per-transport write budget
    beside `link_transport_t`'s `link_timeout_ms` and leave USB at 200 ms -- and budget for
    the OTA re-verification any firmware change requires.
  - Still unproven: `timer.remaining`/`timer.pct` have no pixel
    coverage; only `DigitalClock` is reproduced, so `deskmate_number_font()`'s tier step-down is
    never exercised and `BakedFontMetrics::measure()` is checked at exactly one string width;
    the date box's truncation is unexercised; and the asset-GC teardown has **no automated
    test** — it spans four LVGL/ESP-IDF-bound files with no simulator seam. Four on-device
    observations for it are specified at the end of the plan's Task 10 section.
  - `make -C firmware/host_tests sanitize` is now in CI and in Verification below, because two
    decoder bounds have **no other proof**: delete them and the plain suite still passes, since
    the out-of-bounds write is then rejected by `scene_model_validate()` with the same error
    code the test asserts.

- **Stage 2b (scene templates) is COMPLETE; all six exit criteria are met.** Plan
  `docs/superpowers/plans/2026-08-26-deskmate-scene-templates.md`. **All six C templates are
  now reproduced by host-built scenes and are byte-identical at both orientations: 106
  parity rows, 0 differing pixels, no tolerance.** Protocol stays v1 and additive,
  `PROTOCOL_CURRENT_CAPABILITIES` stays **491**, config schema stays v5.
  - Four firmware needs were found across the stage and **all four rode one image**, so the
    cost was one OTA download and one power cycle rather than four: `SceneArc.opacity`
    (key 9), an axis-wise canvas-bounded **external** rot-rect pivot, `SceneRect.clip`
    (key 7) and `SceneRotRect.clip` (key 10). Deferring each one as it was found, rather
    than fixing it in place, is what made that possible. **The OTA check PASSED
    2026-08-27** on `v2.0.0-scene3` — recorded in `docs/hardware/board-notes.md`. Flat
    internal RAM has now predicted a clean download twice; that is two data points, not a
    law, and the check stays.
  - **But the renderer's four new capabilities have never drawn on hardware.** The check
    was a download; no scene was pushed. Arc opacity, an external pivot and either clip are
    proven only by a **simulator-to-simulator** gate, and real 270° geometry is provable
    only by looking at the panel. Do not describe them as hardware-verified.
  - **The pivot bound was a specification error worth not repeating.** Task 1b pinned "a
    pivot outside the rect is rejected" into both validators — a rule true of every object
    it modelled (the hands, where `pivot_y = length = h`) and false of the one it did not
    (the twelve ticks, which rotate an 8px object about `pivot_y = 160`). Every unit test
    agreed with the wrong rule because every one tested a hand.
  - **LVGL clips children to their parent and a scene's display list is FLAT** — every node
    is a direct child of the screen — so any C template relying on a container to clip needs
    that expressed per node. Two of the nine node kinds draw geometry that can overflow
    their C parent, and the same gate found both. The device realises a clip by parenting
    the node under a styleless wrapper, matching the C **by construction** rather than
    reproducing LVGL's mask arithmetic on the host.
  - **The byte-exact gate has a SECOND blindness, distinct from the documented one.** The
    known one is shared code: both halves run the same possibly-wrong C. The new one is
    **injected binding inputs** — the parity request supplies a pinned `SceneTimer` rather
    than deriving it, so the device's own producer never runs. **A gate that supplies a
    binding's input can prove how a value is drawn and never what it means.** Three device
    -path defects live in exactly that hole and are recorded, not fixed (see the ledger).
  - **`docs/scene/template-parity-ledger.md` is what stage 3 is planned from.** Its
    conclusions contradict the plan's expectations in three places: **`DigitalClock` is
    blocked from retirement too**, not only `ProgressRing` (its date and both small-dial
    hand endpoints are literals); **no builder uses `field.*` at all**; and `timer.pct` is
    **elapsed** percent on the device while the C arc and the parity fixture both use
    **remaining**, so a native ring would grow where the C ring shrinks. Also:
    `timer.remaining:mm:ss` floors where the C ceils, and treats `mm` as a clock minute
    modulo 60 where `format_clock()` prints total minutes to `1440:00`.
  - **Retiring a C template also retires its offline behaviour.** A tap runs
    `template_view_apply_local_action()` for optimistic start/pause feedback on the C view;
    a scene has no equivalent and waits for the host's `PushData`. No builder emits the
    shared stale/error footer either, because all 106 rows exercise the OK state.
  - The `known_gap` marker in `scene_parity.rs` is retained although nothing uses it. It
    asserts a marked case **still differs**, so closing a gap fails the test and forces the
    marker's deletion — an unexplained skip rots into invisible missing coverage, an
    enforced one cannot. It made three gaps visible during this stage.

- **Two plan amendments were added during execution and are marked as such in the plan.**
  Task 9b (persistent device identities) was added by explicit owner direction; Task 10a
  (the app-core boundary) was added because Task 10's implementer correctly refused to
  open a second `device::Session` from a Tauri command, which would have compiled, passed
  its tests, and put two processes on one cable.
- **The server is deployed and live at `deskmate.rodi.one`**, on the owner's homelab
  (docker-vm), behind Cloudflare → cloudflared → Caddy, under the systemd unit in
  `companion/crates/server/deploy/`. It is built for linux/x86_64 in a throwaway
  container matching `companion/rust-toolchain.toml` (**`rust:1.98-bookworm`** since the
  1.98.0 pin) over an rsync'd copy of `companion/` — no Rust toolchain on the VM. Reach
  the VM over **Tailscale** (`docker-vm`, 100.93.166.123): `~/.ssh/config` pins its LAN
  address, which is unreachable from any other network. `sudo -n` works there, and a
  previous deploy leaves a root-owned `companion/target/` — keep it and replace only the
  sources, which turns a cold build into roughly 40 seconds. Device URL is
  `wss://deskmate.rodi.one/v1/device/link`. Redeploy from a `git archive HEAD` export,
  never the working tree.
- **Device identities persist as SHA-256 digests, never as tokens.** Minting is the only
  path that needs the plaintext; authentication only compares. Verified on the live
  deployment: the plaintext does not appear in the store file, and a pre-restart token
  still authenticates afterwards.
- **Provisioning is a cable operation by design.** `WebSocketRuntimeDevice::provision` and
  `factory_reset` return a typed unsupported-on-this-transport error. In networked tier
  the cable is the *configurator* and the server is the *owner*; the firmware's tier gate
  encodes exactly that. Do not add provisioning over the tunnel without specifying it
  first.
- **`CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE=y` changes boot behaviour, not just update
  behaviour.** An image that does not call `esp_ota_mark_app_valid_cancel_rollback()`
  within its validity window is rolled back on the next boot. The validity gate lives in
  `ota_mark_running_image_valid()` and is deliberately free of every dependency that can
  fail — no network, server, config, or NVS content stands between boot and marking valid.
  Do not add one. Two consequences worth knowing: OTA does not update the bootloader, so
  a rollback test must start from a full `idf.py flash`; and a corrected rebuild published
  under the **same version string** as a failed one is refused forever, because the
  refusal keys on the version string rather than image content.
- **Tasks 8 through 12 are open on hardware.** Task 8's four link observations, Task 9's
  headline demo, and Task 11's OTA/rollback/deferral steps are all unobserved, along with
  the whole of Task 12's exit gate. Do not describe any of them as verified. Tasks 3, 4, 5
  and 6 *were* verified on the board on 2026-08-18 and are recorded in
  `docs/hardware/board-notes.md` — including the WiFi crash-loop root-cause — so do not
  redo that work either.
- V1 packaging/hardening is delivered
  (`docs/superpowers/plans/2026-08-15-deskmate-v1-packaging-hardening.md`): the
  repo has a private GitHub remote `cosmicsymmetry/deskmate` with a green `ci`
  workflow (macOS companion gates + release DMG artifact; ESP-IDF build + host
  tests in the IDF container). Version is 1.0.0; bundle targets are app+dmg with
  ad-hoc signing. Audits run non-blocking in CI with
  `docs/security/advisories.md` as the blocking triage record; the security
  review is `docs/security/v1-review.md`. The hands-on install matrix passed
  (results table in the plan, recorded 2026-08-17); it surfaced and fixed one
  defect — the settings window auto-opened on every launch and now auto-opens
  on first run only (`fc3750b`). All V1-exit items are closed; declaring V1
  exit (tag, V2 brainstorm) awaits explicit user authorization. No tags exist.
- **Clock faces carry no title chip and no `DATE` eyebrow** (delivered 2026-08-17; spec
  `docs/superpowers/specs/2026-08-17-deskmate-clock-title-removal-design.md`, plan
  `docs/superpowers/plans/2026-08-17-deskmate-clock-title-removal.md`). All three clock
  surfaces — the `DigitalClock` card, the `AnalogClock` card, and the standalone fallback
  `clock_screen.c` — draw themselves unlabelled, and each stack is centred on its own
  canvas (card hero at `8 * DESKMATE_GRID`, fallback hero at `11 * DESKMATE_GRID`; they
  deliberately differ because the fallback's module is 96px against the card's 136px).
  Do not reintroduce a chip or eyebrow on a clock face, and do not "unify" the two hero
  rails — the 24px shift across a reconnect is accepted and was observed not to read as a
  glitch. The five non-clock templates keep their chips. Schema stays **v4**: `title` is
  still a schema and wire field that the host sends and firmware now ignores; it names the
  card in the companion library, where its editor label is "Name" for clock cards and
  "Heading" elsewhere. Verified on hardware 2026-08-17 — all 14 clock frames
  byte-identical via `framebuffer_diff` at both orientations, fallback and both transition
  directions confirmed by webcam at 90°. Two things are **not** verified: the fallback at
  270°, and the fallback's bottom margin (cropped in the available camera framing).
- **After C-template retirement, the framebuffer diff expects
  `total=76 identical=64 differing=0 excluded=12`, and any differing case is a real
  firmware/simulator disagreement.** The matrix is 18 synthetic scene-node rows plus
  58 device-pushable rows spanning all six retired faces. Eight synthetic rows are
  excluded because they require unprovisioned image/font assets or an unregistered
  `field.status`; the four face-row exclusions below are active again, not vacuous.
  `row-list--truncation-boundary` contributes two of those four. The other two are
  `progress-ring--running-mid-countdown`, which could not pass deterministically:
  `progress_ring.c`'s `current_remaining_ms` keeps counting a *running* ring down from
  `lv_tick_get()` after its fields are pushed, while the simulator's fake tick is fixed
  (an 840 ms anchor offset, `crates/lvgl-sim/csrc/sim_shim.c`), so the label's
  `(remaining_ms + 999) / 1000` ceiling flips a second the moment push-to-capture latency
  crosses 1000 ms. **No running value avoids this** — the label flips every second by
  construction, whatever the duration — so "pin a different value" is not among the
  options. Note this also means V1 acceptance's recorded "0 differing" was luck, not
  proof, and that pre-2026-08-21 runs reported `total=54 … differing=1`.
  The flakiness is only in the *device-vs-simulator* comparison; the simulator's own
  goldens are deterministic. So the case is now **golden-only**: it stays in
  `cases::golden_cases()` (it is the only pixel coverage of the running arc-indicator hue,
  since a zero-length arc draws no indicator) and `exclusion_reason()` in
  `framebuffer_diff.rs` excludes it from hardware, as the `row-list--truncation-boundary`
  pair already was. Two new cases keep hardware coverage: `paused-mid-countdown` is the
  same partial-arc geometry with the ring stopped, and `running-at-zero` is the only
  running ring hardware can be compared on, covering the running status colour. Do not
  "simplify" these back into one case, and do not flip `running-mid-countdown` to
  `running: false` — that would delete the running-hue golden.
- The webcam verification harness (`tools/hwcam/`, usage in
  `docs/hardware/webcam-harness.md`, spec
  `docs/superpowers/specs/2026-08-15-deskmate-webcam-harness-design.md`) was
  commissioned 2026-08-15: agent-driven physical panel verification via OBSBOT
  captures, judged by agent vision only (no CV code by design). Touch and cable
  pulls remain human actions. Prefer it for panel observations in future
  hardware checks; board-notes stays the durable record.
- M1's full software and physical exit gate passes, including ten observed
  unplug/replug cycles, and is tagged `m1`. Because M2 work began in the same shared
  worktree before the physical carryover closed, that tag also contains the M2
  foundation present at M1 exit.
- V1 Plan A (`docs/superpowers/plans/2026-08-11-deskmate-v1-preview-typeface-redesign.md`,
  branch `feat/v1-preview-typeface-redesign`) — preview harness, baked typeface
  pipeline, COMPLICATION visual redesign, the custom OTA/asset/coredump partition
  table, and dev-only framebuffer capture — is software-complete, reviewed clean,
  and **physical acceptance PASSED on 2026-08-14.** An earlier attempt
  (2026-08-13) hit a boot crash-loop; it was root-caused and fixed
  (`b90e711`, `65b5357`, `05d04ce`; the custom partition table itself was
  *not* the cause, only a contributing memory-layout shift — see
  `docs/hardware/board-notes.md`'s "Boot crash-loop root-caused" entry) and
  the fix was independently re-verified from a fresh session before the
  acceptance run below. Full evidence for both sessions is in
  `docs/hardware/board-notes.md` under "V1 physical acceptance — 2026-08-13"
  (the original failure, preserved) and "V1 physical acceptance — 2026-08-14"
  (the passing run), and
  `.superpowers/sdd/2026-08-11-deskmate-v1-preview-typeface-redesign/task-11-report.md`.
  The framebuffer diff passed (56 cases: 54 identical, 0 differing, 0 errored,
  2 excluded by design) at both orientations, which **closes the
  `show_seconds`-false analog-clock defect open since M4** — both orientations'
  no-seconds goldens are pixel-identical to the simulator. The §6.4 acceptance
  matrix (protocol edge cases, malformed/maximal data, rotation 90 and 270)
  passed with no reboot. The 30-minute mixed soak was interrupted at 27 minutes
  by explicit user direction and is **waived, not completed** — the captured
  partial data (heap byte-flat at a single value for the full 27-minute window,
  no counter regression) is recorded, but do not describe a full 30-minute soak
  as observed. The CO5300 even-window rounding check at the fix's 64-line
  flush-strip height was human-observed clean at both orientations on
  2026-08-15 (ticking-clock partial flushes, no artifacts) — recorded in
  board-notes; that flag is closed.
- Target hardware is the Waveshare ESP32-S3-Touch-AMOLED-1.8 **v2**: CO5300 display and
  CST820 touch using the CST816S protocol family. Do not apply v1 SH8601/FT3168 facts.
- Treat the physical 368x448 panel as a 448x368 landscape UI: 90° is the default
  (USB cable down) and 270° is the flipped orientation. Layout and touch logic use
  logical dimensions. The companion setting owns this choice; do not add a device-edge
  or screen gesture that changes orientation, and do not expose portrait orientations.
- All current cards use the clean 448x368 canvas: there is exactly one canvas and one
  layout per template. Size classes no longer exist in the config authoring model as of
  schema v3; `SizeClass::Full` is pinned on the wire for every compiled widget. Do not
  reintroduce dashboards or a status strip.
- The v2 CO5300 requires every LVGL invalidation area to be rounded outward to even
  pixel boundaries before partial flushing. Keep `board_lcd_rounder_cb` registered in
  the `esp_lvgl_port` display config, including for 90°/270° software rotation.
- Firmware is ESP-IDF 5.x/C with LVGL 9. Host tooling is Rust per the design spec.

## Working agreement

- Start by reading this file, checking `git status`, and reading the roadmap, active
  plan, and relevant board notes. The worktree may contain another contributor's work;
  preserve it.
- Execute the active plan in order unless a prerequisite or new finding requires a plan
  amendment. Mark a checkbox complete only after its stated verification passes.
- Keep plans live: record material decisions, deviations, exact verification results,
  and blockers as they are discovered. Never claim hardware verification that was not
  observed on the physical board.
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

For companion work, run formatting, linting, and workspace tests from `companion/` once
that workspace exists:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo test --workspace --doc
```

Both test invocations are required: `--all-targets` adds example and integration
targets but removes doctests, so neither invocation alone covers the workspace.
Keep them as separate lines so a failure names the missing coverage directly; do
not simplify them back to one command. The workspace currently has no bench targets.

Hardware-facing changes also require the on-device checks named in the active plan and
an entry in `docs/hardware/board-notes.md` with the observed result.
