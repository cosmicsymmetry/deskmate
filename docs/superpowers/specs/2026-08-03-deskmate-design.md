# Deskmate — Design

**Date:** 2026-08-03
**Status:** Approved by Rodion (pending written-spec review)

## 1. Product

A monitor/laptop-clip-on desk display (webcam position) with a 1.8" AMOLED
touchscreen, driven by a lightweight menu-bar companion app. Users compose
widget screens from building blocks — data sources, display templates, tap
actions — with zero code.

Designed as a **distributable product**: no assumptions about any specific
user's infrastructure, accounts, or tools. First-run experience is "plug in
USB-C, open the app."

Non-goals for v1: wireless connectivity, battery operation, a code-level
plugin API, mobile companion apps.

## 2. Hardware

- **Target:** Waveshare ESP32-S3 Touch AMOLED 1.8" **v2** (CO5300 panel,
  368×448, capacitive touch, ESP32-S3R8: dual-core 240 MHz, 8 MB PSRAM,
  16 MB flash).
- **Connection:** single USB-C carrying both power and data (USB CDC serial,
  no drivers needed on macOS/Windows/Linux). No battery dependence — the
  device is powered whenever it is connected.
- **Enclosure:** 3D-printed clip-on cover (webcam-style mount for monitor or
  laptop lid). Parallel workstream, out of scope for this spec. The product
  UI uses the physical 368x448 panel as a 448x368 landscape canvas by
  default (90° clockwise, cable down) and supports a 180° flip to 270° so
  the cable can exit from the opposite landscape edge.
- **Future targets** (e.g. ESP32-P4 displays) are enabled by a thin
  board abstraction layer in the firmware, but only this board ships in v1.

## 3. Architecture

"Smart terminal" split — the device renders, the app thinks:

- **Firmware** (C, ESP-IDF 5.x + `esp_lcd` + LVGL 9, built **from
  scratch** — no code reuse from prior projects): renders all UI at the
  panel's native frame rate, owns animations and instant touch feedback,
  holds the current widget configuration and the latest pushed data.
  Standalone fallback when no host is connected: clock + "connect me" hint.
- **Companion app** (Tauri v2): a Rust daemon — serial link management, data
  providers, scheduler, config store — plus a system-webview settings window
  that exists only while open (WKWebView on macOS, WebView2 on Windows).
  Menu-bar / system-tray icon for quick actions. Autostart at login,
  single-instance.
- **Link:** USB CDC serial with a framed, versioned message protocol:
  - down: config sync, data updates, asset push (icons, fonts), time sync
  - up: touch/gesture events, heartbeat/status, protocol version
- **Rendering split rationale:** ESP32-S3 USB is full-speed (12 Mbps), too
  slow for streaming app-rendered frames at animation frame rates; on-device
  LVGL rendering gives 60 fps feel with dirty-region redraws and instant
  touch response.

## 4. Card model

**Superseded in part:** the screen model below (carousel of user-ordered screens with
single-widget or dashboard-grid layouts and three size classes) has been replaced by
the card model. See `docs/superpowers/specs/2026-08-06-deskmate-card-model-design.md`
for the full design and `docs/config/v3.md` for the frozen contract. This section
states the current model; the clean-canvas and mounting-orientation paragraphs below
are unchanged from the original design.

The device is an **ambient, contextual** display: it shows one thing at a time at full
size, rotates on a timer, and lets a bounded set of events take over the screen
temporarily. It is not an at-a-glance panel and not a dashboard.

- A configuration is **one ordered array of cards** (1..8). A card is data source +
  display template + tap action + refresh policy + `presence` + `alert` — the widget
  and screen concepts have merged into one authorable unit. There is no `size` field:
  size classes do not exist in this authoring model, and every template has exactly one
  448x368 layout.
- **Presence** states whether and how a card participates in the rotation:
  `in-rotation { dwell_seconds }` (appears in the carousel, `dwell_seconds` inherits the
  global default when `null`), `alert-only` (configured and polled, never in the
  carousel, exists solely to take over when its alert fires), or `off` (muted: not
  polled, not compiled to the wire, settings preserved).
- **Carousel and timed rotation:** swipe navigates; `carousel.advance` is `manual` or
  `timed { default_dwell_seconds }`. Timed advance is driven entirely by the host, which
  sends `ActivateScreen` when a card's dwell expires — the firmware has no rotation
  timer of its own. Manual navigation and array order remain authoritative regardless of
  timed advance.
- **Alerts** are a closed, bounded mechanic, not a general condition language: `none`,
  `on-timer-finish` (pomodoro only), or `before-event` (calendar only, `lead_minutes`).
  An alert takes over the full canvas with a dismiss affordance (tap = acknowledge),
  reusing the same priority-interrupt arbitration the original design specified. `hold`
  bounds how long the alert asserts itself: `until-dismissed` or `seconds`.

  **Known limitation:** under protocol v1, a bounded `hold.seconds` cannot clear the
  physical panel — there is no host→device dismissal message, so the device continues
  showing the interrupt overlay until an on-device tap regardless of the host-side
  timer. `seconds` bounds only the host's outstanding-alert bookkeeping. See
  `docs/config/v3.md`'s "Known limitation" subsection for the full mechanism and the
  two unresolved resolution options (host-side-only hold vs. an additive dismissal
  message); neither has been chosen.
- **Multi-widget grids are removed from the roadmap.** The 448x368 panel at desk
  distance cannot render a legible multi-tile dashboard; density was never the goal.
- **Clean canvas:** cards have no status strip or other persistent overlay;
  connection/provider state belongs in the companion settings app.
- **Mounting orientation:** the panel remains 448x368 landscape. Settings offers exactly
  90° (USB below) and 270° (USB above); portrait and on-device rotation gestures are
  intentionally unsupported.

## 5. Widget model (config-composable)

A **widget instance** = data source + display template + tap action +
refresh policy. Users compose these in the settings UI; no code.

- **v1 data sources:**
  - clock/date (device-local once time-synced)
  - pomodoro / interval timer (state machine lives in the app)
  - calendar via ICS URL or file (next events, countdown to next meeting)
  - weather (built-in provider, location from config)
  - generic **JSON feed** (URL + JSONPath-style mapping to template fields)
  - RSS/Atom headline
- **v1 display templates** (each with exactly one 448x368 layout; size classes do not
  exist in the card model):
  - big number + label
  - icon + badge + text
  - list of rows
  - progress ring
  - analog and digital clock faces
- **v1 tap actions:** open URL/app on host · start/pause/reset (stateful
  widgets) · acknowledge/dismiss · none.
- **After v1.0:** OS-native providers (macOS EventKit calendar, Focus
  status; Windows equivalents) behind the same data-source trait; animated
  companion/pet widget as a template family.

## 6. Firmware

- **Stack:** ESP-IDF 5.x, `esp_lcd` panel IO for the CO5300 over QSPI
  (Espressif-maintained driver components as the base), LVGL 9, FreeRTOS
  task split: render/LVGL on one core, USB protocol + logic on the other.
- **Own board layer:** pins, panel init, touch (I²C), backlight/brightness —
  written fresh for this board; structured so a second board is a new
  directory, not a rewrite.
- **Template engine:** LVGL screens are instantiated from the template
  library; a config blob from the app binds data keys to template fields.
  Data updates patch bound values in place — no screen rebuilds on refresh.
- **Assets:** app pushes icons (and later fonts) as compiled LVGL-ready
  binaries; firmware caches them in flash keyed by hash.
- **Firmware updates ship through the companion app** (esptool-rs over the
  same USB port, one-click) — required for distributing new templates and
  protocol upgrades without asking users to install toolchains.

## 7. Companion app

Rust workspace crates:

- `protocol` — shared message definitions, serialization, golden tests
  (single source of truth; firmware C structs generated or conformance-
  tested against it).
- `device` — serial port discovery, connect/reconnect state machine,
  flashing.
- `providers` — all data sources behind one trait
  (`poll() -> Vec<FieldValue>` + declared refresh policy); fixture-testable.
- `engine` — scheduler, config store (single JSON/TOML file), widget state,
  interrupt arbitration.
- Tauri shell — tray menu (pause pushing, open settings, device status),
  settings webview (device preview mock, widget gallery, screen arranger,
  drag-to-reorder carousel).

## 8. Error handling

- **Unplugged / host asleep:** device switches to standalone clock; app
  retries discovery quietly.
- **Provider failure:** widget shows a staleness indicator (last-good data +
  subtle age marker) rather than wrong or blank data.
- **Protocol version mismatch:** app prompts a one-click firmware update;
  device keeps working on last-known-good config meanwhile.
- **Malformed feeds/config:** validated in the app before sending; a widget
  can enter an error state, but bad user input must never crash firmware.
- **App crash / quit:** device shows staleness after missed heartbeats,
  falls back to standalone mode after a timeout.

## 9. Testing

- **Protocol:** golden-file conformance tests run by both the Rust crate and
  firmware-side unit tests.
- **Firmware logic** (template binding, touch routing, interrupt
  arbitration): unit tests in a native (host) build environment; hardware-
  independent code kept separate from the board layer to make this possible.
- **Providers:** unit tests against fixture feeds (ICS, JSON, RSS).
- **Release:** on-hardware smoke checklist (flash, first-run, each template,
  touch actions, unplug/replug, firmware-update path).

## Key decisions log

| Decision | Choice | Why |
|---|---|---|
| Connectivity | USB-C only (power + data) | Zero-provisioning UX; AMOLED power draw makes battery impractical; wireless deferred |
| Rendering | On-device (LVGL) | S3 USB too slow for frame streaming; smooth animation + instant touch |
| App stack | Tauri v2 | Lean Rust daemon (~15–30 MB idle), system webview only while settings open, Windows path without rewrite |
| Extension model | B: config-composable | User power without plugin-API maintenance; templates become future plugin substrate |
| Screen model | **Superseded** — Carousel + clean widgets + future tile dashboards | See `docs/superpowers/specs/2026-08-06-deskmate-card-model-design.md`: multi-widget screens and size classes are removed, not deferred; the panel cannot render a legible tile dashboard at desk distance |
| Mounting orientation | Settings-owned 90°/270° choice | Supports both physical attachment directions without portrait layouts or accidental touch rotation |
| Firmware provenance | From scratch (ESP-IDF + esp_lcd + LVGL) | Clean licensing/history for a distributable product; no rsvpnano reuse |
| Device role | Ambient and contextual | One thing at a time, rotating, with events able to take over; density was never achievable at 448x368 |
| Multi-widget screens | Removed | The panel cannot render four legible tiles at desk distance; removing them also removes per-size template variants |
| Size classes | Removed | `standard` already aliased `full`, firmware has no size concept, and nothing renders at `tile` without a grid |
| Widgets and screens | Merged into `cards[]` | The join was 1:1 and unauthorable; merging collapses three UI panels into one list |
| Contextual behaviour | Timed playlist plus bounded alerts | Smaller change than eligibility-and-priority, and reuses the proven interrupt arbitration |
| Alert triggers | Closed set, timer and calendar only | A general condition rule would reintroduce an evaluator over feed-controlled data, and alerts seize the whole screen |
| Presence and alert | Two fields, one cross-field rule | A card is commonly both in rotation and alerting; collapsing them would make that unrepresentable and would lose settings on mute |
| Dwell time | Global default with per-card override | Most users have one intent; per-card-only would be eight decisions and would hide loop length |
| Applying edits | Explicit save retained | The in-app preview carries the iteration loop; live-apply would put keystroke-rate revision churn through the atomic replay path |
| Preview data | Real last-good field values over IPC | A preview with different data is an illustration; the gap sits exactly on JSON mappings and text overflow |
| Wire format (card model) | Unchanged | `ApplyConfig` is a compilation target, not a mirror of config, so the authoring model can be replaced without moving bytes |
