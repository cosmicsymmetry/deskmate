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
  playlists, one active, while preserving the wire and firmware; the current frozen
  contract is `docs/config/v4.md`. That plan also **FIXED** the validation-mislabeling
  defect: a validation failure preserves genuine last-good state, remains typed, and is
  never presented as "your last working settings". M4 Task 3 (the `AnalogClock`,
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
  - **`3f2aa03` is a REGRESSION: it breaks OTA downloads.** It installs a custom HTTP
    event handler on the download client, and `esp_https_ota` depends on its own event
    handling to read the image body, so every download fails with
    `download: ESP_ERR_MBEDTLS_SSL_READ_FAILED` — isolated on the board 2026-08-19 and
    recorded in board-notes. **Do not flash a build carrying it.** The general rule:
    never instrument a subsystem by taking over a callback it owns; the perform loop
    already had `esp_https_ota_get_status_code()`. Two lessons stand on their own: every
    gate stayed green through the whole thing, so green tests say nothing about whether
    OTA still works; and `last_ota_error` is what made its own regression diagnosable,
    which is the argument for the field rather than against it.
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
- **Two plan amendments were added during execution and are marked as such in the plan.**
  Task 9b (persistent device identities) was added by explicit owner direction; Task 10a
  (the app-core boundary) was added because Task 10's implementer correctly refused to
  open a second `device::Session` from a Tauri command, which would have compiled, passed
  its tests, and put two processes on one cable.
- **The server is deployed and live at `deskmate.rodi.one`**, on the owner's homelab
  (docker-vm), behind Cloudflare → cloudflared → Caddy, under the systemd unit in
  `companion/crates/server/deploy/`. It is built for linux/x86_64 in a throwaway
  `rust:1.97-bookworm` container over an rsync'd copy of `companion/` — no Rust toolchain
  on the VM. Device URL is `wss://deskmate.rodi.one/v1/device/link`. Redeploy from a
  `git archive HEAD` export, never the working tree.
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
- **`progress-ring--running-mid-countdown` cannot pass the framebuffer diff
  deterministically.** `progress_ring.c`'s `current_remaining_ms` keeps counting a
  *running* ring down from `lv_tick_get()` after its fields are pushed, while
  `Simulator::render` draws the case's pinned `remaining_seconds` frozen, so whether the
  capture lands before or after the device's next one-second tick decides the comparison.
  Expect `total=54 identical=51 differing=1 excluded=2` with that case differing (it
  alternated orientation between two runs on 2026-08-17), and treat any *other* differing
  case as a real firmware/simulator disagreement. This also means V1 acceptance's recorded
  "0 differing" was luck, not proof. Unfixed; the options are to pin `running: false` for
  the compared case or to exclude it in `exclusion_reason()` as the
  `row-list--truncation-boundary` pair already is.
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
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build
```

For companion work, run formatting, linting, and workspace tests from `companion/` once
that workspace exists:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Hardware-facing changes also require the on-device checks named in the active plan and
an entry in `docs/hardware/board-notes.md` with the observed result.
