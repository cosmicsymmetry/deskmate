/**
 * Dev-only fixture state for the browser mock backend.
 *
 * This file never ships: `vite.config.ts` only aliases `./backendClient` to the
 * mock when `VITE_DESKMATE_MOCK=1`, so a production build resolves the real HTTP
 * client and tree-shakes this module away entirely.
 *
 * The shapes here are the ones in `src/lib/types.ts`, which are themselves checked
 * against Rust serialization in CI — so a drift in the backend contract breaks this
 * harness at type-check time rather than silently rendering a lie.
 */
import type {
  AppConfig,
  AppSnapshot,
  CardDataSnapshot,
  DeviceCounters,
  NetworkSettings,
} from "../lib/types";
import { apiContractFixtures } from "../lib/types.contract";

export const MOCK_COUNTERS: DeviceCounters = {
  host_reconnects: 0,
  valid_frames: 48213,
  malformed_frames: 0,
  crc_errors: 0,
  overflow_frames: 0,
  dropped_responses: 0,
  rx_dropped_bytes: 0,
  dropped_events: 0,
  event_queue_high_water: 3,
  dropped_ui_commands: 0,
  ui_queue_high_water: 2,
  host_dropped_events: 0,
  detected_event_gaps: 0,
};

export function mockConfig(): AppConfig {
  return {
    schema_version: apiContractFixtures.snapshot.config.schema_version,
    preferences: {
      timezone: "Asia/Tbilisi",
      autostart: true,
      paused: false,
      orientation: "landscape",
      brightness: 78,
    },
    cards: [
      {
        kind: "clock",
        id: "clock",
        title: "Desk",
        show_seconds: false,
        template: { kind: "digital-clock" },
        tap_action: { kind: "none" },
        refresh: { kind: "device-local" },
        alert: { kind: "none" },
        dwell_seconds: null,
      },
      {
        kind: "pomodoro",
        id: "pomodoro",
        label: "Deep work",
        duration_seconds: 1500,
        template: { kind: "progress-ring" },
        tap_action: { kind: "start-pause" },
        refresh: { kind: "device-local" },
        alert: { kind: "on-timer-finish", hold: { kind: "until-dismissed" } },
        dwell_seconds: null,
      },
      {
        kind: "picture",
        id: "picture",
        title: "Studio",
        source_id: "studio",
        tap_action: { kind: "none" },
        refresh: { kind: "manual" },
        alert: { kind: "none" },
        dwell_seconds: null,
      },
    ],
    image_sources: [{ id: "studio", name: "Studio" }],
    assets: [],
    advance: { kind: "timed", default_dwell_seconds: 30 },
    updater: { channel: "stable", checks: "notify" },
  };
}

export function mockCardData(): CardDataSnapshot[] {
  return [];
}

export function mockNetworkSettings(): NetworkSettings {
  return { server_url: "https://deskmate.rodi.one", device_id: "desk-01", tier: "networked" };
}

export function mockSnapshot(config: AppConfig): AppSnapshot {
  return {
    config,
    has_saved_config: true,
    // The wire the server speaks. Matching the device's `protocol_version` below
    // is the ordinary case, and the harness has to show the ordinary case: a
    // fixture where they differ would paint a permanent incompatibility banner
    // over every screenshot.
    host_protocol_version: 2,
    runtime: { kind: "running" },
    device: {
      connection: { kind: "online" },
      port_name: "network:desk-01",
      firmware_version: "1.0.0",
      protocol_version: 2,
      max_protocol_version: 2,
      capabilities: [
        "asset-transfer",
        "firmware-update",
        "networking",
        "scene-render",
        "volatile-assets",
        "durable-asset-encoding",
      ],
      unknown_capability_bits: "0",
      uptime_ms: 4_812_000,
      free_heap: 8_012_432,
      rotation: 90,
      tier: "networked",
      wifi_state: "connected",
      wifi_rssi: -54,
      ip: "192.168.1.42",
      last_network_error: null,
      ota_state: "idle",
      active_card_id: "clock",
      counters: MOCK_COUNTERS,
      // What a linked board with four picture frames in flash reports. Nothing
      // renders it; it is here because the harness mirrors the shipped shape.
      asset_store: { used_bytes: 141_312, free_bytes: 6_149_120, asset_count: 4 },
      volatile_assets: {
        committed_count: 4,
        slot_capacity: 16,
        used_bytes: 1_318_960,
        psram_free_bytes: 7_012_352,
        psram_low_water_bytes: 6_803_456,
      },
    },
    pomodoros: [
      {
        card_id: "pomodoro",
        state: "running",
        duration_seconds: 1500,
        remaining_seconds: 764,
      },
    ],
    card_data: mockCardData(),
    card_errors: [],
    persistence: { kind: "clean" },
    diagnostics: {
      commands_processed: 1284,
      command_queue_full: 0,
      subscriber_snapshots_overwritten: 2,
      interrupt_dismissals_ignored: 0,
      taps_dropped: 0,
    },
  };
}
