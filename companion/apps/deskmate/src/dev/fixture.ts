/**
 * Dev-only fixture state for the browser mock backend.
 *
 * This file never ships: `vite.config.ts` only aliases `@tauri-apps/api/*` to the
 * mock when `VITE_DESKMATE_MOCK=1`, so a production build resolves the real IPC
 * bridge and tree-shakes this module away entirely.
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
    schema_version: 4,
    preferences: {
      timezone: "Asia/Tbilisi",
      autostart: true,
      paused: false,
      orientation: "landscape",
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
      },
      {
        kind: "weather",
        id: "weather",
        title: "Outside",
        location: "Tbilisi",
        units: "metric",
        template: { kind: "icon-badge-text", icon_asset_id: null },
        tap_action: { kind: "none" },
        refresh: { kind: "interval", minutes: 30 },
        alert: { kind: "none" },
      },
      {
        kind: "calendar",
        id: "calendar",
        title: "Today",
        source: { kind: "url", value: "https://calendar.example/rodion.ics" },
        template: { kind: "row-list" },
        tap_action: { kind: "none" },
        refresh: { kind: "interval", minutes: 15 },
        alert: { kind: "before-event", lead_minutes: 10, hold: { kind: "seconds", value: 120 } },
      },
      {
        kind: "rss",
        id: "rss",
        title: "Headlines",
        url: "https://news.ycombinator.com/rss",
        max_items: 3,
        template: { kind: "row-list" },
        tap_action: { kind: "none" },
        refresh: { kind: "interval", minutes: 30 },
        alert: { kind: "none" },
      },
      {
        kind: "json-feed",
        id: "json-feed",
        title: "Build status",
        url: "https://ci.example/status.json",
        mappings: [
          { field: "hero", path: "pipeline.state" },
          { field: "caption", path: "pipeline.branch" },
        ],
        template: { kind: "big-number-label" },
        tap_action: { kind: "none" },
        refresh: { kind: "interval", minutes: 5 },
        alert: { kind: "none" },
      },
    ],
    assets: [],
    playlists: [
      {
        id: "day",
        name: "Workday",
        advance: { kind: "timed", default_dwell_seconds: 20 },
        entries: [
          { card_id: "clock", dwell_seconds: 45 },
          { card_id: "pomodoro", dwell_seconds: null },
          { card_id: "weather", dwell_seconds: 15 },
          { card_id: "calendar", dwell_seconds: 30 },
        ],
      },
      {
        id: "evening",
        name: "Evening",
        advance: { kind: "manual" },
        entries: [
          { card_id: "clock", dwell_seconds: null },
          { card_id: "rss", dwell_seconds: null },
        ],
      },
    ],
    active_playlist_id: "day",
    updater: { channel: "stable", checks: "notify" },
  };
}

export function mockCardData(): CardDataSnapshot[] {
  return [
    {
      card_id: "weather",
      fields: [
        { key: "hero", value: { kind: "text", value: "18°" } },
        { key: "caption", value: { kind: "text", value: "Clear · feels 17°" } },
        { key: "icon", value: { kind: "text", value: "clear-day" } },
      ],
    },
    {
      card_id: "calendar",
      fields: [
        { key: "row0", value: { kind: "text", value: "09:30  Standup" } },
        { key: "row1", value: { kind: "text", value: "13:00  Design review" } },
        { key: "row2", value: { kind: "text", value: "16:30  1:1 with Ana" } },
      ],
    },
    {
      card_id: "rss",
      fields: [
        { key: "row0", value: { kind: "text", value: "Show HN: A tiny desk display" } },
        { key: "row1", value: { kind: "text", value: "The case for boring software" } },
      ],
    },
    {
      card_id: "json-feed",
      fields: [
        { key: "hero", value: { kind: "text", value: "PASS" } },
        { key: "caption", value: { kind: "text", value: "main" } },
      ],
    },
  ];
}

export function mockNetworkSettings(): NetworkSettings {
  return { server_url: "https://deskmate.rodi.one", device_id: "desk-01", tier: "networked" };
}

export function mockSnapshot(config: AppConfig): AppSnapshot {
  return {
    config,
    has_saved_config: true,
    runtime: { kind: "running" },
    device: {
      connection: { kind: "online" },
      port_name: "/dev/cu.usbmodem2101",
      firmware_version: "1.0.0",
      protocol_version: 1,
      max_protocol_version: 1,
      capabilities: [
        "core-widgets",
        "config-rotation",
        "extended-templates",
        "firmware-update",
        "networking",
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
      active_screen_id: "clock",
      counters: MOCK_COUNTERS,
    },
    providers: [
      { widget_id: "weather", state: { kind: "fresh" }, last_success_unix_ms: 1, age_seconds: 240 },
      {
        widget_id: "calendar",
        state: { kind: "fresh" },
        last_success_unix_ms: 1,
        age_seconds: 40,
      },
      {
        widget_id: "rss",
        state: { kind: "stale", message: "Feed timed out after 10s" },
        last_success_unix_ms: 1,
        age_seconds: 5400,
      },
      {
        widget_id: "json-feed",
        state: { kind: "fresh" },
        last_success_unix_ms: 1,
        age_seconds: 120,
      },
    ],
    pomodoros: [
      {
        widget_id: "pomodoro",
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
      provider_jobs_started: 96,
      provider_queue_full: 0,
      provider_results_discarded: 0,
      subscriber_snapshots_overwritten: 2,
      interrupt_dismissals_ignored: 0,
    },
  };
}
