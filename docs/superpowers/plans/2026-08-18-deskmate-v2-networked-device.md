# V2 Networked Device Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a Deskmate device be owned over WiFi by a server instead of by a cable, so
the panel keeps updating with the Mac app quit — plus a firmware update mechanism with
rollback for both tiers.

**Architecture:** Device mode (`local` or `networked`) is persistent state in NVS, so the
single-owner invariant is enforced by construction rather than protocol etiquette. The
existing COBS/CBOR frames ride unchanged over a WebSocket-over-TLS connection to a
single-tenant Rust server that reuses `app-core` for ownership and `providers` for data.
`firmware/main/core/` gains pure-C validation and tier logic but no ESP-IDF includes;
all radio, socket and NVS I/O lives in `firmware/main/link/`.

**Tech Stack:** ESP-IDF 5.3+/C with LVGL 9 (`esp_wifi`, `nvs_flash`, `esp-tls` +
`esp_crt_bundle`, `esp_websocket_client`, `esp_https_ota`, SNTP); Rust (workspace crates
`protocol`, `app-core`, `providers`, new `server`; axum + tokio-tungstenite); React +
TypeScript with Bun test (companion settings UI).

**Spec:** `docs/superpowers/specs/2026-08-18-deskmate-v2-networked-device-design.md`

## Global Constraints

Copied from the spec and `CLAUDE.md`. Every task's requirements implicitly include this
section.

- **The wire stays protocol v1.** `PROTOCOL_VERSION` and `MAX_PROTOCOL_VERSION` remain
  `1`. V2 is additive only: new message types and new CBOR map keys, never a version bump.
- New message types are **`TYPE_NETWORK_CONFIG = 13`** and **`TYPE_FACTORY_RESET = 14`**.
  Production types 1–12 are in use. Dev-diag ids `0x7E`/`0x7F` are reserved and must stay
  absent from plain builds.
- New capability bit is **`CAPABILITY_NETWORKING = 1 << 7`**. Bits 0–6 are taken.
  **`CAPABILITY_FIRMWARE_UPDATE = 1 << 6`** (reserved since M4) switches on in Task 11.
- **Link timeouts:** USB keeps its existing `PROTOCOL_LINK_TIMEOUT_MS` of **10000 ms**;
  the network transport uses **45000 ms**. The USB value must not move.
- **Reconnect backoff:** 1 s initial, doubling, capped at **60 s**, with ±20% jitter.
- **Local-tier firmware check:** at boot and every **24 h** thereafter, with jitter.
- **Config schema stays v4** (`docs/config/v4.md`). No migration, no card-model change.
- **Secrets are never readable back over the wire.** Status reports presence and state,
  never the value of a PSK or token.
- **Failure direction is always toward local tier and the standalone clock.** Missing,
  corrupt or half-written NVS must not brick, loop, or strand the device.
- Bytes from the server are **exactly as untrusted as bytes from USB**: bound lengths and
  counts, reject malformed input, recover framing without rebooting.
- Keep hardware-independent logic in `firmware/main/core/` free of ESP-IDF includes and
  host-testable as plain C. Guard LVGL calls outside LVGL callbacks with
  `lvgl_port_lock()`/`lvgl_port_unlock()`.
- Every commit must leave `make -C firmware/host_tests clean test` and
  `cargo test --workspace` green.
- Conventional commit prefixes (`feat:`, `fix:`, `test:`, `docs:`, `chore:`).
- **Never claim hardware verification that was not observed on the physical board.**

## Bounds introduced by this plan

Used verbatim by every task that touches these fields.

| Constant | Value | Applies to |
| --- | --- | --- |
| `PROTOCOL_MAX_SSID_LENGTH` | 32 | WiFi SSID (IEEE 802.11 maximum) |
| `PROTOCOL_MAX_PSK_LENGTH` | 64 | WPA2 passphrase (64 = raw PSK hex) |
| `PROTOCOL_MAX_SERVER_URL_LENGTH` | 128 | `wss://host/path` |
| `PROTOCOL_MAX_DEVICE_TOKEN_LENGTH` | 128 | Bearer token |
| `PROTOCOL_MAX_DEVICE_ID_LENGTH` | 32 | Server-assigned device id |
| `PROTOCOL_MAX_IP_LENGTH` | 15 | Dotted-quad IPv4 |

## File Structure

| File | Change | Responsibility |
| --- | --- | --- |
| `firmware/main/core/protocol_message.h` | Modify | New types, capability bit, bounds, `protocol_network_config_t`, tier/wifi/ota enums, additive status fields |
| `firmware/main/core/protocol_message.c` | Modify | Encode/decode for types 13 and 14; additive status keys |
| `firmware/main/core/net_config.c/.h` | Create | Pure-C validation of network config + tier transition rules |
| `firmware/main/core/link_state.c/.h` | Modify | Per-transport timeout |
| `firmware/main/link/link_transport.h` | Create | `link_transport_t` vtable |
| `firmware/main/link/usb_link.c/.h` | Modify | Implement the vtable |
| `firmware/main/link/net_store.c/.h` | Create | NVS persistence of network config and tier |
| `firmware/main/link/wifi_station.c/.h` | Create | `esp_wifi` station lifecycle + SNTP |
| `firmware/main/link/net_link.c/.h` | Create | WebSocket transport implementing the vtable |
| `firmware/main/link/ota.c/.h` | Create | `esp_https_ota` download, rollback self-check, deferral |
| `firmware/main/link/protocol_task.c` | Modify | Transport selection, restricted USB message set in networked tier |
| `firmware/main/ui/ota_screen.c/.h` | Create | On-panel update progress takeover |
| `firmware/host_tests/*` | Modify/Create | `test_net_config`, extended `test_protocol` and `test_link_state` |
| `companion/crates/protocol/src/message.rs` | Modify | Rust mirror of every wire change |
| `companion/crates/protocol/examples/generate-fixtures.rs` | Modify | Golden frames for the new types |
| `protocol/fixtures/v1/*.bin` | Create | Cross-language golden frames |
| `companion/crates/server/` | Create | Single-tenant server: device link, admin API, firmware endpoints, deploy units |
| `companion/crates/server/tests/hostile_device.rs` | Create | Malformed frames sent to the server over WSS |
| `companion/crates/device/examples/wss_malformed_check.rs` | Create | Malformed frames sent to the board over WSS; asserts no reboot |
| `companion/crates/device/src/session.rs` | Modify | Provisioning and factory-reset calls |
| `companion/apps/deskmate/src/components/NetworkPanel.tsx` | Create | Provisioning and tier UI |
| `docs/hardware/board-notes.md` | Modify | Physical observations |
| `CLAUDE.md`, roadmap, spec status | Modify | Durable state |

---

### Task 1:1 Wire contract — new types, capability, bounds

Everything downstream depends on these names, so this task defines them in both
languages and pins them with cross-language golden frames. No radio, no NVS, no server.

**Files:**
- Modify: `firmware/main/core/protocol_message.h`, `firmware/main/core/protocol_message.c`
- Modify: `companion/crates/protocol/src/message.rs`
- Modify: `companion/crates/protocol/examples/generate-fixtures.rs`
- Modify: `companion/crates/protocol/tests/fixtures.rs`
- Modify: `firmware/host_tests/test_protocol.c`
- Create: `protocol/fixtures/v1/network_config.bin`, `protocol/fixtures/v1/factory_reset.bin`, `protocol/fixtures/v1/status_response_networked.bin`

**Interfaces:**
- Consumes: nothing.
- Produces: `protocol_network_config_t`, `protocol_tier_t`, `protocol_wifi_state_t`,
  `protocol_ota_state_t`, `PROTOCOL_TYPE_NETWORK_CONFIG`, `PROTOCOL_TYPE_FACTORY_RESET`,
  `PROTOCOL_CAPABILITY_NETWORKING`, and the Rust mirrors `NetworkConfig`, `Tier`,
  `WifiState`, `OtaState`. Every later task uses these names exactly as spelled here.

- [x] **Step 1: Write the failing Rust round-trip test**

Append to `companion/crates/protocol/src/message.rs`'s test module:

```rust
    #[test]
    fn network_config_round_trips() {
        let message = Message::NetworkConfig(NetworkConfig {
            ssid: "home-network".into(),
            psk: "correct horse battery staple".into(),
            server_url: "wss://deskmate.example.com/v1/device/link".into(),
            device_id: "dev-0001".into(),
            token: "t".repeat(MAX_DEVICE_TOKEN_LEN),
            utc_offset_minutes: 240,
            tier: Tier::Networked,
        });
        let frame = encode_message(&message, 7).expect("encode");
        let decoded = decode_message(&frame).expect("decode");
        assert_eq!(decoded, message);
    }

    #[test]
    fn network_config_rejects_oversized_ssid() {
        let message = Message::NetworkConfig(NetworkConfig {
            ssid: "s".repeat(MAX_SSID_LEN + 1),
            psk: String::new(),
            server_url: "wss://example.com/l".into(),
            device_id: "dev-0001".into(),
            token: "token".into(),
            utc_offset_minutes: 0,
            tier: Tier::Networked,
        });
        assert!(encode_message(&message, 8).is_err());
    }

    #[test]
    fn factory_reset_round_trips() {
        let frame = encode_message(&Message::FactoryReset, 9).expect("encode");
        assert_eq!(decode_message(&frame).expect("decode"), Message::FactoryReset);
    }
```

- [x] **Step 2: Run the tests to verify they fail**

From `companion/`:

```sh
cargo test -p protocol network_config factory_reset
```

Expected: compile failure — `NetworkConfig`, `Tier`, `MAX_SSID_LEN` and
`Message::FactoryReset` do not exist.

- [x] **Step 3: Add the Rust constants and types**

In `companion/crates/protocol/src/message.rs`, beside the existing capability constants:

```rust
pub const CAPABILITY_NETWORKING: u64 = 1 << 7;

pub const MAX_SSID_LEN: usize = 32;
pub const MAX_PSK_LEN: usize = 64;
pub const MAX_SERVER_URL_LEN: usize = 128;
pub const MAX_DEVICE_TOKEN_LEN: usize = 128;
pub const MAX_DEVICE_ID_LEN: usize = 32;
pub const MAX_IP_LEN: usize = 15;

pub const TYPE_NETWORK_CONFIG: u8 = 13;
pub const TYPE_FACTORY_RESET: u8 = 14;

/// Returned when a message is valid but not permitted on this transport in
/// the device's current tier -- see `net_config_usb_message_allowed`.
pub const ERROR_WRONG_TIER: u8 = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Local = 0,
    Networked = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WifiState {
    Down = 0,
    Connecting = 1,
    Connected = 2,
    Failed = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OtaState {
    Idle = 0,
    Checking = 1,
    Downloading = 2,
    PendingVerify = 3,
    Failed = 4,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkConfig {
    pub ssid: String,
    pub psk: String,
    pub server_url: String,
    pub device_id: String,
    pub token: String,
    pub utc_offset_minutes: i16,
    pub tier: Tier,
}
```

Add `NetworkConfig(NetworkConfig)` and `FactoryReset` variants to `Message`, and encode
`NetworkConfig` as a deterministic CBOR map with integer keys `1..=7` in the field order
above, matching how `ApplyConfig` is encoded. `FactoryReset` carries an empty payload,
matching `Heartbeat`. Enforce every bound from the table in the Global Constraints
section during encode **and** decode; `utc_offset_minutes` reuses the existing
`MIN_UTC_OFFSET_MINUTES`/`MAX_UTC_OFFSET_MINUTES` range.

- [x] **Step 4: Run the Rust tests to verify they pass**

```sh
cargo test -p protocol network_config factory_reset
```

Expected: PASS.

- [x] **Step 5: Add the additive StatusResponse fields**

Extend `StatusResponse` with `tier: Tier`, `wifi_state: WifiState`, `wifi_rssi: i8`,
`ip: String`, `ota_state: OtaState`, and `last_network_error: Option<String>` (bounded by
`MAX_DIAGNOSTIC_LEN`). Encode them under **new integer keys appended after the highest
key currently used**, so an older decoder skips them as unknown. Do not renumber any
existing key. Add:

```rust
    #[test]
    fn status_response_network_fields_round_trip() {
        let mut status = StatusResponse::current();
        status.tier = Tier::Networked;
        status.wifi_state = WifiState::Connected;
        status.wifi_rssi = -58;
        status.ip = "192.168.1.42".into();
        status.ota_state = OtaState::Idle;
        let frame = encode_message(&Message::StatusResponse(status.clone()), 3).expect("encode");
        let Message::StatusResponse(decoded) = decode_message(&frame).expect("decode") else {
            panic!("wrong variant");
        };
        assert_eq!(decoded, status);
    }
```

If `StatusResponse::current()` does not exist, use whichever constructor
`generate-fixtures.rs` already uses for `status_response.bin`.

- [x] **Step 6: Generate the new golden fixtures**

Add three frames to `companion/crates/protocol/examples/generate-fixtures.rs`:
`network_config.bin` (all fields at maximum length), `factory_reset.bin`, and
`status_response_networked.bin` (tier networked, WiFi connected). Then from `companion/`:

```sh
cargo run -p protocol --example generate-fixtures
```

Add the three names to the `valid_golden_frames_decode` list in
`companion/crates/protocol/tests/fixtures.rs`.

- [x] **Step 7: Write the failing C fixture assertions**

In `firmware/host_tests/test_protocol.c`, inside `test_valid_fixtures()`, add:

```c
    assert_valid_fixture("network_config.bin", PROTOCOL_TYPE_NETWORK_CONFIG, 7U);
    assert_valid_fixture("factory_reset.bin", PROTOCOL_TYPE_FACTORY_RESET, 9U);
    assert_valid_fixture("status_response_networked.bin",
                         PROTOCOL_TYPE_STATUS_RESPONSE, 3U);
```

The third argument is the frame's sequence number; use the values passed to
`encode_message` in Step 6.

- [x] **Step 8: Run the C tests to verify they fail**

```sh
make -C firmware/host_tests clean test
```

Expected: `test_protocol` FAILS — the C decoder rejects type 13 as unsupported.

- [x] **Step 9: Mirror the contract in C**

In `firmware/main/core/protocol_message.h` add the two enum members to
`protocol_message_type_t`, add `PROTOCOL_ERROR_WRONG_TIER = 15` to
`protocol_error_code_t` (codes 1-14 are in use), the capability macro, the six
length macros, and:

```c
typedef enum {
    PROTOCOL_TIER_LOCAL = 0,
    PROTOCOL_TIER_NETWORKED = 1,
} protocol_tier_t;

typedef enum {
    PROTOCOL_WIFI_DOWN = 0,
    PROTOCOL_WIFI_CONNECTING = 1,
    PROTOCOL_WIFI_CONNECTED = 2,
    PROTOCOL_WIFI_FAILED = 3,
} protocol_wifi_state_t;

typedef enum {
    PROTOCOL_OTA_IDLE = 0,
    PROTOCOL_OTA_CHECKING = 1,
    PROTOCOL_OTA_DOWNLOADING = 2,
    PROTOCOL_OTA_PENDING_VERIFY = 3,
    PROTOCOL_OTA_FAILED = 4,
} protocol_ota_state_t;

typedef struct {
    char ssid[PROTOCOL_MAX_SSID_LENGTH + 1U];
    char psk[PROTOCOL_MAX_PSK_LENGTH + 1U];
    char server_url[PROTOCOL_MAX_SERVER_URL_LENGTH + 1U];
    char device_id[PROTOCOL_MAX_DEVICE_ID_LENGTH + 1U];
    char token[PROTOCOL_MAX_DEVICE_TOKEN_LENGTH + 1U];
    int16_t utc_offset_minutes;
    protocol_tier_t tier;
} protocol_network_config_t;
```

Add `protocol_network_config_t network_config;` to the message union, and the six new
fields to `protocol_status_response_t`. Implement decode for type 13 and 14 in
`protocol_message.c` following the existing `PROTOCOL_TYPE_APPLY_CONFIG` decoder: reject
unknown or duplicate keys, reject any string exceeding its bound, and reject an
out-of-range `utc_offset_minutes`. Encode the additive status keys in the same
deterministic key order used by the Rust side.

- [x] **Step 10: Run the C tests to verify they pass**

```sh
make -C firmware/host_tests clean test
```

Expected: all pass, including the three new fixture assertions.

- [x] **Step 11: Add rejection tests for malformed network config**

In `firmware/host_tests/test_protocol.c`, add a test that a `network_config` frame whose
`ssid` is 33 bytes decodes to `PROTOCOL_MESSAGE_ERR_*` rather than succeeding, and that a
frame with an unknown map key inside the network-config map is rejected. Build the
malformed bytes by hand in the test, as the existing malformed-frame tests do.

- [x] **Step 12: Run the full gates**

From the repository root:

```sh
make -C firmware/host_tests clean test
```

From `companion/`:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: all pass.

- [x] **Step 13: Commit**

```bash
git add firmware/main/core/protocol_message.h firmware/main/core/protocol_message.c \
        firmware/host_tests/test_protocol.c \
        companion/crates/protocol/src/message.rs \
        companion/crates/protocol/examples/generate-fixtures.rs \
        companion/crates/protocol/tests/fixtures.rs \
        protocol/fixtures/v1
git commit -m "feat: protocol carries network config and factory reset

Types 13 and 14 plus CAPABILITY_NETWORKING (bit 7), and six additive
StatusResponse fields reporting tier, WiFi and OTA state. The wire stays
protocol v1: new types are gated by the capability handshake and new status
keys are appended, so an older decoder skips them.

Secrets travel down only. Status reports presence and state, never the value
of a PSK or token.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01SwAMBn6h8WMLU4ADiA8LwG"
```

---

### Task 2:2 Tier state machine and config validation (pure C)

The rules that decide whether a stored config is usable, and what tier the device runs
in, belong in `core/` where they are host-testable without a board. This task writes no
ESP-IDF code.

**Files:**
- Create: `firmware/main/core/net_config.c`, `firmware/main/core/net_config.h`
- Create: `firmware/host_tests/test_net_config.c`
- Modify: `firmware/host_tests/Makefile`

**Interfaces:**
- Consumes: `protocol_network_config_t`, `protocol_tier_t` from Task 1.
- Produces: `net_config_validate()`, `net_config_effective_tier()`,
  `net_config_usb_message_allowed()`, and `net_config_error_t`. Tasks 3, 8 and 11 call
  these by exactly these names.

- [x] **Step 1: Write the failing test**

Create `firmware/host_tests/test_net_config.c`:

```c
#include <assert.h>
#include <string.h>

#include "../main/core/net_config.h"

static protocol_network_config_t valid_networked(void)
{
    protocol_network_config_t config;
    memset(&config, 0, sizeof(config));
    strcpy(config.ssid, "home-network");
    strcpy(config.psk, "correct horse battery staple");
    strcpy(config.server_url, "wss://deskmate.example.com/v1/device/link");
    strcpy(config.device_id, "dev-0001");
    strcpy(config.token, "abc123");
    config.utc_offset_minutes = 240;
    config.tier = PROTOCOL_TIER_NETWORKED;
    return config;
}

static void test_valid_networked_config_passes(void)
{
    protocol_network_config_t config = valid_networked();
    assert(net_config_validate(&config) == NET_CONFIG_OK);
    assert(net_config_effective_tier(&config) == PROTOCOL_TIER_NETWORKED);
}

// A networked device with no server URL cannot reach anyone. The rule from the
// spec is that every failure lands in local tier with the standalone clock, so
// this must degrade rather than be rejected into an unusable state.
static void test_networked_without_server_url_falls_back_to_local(void)
{
    protocol_network_config_t config = valid_networked();
    config.server_url[0] = '\0';
    assert(net_config_validate(&config) == NET_CONFIG_ERR_MISSING_SERVER_URL);
    assert(net_config_effective_tier(&config) == PROTOCOL_TIER_LOCAL);
}

static void test_networked_without_token_falls_back_to_local(void)
{
    protocol_network_config_t config = valid_networked();
    config.token[0] = '\0';
    assert(net_config_validate(&config) == NET_CONFIG_ERR_MISSING_TOKEN);
    assert(net_config_effective_tier(&config) == PROTOCOL_TIER_LOCAL);
}

static void test_networked_without_ssid_falls_back_to_local(void)
{
    protocol_network_config_t config = valid_networked();
    config.ssid[0] = '\0';
    assert(net_config_validate(&config) == NET_CONFIG_ERR_MISSING_SSID);
    assert(net_config_effective_tier(&config) == PROTOCOL_TIER_LOCAL);
}

// A plaintext ws:// URL would carry the bearer token in the clear.
static void test_non_tls_server_url_is_rejected(void)
{
    protocol_network_config_t config = valid_networked();
    strcpy(config.server_url, "ws://deskmate.example.com/v1/device/link");
    assert(net_config_validate(&config) == NET_CONFIG_ERR_INSECURE_URL);
    assert(net_config_effective_tier(&config) == PROTOCOL_TIER_LOCAL);
}

// Local tier needs no credentials at all -- an empty config is a valid local
// device, which is what a factory-fresh board is.
static void test_empty_config_is_a_valid_local_device(void)
{
    protocol_network_config_t config;
    memset(&config, 0, sizeof(config));
    config.tier = PROTOCOL_TIER_LOCAL;
    assert(net_config_validate(&config) == NET_CONFIG_OK);
    assert(net_config_effective_tier(&config) == PROTOCOL_TIER_LOCAL);
}

// Local tier still joins WiFi for SNTP and firmware, so an SSID is meaningful
// there, but its absence is not an error.
static void test_local_tier_without_ssid_is_not_an_error(void)
{
    protocol_network_config_t config;
    memset(&config, 0, sizeof(config));
    config.tier = PROTOCOL_TIER_LOCAL;
    strcpy(config.ssid, "home-network");
    assert(net_config_validate(&config) == NET_CONFIG_OK);
}

// The restricted USB message set in networked tier. Status, provisioning and
// reset are always allowed; anything that would make the cable an owner is not.
static void test_usb_message_gate_in_networked_tier(void)
{
    assert(net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                          PROTOCOL_TYPE_STATUS_REQUEST));
    assert(net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                          PROTOCOL_TYPE_NETWORK_CONFIG));
    assert(net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                          PROTOCOL_TYPE_FACTORY_RESET));
    assert(net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                          PROTOCOL_TYPE_HEARTBEAT));

    assert(!net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                           PROTOCOL_TYPE_APPLY_CONFIG));
    assert(!net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                           PROTOCOL_TYPE_PUSH_DATA));
    assert(!net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                           PROTOCOL_TYPE_TIME_SYNC));
    assert(!net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                           PROTOCOL_TYPE_ACTIVATE_SCREEN));
    assert(!net_config_usb_message_allowed(PROTOCOL_TIER_NETWORKED,
                                           PROTOCOL_TYPE_TRIGGER_INTERRUPT));
}

// In local tier the cable owns the device, so nothing is gated.
static void test_usb_message_gate_in_local_tier_allows_everything(void)
{
    assert(net_config_usb_message_allowed(PROTOCOL_TIER_LOCAL,
                                          PROTOCOL_TYPE_APPLY_CONFIG));
    assert(net_config_usb_message_allowed(PROTOCOL_TIER_LOCAL,
                                          PROTOCOL_TYPE_PUSH_DATA));
    assert(net_config_usb_message_allowed(PROTOCOL_TIER_LOCAL,
                                          PROTOCOL_TYPE_TIME_SYNC));
}

int main(void)
{
    test_valid_networked_config_passes();
    test_networked_without_server_url_falls_back_to_local();
    test_networked_without_token_falls_back_to_local();
    test_networked_without_ssid_falls_back_to_local();
    test_non_tls_server_url_is_rejected();
    test_empty_config_is_a_valid_local_device();
    test_local_tier_without_ssid_is_not_an_error();
    test_usb_message_gate_in_networked_tier();
    test_usb_message_gate_in_local_tier_allows_everything();
    return 0;
}
```

- [x] **Step 2: Register the test target**

In `firmware/host_tests/Makefile`, add `test_net_config` to the `test:` prerequisite
list and to the run list, then add the rule:

```make
test_net_config: test_net_config.c ../main/core/net_config.c $(PROTOCOL_SRCS)
	$(CC) $(CFLAGS) -I$(CBOR_DIR) -o $@ $^
```

- [x] **Step 3: Run the test to verify it fails**

```sh
make -C firmware/host_tests clean test
```

Expected: compile failure — `net_config.h` does not exist.

- [x] **Step 4: Write the header**

Create `firmware/main/core/net_config.h`:

```c
#pragma once

#include <stdbool.h>

#include "protocol_message.h"

typedef enum {
    NET_CONFIG_OK = 0,
    NET_CONFIG_ERR_MISSING_SSID,
    NET_CONFIG_ERR_MISSING_SERVER_URL,
    NET_CONFIG_ERR_MISSING_TOKEN,
    NET_CONFIG_ERR_INSECURE_URL,
} net_config_error_t;

/**
 * Validate a stored or received network config.
 *
 * Local tier requires nothing: a zeroed config is a valid factory-fresh local
 * device. Networked tier requires an SSID, a wss:// server URL and a token.
 */
net_config_error_t net_config_validate(const protocol_network_config_t *config);

/**
 * The tier the device may actually run in, which is never more privileged than
 * the tier the config asks for. Anything that fails validation degrades to
 * local, per the spec's rule that failure always lands on the standalone clock.
 */
protocol_tier_t net_config_effective_tier(const protocol_network_config_t *config);

/**
 * Whether a message arriving over USB may be acted on in the given tier.
 *
 * In networked tier the cable is a configurator, not an owner: it may read
 * status, write provisioning, and factory-reset, but may not apply config,
 * push data, sync time, activate screens or trigger interrupts.
 */
bool net_config_usb_message_allowed(protocol_tier_t tier,
                                    protocol_message_type_t type);
```

- [x] **Step 5: Write the minimal implementation**

Create `firmware/main/core/net_config.c` implementing exactly the three functions. Use
`strncmp(config->server_url, "wss://", 6) == 0` for the TLS check and treat a
`NULL` config as `NET_CONFIG_ERR_MISSING_SSID` with an effective tier of local. Include
only `<string.h>` and the two project headers — no ESP-IDF.

- [x] **Step 6: Run the test to verify it passes**

```sh
make -C firmware/host_tests clean test
```

Expected: all pass, including `test_net_config`.

- [x] **Step 7: Commit**

```bash
git add firmware/main/core/net_config.c firmware/main/core/net_config.h \
        firmware/host_tests/test_net_config.c firmware/host_tests/Makefile
git commit -m "feat: tier state machine and network config validation

Pure C in core/, host-tested without a board. Two rules the rest of V2 leans
on: every validation failure degrades to local tier rather than to an unusable
state, and in networked tier the USB link may read status, write provisioning
and factory-reset but may not apply config, push data or sync time -- the cable
is a configurator, not an owner.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01SwAMBn6h8WMLU4ADiA8LwG"
```

---

### Task 3:3 NVS persistence and provisioning over USB

The device's first use of NVS. This task ends with a real, board-testable deliverable
that needs no radio at all: `deskmate-cli` can provision the device over the cable, the
tier survives a reboot, status reports it, and a networked device refuses ownership
messages on USB.

**Files:**
- Create: `firmware/main/link/net_store.c`, `firmware/main/link/net_store.h`
- Modify: `firmware/main/link/protocol_task.c`
- Modify: `firmware/main/main.c` (NVS init)
- Modify: `companion/crates/device/src/session.rs`
- Modify: `companion/crates/deskmate-cli/src/main.rs`

**Interfaces:**
- Consumes: Task 1's `protocol_network_config_t`, `PROTOCOL_TYPE_NETWORK_CONFIG`,
  `PROTOCOL_TYPE_FACTORY_RESET`, `PROTOCOL_ERROR_WRONG_TIER`; Task 2's
  `net_config_validate()`, `net_config_effective_tier()`,
  `net_config_usb_message_allowed()`.
- Produces: `net_store_load()`, `net_store_save()`, `net_store_erase()`, and
  `net_store_current_tier()`. Tasks 6, 8 and 11 read the stored config through
  `net_store_load()`.

- [x] **Step 1: Write the header**

Create `firmware/main/link/net_store.h`:

```c
#pragma once

#include "esp_err.h"

#include "core/protocol_message.h"

/**
 * Load the persisted network config.
 *
 * On a factory-fresh device, on a missing namespace, or on any read error, the
 * config is zeroed and the tier is local. This function never fails in a way
 * the caller must handle differently -- the failure direction is always toward
 * local tier and the standalone clock.
 */
void net_store_load(protocol_network_config_t *out);

/** Persist a validated config. Returns ESP_ERR_INVALID_ARG on a NULL argument. */
esp_err_t net_store_save(const protocol_network_config_t *config);

/** Erase the namespace, returning the device to factory-fresh local tier. */
esp_err_t net_store_erase(void);

/** The effective tier of the persisted config, cached after first load. */
protocol_tier_t net_store_current_tier(void);
```

- [x] **Step 2: Implement the store**

Create `firmware/main/link/net_store.c`. Use one namespace `"deskmate"` with
`nvs_get_str`/`nvs_set_str` per field and `nvs_get_i8` for `tier`, `nvs_get_i16` for
`utc_offset_minutes`. Rules the implementation must follow:

- `net_store_load()` zeroes `*out` first, then fills what it can. Any `nvs_open` or
  `nvs_get_*` error leaves that field empty rather than aborting the load.
- After loading, run `net_config_effective_tier()` and store *that* into `out->tier`, so a
  half-written config can never present itself as networked.
- `net_store_save()` writes every field then calls `nvs_commit()` **once**. A commit
  failure returns the error and leaves the previous config intact.
- Log at `ESP_LOGW` on any degradation, and never log a PSK or token value.

- [x] **Step 3: Initialise NVS at boot**

In `firmware/main/main.c`, before `board_display_init()`:

```c
    esp_err_t nvs_status = nvs_flash_init();
    if (nvs_status == ESP_ERR_NVS_NO_FREE_PAGES ||
        nvs_status == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        // A truncated or version-shifted namespace is recoverable by erasing
        // it: the device returns to factory-fresh local tier, which is the
        // failure direction the spec requires.
        ESP_ERROR_CHECK(nvs_flash_erase());
        nvs_status = nvs_flash_init();
    }
    ESP_ERROR_CHECK(nvs_status);
```

- [x] **Step 4: Handle the two new messages in `protocol_task.c`**

Add cases to the message dispatch:

- `PROTOCOL_TYPE_NETWORK_CONFIG`: run `net_config_validate()`. On
  `NET_CONFIG_ERR_INSECURE_URL` reply with `PROTOCOL_ERROR_INVALID_PAYLOAD` and do not
  save. Otherwise `net_store_save()` and reply with an ACK. **Do not apply the new tier
  live** — it takes effect on the next boot, which keeps the transition a single
  well-defined event rather than a hot swap.
- `PROTOCOL_TYPE_FACTORY_RESET`: `net_store_erase()`, then ACK.

- [x] **Step 5: Apply the tier gate to USB messages**

At the top of the dispatch, before acting on any message that arrived on the USB
transport:

```c
    if (!net_config_usb_message_allowed(net_store_current_tier(), message.type)) {
        transmit_error(context, request_id, PROTOCOL_ERROR_WRONG_TIER,
                       "device is owned over the network");
        return;
    }
```

- [x] **Step 6: Populate the additive status fields**

In the status builder (around `protocol_task.c:213`), set `status->tier` from
`net_store_current_tier()`. Leave `wifi_state` at `PROTOCOL_WIFI_DOWN`, `wifi_rssi` at 0,
`ip` empty and `ota_state` at `PROTOCOL_OTA_IDLE` — Tasks 6 and 11 fill those in.

- [x] **Step 7: Build the firmware**

```sh
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build
```

Expected: builds with no warnings. Remember to add `nvs_flash` to the component
requirements in `firmware/main/CMakeLists.txt` if the build cannot find it.

- [x] **Step 8: Add host-side provisioning to the device crate and CLI**

In `companion/crates/device/src/session.rs` add `provision(&mut self, config:
&NetworkConfig) -> Result<...>` and `factory_reset(&mut self) -> Result<...>`, both
following the existing request/ACK pattern used by `apply_config`.

In `companion/crates/deskmate-cli/src/main.rs` add two commands to the dispatch beside
`"status"`:

- `provision` with options `--ssid`, `--psk`, `--server-url`, `--device-id`, `--token`,
  `--offset-minutes`, `--tier` (`local` or `networked`).
- `factory-reset` with no options beyond `--port`.

Update the usage text so `unknown command` errors stay accurate.

- [x] **Step 9: Run the workspace gates**

From `companion/`:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: all pass.

- [x] **Step 10: Verify on the physical board**

Flash and exercise the round trip. This is the first hardware gate in V2 and it needs no
radio.

```sh
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware flash monitor
```

Then from `companion/`, with the monitor closed so the port is free:

```sh
cargo run -p deskmate-cli -- status --port <serial-port> --json
cargo run -p deskmate-cli -- provision --port <serial-port> \
    --ssid <ssid> --psk <psk> \
    --server-url wss://example.invalid/v1/device/link \
    --device-id dev-0001 --token placeholder --offset-minutes 240 --tier networked
```

Confirm all five: status reports `tier: local` before provisioning; the provision command
ACKs; **after a power cycle** status reports `tier: networked`; a `push-data` command now
fails with the wrong-tier error; and `factory-reset` followed by a power cycle returns
status to `tier: local`.

- [x] **Step 11: Record the observation**

Add a dated entry to `docs/hardware/board-notes.md` recording exactly what was observed,
including anything that did not pass. Do not write "verified" against anything not seen.

- [x] **Step 12: Commit**

```bash
git add firmware/main/link/net_store.c firmware/main/link/net_store.h \
        firmware/main/link/protocol_task.c firmware/main/main.c \
        firmware/main/CMakeLists.txt \
        companion/crates/device/src/session.rs \
        companion/crates/deskmate-cli/src/main.rs \
        docs/hardware/board-notes.md
git commit -m "feat: device persists network config and enforces the tier gate

The firmware's first use of NVS. Provisioning arrives over the cable, survives
a power cycle, and takes effect on the next boot rather than hot-swapping the
owner mid-session.

A networked device answers apply-config, push-data, time-sync, activate-screen
and trigger-interrupt on USB with a wrong-tier error: the cable is a
configurator, not an owner. Corrupt or version-shifted NVS erases back to
factory-fresh local tier, which is the required failure direction.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01SwAMBn6h8WMLU4ADiA8LwG"
```

---

### Task 4:4 Transport vtable

A pure refactor with no behaviour change, so that Task 8 can add a second transport
without touching the protocol task's logic. `protocol_task.c` calls `usb_link_*` at four
sites today: lines 184, 240, 624 and 729.

**Files:**
- Create: `firmware/main/link/link_transport.h`
- Modify: `firmware/main/link/usb_link.c`, `firmware/main/link/usb_link.h`
- Modify: `firmware/main/link/protocol_task.c`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `link_transport_t` and `usb_link_transport()`. Task 8 adds
  `net_link_transport()` with the identical shape.

- [x] **Step 1: Define the vtable**

Create `firmware/main/link/link_transport.h`:

```c
#pragma once

#include <stddef.h>
#include <stdint.h>

#include "esp_err.h"
#include "freertos/FreeRTOS.h"

/**
 * One protocol transport. The protocol task holds exactly one at a time,
 * selected by tier, and knows nothing about cables or sockets.
 *
 * `read` returns zero on timeout. `write_frame` receives one complete,
 * zero-delimited wire frame -- the COBS framing is identical on every
 * transport, so protocol_frame.c is unchanged.
 */
typedef struct {
    const char *name;
    size_t (*read)(uint8_t *out, size_t capacity, TickType_t timeout_ticks);
    esp_err_t (*write_frame)(const uint8_t *frame, size_t length,
                             TickType_t timeout_ticks);
    uint32_t (*dropped_bytes)(void);
    uint32_t link_timeout_ms;
} link_transport_t;
```

- [x] **Step 2: Expose the USB implementation**

Add to `firmware/main/link/usb_link.h`:

```c
#include "link/link_transport.h"

/** The USB transport. `link_timeout_ms` is PROTOCOL_LINK_TIMEOUT_MS (10000). */
const link_transport_t *usb_link_transport(void);
```

Implement it in `usb_link.c` as a `static const link_transport_t` whose members point at
the existing `usb_link_read`, `usb_link_write_frame` and `usb_link_rx_dropped_bytes`.

- [x] **Step 3: Route the protocol task through the vtable**

In `protocol_task.c`, hold `static const link_transport_t *s_transport;`, initialise it to
`usb_link_transport()`, and replace the four direct calls with
`s_transport->write_frame(...)`, `s_transport->dropped_bytes()` and
`s_transport->read(...)`.

- [x] **Step 4: Build and confirm nothing changed**

```sh
make -C firmware/host_tests clean test
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build
```

Expected: both pass. Host tests do not compile `protocol_task.c`, so they should be
untouched — run them to prove that rather than to discover it.

- [x] **Step 5: Verify behaviour is unchanged on the board**

Flash and run `cargo run -p deskmate-cli -- status --port <serial-port>` plus one
`push-data`. Expected: identical behaviour to Task 3. A refactor that changes observable
behaviour is a failed refactor — stop and diagnose rather than continuing.

- [x] **Step 6: Commit**

```bash
git add firmware/main/link/link_transport.h firmware/main/link/usb_link.c \
        firmware/main/link/usb_link.h firmware/main/link/protocol_task.c
git commit -m "refactor: protocol task talks to a transport vtable

No behaviour change. usb_link becomes one implementation of link_transport_t
so the WebSocket transport can be a sibling rather than a special case, and so
the per-transport link timeout has somewhere to live.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01SwAMBn6h8WMLU4ADiA8LwG"
```

---

### Task 5:5 Per-transport link timeout

`link_state.c:29` compares against the compile-time `PROTOCOL_LINK_TIMEOUT_MS`. Over the
internet, 10 s will flap and drop the device to the standalone clock on ordinary jitter.
USB must keep 10000 ms; the network transport uses 45000 ms.

**Files:**
- Modify: `firmware/main/core/link_state.h`, `firmware/main/core/link_state.c`
- Modify: `firmware/host_tests/test_link_state.c`
- Modify: `firmware/main/link/protocol_task.c`

**Interfaces:**
- Consumes: Task 4's `link_transport_t.link_timeout_ms`.
- Produces: `link_state_init_with_timeout(link_state_t *, uint32_t timeout_ms)`.
  `link_state_init()` keeps its existing signature and behaviour.

- [x] **Step 1: Write the failing test**

Add to `firmware/host_tests/test_link_state.c`:

```c
// The USB default must not move: ten M1 unplug/replug cycles were validated
// against it.
static void test_default_timeout_is_unchanged(void)
{
    link_state_t state;
    link_state_init(&state);
    link_state_note_valid_request(&state, 0U);
    assert(link_state_poll(&state, 9999U));
    assert(!link_state_poll(&state, 10000U));
}

static void test_network_timeout_is_longer(void)
{
    link_state_t state;
    link_state_init_with_timeout(&state, 45000U);
    link_state_note_valid_request(&state, 0U);
    // Well past the USB deadline, and still online.
    assert(link_state_poll(&state, 20000U));
    assert(link_state_poll(&state, 44999U));
    assert(!link_state_poll(&state, 45000U));
}
```

- [x] **Step 2: Run the test to verify it fails**

```sh
make -C firmware/host_tests clean test
```

Expected: compile failure — `link_state_init_with_timeout` is undefined.

- [x] **Step 3: Add the timeout to the state**

Add `uint32_t timeout_ms;` to `link_state_t` in `link_state.h` and declare:

```c
/**
 * Initialise with an explicit host-loss deadline. link_state_init() delegates
 * here with PROTOCOL_LINK_TIMEOUT_MS, so the USB path is bit-identical.
 */
void link_state_init_with_timeout(link_state_t *state, uint32_t timeout_ms);
```

In `link_state.c`, make `link_state_init()` call the new function with
`PROTOCOL_LINK_TIMEOUT_MS`, and change line 29's comparison to use `state->timeout_ms`.

- [x] **Step 4: Run the test to verify it passes**

```sh
make -C firmware/host_tests clean test
```

Expected: all pass, including the two new cases.

- [x] **Step 5: Initialise from the active transport**

In `protocol_task.c`, replace the `link_state_init(...)` call with
`link_state_init_with_timeout(&state, s_transport->link_timeout_ms)`.

- [x] **Step 6: Build and commit**

```sh
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build
```

```bash
git add firmware/main/core/link_state.h firmware/main/core/link_state.c \
        firmware/host_tests/test_link_state.c firmware/main/link/protocol_task.c
git commit -m "feat: host-loss deadline is per transport

USB keeps 10000 ms unchanged -- M1 validated ten unplug/replug cycles against
it -- while the network transport will use 45000 ms, because ordinary internet
jitter would otherwise drop the device to the standalone clock every few
minutes. link_state_init() delegates to the new function with the old constant,
so the USB path is bit-identical.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01SwAMBn6h8WMLU4ADiA8LwG"
```

---

### Task 6:6 WiFi station and SNTP

The radio comes up on both tiers. This task ends with a board-testable deliverable that
still involves no server: the device joins WiFi and the standalone clock shows correct
local time with no host attached at all.

**Files:**
- Create: `firmware/main/link/wifi_station.c`, `firmware/main/link/wifi_station.h`
- Modify: `firmware/main/main.c`, `firmware/main/link/protocol_task.c`
- Modify: `firmware/main/idf_component.yml` / `firmware/sdkconfig.defaults`

**Interfaces:**
- Consumes: Task 3's `net_store_load()`.
- Produces: `wifi_station_start()`, `wifi_station_state()`, `wifi_station_rssi()`,
  `wifi_station_ip()`, and `wifi_station_time_synced()`. Tasks 8 and 11 gate their work
  on `wifi_station_state() == PROTOCOL_WIFI_CONNECTED`.

- [x] **Step 1: Write the header**

Create `firmware/main/link/wifi_station.h`:

```c
#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "esp_err.h"

#include "core/protocol_message.h"

/**
 * Bring up the station using the persisted credentials and start SNTP.
 *
 * Returns ESP_OK when the attempt started, not when the join succeeded --
 * progress is observed through wifi_station_state(). With no stored SSID this
 * is a no-op returning ESP_OK: a device with no credentials is a working local
 * device, not an error.
 */
esp_err_t wifi_station_start(void);

protocol_wifi_state_t wifi_station_state(void);
int8_t wifi_station_rssi(void);

/** Dotted-quad IPv4, or an empty string when not connected. */
const char *wifi_station_ip(void);

/** True once SNTP has set the system clock at least once. */
bool wifi_station_time_synced(void);
```

- [x] **Step 2: Implement the station**

Create `firmware/main/link/wifi_station.c` using `esp_netif_init()`,
`esp_event_loop_create_default()`, `esp_netif_create_default_wifi_sta()`,
`esp_wifi_init()`, `esp_wifi_set_mode(WIFI_MODE_STA)`, `esp_wifi_set_config()` and
`esp_wifi_start()`. Requirements:

- Register handlers for `WIFI_EVENT_STA_DISCONNECTED` and `IP_EVENT_STA_GOT_IP`.
- Reconnect with the plan-wide backoff: **1 s initial, doubling, capped at 60 s, ±20%
  jitter.** Derive the jitter from `esp_random()`.
- Start SNTP with `esp_sntp_setservername(0, "pool.ntp.org")` once an IP arrives, and set
  `wifi_station_time_synced()` from the SNTP notification callback.
- After the first sync, apply the stored `utc_offset_minutes` so the standalone clock
  shows local time. Since `clock_source_now()` returns UTC, the offset must be applied
  where the existing host time-sync path applies it — follow `link_state_local_seconds()`
  rather than adding a second convention.

- [x] **Step 3: Start the station at boot**

In `firmware/main/main.c`, after NVS init and the display bring-up, call
`wifi_station_start()` and log the result. Do not block boot on it: the clock screen must
appear whether or not WiFi ever joins.

- [x] **Step 4: Report WiFi in status**

In `protocol_task.c`'s status builder, fill `wifi_state`, `wifi_rssi` and `ip` from the
three accessors. The SSID and PSK are never reported.

- [x] **Step 5: Build**

```sh
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build
```

Expected: builds clean. Note the image size from the build summary and record it — this
is the start of the re-baselining the spec §3.4 requires.

- [x] **Step 6: Verify on the physical board**

Flash, then provision credentials for a real network with `--tier local`:

```sh
cargo run -p deskmate-cli -- provision --port <serial-port> \
    --ssid <ssid> --psk <psk> --server-url wss://example.invalid/v1/device/link \
    --device-id dev-0001 --token placeholder --offset-minutes 240 --tier local
```

Power cycle, then confirm all four: `status --json` reports `wifi_state: connected` with
a plausible RSSI and IP; the panel's standalone clock shows the **correct local time**
with no host attached (unplug the cable and use the webcam harness per
`docs/hardware/webcam-harness.md`); the time survives a reboot with no host; and pulling
the network makes `wifi_state` go to `connecting` without rebooting the device.

- [x] **Step 7: Record the observation and commit**

Add the dated entry to `docs/hardware/board-notes.md`, including the new image size and
free-heap figure. Then:

```bash
git add firmware/main/link/wifi_station.c firmware/main/link/wifi_station.h \
        firmware/main/main.c firmware/main/link/protocol_task.c \
        firmware/main/idf_component.yml firmware/sdkconfig.defaults \
        docs/hardware/board-notes.md
git commit -m "feat: device joins WiFi and takes time from SNTP

Both tiers bring the radio up. The standalone clock now shows correct local
time with no host ever attached: SNTP supplies UTC and the provisioned
utc_offset_minutes supplies the zone, since SNTP carries no zone of its own.

Reconnect uses 1s doubling to a 60s cap with jitter, so a fleet does not
resynchronise onto the network the instant an outage ends. Boot never blocks on
the join -- the clock screen appears whether or not WiFi succeeds.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01SwAMBn6h8WMLU4ADiA8LwG"
```

---

### Task 7:7 Server skeleton — device link, auth, firmware endpoints

A new workspace crate. This task has no device involvement at all and is testable
entirely with `cargo test`: a WebSocket client with a good token gets a session, a bad
token is refused, and the firmware endpoints answer correctly.

**Files:**
- Create: `companion/crates/server/Cargo.toml`
- Create: `companion/crates/server/src/main.rs`, `src/lib.rs`, `src/auth.rs`,
  `src/device_link.rs`, `src/firmware.rs`, `src/registry.rs`
- Create: `companion/crates/server/tests/device_link.rs`
- Modify: `companion/Cargo.toml` (workspace members)

**Interfaces:**
- Consumes: `protocol` for the codec.
- Produces: `server::app(state: ServerState) -> axum::Router`,
  `server::registry::Registry` with `mint(&self) -> DeviceIdentity` and
  `authenticate(&self, token: &str) -> Option<DeviceId>`, and
  `server::ServerState`. Task 9 adds the ownership runtime behind this same router.

- [x] **Step 1: Create the crate and register it**

Add `"crates/server"` to `members` in `companion/Cargo.toml`. Create
`companion/crates/server/Cargo.toml` depending on `protocol` (path), `axum` with the `ws`
feature, `tokio` with `rt-multi-thread`, `macros` and `signal`, `serde` with `derive`,
`serde_json`, `thiserror`, and `tracing` + `tracing-subscriber`. Inherit
`edition`, `rust-version`, `license` and `[lints]` from the workspace as the other crates
do.

- [x] **Step 2: Write the failing test**

Create `companion/crates/server/tests/device_link.rs`:

```rust
//! The device-facing surface. These tests never touch a board: they exercise
//! the same three endpoints the firmware will call, over a loopback listener.

use server::{ServerState, app};

async fn spawn() -> (String, server::registry::DeviceIdentity) {
    let state = ServerState::in_memory();
    let identity = state.registry().mint();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    (format!("127.0.0.1:{}", address.port()), identity)
}

#[tokio::test]
async fn device_link_accepts_a_minted_token() {
    let (host, identity) = spawn().await;
    let request = http::Request::builder()
        .uri(format!("ws://{host}/v1/device/link"))
        .header("Authorization", format!("Bearer {}", identity.token))
        .header("Host", &host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tokio_tungstenite::tungstenite::handshake::client::generate_key())
        .body(())
        .unwrap();
    let (_stream, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.status(), 101);
}

#[tokio::test]
async fn device_link_refuses_an_unknown_token() {
    let (host, _identity) = spawn().await;
    let request = http::Request::builder()
        .uri(format!("ws://{host}/v1/device/link"))
        .header("Authorization", "Bearer not-a-real-token")
        .header("Host", &host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tokio_tungstenite::tungstenite::handshake::client::generate_key())
        .body(())
        .unwrap();
    assert!(tokio_tungstenite::connect_async(request).await.is_err());
}

#[tokio::test]
async fn device_link_refuses_a_missing_authorization_header() {
    let (host, _identity) = spawn().await;
    let request = http::Request::builder()
        .uri(format!("ws://{host}/v1/device/link"))
        .header("Host", &host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tokio_tungstenite::tungstenite::handshake::client::generate_key())
        .body(())
        .unwrap();
    assert!(tokio_tungstenite::connect_async(request).await.is_err());
}

#[tokio::test]
async fn firmware_check_reports_no_update_when_current() {
    let (host, identity) = spawn().await;
    let body = reqwest::Client::new()
        .get(format!("http://{host}/v1/device/firmware?current=1.0.0"))
        .bearer_auth(&identity.token)
        .send()
        .await
        .unwrap();
    assert_eq!(body.status(), 204);
}

#[tokio::test]
async fn firmware_check_refuses_an_unknown_token() {
    let (host, _identity) = spawn().await;
    let response = reqwest::Client::new()
        .get(format!("http://{host}/v1/device/firmware?current=1.0.0"))
        .bearer_auth("not-a-real-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
}
```

Add `tokio-tungstenite`, `reqwest` and `http` as `[dev-dependencies]`.

- [x] **Step 3: Run the test to verify it fails**

From `companion/`:

```sh
cargo test -p server
```

Expected: compile failure — the crate has no `app`, `ServerState` or `registry`.

- [x] **Step 4: Implement the registry**

`src/registry.rs`: a `Registry` holding `Mutex<HashMap<String, DeviceId>>` mapping token
to device. `mint()` generates a `device_id` (`dev-0001` style, monotonic) and a random
token of at least 32 bytes rendered as hex, inserts it, and returns
`DeviceIdentity { device_id, token }`. `authenticate(&self, token: &str)` returns the
device id. Compare tokens in constant time — a plain `==` on a secret invites timing
analysis, and the fix costs one dependency-free helper.

- [x] **Step 5: Implement auth extraction and the router**

`src/auth.rs`: parse `Authorization: Bearer <token>`, returning `401` on a missing,
malformed or unknown token. `src/lib.rs`: `ServerState` holding the registry and, later, the runtime, with
`ServerState::in_memory()` for tests, `state.registry()` and `state.admin_token()`
accessors (Task 9's tests call all three by these names); `app(state)` returning a `Router` with `/v1/device/link` (WebSocket
upgrade), `/v1/device/firmware` and `/v1/firmware/{version}.bin`.

`src/device_link.rs`: on upgrade, log the device id and hold the socket open, echoing
nothing yet. Task 9 replaces the body of this handler with the ownership runtime.

`src/firmware.rs`: `/v1/device/firmware?current=<version>` returns `204 No Content` when
the requested version matches the newest available, otherwise `200` with
`{"version": ..., "url": ...}`. Serve images from a configured directory.

- [x] **Step 6: Run the tests to verify they pass**

```sh
cargo test -p server
```

Expected: all five pass.

- [x] **Step 7: Add the binary entry point**

`src/main.rs`: read a bind address, a firmware directory and an admin token from
environment variables, build `ServerState`, and serve with graceful shutdown on
`SIGINT`/`SIGTERM`. Print the bind address at startup.

- [x] **Step 8: Write the deployment unit and its runbook**

The spec's §5.4 commits to "a single binary plus `cloudflared`, run under systemd or
launchd". Create `companion/crates/server/deploy/` holding a `launchd` plist and a
`systemd` unit that set the bind address, firmware directory and admin token from an
environment file, plus a `README.md` giving the exact `cloudflared tunnel` invocation and
where the admin token is expected to live. A server that only ever runs from a developer
shell is not deployed, and the runbook is what makes the tunnel hostname reproducible
across restarts.

- [x] **Step 9: Run the workspace gates and commit**

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

```bash
git add companion/Cargo.toml companion/Cargo.lock companion/crates/server
git commit -m "feat: single-tenant server with device link and firmware endpoints

Three device-facing endpoints, which are the whole contract the firmware sees:
a WebSocket link, a firmware check, and image download. Bearer tokens are
minted per device and compared in constant time.

The link handler holds the socket and does nothing yet -- ownership arrives in
a later task. No accounts, no storage, no multi-tenancy: those are V3, behind
this same contract.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01SwAMBn6h8WMLU4ADiA8LwG"
```

---

### Task 8:8 WebSocket transport on the device

The device dials out and the existing frames flow over the socket. Deliverable: a
networked device answers `status` from the server rather than from the cable.

**Files:**
- Create: `firmware/main/link/net_link.c`, `firmware/main/link/net_link.h`
- Modify: `firmware/main/link/protocol_task.c`
- Modify: `firmware/main/idf_component.yml`, `firmware/sdkconfig.defaults`
- Modify: `companion/crates/server/src/device_link.rs`

**Interfaces:**
- Consumes: Task 4's `link_transport_t`, Task 3's `net_store_load()`, Task 6's
  `wifi_station_state()`.
- Produces: `net_link_start()` and `net_link_transport()`, matching
  `usb_link_transport()`'s shape with `link_timeout_ms = 45000`.

- [x] **Step 1: Add the dependencies**

Add `espressif/esp_websocket_client` to `firmware/main/idf_component.yml`. In
`firmware/sdkconfig.defaults` enable `CONFIG_ESP_TLS_USING_MBEDTLS=y` and
`CONFIG_MBEDTLS_CERTIFICATE_BUNDLE=y` so `esp_crt_bundle_attach` is available.

- [x] **Step 2: Write the header**

Create `firmware/main/link/net_link.h`:

```c
#pragma once

#include "esp_err.h"

#include "link/link_transport.h"

/**
 * Connect to the provisioned server URL, presenting the stored bearer token in
 * the WebSocket handshake.
 *
 * Returns ESP_ERR_INVALID_STATE when the device is not in networked tier or
 * has no usable config -- both are ordinary conditions, not faults.
 */
esp_err_t net_link_start(void);

/** The network transport. `link_timeout_ms` is 45000. */
const link_transport_t *net_link_transport(void);
```

- [x] **Step 3: Implement the client**

Create `firmware/main/link/net_link.c`:

- Build an `esp_websocket_client_config_t` with `uri` from the stored `server_url`,
  `crt_bundle_attach = esp_crt_bundle_attach`, and a `headers` string containing
  `"Authorization: Bearer <token>\r\n"`. Never log that header.
- Register an event handler. On `WEBSOCKET_EVENT_DATA` with `op_code` 2 (binary), append
  the payload to a bounded RX ring sized `USB_LINK_MAX_WIRE_FRAME_SIZE * 4`; drop and
  count overflow rather than growing.
- `read()` drains that ring; `write_frame()` calls
  `esp_websocket_client_send_bin()` with the complete zero-delimited frame — **COBS
  framing is retained inside the WebSocket message**, so `protocol_frame.c` is unchanged.
- Reconnect with the same backoff as Task 6: 1 s doubling to a 60 s cap, ±20% jitter.
  `esp_websocket_client` has its own reconnect; configure
  `reconnect_timeout_ms` and `network_timeout_ms` rather than adding a second loop.
- Treat every received byte as untrusted: the frame decoder already bounds lengths, and
  the ring must never be indexed past its capacity.

- [x] **Step 4: Select the transport by tier**

In `protocol_task.c`, choose the transport once at task start:

```c
    if (net_store_current_tier() == PROTOCOL_TIER_NETWORKED &&
        net_link_start() == ESP_OK) {
        s_transport = net_link_transport();
    } else {
        s_transport = usb_link_transport();
    }
```

The USB driver still initialises in both tiers so the cable can carry provisioning,
status and factory reset. In networked tier those arrive through a **second, restricted
reader** that runs `net_config_usb_message_allowed()` and never becomes the owner
transport. Keep that reader small: it handles status, network-config, factory-reset and
heartbeat only.

- [x] **Step 5: Echo frames on the server so the link is observable**

In `companion/crates/server/src/device_link.rs`, decode each binary message with
`protocol::decode_wire_frame` + `decode_message`, log the message type at `info`, and
answer `StatusRequest` with a `StatusResponse` so the round trip is provable before the
runtime exists. Reject a message that fails to decode by closing the socket — malformed
input from a device is as suspect as malformed input from a host.

- [x] **Step 6: Re-run malformed-input coverage over the WSS path**

The spec's §7.1 requires this explicitly, and it is the step most easily skipped on the
grounds that "it is the same decoder". It is the same decoder reached through a different
buffer, and the buffer is the new code.

Add `companion/crates/server/tests/hostile_device.rs`, which connects as a device and
sends, in turn: a frame with a corrupt COBS delimiter; a frame whose declared payload
length exceeds `MAX_PAYLOAD_SIZE`; a valid frame carrying an unknown message type; a
truncated frame; and two valid frames concatenated into one WebSocket message. Assert the
server rejects or ignores each without panicking and — for the concatenated case — that
it still decodes both.

Then add the mirror-image case on the device: extend
`companion/crates/device/examples/` with a `wss_malformed_check.rs` that sends the same
five shapes *to* the board over the network transport and asserts, via a following
`status` request, that the device is still answering and has **not rebooted** — compare
`uptime_ms` before and after. Framing recovery without a reboot is the rule from
`CLAUDE.md`, and the network path has never been tested against it.

- [x] **Step 7: Build and run the server locally**

```sh
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build
```

From `companion/`, in a second terminal:

```sh
cargo run -p server
```

Expose it with `cloudflared tunnel --url http://localhost:<port>` and note the public
`https://` hostname it prints. The device's `server_url` is that hostname with the
`wss://` scheme and the `/v1/device/link` path.


> **Delivered differently, and permanently.** No `cloudflared --url` quick tunnel was
> used. The repository owner elected to give the server a real home on the existing
> homelab instead: it is built for linux/x86_64 in a throwaway `rust:1.97-bookworm`
> container over an rsync'd copy of `companion/` (no Rust toolchain on the VM), installed
> to `/usr/local/bin/deskmate-server` on docker-vm, and run under this plan's own systemd
> unit with `/etc/deskmate/server.env`. Public path: Cloudflare HTTPS -> `cloudflared`
> (orion) -> Caddy (docker-vm:80) -> `192.168.8.20:8443`. The device URL is
> **`wss://deskmate.rodi.one/v1/device/link`**. Proven end to end from the Mac on
> 2026-08-18: `GET /` returns 404 from axum's own router, and `/v1/device/link` and
> `/v1/device/firmware` both return 401. Caddy carries no edge auth by design -- the
> device and the companion each send their own bearer in `Authorization`, which
> `basic_auth` would consume.

- [ ] **Step 8: Verify on the physical board**

> **DEFERRED to Task 9's board session -- this is a plan defect, not a choice.** Step 8
> requires provisioning with "a token minted by the server", but `Registry::mint()` has no
> HTTP route until **Task 9 Step 5** creates `POST /v1/devices`, and the registry is
> in-memory with no persistence. There is currently no way to obtain a device token at
> all, so this step is not executable as written at Task 8. Writing a throwaway bootstrap
> route or seed variable would mean unreviewed code whose only purpose is to be deleted
> one task later. Task 9 needs the same board and the same server, so Task 8's four
> observations (accepted connection; periodic status round-trips; panel stays on the
> standalone clock; killing the server yields visibly widening reconnect gaps, not a
> reboot or a spin) are carried into that session as separately-recorded items.
> Task 8 is **software complete / hardware open**, the same split Task 3 used.


Provision with the tunnel hostname and a token minted by the server, then power cycle:

```sh
cargo run -p deskmate-cli -- provision --port <serial-port> \
    --ssid <ssid> --psk <psk> \
    --server-url wss://<tunnel-host>/v1/device/link \
    --device-id <minted-id> --token <minted-token> \
    --offset-minutes 240 --tier networked
```

Confirm all four: the server logs an accepted connection; it periodically requests status
and logs the device's status response; the device's panel stays on the standalone clock
(no config has been sent yet, which is correct); and killing the server leaves the device
retrying with visibly widening gaps rather than rebooting or spinning.

- [ ] **Step 9: Record and commit**

Add the dated board-notes entry, then:

```bash
git add firmware/main/link/net_link.c firmware/main/link/net_link.h \
        firmware/main/link/protocol_task.c firmware/main/idf_component.yml \
        firmware/sdkconfig.defaults companion/crates/server/src/device_link.rs \
        companion/crates/server/tests/hostile_device.rs \
        companion/crates/device/examples/wss_malformed_check.rs \
        docs/hardware/board-notes.md
git commit -m "feat: device carries the protocol over WebSocket to the server

The same COBS/CBOR frames, unchanged, inside WSS binary messages. COBS is
redundant there -- WebSocket already delimits -- and kept anyway so
protocol_frame.c and frame.rs stay literally identical across both transports.

The bearer token rides in the handshake header, so authentication is a property
of the transport and the wire stays protocol v1. Trust comes from the bundled
Mozilla roots, so no certificate is pinned and rotation is never coupled to a
firmware update.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01SwAMBn6h8WMLU4ADiA8LwG"
```

---

### Task 9:9 Server owns the device

The milestone's centre. `app-core`'s `RuntimeDevice` trait is the seam: the server
implements it over the WebSocket and calls the same `RuntimeHandle` the Mac app uses, so
ownership has one implementation rather than two that must agree.

**Files:**
- Create: `companion/crates/server/src/runtime_device.rs`, `src/admin.rs`, `src/store.rs`
- Modify: `companion/crates/server/src/lib.rs`, `src/device_link.rs`
- Create: `companion/crates/server/tests/ownership.rs`

**Interfaces:**
- Consumes: Task 7's `ServerState` and `Registry`; `app_core::{RuntimeHandle,
  RuntimeDevice, DeviceConnection, RuntimeOptions, AppConfig, ConfigStore}`.
- Produces: `WebSocketRuntimeDevice` implementing `RuntimeDevice`, and the admin routes
  `POST /v1/devices`, `PUT /v1/devices/{id}/config`, `GET /v1/devices/{id}`. Task 10's
  Mac UI calls exactly these.

- [x] **Step 1: Write the failing test**

Create `companion/crates/server/tests/ownership.rs`:

```rust
//! Ownership, exercised with a fake device socket rather than a board.

use futures_util::{SinkExt, StreamExt};
use protocol::{Message, decode_message, decode_wire_frame};
use server::{ServerState, app};
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// Boots a server on a loopback port and mints one device identity.
/// Mirrors `tests/device_link.rs`'s helper; kept separate so the two files
/// can diverge without one silently changing the other's fixture.
async fn spawn() -> (String, server::registry::DeviceIdentity, String) {
    let state = ServerState::in_memory();
    let identity = state.registry().mint();
    let admin_token = state.admin_token().to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    (format!("127.0.0.1:{}", address.port()), identity, admin_token)
}

async fn connect_device(
    host: &str,
    token: &str,
) -> Result<
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    tokio_tungstenite::tungstenite::Error,
> {
    let request = http::Request::builder()
        .uri(format!("ws://{host}/v1/device/link"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Host", host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .unwrap();
    tokio_tungstenite::connect_async(request).await.map(|(s, _)| s)
}

/// Reads frames until one decodes to the variant the predicate accepts, or the
/// deadline expires. The runtime sends time-sync and status traffic of its own,
/// so a test that asserted on "the next frame" would be flaky by construction.
async fn await_message(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    matches: impl Fn(&Message) -> bool,
) -> Message {
    let deadline = tokio::time::Duration::from_secs(5);
    tokio::time::timeout(deadline, async {
        while let Some(Ok(WsMessage::Binary(bytes))) = socket.next().await {
            let frame = decode_wire_frame(&bytes).expect("device received a malformed frame");
            let message = decode_message(&frame).expect("device received an undecodable message");
            if matches(&message) {
                return message;
            }
        }
        panic!("socket closed before the expected message arrived");
    })
    .await
    .expect("timed out waiting for the expected message")
}

#[tokio::test]
async fn config_written_by_admin_reaches_the_device() {
    let (host, identity, admin_token) = spawn().await;
    let mut socket = connect_device(&host, &identity.token).await.expect("connect");

    let config = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/one-clock-card.json"
    ))
    .expect("fixture");

    let response = reqwest::Client::new()
        .put(format!("http://{host}/v1/devices/{}/config", identity.device_id))
        .bearer_auth(&admin_token)
        .header("Content-Type", "application/json")
        .body(config)
        .send()
        .await
        .expect("admin request");
    assert_eq!(response.status(), 200);

    let message = await_message(&mut socket, |m| matches!(m, Message::ApplyConfig(_))).await;
    let Message::ApplyConfig(applied) = message else {
        unreachable!("await_message filtered on this variant");
    };
    assert_eq!(applied.widgets.len(), 1);
    assert_eq!(applied.widgets[0].widget_id, "clock-1");
}

#[tokio::test]
async fn a_second_socket_for_the_same_device_is_refused() {
    // The single-owner invariant at the server's own boundary. A second link
    // must be refused rather than silently taking over -- a takeover is how
    // two owners appear without anyone deciding to allow them.
    let (host, identity, _admin_token) = spawn().await;
    let _first = connect_device(&host, &identity.token).await.expect("first connects");
    assert!(
        connect_device(&host, &identity.token).await.is_err(),
        "a second link for a device that already has one was accepted"
    );
}

#[tokio::test]
async fn admin_routes_refuse_a_bad_admin_token() {
    let (host, identity, _admin_token) = spawn().await;
    let client = reqwest::Client::new();

    let write = client
        .put(format!("http://{host}/v1/devices/{}/config", identity.device_id))
        .bearer_auth("not-the-admin-token")
        .header("Content-Type", "application/json")
        .body("{}")
        .send()
        .await
        .expect("request");
    assert_eq!(write.status(), 401);

    let read = client
        .get(format!("http://{host}/v1/devices/{}", identity.device_id))
        .bearer_auth("not-the-admin-token")
        .send()
        .await
        .expect("request");
    assert_eq!(read.status(), 401);

    // A device's own token must not open the admin surface either.
    let crossover = client
        .get(format!("http://{host}/v1/devices/{}", identity.device_id))
        .bearer_auth(&identity.token)
        .send()
        .await
        .expect("request");
    assert_eq!(crossover.status(), 401);
}
```

Create `companion/crates/server/tests/fixtures/one-clock-card.json` holding a minimal
valid schema-v4 config with a single clock card whose id is `clock-1` in the active
playlist. Add `futures-util` to `[dev-dependencies]`.

- [x] **Step 2: Run the tests to verify they fail**

```sh
cargo test -p server ownership
```

Expected: compile failure — the admin routes do not exist.

- [x] **Step 3: Implement `WebSocketRuntimeDevice`**

`src/runtime_device.rs` implements every `RuntimeDevice` method by encoding the matching
`protocol::Message` and awaiting its ACK over the socket:

```rust
impl RuntimeDevice for WebSocketRuntimeDevice {
    fn connect(&mut self) -> Result<DeviceConnection, DeviceError>;
    fn status(&mut self) -> Result<StatusResponse, DeviceError>;
    fn time_sync(&mut self, sync: TimeSync) -> Result<(), DeviceError>;
    fn apply_layout(
        &mut self,
        rotation: u16,
        widgets: Vec<WidgetConfig>,
        screens: Vec<ScreenConfig>,
    ) -> Result<(), DeviceError>;
    fn push_fields(&mut self, widget_id: String, fields: Vec<Field>) -> Result<(), DeviceError>;
    fn activate_screen(&mut self, screen_id: String) -> Result<(), DeviceError>;
    fn trigger_interrupt(&mut self, interrupt: TriggerInterrupt) -> Result<(), DeviceError>;
    fn try_recv_event(&mut self) -> Option<ReceivedEvent>;
    fn diagnostics(&self) -> SessionDiagnostics;
}
```

The trait is synchronous and `Send + 'static`, so bridge to the async socket with a
channel pair rather than blocking on a runtime inside these methods.

- [x] **Step 4: Start the runtime when a device connects**

In `src/device_link.rs`, on an authenticated upgrade: refuse if that device already has a
live link, then construct `WebSocketRuntimeDevice`, call `RuntimeHandle::start(...)` with
the stored config, and hold both until the socket closes. On close, shut the runtime down
so a reconnect starts cleanly.

- [x] **Step 5: Implement config storage and the admin API**

`src/store.rs`: load and save a schema-v4 `AppConfig` as JSON on disk using
`app_core::ConfigStore`, so the server and the Mac app validate configuration
identically. `src/admin.rs`: `POST /v1/devices` mints an identity;
`PUT /v1/devices/{id}/config` validates and stores a config then applies it through the
live `RuntimeHandle`; `GET /v1/devices/{id}` returns connection state, last-seen time and
the current `AppSnapshot` summary. All three require the admin token.

- [x] **Step 6: Enable providers server-side**

Construct `RuntimeOptions` with `SystemProviderRefresher` and
`SystemCalendarRefresher`, so weather, ICS calendar, RSS and JSON feeds are fetched by
the server. This is what makes the device work with the Mac off.

- [x] **Step 7: Run the tests to verify they pass**

```sh
cargo test -p server
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: all pass.

- [ ] **Step 8: The headline demo, on the physical board**

> **DEFERRED to the morning board session**, together with Task 8's four carried
> observations. Everything this step needs on the software side is in place and the server
> is deployed at `wss://deskmate.rodi.one/v1/device/link`; what is missing is the board and
> a human to watch the panel, tap a card, quit the Mac app and pull the cable. The five
> claims to confirm are unchanged: the panel leaves the standalone clock and renders the
> configured cards; the weather card shows real fetched data; quitting the Mac app changes
> nothing; unplugging USB changes nothing; and tapping a pomodoro card starts it, with the
> observed latency recorded.


With the server running behind the tunnel and the device provisioned as networked from
Task 8:

```sh
curl -X PUT https://<tunnel-host>/v1/devices/<id>/config \
     -H "Authorization: Bearer <admin-token>" \
     -H "Content-Type: application/json" \
     --data @<a schema-v4 config with a clock card and a weather card>
```

Confirm all five: the panel leaves the standalone clock and renders the configured cards;
the weather card shows real fetched data; **quitting the Mac app changes nothing**;
unplugging USB entirely changes nothing; and tapping a pomodoro card starts it, proving
events travel up and state comes back down over the network.

- [ ] **Step 9: Record and commit**

Record the observation in `docs/hardware/board-notes.md` — this is V2's central claim, so
record exactly what was seen, including latency on the pomodoro tap.

```bash
git add companion/crates/server docs/hardware/board-notes.md
git commit -m "feat: server owns the device over the network

app-core's RuntimeDevice trait is the seam: the server implements it over the
WebSocket and drives the same RuntimeHandle the Mac app drives, so ownership
has one implementation rather than two that must be kept in agreement.
Providers run server-side, which is what makes the device work with the Mac
quit.

The single-owner invariant is enforced at the server boundary too: a second
socket for a device that already has a live link is refused rather than
silently taking over.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01SwAMBn6h8WMLU4ADiA8LwG"
```

---

### Task 9b:9b Device identities survive a restart (amendment)

**Added 2026-08-19 by explicit direction of the repository owner**, after Task 9's review
classified in-memory identities as a limitation whose documentation did not exist. This
was not in the approved plan; it is recorded here rather than left to diverge.

The problem is operational, not theoretical. `deploy/deskmate-server.service` sets
`Restart=on-failure` with `RestartSec=2`, so an unattended crash silently deprovisions
every device. The board then presents a token the server has never heard of, 401s
forever, and can only be recovered with a USB cable — which is precisely the failure mode
V2 exists to remove.

**The owner's decision, and the one refinement made to it.** The owner accepted storing
device credentials on disk, reasoning that a desk accessory sits next to the computer it
serves. Note the file does not live next to the board: it lives on the shared homelab VM
that also fronts unrelated services through the same tunnel. That does not change the
decision, but it makes the cheaper form of it clearly correct — **store a SHA-256 of each
token, never the token**. `POST /v1/devices` is the only code path that ever needs the
plaintext, and it has it in hand at mint time; `Registry::authenticate` only ever needs to
*verify*. A copy of the state file therefore grants nothing. Because the token is 32 bytes
of CSPRNG output rather than a human-chosen secret, a plain digest is sufficient and a
password KDF would buy nothing.

**Files:**
- Modify: `companion/crates/server/src/registry.rs`, `src/lib.rs`, `src/main.rs`
- Modify: `companion/crates/server/deploy/README.md`, `deploy/deskmate-server.env.example`
- Modify: `companion/crates/server/tests/ownership.rs`

**Interfaces:**
- Consumes: Task 7's `Registry`/`DeviceIdentity` and Task 9's `ConfigStore`-backed state
  directory.
- Produces: a `Registry` that loads at boot and persists on mint, holding digests only.

- [x] **Step 1: Write the failing tests**

Cover, at minimum: a minted identity authenticates after the registry is dropped and
reloaded from the same path; the persisted bytes contain the digest and **not** the token
(assert the literal token string is absent from the file); a corrupt or truncated store
degrades to an empty registry rather than panicking or refusing to start; and `mint`
remains monotonic across a reload rather than reissuing an existing `dev-NNNN` id.

- [x] **Step 2: Persist digests, not tokens**

`authenticate` hashes the presented token and compares digests in constant time, keeping
the existing whole-table scan on a miss so a rejection's cost still cannot vary with how
many leading bytes matched. The store is a file under the same state directory
`DESKMATE_CONFIG_DIR` already lives in, created `0600`, written by the same
write-temp-then-rename discipline `app_core::ConfigStore` uses so a crash mid-write cannot
leave a half-file.

- [x] **Step 3: Fail toward a working device, never a locked-out one**

An unreadable store must not prevent the server starting; a device that cannot be
authenticated must log enough to distinguish "unknown token" from "store failed to load",
without printing the token or any prefix of it.

- [x] **Step 4: Update the runbook and the env example**

Replace the limitation text Task 9's fix round added with the resulting behaviour, and say
where the file lives, what it contains, and that losing it means re-minting rather than
recovering.

- [x] **Step 5: Run the gates and commit**

---

### Task 10a:10a The app-core boundary Task 10 needs (amendment)

**Added 2026-08-19.** Task 10's implementer stopped and reported BLOCKED rather than
writing code, and it was right to. Task 10 assumes the Tauri shell can call
`device::Session::provision` and `factory_reset` and can see the device's tier. Neither
reaches the app: the Mac talks to the board through `app_core::RuntimeHandle`, which owns
the serial port exclusively and exposes no provisioning or factory-reset command, and
`SerialRuntimeDevice` keeps its `ConnectedSession` private.

**Opening a second `device::Session` from a Tauri command is the wrong answer** even
though it would compile and pass tests. It would compete with the live app-core session
for the same port, make provisioning timing-dependent, and reintroduce exactly the
multi-process ownership hole M2 suffered and M3 was built to close. The implementer
declined to do it; this task exists so nobody does it later under time pressure.

Three gaps, all in `companion/crates/app-core/`:

1. `DeviceSnapshot` carries none of V2's additive status fields — no `tier`, `wifi_state`,
   `wifi_rssi`, `ip`, `last_network_error` or `ota_state`. `runtime::update_device_status`
   receives them from the wire and drops them on the floor. `DeviceCapability` also stops
   at `FirmwareUpdate`, so `CAPABILITY_NETWORKING` reads as unknown.
2. There is no provisioning or factory-reset path through the single owner.
3. There is nowhere to keep the server base URL, device id, and the write-only admin and
   device tokens. **They must not go into `AppConfig`** — the plan's Global Constraints
   freeze the config schema at v4 with no migration, and `ConfigStore` is strict, so an
   added field is a contract break, not a convenience.

**Files:**
- Modify: `companion/crates/app-core/src/runtime.rs`, `src/state.rs`
- Create: a small separate settings store in `companion/crates/app-core/` for the network
  credentials, alongside `ConfigStore` rather than inside it

**Interfaces:**
- Consumes: Task 3's `provision`/`factory_reset` on `device::Session`.
- Produces: the projected status fields, two runtime commands, and the credential store
  that Task 10's UI and typed IPC consume.

- [ ] **Step 1: Write the failing tests**

Cover: every additive V2 status field surviving the wire-to-snapshot projection;
`CAPABILITY_NETWORKING` decoding rather than reading as unknown; a provisioning command
executing on the **existing** session with no second port open; and the credential store
round-tripping without touching `AppConfig`.

- [ ] **Step 2: Project the V2 status fields into `DeviceSnapshot`**

Additive only. This is the third time in this project that firmware has filled a status
field nothing consumes — it cost a wasted board session at Task 3 and was caught again at
Task 6. Project every field, not only the ones Task 10 happens to render.

- [ ] **Step 3: Add provisioning and factory-reset runtime commands**

They execute on the existing `SerialRuntimeDevice`/`ConnectedSession`. No second session,
ever. A provisioning attempt while the runtime is disconnected must return a typed error,
not open a port of its own.

- [ ] **Step 4: Add the credential store**

Server base URL, device id, admin token, device token. Tokens are write-only: accepted,
never returned to a caller, never present in any snapshot or event. `AppConfig` is
untouched and `docs/config/v4.md` stays the frozen contract.

- [ ] **Step 5: Run the gates and commit**

---

### Task 10:10 Companion app — provisioning and tier UI

The Mac app becomes the setup surface. In local tier it writes settings to the device as
it does today; in networked tier it writes them to the server and shows that it is doing
so, because a UI that silently changes destination is a UI that will be mistrusted.

**Files:**
- Create: `companion/apps/deskmate/src/components/NetworkPanel.tsx`
- Modify: `companion/apps/deskmate/src-tauri/src/lib.rs` (typed IPC commands)
- Modify: `companion/apps/deskmate/src/state/useAppState.ts`
- Modify: `companion/apps/deskmate/tests/components.test.tsx`

**Interfaces:**
- Consumes: Task 3's `provision`/`factory_reset` on `device::Session`; Task 9's admin
  routes.
- Produces: nothing later tasks depend on.

- [ ] **Step 1: Write the failing test**

Add to `companion/apps/deskmate/tests/components.test.tsx`, reusing the existing render
helpers:

```tsx
  test("network panel shows the device as locally owned before provisioning", () => {
    const html = renderNetworkPanel({ tier: "local", wifiState: "down", ip: "" });
    expect(html).toContain("Owned by this Mac");
    expect(html).not.toContain("Owned by the server");
  });

  test("network panel says where settings are written once networked", () => {
    // The destination changing invisibly is the failure mode worth pinning:
    // the same edit goes to a different place depending on tier.
    const html = renderNetworkPanel({
      tier: "networked",
      wifiState: "connected",
      ip: "192.168.1.42",
    });
    expect(html).toContain("Owned by the server");
    expect(html).toContain("192.168.1.42");
    expect(html).toContain("Settings are saved to the server");
  });

  test("network panel never renders a stored secret", () => {
    const html = renderNetworkPanel({
      tier: "networked",
      wifiState: "connected",
      ip: "192.168.1.42",
      ssid: "home-network",
    });
    expect(html).toContain("home-network");
    expect(html).not.toContain("psk");
    expect(html).not.toContain("token");
  });
```

- [ ] **Step 2: Run the test to verify it fails**

From `companion/apps/deskmate/`:

```sh
bun test tests/components.test.tsx
```

Expected: FAIL — `renderNetworkPanel` does not exist.

- [ ] **Step 3: Add the typed IPC commands**

In `src-tauri/src/lib.rs` add commands mirroring the existing typed-IPC pattern:
`provision_device(config)`, `factory_reset_device()`, and `set_server_endpoint(url,
admin_token)`. Each returns the same typed error shape the existing commands use, so the
UI's error handling does not fork.

- [ ] **Step 4: Build the panel**

Create `NetworkPanel.tsx` showing: current tier, WiFi state, RSSI and IP; a form for
SSID, passphrase and server URL; buttons to pair (write `tier: networked`), unpair
(write `tier: local`) and factory-reset; and a clear line stating where settings are
saved. The passphrase field is write-only — the device never returns it, so the form must
not pretend to show a stored value.

Follow the accessibility conventions established by the M3 settings work: labelled
controls, keyboard reachable, and status changes announced.

- [ ] **Step 5: Route settings writes by tier**

In `useAppState.ts`, when the device reports `networked`, send configuration edits to the
server's `PUT /v1/devices/{id}/config` instead of down the serial link. A failed write
must surface as a typed error and must **not** be described as "your last working
settings" — that mislabeling defect was fixed in the playlists work and must not
reappear.

- [ ] **Step 6: Run the app gates**

From `companion/apps/deskmate/`:

```sh
bun test
bunx tsc --noEmit
bunx biome lint .
bunx biome format .
```

Use `biome lint` and `biome format`, not `biome check` — `check` additionally runs
assists including `organizeImports`, which this project has never enforced and which
reports pre-existing failures on an untouched tree.

- [ ] **Step 7: Commit**

```bash
git add companion/apps/deskmate
git commit -m "feat: companion app provisions the device and shows who owns it

The Mac is the setup surface for both tiers: it writes WiFi credentials, server
URL and token over the cable, and pairs or unpairs by writing the tier.

Once networked, settings edits go to the server rather than down the serial
link, and the panel says so. A destination that changes invisibly is the
failure mode worth guarding against. The passphrase field is write-only,
because the device never returns it.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01SwAMBn6h8WMLU4ADiA8LwG"
```

---

### Task 11:11 Firmware update and rollback

The last feature task. V1 already poured the foundation: `ota_0`/`ota_1` at 4 MB each,
`otadata`, and an OTA-aware bootloader.

**Files:**
- Create: `firmware/main/link/ota.c`, `firmware/main/link/ota.h`
- Create: `firmware/main/ui/ota_screen.c`, `firmware/main/ui/ota_screen.h`
- Modify: `firmware/main/main.c`, `firmware/main/link/protocol_task.c`
- Modify: `firmware/main/core/protocol_message.h` (capability bit 6 switches on)
- Modify: `firmware/sdkconfig.defaults`

**Interfaces:**
- Consumes: Task 6's `wifi_station_state()`, Task 3's `net_store_load()`, Task 7's
  firmware endpoints.
- Produces: `ota_mark_running_image_valid()`, `ota_check_now()`, `ota_state()`.

- [ ] **Step 1: Enable rollback**

In `firmware/sdkconfig.defaults`:

```
CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE=y
```

- [ ] **Step 2: Write the header**

Create `firmware/main/link/ota.h`:

```c
#pragma once

#include <stdbool.h>

#include "esp_err.h"

#include "core/protocol_message.h"

/**
 * Confirm the running image so the bootloader stops holding a rollback.
 *
 * The self-check is deliberately local-only: display up, LVGL running,
 * protocol task alive, NVS readable. Requiring "reached the server" would roll
 * back a perfectly good image across the whole fleet during any outage --
 * network reachability is a runtime condition, never a validity condition.
 */
esp_err_t ota_mark_running_image_valid(void);

/**
 * Ask the server whether a newer image exists and install it if so.
 *
 * Returns ESP_ERR_INVALID_STATE while an interrupt is live or a pomodoro is
 * running: an update that eats a focus session to install itself is a defect
 * regardless of correctness.
 */
esp_err_t ota_check_now(void);

protocol_ota_state_t ota_state(void);
```

- [ ] **Step 3: Implement the update path**

Create `firmware/main/link/ota.c`:

- `ota_check_now()` GETs `<server>/v1/device/firmware?current=<version>` with the bearer
  token. `204` means nothing to do. `200` yields a version and URL.
- Download with `esp_https_ota()` using `crt_bundle_attach = esp_crt_bundle_attach`,
  driving it through `esp_https_ota_perform()` so progress can be reported from
  `esp_https_ota_get_image_len_read()`.
- Refuse to start when `interrupt_state` reports an active interrupt or a running
  pomodoro; retry at the next scheduled check.
- On success call `esp_ota_set_boot_partition()` and restart. On any failure, abort
  cleanly, set state to `PROTOCOL_OTA_FAILED`, and leave the running image untouched.
- Schedule: at boot and every 24 h thereafter with jitter, in **both** tiers.

- [ ] **Step 4: Implement the validity gate**

`ota_mark_running_image_valid()` checks `esp_ota_get_state_partition()` for
`ESP_OTA_IMG_PENDING_VERIFY`; if so, verify the four local conditions and call
`esp_ota_mark_app_valid_cancel_rollback()`. Call it from `main.c` **after** the display,
LVGL, NVS and protocol task are all up — not before, or the check proves nothing.

- [ ] **Step 5: Build the progress takeover**

Create `firmware/main/ui/ota_screen.c` showing "Updating" with a percentage, following
the existing template style helpers. Guard every LVGL call with
`lvgl_port_lock()`/`lvgl_port_unlock()` — this runs from the OTA task, not an LVGL
callback.

- [ ] **Step 6: Switch the capability bit on**

In `firmware/main/core/protocol_message.h` add `PROTOCOL_CAPABILITY_FIRMWARE_UPDATE` to
`PROTOCOL_CURRENT_CAPABILITIES`, and mirror it in `CURRENT_CAPABILITIES` in
`companion/crates/protocol/src/message.rs`. Update any test that asserts the exact
capability value — with networking (bit 7) and firmware update (bit 6) the value becomes
`0b1100_1011` = **203**.

- [ ] **Step 7: Report OTA state in status**

Fill `status->ota_state` from `ota_state()` in the status builder.

- [ ] **Step 8: Run the gates**

```sh
make -C firmware/host_tests clean test
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build
```

From `companion/`:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- [ ] **Step 9: Verify the happy path on the physical board**

Build version A, flash it over USB, then build version B with a visible difference and
place it in the server's firmware directory. Confirm all five: the device reports the new
version available; the panel shows the update takeover with advancing progress; the
device reboots into version B; `status` reports version B; and the previous slot still
holds version A.

- [ ] **Step 10: Verify rollback on the physical board**

This is the check that matters most, because it is the one that saves a fleet. Build a
deliberately broken version C — the simplest honest break is an early `abort()` before
`ota_mark_running_image_valid()` is reached — serve it, and confirm the device installs
it, fails to validate, and **returns to version B on the next reset**.

Do not skip this on the grounds that the code looks right. An unexercised rollback path
is indistinguishable from a missing one.

- [ ] **Step 11: Verify the deferral**

Start a pomodoro, then trigger a check. Confirm the update does not begin, and that it
proceeds once the pomodoro finishes.

- [ ] **Step 12: Record and commit**

```bash
git add firmware/main/link/ota.c firmware/main/link/ota.h \
        firmware/main/ui/ota_screen.c firmware/main/ui/ota_screen.h \
        firmware/main/main.c firmware/main/link/protocol_task.c \
        firmware/main/core/protocol_message.h firmware/sdkconfig.defaults \
        companion/crates/protocol/src/message.rs docs/hardware/board-notes.md
git commit -m "feat: over-the-air firmware update with rollback

esp_https_ota into the inactive slot, then a local-only self-check before
cancelling the bootloader's rollback. Requiring the server to be reachable as
proof of a good image would roll back the whole fleet during any outage, so
validity is display up, LVGL running, protocol task alive, NVS readable --
nothing that depends on the network.

Updates defer while an interrupt is live or a pomodoro is running, and the
panel shows progress rather than freezing for the length of a multi-megabyte
download.

CURRENT_CAPABILITIES becomes 203: core widgets, config rotation, extended
templates, firmware update, networking.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01SwAMBn6h8WMLU4ADiA8LwG"
```

---

### Task 12:12 Physical exit gate and documentation

The spec's §7.2 gate, run as one session on the physical board, plus the durable record.
This task cannot be completed by an agent alone: flashing, cable pulls, touching and
judging are human actions, and per `CLAUDE.md` nothing may be recorded as verified that
was not observed.

**Files:**
- Modify: `docs/hardware/board-notes.md`
- Modify: `CLAUDE.md`
- Modify: `docs/superpowers/plans/2026-08-03-deskmate-roadmap.md`
- Modify: `docs/superpowers/specs/2026-08-18-deskmate-v2-networked-device-design.md`

**Interfaces:**
- Consumes: everything.
- Produces: the durable record.

- [ ] **Step 1: Run the full software set before flashing**

```sh
make -C firmware/host_tests clean test
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build
```

From `companion/`:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Do not flash on a red suite.

- [ ] **Step 2: Capture the new heap and image baseline**

Before the soak, record the image size from the build summary and the free-heap figure
from `status --json` on a freshly booted networked device. **Capture the baseline first
rather than deriving it from the soak**, or a regression can hide inside the
re-baselining.

- [ ] **Step 3: Gate item 1 — the headline demo**

Provision over USB, unplug the cable, quit the Mac app, and confirm cards keep updating
from the server. Capture the panel with the webcam harness.

- [ ] **Step 4: Gate item 2 — tier switch both directions**

Pair to networked, confirm USB config is refused with the wrong-tier error, unpair back
to local, and confirm the Mac owns it again.

- [ ] **Step 5: Gate item 3 — link loss and recovery**

Kill the server; confirm the device falls to the standalone clock after the 45 s deadline
and recovers when the server returns, **without a reboot**. Repeat by dropping WiFi
instead of the server.

- [ ] **Step 6: Gate item 4 — OTA and rollback**

Re-run Task 11's Steps 9 and 10 as part of the gate session, so the record covers one
continuous run rather than a stitched-together history.

- [ ] **Step 7: Gate item 5 — free tier is radio-silent for config and data**

Set the device to local tier with WiFi credentials present. Confirm the time is correct
via SNTP, the firmware check works, and the **server's logs show no control connection**
for that device for the duration.

- [ ] **Step 8: Gate item 6 — re-baselined soak**

Run a 30-minute mixed soak with the radio up: rotation, taps, provider refreshes,
interrupts. Confirm the heap is flat at the Step 2 baseline and the queue high-water
stays bounded. Record the actual figures, not a verdict.

- [ ] **Step 9: Gate item 7 — alert replay across a network reconnect**

Schedule a bounded-hold alert, drop the link before it fires, and confirm it is delivered
on reconnect. This path has produced real defects twice; the existing
`companion/crates/app-core/examples/alert_replay_check.rs` is the closest precedent for
how to drive it.

- [ ] **Step 10: Record everything in board-notes**

Add a dated section following the file's existing convention. Record what was observed at
each item, **including anything that did not pass**. Write no "verified" against anything
not seen.

- [ ] **Step 11: Update the durable documents**

- `CLAUDE.md`: add V2's lasting constraints to "Current state" — tier is persistent NVS
  state, the cable is a configurator not an owner in networked tier, provisioning is
  USB-only with no SoftAP, OTA validity is local-only, `CURRENT_CAPABILITIES` is 203, and
  the wire is still protocol v1.
- Roadmap: mark V2's row complete with the observation date, and confirm V3's narrowed
  scope still reads correctly after implementation.
- Spec: change the status line to `delivered` with the observation date.

- [ ] **Step 12: Commit**

```bash
git add docs/hardware/board-notes.md CLAUDE.md \
        docs/superpowers/plans/2026-08-03-deskmate-roadmap.md \
        docs/superpowers/specs/2026-08-18-deskmate-v2-networked-device-design.md
git commit -m "docs: V2 physical exit gate observed

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01SwAMBn6h8WMLU4ADiA8LwG"
```
