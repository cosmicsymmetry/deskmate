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
  laptop lid). Parallel workstream, out of scope for this spec, but the
  software must tolerate both portrait orientations (config-selectable
  180° rotation) so the cable can exit either side.
- **Future targets** (e.g. ESP32-P4 round displays) are enabled by a thin
  board abstraction layer in the firmware, but only this board ships in v1.

## 3. Architecture

"Smart terminal" split — the device renders, the app thinks:

- **Firmware** (C++, ESP-IDF 5.x + `esp_lcd` + LVGL 9, built **from
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

## 4. Screen model

- A **carousel of user-ordered screens**; swipe to navigate; optional
  auto-rotate timer.
- Each screen is either **one widget** (at `full` or `standard` size) or a
  **dashboard grid** of 2–4 widgets at `tile` size.
- **Size classes:** `full` (whole panel, status strip hidden), `standard`
  (panel minus status strip), `tile` (grid cell). Each display template
  declares which size classes it supports; templates without `tile` support
  cannot be placed on a dashboard screen — no forced squeezing.
- **Status strip:** slim always-on overlay (time · connection dot ·
  notification dots). The active widget opts out by using `full`.
- **Priority interrupts:** a widget may declare urgency (meeting starting,
  timer finished) and temporarily take over the screen, then yield back to
  the previous screen. Interrupt display uses the widget's `standard` or
  `full` layout plus a dismiss affordance (tap = acknowledge).

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
- **v1 display templates** (each with per-size-class layouts):
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
| Screen model | Carousel + size classes + opt-out status strip | Combines fullscreen, strip, and dashboard grid as one layout system |
| Firmware provenance | From scratch (ESP-IDF + esp_lcd + LVGL) | Clean licensing/history for a distributable product; no rsvpnano reuse |
