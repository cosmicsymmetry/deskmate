export interface AppConfig {
  schema_version: number;
  preferences: AppPreferences;
  cards: CardSettings[];
  image_sources: ImageSource[];
  assets: AssetSettings[];
  advance: CarouselAdvance;
  updater: UpdaterSettings;
}

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
  | { kind: "progress-ring" };

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
      dwell_seconds: number | null;
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
      dwell_seconds: number | null;
    }
  | {
      kind: "picture";
      id: string;
      title: string;
      source_id: string;
      tap_action: WidgetTapAction;
      refresh: RefreshPolicy;
      alert: CardAlert;
      dwell_seconds: number | null;
    };

export type CardKind = CardSettings["kind"];
export type AddableCardKind = Exclude<CardKind, "picture">;

export interface ImageSource {
  id: string;
  name: string;
}

export interface ImageSourceDescriptor {
  id: string;
  name: string;
  face: FaceDescriptor | null;
  /** Null exactly when `face` is: an external producer's source has nothing to report. */
  face_status: FaceStatus | null;
}

/**
 * Why a server-drawn face is or is not drawing, in the server's words. This is the only
 * place the owner learns that a coin ID was not found or an API refused: the panel
 * just says "Waiting for the first picture".
 */
export interface FaceStatus {
  state: "needs-settings" | "drawing" | "drawn" | "needs-attention" | "retrying" | "unavailable";
  /** The faces package's own sentence, for `needs-attention` and `retrying`. */
  message: string | null;
  at_unix_seconds: number | null;
}

export interface FaceDescriptor {
  /** Opaque server metadata. The app renders `fields` and never branches on this value. */
  kind: string;
  label: string;
  /**
   * What the face says a tap on the panel does, in its own words. Absent means this
   * face ignores taps -- the server drops them rather than re-rendering.
   */
  tap?: string;
  fields: FaceFieldDescriptor[];
}

export type FaceFieldDescriptor =
  | {
      type: "text" | "url";
      key: string;
      label: string;
      value: string;
      placeholder: string;
    }
  | {
      type: "enum";
      key: string;
      label: string;
      value: string;
      options: FaceFieldOption[];
    };

export interface FaceFieldOption {
  value: string;
  label: string;
}

/** Raw secret-bearing response returned by the image-source mint route. */
export interface MintSourceResponse {
  id: string;
  token: string;
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
  /**
   * The wire version the server speaks.
   *
   * Reported rather than hard-coded so compatibility checks stay aligned with
   * the server across protocol revisions.
   */
  host_protocol_version: number;
  runtime: RuntimeState;
  device: DeviceSnapshot;
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
  active_card_id: string | null;
  counters: DeviceCounters;
  asset_store: DeviceAssetStore | null;
  volatile_assets: DeviceVolatileAssets | null;
}

/**
 * The display's durable flash asset store, as it reports it. Flash numbers, not
 * PSRAM ones: they say whether the flash is still being written for picture
 * frames, not how much room the volatile frame tier has left. Nothing renders
 * them -- the snapshot's change key blanks them, because they move on every
 * frame a face draws.
 */
export interface DeviceAssetStore {
  used_bytes: number;
  free_bytes: number;
  asset_count: number;
}

/**
 * The display's volatile frame pool and the PSRAM heap its frames come from.
 * Null on any firmware built before the pool existed. Unlike the flash store
 * above, these numbers say whether another frame will fit. Nothing renders them.
 */
export interface DeviceVolatileAssets {
  committed_count: number;
  slot_capacity: number;
  used_bytes: number;
  psram_free_bytes: number;
  psram_low_water_bytes: number;
}

export type DeviceTier = "local" | "networked";

export type DeviceWifiState = "down" | "connecting" | "connected" | "failed";

export type DeviceOtaState = "idle" | "checking" | "downloading" | "pending-verify" | "failed";

/** The server origin and display selected by this browser session. */
export interface NetworkSettings {
  server_url: string;
  device_id: string;
  tier: DeviceTier | null;
}

export interface DeviceRow {
  id: string;
  connected: boolean;
  has_saved_config: boolean;
  configured_at: number | null;
  state: "pending" | "active";
}

/**
 * Protocol v2 re-based these. The bits that described rendering a template --
 * core widgets, config rotation, dashboard layouts, extended templates, host
 * tap actions -- say nothing about a device that only draws scenes.
 */
export type DeviceCapability =
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

export interface PomodoroSnapshot {
  card_id: string;
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

export type PersistenceState =
  | { kind: "clean" }
  | { kind: "saving" }
  | { kind: "recoverable-error"; message: string }
  | { kind: "validation-failed"; message: string; issues: ValidationIssue[] };

export interface RuntimeDiagnostics {
  commands_processed: number;
  command_queue_full: number;
  subscriber_snapshots_overwritten: number;
  interrupt_dismissals_ignored: number;
  taps_dropped: number;
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

/**
 * `png_base64` is null exactly when the renderer produced no pixels; `state` then
 * carries the word for why. Built-in cards keep `png_base64` set and `state`
 * null; a picture explains that its source owns the pushed frame.
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

/**
 * Every structured failure the HTTP API can return, and nothing else.
 *
 * This union contains only categories emitted by the HTTP API. Listing a category
 * the server cannot produce would make `toApiError` accept a shape nothing sends
 * and invite a branch that can never run.
 */
export type ApiError =
  | MessageError<"invalid-payload">
  | (MessageError<"payload-too-large"> & { maximum_bytes: number })
  | (MessageError<"validation"> & { issues: ValidationIssue[] })
  | MessageError<"persistence">
  | MessageError<"runtime-unavailable">
  | MessageError<"not-found">
  | MessageError<"device">
  | MessageError<"internal">;

// This fixture shape is generated from Rust serialization in a server test, then
// compiled against these declarations. Either side changing makes CI fail.
export interface ApiContractFixtures {
  snapshot: AppSnapshot;
  configs: AppConfig[];
  card_settings: CardSettings[];
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
  pomodoro_states: PomodoroState[];
  card_data: CardDataSnapshot[];
  persistence_states: PersistenceState[];
  validation_codes: ValidationCode[];
  pomodoro_actions: PomodoroAction[];
  errors: ApiError[];
  draft_validation: DraftValidation;
  config_apply_result: ConfigApplyResult;
  preview_frame: PreviewFrame;
  device_rows: DeviceRow[];
  mint_source_responses: MintSourceResponse[];
  image_source_descriptors: ImageSourceDescriptor[];
  face_descriptors: FaceDescriptor[];
  card_error_kinds: CardError["kind"][];
  card_field_values: CardFieldValue[];
  store_warnings: StoreWarning[];
  device_tiers: DeviceTier[];
  device_wifi_states: DeviceWifiState[];
  device_ota_states: DeviceOtaState[];
  face_field_descriptors: FaceFieldDescriptor[];
}
