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
- M2 (template engine and first widgets) is the active implementation milestone. Its
  plan is `docs/superpowers/plans/2026-08-04-deskmate-m2-template-first-widgets.md`.
  M1's full software and physical exit gate passes, including ten observed
  unplug/replug cycles, and is tagged `m1`. Because M2 work began in the same shared
  worktree before the physical carryover closed, that tag also contains the completed
  M2 foundation present at M1 exit.
- Target hardware is the Waveshare ESP32-S3-Touch-AMOLED-1.8 **v2**: CO5300 display and
  CST820 touch using the CST816S protocol family. Do not apply v1 SH8601/FT3168 facts.
- Treat the physical 368x448 panel as a 448x368 landscape UI: 90° is the default
  (USB cable down) and 270° is the flipped orientation. Layout and touch logic use
  logical dimensions.
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
