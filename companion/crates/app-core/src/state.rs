use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::AppConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSnapshot {
    pub config: AppConfig,
    pub runtime: RuntimeState,
    pub device: DeviceSnapshot,
    pub providers: Vec<ProviderSnapshot>,
    pub pomodoros: Vec<PomodoroSnapshot>,
    pub card_data: Vec<CardDataSnapshot>,
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
    pub max_protocol_version: Option<u8>,
    pub capabilities: Vec<DeviceCapability>,
    #[serde(
        serialize_with = "serialize_capability_bits",
        deserialize_with = "deserialize_capability_bits"
    )]
    pub unknown_capability_bits: u64,
    pub uptime_ms: Option<u64>,
    pub free_heap: Option<u32>,
    pub rotation: Option<u16>,
    pub active_screen_id: Option<String>,
    pub counters: DeviceCounters,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn serialize_capability_bits<S>(bits: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&format!("0x{bits:016x}"))
}

fn deserialize_capability_bits<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let digits = value.strip_prefix("0x").ok_or_else(|| {
        serde::de::Error::custom("capability bits must use a 0x-prefixed hexadecimal string")
    })?;
    if digits.len() != 16 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(serde::de::Error::custom(
            "capability bits must contain exactly 16 hexadecimal digits",
        ));
    }
    u64::from_str_radix(digits, 16).map_err(serde::de::Error::custom)
}

impl DeviceSnapshot {
    pub fn capability_bits(&self) -> u64 {
        self.capabilities
            .iter()
            .fold(0, |bits, capability| bits | capability.bit())
            | self.unknown_capability_bits
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceCapability {
    CoreWidgets,
    ConfigRotation,
    DashboardLayouts,
    ExtendedTemplates,
    HostTapActions,
    AssetTransfer,
    FirmwareUpdate,
}

impl DeviceCapability {
    pub const fn bit(self) -> u64 {
        match self {
            Self::CoreWidgets => protocol::CAPABILITY_CORE_WIDGETS,
            Self::ConfigRotation => protocol::CAPABILITY_CONFIG_ROTATION,
            Self::DashboardLayouts => protocol::CAPABILITY_DASHBOARD_LAYOUTS,
            Self::ExtendedTemplates => protocol::CAPABILITY_EXTENDED_TEMPLATES,
            Self::HostTapActions => protocol::CAPABILITY_HOST_TAP_ACTIONS,
            Self::AssetTransfer => protocol::CAPABILITY_ASSET_TRANSFER,
            Self::FirmwareUpdate => protocol::CAPABILITY_FIRMWARE_UPDATE,
        }
    }

    pub fn from_bits(bits: u64) -> Vec<Self> {
        const ALL: [DeviceCapability; 7] = [
            DeviceCapability::CoreWidgets,
            DeviceCapability::ConfigRotation,
            DeviceCapability::DashboardLayouts,
            DeviceCapability::ExtendedTemplates,
            DeviceCapability::HostTapActions,
            DeviceCapability::AssetTransfer,
            DeviceCapability::FirmwareUpdate,
        ];
        ALL.into_iter()
            .filter(|capability| bits & capability.bit() != 0)
            .collect()
    }

    pub const fn known_bits() -> u64 {
        protocol::CAPABILITY_CORE_WIDGETS
            | protocol::CAPABILITY_CONFIG_ROTATION
            | protocol::CAPABILITY_DASHBOARD_LAYOUTS
            | protocol::CAPABILITY_EXTENDED_TEMPLATES
            | protocol::CAPABILITY_HOST_TAP_ACTIONS
            | protocol::CAPABILITY_ASSET_TRANSFER
            | protocol::CAPABILITY_FIRMWARE_UPDATE
    }
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

/// Mirrors `protocol::FieldValue` for the settings webview. The wire type
/// deliberately does not derive `Serialize` (it must stay free of
/// presentation concerns), so this DTO carries the same last-good values
/// across the IPC boundary instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CardFieldValue {
    Text { value: String },
    Integer { value: i64 },
    Boolean { value: bool },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardField {
    pub key: String,
    pub value: CardFieldValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardDataSnapshot {
    pub card_id: String,
    pub fields: Vec<CardField>,
}

impl CardDataSnapshot {
    pub(crate) fn from_protocol(card_id: &str, fields: &[protocol::Field]) -> Self {
        Self {
            card_id: card_id.to_owned(),
            fields: fields
                .iter()
                .map(|field| CardField {
                    key: field.key.clone(),
                    value: match &field.value {
                        protocol::FieldValue::Text(value) => CardFieldValue::Text {
                            value: value.clone(),
                        },
                        protocol::FieldValue::Integer(value) => {
                            CardFieldValue::Integer { value: *value }
                        }
                        protocol::FieldValue::Boolean(value) => {
                            CardFieldValue::Boolean { value: *value }
                        }
                    },
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PersistenceState {
    Clean,
    Saving,
    RecoverableError { message: String },
}
