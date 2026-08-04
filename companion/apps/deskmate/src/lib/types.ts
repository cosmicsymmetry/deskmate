export interface AppConfig {
  schema_version: number;
  preferences: AppPreferences;
  widgets: WidgetSettings[];
  screens: ScreenSettings[];
}

export interface AppPreferences {
  timezone: string;
  autostart: boolean;
  paused: boolean;
}

export type WidgetSize = "full" | "standard";

export type WidgetSettings =
  | {
      kind: "clock";
      id: string;
      size: WidgetSize;
      title: string;
      show_seconds: boolean;
    }
  | {
      kind: "pomodoro";
      id: string;
      size: WidgetSize;
      label: string;
      duration_seconds: number;
    }
  | {
      kind: "calendar";
      id: string;
      size: WidgetSize;
      title: string;
      source: CalendarSource;
      refresh_minutes: number;
    };

export type CalendarSource =
  | { kind: "file"; value: string }
  | { kind: "url"; value: string };

export interface ScreenSettings {
  id: string;
  widget_id: string;
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
  | "invalid-source";

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
  uptime_ms: number | null;
  free_heap: number | null;
  rotation: number | null;
  active_screen_id: string | null;
  counters: DeviceCounters;
}

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
