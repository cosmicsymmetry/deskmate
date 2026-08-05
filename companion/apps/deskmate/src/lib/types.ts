export interface AppConfig {
  schema_version: number;
  preferences: AppPreferences;
  widgets: WidgetSettings[];
  screens: ScreenSettings[];
  assets: AssetSettings[];
  carousel: CarouselSettings;
  updater: UpdaterSettings;
}

export interface AppPreferences {
  timezone: string;
  autostart: boolean;
  paused: boolean;
  orientation: DisplayOrientation;
}

export type DisplayOrientation = "landscape" | "landscape-flipped";

export type WidgetSize = "full" | "standard" | "tile";

export type DisplayTemplate =
  | { kind: "digital-clock" }
  | { kind: "analog-clock" }
  | { kind: "progress-ring" }
  | { kind: "row-list" }
  | { kind: "big-number-label" }
  | { kind: "icon-badge-text"; icon_asset_id: string | null };

export type WidgetTapAction =
  | { kind: "none" }
  | { kind: "start-pause" }
  | { kind: "reset" }
  | { kind: "dismiss" }
  | { kind: "open-url"; url: string }
  | { kind: "open-application"; application_id: string };

export type RefreshPolicy =
  | { kind: "device-local" }
  | { kind: "manual" }
  | { kind: "interval"; minutes: number };

export type WidgetInterruptPolicy = "disabled" | "enabled";
export type WeatherUnits = "metric" | "imperial";

export interface JsonFieldMapping {
  field: string;
  path: string;
}

export type WidgetSettings =
  | {
      kind: "clock";
      id: string;
      size: WidgetSize;
      title: string;
      show_seconds: boolean;
      template: DisplayTemplate;
      tap_action: WidgetTapAction;
      refresh: RefreshPolicy;
      interrupt_policy: WidgetInterruptPolicy;
    }
  | {
      kind: "pomodoro";
      id: string;
      size: WidgetSize;
      label: string;
      duration_seconds: number;
      template: DisplayTemplate;
      tap_action: WidgetTapAction;
      refresh: RefreshPolicy;
      interrupt_policy: WidgetInterruptPolicy;
    }
  | {
      kind: "calendar";
      id: string;
      size: WidgetSize;
      title: string;
      source: CalendarSource;
      template: DisplayTemplate;
      tap_action: WidgetTapAction;
      refresh: RefreshPolicy;
      interrupt_policy: WidgetInterruptPolicy;
    }
  | {
      kind: "weather";
      id: string;
      size: WidgetSize;
      title: string;
      location: string;
      units: WeatherUnits;
      template: DisplayTemplate;
      tap_action: WidgetTapAction;
      refresh: RefreshPolicy;
      interrupt_policy: WidgetInterruptPolicy;
    }
  | {
      kind: "json-feed";
      id: string;
      size: WidgetSize;
      title: string;
      url: string;
      mappings: JsonFieldMapping[];
      template: DisplayTemplate;
      tap_action: WidgetTapAction;
      refresh: RefreshPolicy;
      interrupt_policy: WidgetInterruptPolicy;
    }
  | {
      kind: "rss";
      id: string;
      size: WidgetSize;
      title: string;
      url: string;
      max_items: number;
      template: DisplayTemplate;
      tap_action: WidgetTapAction;
      refresh: RefreshPolicy;
      interrupt_policy: WidgetInterruptPolicy;
    };

export type CalendarSource = { kind: "file"; value: string } | { kind: "url"; value: string };

export interface ScreenSettings {
  id: string;
  layout: ScreenLayout;
}

export type ScreenLayout =
  | { kind: "single"; widget_id: string }
  | { kind: "dashboard"; columns: number; rows: number; tiles: TileSettings[] };

export interface TileSettings {
  widget_id: string;
  column: number;
  row: number;
  column_span: number;
  row_span: number;
}

export interface AssetSettings {
  id: string;
  source: { kind: "file"; value: string };
  kind:
    | { kind: "icon"; width: number; height: number }
    | { kind: "font"; pixel_size: number; glyph_ranges: GlyphRange[] };
  maximum_bytes: number;
}

export interface GlyphRange {
  start: number;
  end: number;
}

export interface CarouselSettings {
  auto_advance_seconds: number | null;
}

export interface UpdaterSettings {
  channel: "stable" | "beta" | "manual";
  checks: "disabled" | "notify";
}

export interface FirmwareArtifactMetadata {
  version: string;
  model: string;
  byte_length: number;
  sha256_hex: string;
  signing_key_id: string;
  signature_base64: string;
}

export type ValidationCode =
  | "unsupported-version"
  | "empty"
  | "too-long"
  | "too-many"
  | "duplicate-id"
  | "missing-reference"
  | "missing-screen"
  | "duplicate-reference"
  | "unsupported-size"
  | "out-of-range"
  | "invalid-timezone"
  | "invalid-source"
  | "invalid-composition"
  | "overlap"
  | "too-large"
  | "requires-capability";

export interface ValidationIssue {
  path: string;
  code: ValidationCode;
  message: string;
}

export interface AppSnapshot {
  config: AppConfig;
  runtime: RuntimeState;
  device: DeviceSnapshot;
  providers: ProviderSnapshot[];
  pomodoros: PomodoroSnapshot[];
  persistence: PersistenceState;
  diagnostics: RuntimeDiagnostics;
}

export type RuntimeState =
  | { kind: "starting" }
  | { kind: "running" }
  | { kind: "paused" }
  | { kind: "error"; message: string };

export type ConnectionState =
  | { kind: "disconnected"; reason: string | null }
  | { kind: "connecting" }
  | { kind: "online" }
  | { kind: "standalone" };

export interface DeviceSnapshot {
  connection: ConnectionState;
  port_name: string | null;
  firmware_version: string | null;
  protocol_version: number | null;
  max_protocol_version: number | null;
  capabilities: DeviceCapability[];
  unknown_capability_bits: string;
  uptime_ms: number | null;
  free_heap: number | null;
  rotation: number | null;
  active_screen_id: string | null;
  counters: DeviceCounters;
}

export type DeviceCapability =
  | "core-widgets"
  | "config-rotation"
  | "dashboard-layouts"
  | "extended-templates"
  | "host-tap-actions"
  | "asset-transfer"
  | "firmware-update";

export interface DeviceCounters {
  reconnects: number;
  valid_frames: number;
  malformed_frames: number;
  crc_errors: number;
  overflow_frames: number;
  dropped_responses: number;
  rx_dropped_bytes: number;
  dropped_events: number;
  event_queue_high_water: number;
  dropped_ui_commands: number;
  ui_queue_high_water: number;
  host_dropped_events: number;
  detected_event_gaps: number;
}

export interface ProviderSnapshot {
  widget_id: string;
  state: ProviderState;
  last_success_unix_ms: number | null;
  age_seconds: number | null;
}

export type ProviderState =
  | { kind: "idle" }
  | { kind: "refreshing" }
  | { kind: "fresh" }
  | { kind: "stale"; message: string }
  | { kind: "error"; message: string };

export interface PomodoroSnapshot {
  widget_id: string;
  state: PomodoroState;
  duration_seconds: number;
  remaining_seconds: number;
}

export type PomodoroState = "idle" | "running" | "paused" | "completed";

export type PersistenceState =
  | { kind: "clean" }
  | { kind: "saving" }
  | { kind: "recoverable-error"; message: string };

export interface RuntimeDiagnostics {
  commands_processed: number;
  command_queue_full: number;
  provider_jobs_started: number;
  provider_queue_full: number;
  provider_results_discarded: number;
  subscriber_snapshots_overwritten: number;
}

export type PomodoroAction = "start" | "pause" | "toggle" | "reset";

export interface DraftValidation {
  valid: boolean;
  issues: ValidationIssue[];
}

export type StoreWarning = {
  kind: "io";
  operation: string;
  message: string;
};

export interface SaveReceipt {
  generation: number;
  warning: StoreWarning | null;
}

export interface ConfigApplyResult {
  save: SaveReceipt;
}

export interface AutostartStatus {
  enabled: boolean;
  preference_enabled: boolean;
}

type MessageError<Category extends string> = {
  category: Category;
  message: string;
};

export type IpcError =
  | MessageError<"invalid-payload">
  | (MessageError<"payload-too-large"> & { maximum_bytes: number })
  | (MessageError<"validation"> & { issues: ValidationIssue[] })
  | MessageError<"persistence">
  | MessageError<"runtime-busy">
  | MessageError<"runtime-unavailable">
  | MessageError<"not-found">
  | MessageError<"device">
  | MessageError<"provider">
  | MessageError<"autostart">
  | MessageError<"window">
  | MessageError<"internal">;

// This fixture shape is generated from Rust serialization in a backend test, then
// compiled against these declarations. Either side changing makes CI fail.
export interface IpcContractFixtures {
  snapshot: AppSnapshot;
  configs: AppConfig[];
  widget_settings: WidgetSettings[];
  widget_sizes: WidgetSize[];
  calendar_sources: CalendarSource[];
  display_templates: DisplayTemplate[];
  tap_actions: WidgetTapAction[];
  refresh_policies: RefreshPolicy[];
  interrupt_policies: WidgetInterruptPolicy[];
  weather_units: WeatherUnits[];
  screen_layouts: ScreenLayout[];
  asset_sources: AssetSettings["source"][];
  asset_kinds: AssetSettings["kind"][];
  update_channels: UpdaterSettings["channel"][];
  update_check_policies: UpdaterSettings["checks"][];
  firmware_artifacts: FirmwareArtifactMetadata[];
  display_orientations: DisplayOrientation[];
  device_capabilities: DeviceCapability[];
  runtime_states: RuntimeState[];
  connection_states: ConnectionState[];
  provider_states: ProviderState[];
  pomodoro_states: PomodoroState[];
  persistence_states: PersistenceState[];
  validation_codes: ValidationCode[];
  pomodoro_actions: PomodoroAction[];
  errors: IpcError[];
  draft_validation: DraftValidation;
  config_apply_result: ConfigApplyResult;
  autostart_status: AutostartStatus;
}
