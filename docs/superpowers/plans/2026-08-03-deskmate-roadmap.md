# Deskmate v1 — Milestone Roadmap

Spec: `docs/superpowers/specs/2026-08-03-deskmate-design.md`

**Current:** M0 and M1 are complete and tagged `m0` and `m1`. M2 remains active. Because
M2 implementation began in the same shared worktree before M1's final physical
carryover closed, the `m1` checkpoint also contains that completed M2 foundation.

One plan per milestone; each milestone ends with working, demonstrable
software. Ordering rationale: retire hardware risk first (from-scratch
firmware is the biggest unknown), build the link with a headless CLI before
any GUI, prove the widget architecture with three widgets that exercise
every subsystem, then wrap the proven core in the Tauri app, then fill in
breadth.

| # | Status | Milestone | Deliverable (demo) | Plan |
|---|--------|-----------|--------------------|------|
| M0 | Complete | Hardware bring-up + walking skeleton | Device shows ticking clock; touch moves a dot; brightness + 180° rotation work | `2026-08-03-deskmate-m0-bringup.md` |
| M1 | Complete | Protocol + link + CLI harness | `deskmate-cli status` / `time-sync` / `push-data` work from a terminal; device falls back to standalone clock on unplug | `2026-08-04-deskmate-m1-protocol-link-cli.md` |
| M2 | Active | Template engine + first widgets | Clock, pomodoro (progress ring, tap start/pause, done-interrupt), ICS calendar — all driven via CLI; carousel + status strip | `2026-08-04-deskmate-m2-template-first-widgets.md` |
| M3 | Pending | Companion app (Tauri v2) | Tray app replaces CLI for daily use: config store, providers, settings UI with widget gallery + screen arranger | (write at M2 exit) |
| M4 | Pending | v1 completion | Weather/JSON-feed/RSS providers, remaining spec templates (big number + label, icon + badge + text, analog clock face), `tile` layouts + dashboard grids, "open URL/app on host" tap action, auto-rotate, asset push, staleness, wider ICS recurrence, in-app firmware update, production VID/PID, release smoke checklist | (write at M3 exit) |

First-widget order and why:

1. **Digital clock** — zero data deps; doubles as the spec's standalone
   fallback screen.
2. **Pomodoro + progress ring** — proves the full bidirectional loop
   (device tap → event up → app state machine → data push down) and
   priority interrupts, with no external network dependency.
3. **Calendar (ICS)** — first external data: parsing, list-of-rows
   template, staleness indicator.

Plans are written just-in-time at each milestone exit so learnings feed forward. M1
starts with a USB transport proof because M0 established serial flashing/logging but did
not establish a dedicated, log-free application channel.
