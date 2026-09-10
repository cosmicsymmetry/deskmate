export interface AppConfig {
  schema_version: number;
  preferences: AppPreferences;
  cards: CardSettings[];
  image_sources: ImageSource[];
  assets: AssetSettings[];
  playlists: Playlist[];
  active_playlist_id: string;
  updater: UpdaterSettings;
}

export const MAX_PLAYLISTS = 8;
export const MAX_PLAYLIST_ENTRIES = 8;
export const MAX_PLAYLIST_NAME_LEN = 48;

export interface AppPreferences {
  timezone: string;
  autostart: boolean;
  paused: boolean;
  orientation: DisplayOrientation;
}

export type DisplayOrientation = "landscape" | "landscape-flipped";

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

export type AlertHold = { kind: "until-dismissed" } | { kind: "seconds"; value: number };

export type CardAlert = { kind: "none" } | { kind: "on-timer-finish"; hold: AlertHold };

export type CardSettings =
  | {
      kind: "clock";
      id: string;
      title: string;
      show_seconds: boolean;
      template: DisplayTemplate;
      tap_action: WidgetTapAction;
      refresh: RefreshPolicy;
      alert: CardAlert;
    }
  | {
      kind: "pomodoro";
      id: string;
      label: string;
      duration_seconds: number;
      template: DisplayTemplate;
      tap_action: WidgetTapAction;
      refresh: RefreshPolicy;
      alert: CardAlert;
    }
  | {
      kind: "plugin";
      id: string;
      title: string;
      plugin_id: string;
      tap_action: WidgetTapAction;
      refresh: RefreshPolicy;
      alert: CardAlert;
    }
  | {
      kind: "picture";
      id: string;
      title: string;
      source_id: string;
      tap_action: WidgetTapAction;
      refresh: RefreshPolicy;
      alert: CardAlert;
    };

export type CardKind = CardSettings["kind"];
export type AddableCardKind = Exclude<CardKind, "plugin" | "picture">;

export interface ImageSource {
  id: string;
  name: string;
}

/** Secret-bearing result returned once when the server creates an image source. */
export interface MintedImageSource {
  source_id: string;
  token: string;
  push_url: string;
}

export interface AssetSettings {
  id: string;
  source: { kind: "file"; value: string };
  kind: { kind: "font" } | { kind: "icon-font"; glyphs: IconGlyphMapping[] } | { kind: "image" };
  maximum_bytes: number;
}

export interface IconGlyphMapping {
  name: string;
  codepoint: number;
}

export type CarouselAdvance = { kind: "manual" } | { kind: "timed"; default_dwell_seconds: number };

export interface Playlist {
  id: string;
  name: string;
  advance: CarouselAdvance;
  entries: PlaylistEntry[];
}

export interface PlaylistEntry {
  card_id: string;
  dwell_seconds: number | null;
}

export interface UpdaterSettings {
  channel: "stable" | "beta" | "manual";
  checks: "disabled" | "notify";
}

export type ValidationCode =
  | "unsupported-version"
  | "empty"
  | "too-long"
  | "too-many"
  | "duplicate-id"
  | "missing-reference"
  | "out-of-range"
  | "invalid-timezone"
  | "invalid-source"
  | "invalid-composition"
  | "too-large"
  | "requires-capability";

export interface ValidationIssue {
  path: string;
  code: ValidationCode;
  message: string;
}

export interface AppSnapshot {
  config: AppConfig;
  /** True once a settings document has existed on disk, independent of draft edits. */
  has_saved_config: boolean;
  runtime: RuntimeState;
  device: DeviceSnapshot;
  providers: ProviderSnapshot[];
  pomodoros: PomodoroSnapshot[];
  card_data: CardDataSnapshot[];
  card_errors: CardError[];
  persistence: PersistenceState;
  diagnostics: RuntimeDiagnostics;
}

/// A card whose last data push the display understood and refused. Retrying the
/// identical payload can only fail again, so the runtime drops it and surfaces this
/// instead of looping.
export interface CardError {
  kind: "data-refused" | "scene-refused";
  card_id: string;
  message: string;
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
  tier: DeviceTier | null;
  wifi_state: DeviceWifiState | null;
  wifi_rssi: number | null;
  ip: string | null;
  last_network_error: string | null;
  ota_state: DeviceOtaState | null;
  active_screen_id: string | null;
  counters: DeviceCounters;
}

export type DeviceTier = "local" | "networked";

export type DeviceWifiState = "down" | "connecting" | "connected" | "failed";

export type DeviceOtaState = "idle" | "checking" | "downloading" | "pending-verify" | "failed";

/** The persisted, non-secret portion of the Mac app's server settings. */
export interface NetworkSettings {
  server_url: string;
  device_id: string;
  tier: DeviceTier | null;
}

/**
 * A write-only provisioning request. Secret fields are accepted by IPC but are
 * deliberately absent from every response, snapshot, and event type.
 */
export interface ProvisionDeviceInput {
  ssid: string;
  passphrase: string;
  server_url: string;
  device_id: string;
  device_token: string;
  tier: DeviceTier;
}

export type DeviceCapability =
  | "core-widgets"
  | "config-rotation"
  | "dashboard-layouts"
  | "extended-templates"
  | "host-tap-actions"
  | "asset-transfer"
  | "firmware-update"
  | "networking"
  | "scene-render"
  | "volatile-assets"
  | "durable-asset-encoding";

export interface DeviceCounters {
  host_reconnects: number;
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

export type CardFieldValue =
  | { kind: "text"; value: string }
  | { kind: "integer"; value: number }
  | { kind: "boolean"; value: boolean };

export interface CardField {
  key: string;
  value: CardFieldValue;
}

export interface CardDataSnapshot {
  card_id: string;
  fields: CardField[];
}

export type PluginTemplateKind = "display-list" | "svg";

export interface PluginCatalogAsset {
  file: string;
  kind: string;
  byte_length: number;
  digest: string;
}

export interface PluginCatalogEntry {
  id: string;
  name: string;
  version: string;
  node_count: number;
  assets: PluginCatalogAsset[];
  display_name: string | null;
  description: string | null;
  manifest_version: number;
  template: PluginTemplateKind;
  refresh_minutes: number;
}

export interface PluginLoadFailure {
  id: string;
  error: string;
}

export interface PluginCatalog {
  plugins: PluginCatalogEntry[];
  load_failures: PluginLoadFailure[];
}

/** The server's own view of one plugin or picture card, projected onto this window's snapshot. */
export interface ServerCardState {
  card_id: string;
  provider: ProviderState;
  hero: string | null;
  errors: CardError[];
}

export type PersistenceState =
  | { kind: "clean" }
  | { kind: "saving" }
  | { kind: "recoverable-error"; message: string }
  | { kind: "validation-failed"; message: string; issues: ValidationIssue[] };

export interface RuntimeDiagnostics {
  commands_processed: number;
  command_queue_full: number;
  provider_jobs_started: number;
  provider_queue_full: number;
  provider_results_discarded: number;
  subscriber_snapshots_overwritten: number;
  interrupt_dismissals_ignored: number;
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

/**
 * `png_base64` is null exactly when the renderer produced no pixels; `state` then
 * carries the word for why ("Waiting for the first refresh", "Plugin cards render on
 * the server", the server's own error). Built-in cards keep `png_base64` set and
 * `state` null, so nothing about them changes.
 */
export interface PreviewFrame {
  png_base64: string | null;
  sample: boolean;
  state: string | null;
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
  | MessageError<"incompatible-server">
  | MessageError<"not-found">
  | MessageError<"device">
  | MessageError<"provider">
  | MessageError<"autostart">
  | MessageError<"window">
  | MessageError<"internal">
  | MessageError<"unsupported">;

// This fixture shape is generated from Rust serialization in a backend test, then
// compiled against these declarations. Either side changing makes CI fail.
export interface IpcContractFixtures {
  snapshot: AppSnapshot;
  configs: AppConfig[];
  card_settings: CardSettings[];
  playlists: Playlist[];
  playlist_entries: PlaylistEntry[];
  card_alerts: CardAlert[];
  alert_holds: AlertHold[];
  carousel_advances: CarouselAdvance[];
  display_templates: DisplayTemplate[];
  tap_actions: WidgetTapAction[];
  refresh_policies: RefreshPolicy[];
  asset_sources: AssetSettings["source"][];
  asset_kinds: AssetSettings["kind"][];
  update_channels: UpdaterSettings["channel"][];
  update_check_policies: UpdaterSettings["checks"][];
  display_orientations: DisplayOrientation[];
  device_capabilities: DeviceCapability[];
  runtime_states: RuntimeState[];
  connection_states: ConnectionState[];
  provider_states: ProviderState[];
  pomodoro_states: PomodoroState[];
  card_data: CardDataSnapshot[];
  persistence_states: PersistenceState[];
  validation_codes: ValidationCode[];
  pomodoro_actions: PomodoroAction[];
  errors: IpcError[];
  draft_validation: DraftValidation;
  config_apply_result: ConfigApplyResult;
  autostart_status: AutostartStatus;
  plugin_catalog: PluginCatalog;
  server_card_state: ServerCardState[];
  preview_frame: PreviewFrame;
}
