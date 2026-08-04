use serde::{Deserialize, Serialize};

use crate::AppConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSnapshot {
    pub config: AppConfig,
    pub runtime: RuntimeState,
    pub device: DeviceSnapshot,
    pub providers: Vec<ProviderSnapshot>,
    pub pomodoros: Vec<PomodoroSnapshot>,
    pub persistence: PersistenceState,
    pub diagnostics: RuntimeDiagnostics,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeDiagnostics {
    pub commands_processed: u64,
    pub command_queue_full: u64,
    pub provider_jobs_started: u64,
    pub provider_queue_full: u64,
    pub provider_results_discarded: u64,
    pub subscriber_snapshots_overwritten: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RuntimeState {
    Starting,
    Running,
    Paused,
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ConnectionState {
    Disconnected { reason: Option<String> },
    Connecting,
    Online,
    Standalone,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSnapshot {
    pub connection: ConnectionState,
    pub port_name: Option<String>,
    pub firmware_version: Option<String>,
    pub protocol_version: Option<u8>,
    pub uptime_ms: Option<u64>,
    pub free_heap: Option<u32>,
    pub rotation: Option<u16>,
    pub active_screen_id: Option<String>,
    pub counters: DeviceCounters,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCounters {
    pub reconnects: u64,
    pub valid_frames: u32,
    pub malformed_frames: u32,
    pub crc_errors: u32,
    pub overflow_frames: u32,
    pub dropped_responses: u32,
    pub rx_dropped_bytes: u32,
    pub dropped_events: u32,
    pub event_queue_high_water: u32,
    pub dropped_ui_commands: u32,
    pub ui_queue_high_water: u32,
    pub host_dropped_events: u64,
    pub detected_event_gaps: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSnapshot {
    pub widget_id: String,
    pub state: ProviderState,
    pub last_success_unix_ms: Option<i64>,
    pub age_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ProviderState {
    Idle,
    Refreshing,
    Fresh,
    Stale { message: String },
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PomodoroSnapshot {
    pub widget_id: String,
    pub state: PomodoroState,
    pub duration_seconds: u32,
    pub remaining_seconds: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PomodoroState {
    Idle,
    Running,
    Paused,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PersistenceState {
    Clean,
    Saving,
    RecoverableError { message: String },
}
