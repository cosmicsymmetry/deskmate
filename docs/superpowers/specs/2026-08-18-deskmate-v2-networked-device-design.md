# Deskmate V2 — Networked Device — Design

> **HISTORICAL DESIGN.** V2 was delivered and exited. References to the Mac app describe
> the architecture at that time; the Tauri companion was deleted on 2026-09-18.

**Date:** 2026-08-18
**Status:** Approved section-by-section in brainstorming.
**Prior contracts:** `docs/superpowers/specs/2026-08-03-deskmate-design.md` (base
architecture) and `docs/superpowers/specs/2026-08-11-deskmate-v1-reset-design.md`
(product architecture and roadmap re-cut) remain the reference for everything this
document does not amend. The frozen config contract is `docs/config/v4.md`.

## 0. What V2 delivers

A device that can be owned over the network instead of over a cable, proven against a
real single-tenant server, plus a firmware update mechanism for both tiers.

The headline demonstration: provision a device over USB, unplug the cable, **quit the
Mac app**, and the cards keep updating.

V2 deliberately stops short of accounts, OAuth-held integration credentials, and
multi-tenancy. Those are V3, and they sit behind the device-facing contract this stage
freezes, so V3 replaces the server implementation without touching firmware.

## 1. Decisions taken during brainstorming

| Question | Decision |
|---|---|
| What does the device talk to, given V3's server does not exist? | A real cloud server, built as a single-tenant stub with no accounts. |
| How much does that server do? | It owns runtime state in networked tier and runs providers server-side. Not a relay. |
| Who authors configuration? | Always the Mac. In networked tier it writes through to the server over HTTPS. |
| How is the device provisioned? | Over USB from the Mac. **No SoftAP, no captive portal, no BLE.** |
| How does the device authenticate? | Per-device bearer token in NVS over server-authenticated TLS. No client certificates, no PKI. |
| Does the free tier use the radio? | Yes, for SNTP and firmware updates only. Never for config, widget data, time-sync-from-server, or events. |
| Where does the server run? | A machine the user controls, exposed via Cloudflare Tunnel with a publicly-trusted certificate. |
| What transport carries the protocol? | WebSocket over TLS, carrying today's binary frames unchanged. |
| Does the wire version move? | **No.** V2 is additive within protocol v1. |

### 1.1 Why not the alternatives

**Not a dumb relay.** A relay keeps the Mac as owner and needs no accounts, but it
cannot deliver the one thing the networked tier exists for: the device working while
the Mac is off.

**Not TRMNL's polling model.** TRMNL's device polls `GET /api/display` on an interval
and the server owns everything; that works because e-ink refreshes on a ~15-minute
cadence and the device has essentially no interaction. Deskmate carries taps up
(`device_event_queue.c`) and interrupts down (`interrupt_state.c`), and its
alert-replay-on-reconnect behaviour assumes a live link. The server side must be a
persistent connection even though the calendar data riding it changes hourly.

What *is* borrowed from TRMNL is the shape of the deliverable: the device-facing
contract is the product surface, and any given server is one implementation of it.

**Not a raw TLS socket with protocol v2.** Cloudflare Tunnel proxies HTTP and
WebSocket cleanly and arbitrary TCP poorly. WebSocket also provides a natural place
for the bearer token — an HTTP header at handshake — which removes the need for an
in-band auth message and therefore for a version bump.

**Not a separate HTTP/JSON device API.** It would mean two contracts to keep in sync
forever, duplicated untrusted-input hardening, and it still could not carry taps or
interrupts without adding a socket.

## 2. Ownership model

Device mode is **persistent device state, not a negotiation**. NVS holds a `tier` of
`local` or `networked`, set during pairing. This makes M3's single-owner invariant
enforceable by construction rather than by protocol etiquette.

### 2.1 Local tier (free)

The Mac owns over USB exactly as it does today; M3's runtime is unchanged. The radio
comes up only to reach SNTP and the firmware endpoint. It never carries config, widget
data, time-sync-from-server, or device events, and the device rejects any attempt to
make it.

### 2.2 Networked tier (paid)

The server owns over WSS: it sends time, config, widget data and interrupts, and
receives taps. **A Mac plugged in over USB does not become an owner.** In this mode the
device accepts only a restricted message set on USB — status, provisioning writes, tier
switch, factory reset — and answers config, push-data and time-sync with a typed
rejection.

The Mac's settings UI sees the device report `networked` and writes the user's edits to
the *server* over HTTPS; the server pushes them down. This satisfies "the cable is
needed for initial settings" without the cable ever taking the wheel.

### 2.3 How the Mac knows

An additive `tier` field on `StatusResponse`. This is exactly what M4 Task 1's additive
protocol-v1 handshake and its cross-language compatibility fixtures were built to
permit. `PROTOCOL_VERSION` stays 1.

### 2.4 Recorded consequences

- Pomodoro's tap → event → state → push loop follows ownership. In networked tier a tap
  crosses the internet and back. Acceptable for a 25-minute timer, and stated here so it
  is not discovered as a surprise.
- The PCF85063A RTC (present at `0x51`, currently undriven) **stays undriven in V2**.
  SNTP covers time on both tiers, so a new I2C driver earns nothing at this stage.

## 3. Device network stack and transport

### 3.1 New dependencies

Added to `firmware/main/idf_component.yml` / sdkconfig: `esp_wifi` station mode,
`nvs_flash`, `esp-tls` with `esp_crt_bundle`, `espressif/esp_websocket_client`,
`esp_https_ota`, and SNTP. Trust comes from the bundled Mozilla roots, so the tunnel's
certificate validates with **no pinning** and therefore no coupling between certificate
rotation and firmware updates.

### 3.2 Transport abstraction

A `link_transport_t` vtable in `firmware/main/link/` exposing `read`, `write_frame` and
`dropped_bytes`. `usb_link.c` becomes one implementation; `net_link.c` — an
`esp_websocket_client` whose event callbacks feed a bounded RX ring — becomes the other.
`protocol_task.c` currently calls `usb_link_*` directly at four sites (lines 184, 240,
624, 729); those go through the vtable. It holds **exactly one active transport**,
selected by tier.

`firmware/main/core/` is untouched: same COBS framing, same CBOR messages, same bounds,
same host tests.

**COBS is retained inside WebSocket frames** even though WebSocket already delimits
messages. The redundancy is deliberate: it keeps `protocol_frame.c` and `frame.rs`
literally unchanged and lets one codec serve both transports.

### 3.3 Connection lifecycle

The WebSocket client runs **only in networked tier**. In local tier there is no control
connection to the server at all, which is what makes the free tier's privacy claim true
rather than merely policy.

Connections are outbound-only — no inbound ports, no NAT traversal, no discovery.
Reconnection uses exponential backoff with jitter: 1 s initial, doubling, capped at
60 s, with up to +/-20% jitter so a fleet does not resynchronise onto the server after
an outage.

**`LINK_TIMEOUT_MS` must be parameterised.** Its current 10 s is tuned for a cable; over
the internet it will flap and drop the device to the standalone clock on ordinary
jitter. `link_state.c` gains a per-transport timeout — it is host-tested core, so this
is cheap and directly testable. **USB keeps its existing 10 s; the network transport
uses 45 s.** These are the values the plan implements unless a measurement contradicts
them.

### 3.4 Recorded consequences

- WiFi plus TLS costs roughly 50–60 KB of RAM and moves the image from 0.77 MB toward
  the 1.5–2.5 MB the V1 partition table was sized for. **V2's soak criteria must be
  re-baselined.** V1's byte-flat heap figures are not the comparison.
- Per `CLAUDE.md`, bytes from the server are exactly as untrusted as bytes from USB:
  same bounding, same rejection of malformed input, same recover-without-reboot rule.

## 4. Provisioning, identity and tier switching

### 4.1 Persistent state

V2 introduces the device's first use of NVS — `grep` finds no `nvs_flash` or `nvs_open`
anywhere in `firmware/main/` today. One namespace holds `ssid`, `psk`, `server_url`,
`device_id`, `token`, `tier`, and `utc_offset_minutes`.

Written only over USB. **Secrets are never readable back over the wire**: status reports
presence and state, never values.

**Accepted risk:** the PSK and token sit in unencrypted flash and could be recovered
from a physically stolen unit. This follows from choosing bearer-token identity over
eFuse-backed identity, and is upgradeable later without changing the contract.

### 4.2 Wire surface

Two additions, both gated by a new `CAPABILITY_NETWORKING` (**bit 7**; bits 0–6 are
taken), with `CAPABILITY_FIRMWARE_UPDATE` (bit 6, reserved since M4) switching on:

- `TYPE_NETWORK_CONFIG` (**13**) — a write carrying credentials, server URL, token, UTC
  offset and target tier.
- `TYPE_FACTORY_RESET` (**14**) — erases the namespace (§4.4).

Production types 1–12 are in use; 13 and 14 are the next free values. The dev-diag
capture ids `0x7E`/`0x7F` are unaffected and remain absent from plain builds.
- Additive `StatusResponse` fields — tier, WiFi state, RSSI, IP, last network error,
  OTA state.

New message *types* remain compatible within protocol v1: the host sends them only when
the device advertises the capability, and unknown types are already rejected without
reboot. This is what the capability handshake exists for.

### 4.3 Pairing and unpairing

**To networked:** the Mac asks the server to mint a device identity, receives
`device_id` and `token`, writes credentials plus `tier = networked` over USB, and the
device restarts into networked mode and dials out.

**To local:** the Mac writes `tier = local`; the device drops the connection and the
server releases it.

Ownership changes only because a human plugged in a cable and asked. Never
automatically, never on a timeout.

### 4.4 Failure direction

Missing, corrupt or half-written NVS drops the device to local tier and the standalone
clock, consistent with the existing rule that the standalone clock survives boot, host
loss, malformed input and version mismatch.

A **factory-reset message** erases the namespace, so a device pointed at a dead server
is recoverable over the cable rather than by reflash.

### 4.5 Accepted limitation

Changing WiFi networks requires the cable. This follows directly from having no SoftAP.

## 5. The server

A new **`crates/server`** in the existing Rust workspace, reusing `crates/protocol` for
the codec, `crates/app-core` for the single-owner runtime, and `crates/providers` for
weather, ICS calendar, RSS and JSON feeds.

Reusing `app-core` is the point: the ownership runtime M3 built and hardened is the same
code server-side, so "who owns the device" has one implementation rather than two that
must be kept in agreement.

The `companion/` directory name becomes slightly inaccurate. Renaming is churn V2 skips.

### 5.1 Device-facing surface

- `GET /v1/device/link` — WebSocket upgrade, `Authorization: Bearer`. Networked tier
  only. Carries today's frames.
- `GET /v1/device/firmware` — bearer-authenticated update check. The **only** endpoint a
  local-tier device ever touches.
- `GET /v1/firmware/<version>.bin` — the image, for `esp_https_ota`.

### 5.2 Mac-facing surface

Mint a device identity, write config, read device state. Authenticated in V2 by a
**single admin token**, the explicit stand-in for V3's accounts.

### 5.3 Deliberately primitive

Config persists as a schema-v4 JSON file on disk, not a database. Single-tenant, so
exactly one `app-core` runtime. No accounts, no OAuth, no multi-tenancy, no billing, no
plugin rendering.

### 5.4 Deployment

A single binary plus `cloudflared` on a machine the user controls, run under systemd or
launchd.

### 5.5 Time

Local tier takes time from **SNTP directly** — it never contacts the Deskmate server for
it, so the free tier's only server dependency is firmware. Because SNTP yields UTC with
no zone, the **UTC offset is written to NVS during provisioning**, which is what lets the
standalone clock show correct local time with no host ever attached.

## 6. Firmware update and rollback

The mechanism is already provisioned for by V1: `ota_0`/`ota_1` at 4 MB each, `otadata`
present, OTA-aware bootloader. V2 adds the code.

**Delivery.** `esp_https_ota` streams the image into the inactive slot over HTTPS through
the tunnel. Local tier checks `/v1/device/firmware` at boot and every 24 h
thereafter, with jitter; networked tier is told over the existing link. A ~2.5 MB image into a 4 MB slot has ample
headroom.

**Rollback.** `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE` is enabled. A newly flashed image
boots as `PENDING_VERIFY` and calls `esp_ota_mark_app_valid_cancel_rollback()` only after
a self-check passes; otherwise the next reset returns to the previous slot.

**The self-check is local-only, and this is load-bearing.** Requiring "reached the
server" as proof of a good image would make a router outage, an expired tunnel, or a
server restart roll back a perfectly good image — on every device at once. The validity
gate is therefore: display up, LVGL running, protocol task alive, NVS readable. Network
reachability is a runtime condition, never a validity condition.

**Restraint on timing.** No update begins while an interrupt is live or a pomodoro is
running; it defers. An update that consumes a focus session to install itself is a defect
regardless of correctness.

**On-panel feedback.** A takeover showing progress. A panel frozen for the length of a
multi-megabyte download over a home connection reads as a crash, and glanceability is the
product.

**Recovery stays USB reflash**, per the reset spec §5.1's recorded consequence of having
no factory recovery slot. Power loss mid-write is safe by construction: a partially
written slot is not booted. The board's battery narrows the window further.

## 7. Verification and exit gate

### 7.1 Automated

The tier state machine and the parsing and validation of stored network config live in
`main/core/` as plain C with no ESP-IDF includes, covered by
`make -C firmware/host_tests test`. Only NVS and socket I/O live in `main/link/`.

The new message type and additive status fields extend the cross-language compatibility
fixtures built in M4 Task 1 — the corpus that already proves `crates/protocol` and
`protocol_message.c` agree.

Malformed-input coverage is re-run **over the WSS path**.

`crates/server` gets its own tests: bearer rejection, config write-through, firmware
manifest, single-owner enforcement.

### 7.2 Physical exit gate

Recorded in `docs/hardware/board-notes.md`, using the webcam harness (`tools/hwcam/`) for
panel observations per the standing preference.

1. **Headline demo.** Provision over USB, unplug the cable, quit the Mac app, and watch
   cards keep updating from the server. Prove this first.
2. Tier switch in **both** directions, including the device refusing config over USB
   while networked.
3. Link loss and recovery — server down, WiFi down — falling to the standalone clock and
   recovering **without a reboot**.
4. OTA across a real version change, **plus a deliberately bad image proving rollback**.
5. Free tier verified radio-silent for config and data: correct SNTP time, working
   firmware check, and **no control connection in the server's logs**.
6. A re-baselined 30-minute mixed soak with the radio up: flat heap at the *new*
   baseline, bounded queue high-water.
7. **Alert replay across a network reconnect**, since that path has produced real defects
   twice.

## 8. Out of scope for V2

Accounts; Google OAuth and any server-held integration credentials; multi-tenancy;
billing; the plugin contract, image cards and headless-Chromium rendering; SoftAP, BLE
and captive-portal provisioning; LAN discovery and mDNS; secure boot and flash
encryption; a PCF85063A driver; portrait orientations; dashboards; raising the 8-widget
wire cap; and any protocol **version** bump. V2 is additive within protocol v1.

## 9. Risks and open items

- **First NVS use.** Corruption, half-writes and wear are new failure modes for this
  firmware. §4.4 fixes the failure direction; the plan must test it explicitly rather
  than assume it.
- **Timeout retuning.** §3.3's parameterised `LINK_TIMEOUT_MS` changes behaviour on the
  USB path too if set carelessly. The USB value must not move.
- **Re-baselined soak.** Because V1's heap figures stop being comparable, a regression
  could hide inside the re-baselining. The plan should capture the new baseline before
  the soak, not derive it from the soak.
- **Carried forward, still unresolved:** `unknown_field_count` remains unobservable on
  hardware. It is not V2's to fix and may not be described as resolved.
- **Carried forward, now RESOLVED 2026-08-21:** `progress-ring--running-mid-countdown`
  could not pass the framebuffer diff deterministically. It is now golden-only and
  excluded from the hardware comparison, with `paused-mid-countdown` and
  `running-at-zero` added to keep hardware coverage. The gate's framebuffer diff should
  now read `total=58 identical=54 differing=0 excluded=4`; see `CLAUDE.md`.

## 10. Roadmap impact

The roadmap's V2 row is replaced by this document's scope. V3's row narrows accordingly:
with the device-facing contract, identity, provisioning, transport and update mechanism
delivered here, **V3 becomes accounts, OAuth-held integrations, config storage and
multi-tenancy behind an unchanged contract** — a server-side milestone that should
require no firmware change.
