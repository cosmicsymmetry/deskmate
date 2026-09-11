# Deskmate v1 — Milestone Roadmap

Spec: `docs/superpowers/specs/2026-08-03-deskmate-design.md`

**Current:** V1 and V2 are EXITED and TAGGED (`v1`, `v2`) by explicit owner direction
on 2026-09-06. V1's `v1` tag is at `7abd496` (all V1-exit items closed); V2's `v2` tag is
at `fdf85ba` (the verified networked-device state, which excludes the later `feat/one-loop`
merge). Owner declared V2 exited with three hardware observations deferred, not observed,
and not treated as exit blockers: Task 9 tap latency, the BUSY/OTA-owner refusal, and the
§6 96 px glyph-cache timing (each needs the board with a person present or a firmware
test-hook; see board-notes). Next milestone is V3 (server host). Scene
stages 2a and 2b are delivered. Stages 3a, 3b, and 4 are delivered and, as of the
2026-09-06 hardware session (board-notes, "Stage 4 Task 7"), confirmed on the board:
3a's Gate B is closed, stage 4's Task 7 session and on-target framebuffer check
(historical 96/10/86, 0 differing; current unobserved inventory predicts 78/8/70) both
PASSED, and 3b's asset-GC teardown is partially observed —
only its BUSY/OTA-owner variant, which needs a pending OTA in flight, remains owed.
Stage 5 remains a risk review only; implementation awaits review completion and explicit
owner approval. M0/M1 are tagged `m0`/`m1`; later milestones remain untagged without
that authorization.

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
| V1 | **EXITED and tagged `v1` (2026-09-06, owner direction; tag at `7abd496`)** | Local Deskmate | Custom partition table (OTA slots + asset region), baked typeface, pixel-exact host-LVGL preview harness with golden frames, built-in widget redesign — all delivered and physically accepted 2026-08-14; playlist authoring model (schema v4, `presence` removed) delivered and merged 2026-08-15 (wire unchanged, no on-device delta). The CO5300 even-window check at the 64-line flush strip closed 2026-08-15 (human-observed clean at both orientations). Packaging/hardening delivered: private `cosmicsymmetry/deskmate` remote with the green `ci` workflow (macOS companion gate + DMG artifact; ESP-IDF build + host tests), advisories triaged in `docs/security/advisories.md`, security review in `docs/security/v1-review.md`, and the hands-on install matrix passed (recorded 2026-08-17 in the packaging plan; one defect found and fixed — settings window now auto-opens on first run only, `fc3750b`). V1 exited and tagged `v1` at `7abd496` on 2026-09-06 by owner direction | `2026-08-11-deskmate-v1-preview-typeface-redesign.md`, `2026-08-11-deskmate-v1-playlists.md`, `2026-08-15-deskmate-v1-packaging-hardening.md` |
| V2 | **EXITED and tagged `v2` (2026-09-06, owner direction; tag at `fdf85ba`)** | Networked device | Device owned over the network instead of a cable: WiFi station, TLS, USB-based provisioning (no SoftAP), bearer-token device identity, tier pairing, WebSocket transport carrying today's frames unchanged, and OTA with rollback. Proven against a single-tenant no-accounts stub server. Headline demo: provision over USB, unplug, quit the Mac app, cards keep updating. Additive within protocol v1. **Delivered and verified on hardware**: the server owns the device, providers run server-side, real weather renders with the Mac quit, OTA installs and reboots, and rollback works unattended. Exited by owner direction 2026-09-06 with three hardware observations deferred, not observed, and not treated as blockers: Task 9 tap latency, the BUSY/OTA-owner refusal, and the §6 96 px glyph timing (Task 8's widening-backoff observation WAS discharged on the shipping build 2026-09-06) | `docs/superpowers/plans/2026-08-18-deskmate-v2-networked-device.md` |
| V3 | Planned (narrowed by V2's design §10) | Server host | Accounts, OAuth-held integration credentials (Google Calendar and similar), config storage, and multi-tenancy — behind the device-facing contract V2 freezes. Should require no firmware change, because V2 delivers the contract, identity, provisioning, transport and update mechanism | own brainstorm at V2 exit |
| V4 | Planned; **partly superseded** by the scene-rendering design | Plugin platform | Image cards, asset cache, billing (paid tier), and a plugin contract. **The "HTML plugin contract, headless-Chromium rendering" originally written here is retired**: `docs/superpowers/specs/2026-08-22-deskmate-plugin-scene-rendering-design.md` §5 rules a headless browser out permanently ("it would make the homelab an arbitrary-code-execution host. Deferred indefinitely, not scheduled") and names `resvg` as the only permitted rasterizer. The plugin contract is the declarative manifest frozen in `docs/plugins/manifest-v1.md`, delivered in stage 3b. Public uploads and the sandbox are that spec's stage 5, with their own risk review | own brainstorm at V3 exit; superseding decisions in the scene-rendering spec |

The scene renderer's five stages each get their own plan, written at the previous stage's
exit (the spec is the architecture for all five, not one implementation plan):

| Stage | Plan | State |
| --- | --- | --- |
| 2a | `2026-08-23-deskmate-scene-renderer.md` | Delivered; drew on the panel 2026-08-25/26 |
| 2b | `2026-08-26-deskmate-scene-templates.md` | Delivered; all six faces byte-identical |
| 3a | `2026-08-27-deskmate-scene-native-rendering.md` | Delivered; the six C templates no longer ship. Gate B is closed: the templates-removed OTA download passed 2026-08-28 (`live1` -> `live2`), and the last owed piece — rollback survival across a second boot — closed 2026-09-06, when live2 had survived every subsequent reboot and its templates-removed successor `v2.0.0-raster1` installed first-try and survived the rollback window (board-notes, "Stage 4 Task 7 Phase B") |
| 3b | `2026-08-28-deskmate-plugin-manifest.md` | **Task 9 PASSED on the board 2026-08-30** — both plugin faces at 270° and 90°, a runtime glyph at 72 px, the image node clean, and `field.*` drawing a real value for the first time. Step 6 (asset-GC teardown) was partially observed 2026-09-06 — the teardown/release/rebuild works live with no reboot, but the font-vanish moment is not panel-visible on dev-0005's card set — and only its BUSY/OTA-owner variant (needs a pending OTA in flight) is still owed |
| 4 | `2026-08-29-deskmate-rasterization.md` | **Software-complete 2026-09-01** (Tasks 1-6: negotiation, manifest v2, resvg rasterizer, volatile PSRAM assets + bit 9 / capabilities 1003, RLE565 wire, the 30 s-floor executor, evidence rows). **Task 7's hardware session ran 2026-09-06 and PASSED** — the `v2.0.0-raster1` OTA installed first-try and survived the rollback window, capabilities read 1003, native and raster cards drew at 270° and 90°, the typed refuse rule and the 30 s floor held exactly, and 20-revision volatile churn kept the heap flat — and **Task 6 Step 5's on-target `framebuffer_diff` PASSED the same day: historical 96 total / 10 excluded / 86 identical / 0 differing; after manifest removal the unobserved software prediction is 78/8/70** (the original prediction was 96/8/88; corrected by three test-harness-only fixes, no firmware change). Only the BUSY/OTA-owner variant remains owed, and 3b's Phase A teardown was only partially observable (board-notes, "Stage 4 Task 7") |
| 5 | `2026-09-01-deskmate-plugin-upload-risk-review.md` | Risk-review plan written (Task 8 Step 3); implementation begins only after the review completes and the owner explicitly approves |

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
