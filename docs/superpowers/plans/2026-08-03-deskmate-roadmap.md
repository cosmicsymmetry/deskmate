# Deskmate v1 — Milestone Roadmap

Spec: `docs/superpowers/specs/2026-08-03-deskmate-design.md`

**Current:** V2's design is approved (`docs/superpowers/specs/2026-08-18-deskmate-v2-networked-device-design.md`); its implementation plan is not yet written. M0, M1, M2, and M3 are complete. M4 delivered Tasks 1-4 (contract
freeze, bounded providers, the card model, extended templates, timed rotation and
alerts), then was superseded on 2026-08-11 by an explicit user-directed reset:
`docs/superpowers/specs/2026-08-11-deskmate-v1-reset-design.md`. That spec re-cuts
the remaining work into four stages — **V1 Local Deskmate** (active: partition
table, baked typeface, pixel-exact LVGL preview harness, built-in widget redesign,
playlist authoring model), **V2 Networked device** (WiFi/provisioning/pairing/OTA),
**V3 Server host**, and **V4 Plugin platform** (server-rendered HTML plugins as the
paid tier). M4's undelivered tasks are redistributed, not lost — see the spec's §7
disposition table. M0/M1 are tagged `m0`/`m1`; later milestones are untagged because
tags require explicit authorization. Because M2 implementation began in the same
shared worktree before M1's final physical carryover closed, the `m1` checkpoint
also contains the M2 foundation present at that point.

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
| M2 | Complete | Template engine + first widgets | Clock, pomodoro (progress ring, tap start/pause, done-interrupt), ICS calendar — all driven via CLI; carousel + status strip | `2026-08-04-deskmate-m2-template-first-widgets.md` |
| M3 | Complete | Companion app (Tauri v2) | Tray app replaces CLI for daily use: config store, providers, settings UI with widget gallery + screen arranger | `2026-08-04-deskmate-m3-companion-app.md` |
| M4 | Superseded (Tasks 1-4 delivered) | v1 completion | Delivered: weather/JSON-feed/RSS providers, extended templates, the card model, timed rotation and alerts. Remainder redistributed by the 2026-08-11 reset spec §7 | `2026-08-05-deskmate-m4-v1-completion.md` |
| V1 | All exit items closed (declaring V1 exit awaits explicit user authorization) | Local Deskmate | Custom partition table (OTA slots + asset region), baked typeface, pixel-exact host-LVGL preview harness with golden frames, built-in widget redesign — all delivered and physically accepted 2026-08-14; playlist authoring model (schema v4, `presence` removed) delivered and merged 2026-08-15 (wire unchanged, no on-device delta). The CO5300 even-window check at the 64-line flush strip closed 2026-08-15 (human-observed clean at both orientations). Packaging/hardening delivered: private `cosmicsymmetry/deskmate` remote with the green `ci` workflow (macOS companion gate + DMG artifact; ESP-IDF build + host tests), advisories triaged in `docs/security/advisories.md`, security review in `docs/security/v1-review.md`, and the hands-on install matrix passed (recorded 2026-08-17 in the packaging plan; one defect found and fixed — settings window now auto-opens on first run only, `fc3750b`). V1 tag and V2 brainstorm await explicit user authorization | `2026-08-11-deskmate-v1-preview-typeface-redesign.md`, `2026-08-11-deskmate-v1-playlists.md`, `2026-08-15-deskmate-v1-packaging-hardening.md` |
| V2 | Design approved 2026-08-18 (plan pending) | Networked device | Device owned over the network instead of a cable: WiFi station, TLS, USB-based provisioning (no SoftAP), bearer-token device identity, tier pairing, WebSocket transport carrying today's frames unchanged, and OTA with rollback. Proven against a single-tenant no-accounts stub server. Headline demo: provision over USB, unplug, quit the Mac app, cards keep updating. Additive within protocol v1 | `docs/superpowers/specs/2026-08-18-deskmate-v2-networked-device-design.md` |
| V3 | Planned (narrowed by V2's design §10) | Server host | Accounts, OAuth-held integration credentials (Google Calendar and similar), config storage, and multi-tenancy — behind the device-facing contract V2 freezes. Should require no firmware change, because V2 delivers the contract, identity, provisioning, transport and update mechanism | own brainstorm at V2 exit |
| V4 | Planned | Plugin platform | HTML plugin contract, headless-Chromium rendering, image cards, asset cache, billing (paid tier) | own brainstorm at V3 exit |

First-widget order and why:

1. **Digital clock** — zero data deps; doubles as the spec's standalone
   fallback screen.
2. **Pomodoro + progress ring** — proves the full bidirectional loop
   (device tap → event up → app state machine → data push down) and
   priority interrupts, with no external network dependency.
3. **Calendar (ICS)** — first external data: parsing, list-of-rows
   template, staleness indicator.

Plans are written just-in-time at each milestone exit so learnings feed forward. The M4
plan was drafted from M3's findings and activated after its physical exit gate closed.
M1
starts with a USB transport proof because M0 established serial flashing/logging but did
not establish a dedicated, log-free application channel.
