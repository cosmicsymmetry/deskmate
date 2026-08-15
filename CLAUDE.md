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
  until-dismissed reconnect paths. The original observation did not record its hold
  configuration, so hardware re-verification remains pending.
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
  as observed. Not yet checked by a human: the CO5300 even-window rounding
  behavior at the fix's new 64-line flush-strip height, at both orientations —
  flagged in board-notes, needs eyes on the physical panel.
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
