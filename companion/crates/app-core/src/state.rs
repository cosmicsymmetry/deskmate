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
    /// Cards whose last data push the device understood and refused. Retrying an
    /// identical payload can only fail again, so the runtime drops it from the dirty
    /// set and records it here instead of looping. See `CardError`.
    pub card_errors: Vec<CardError>,
    pub persistence: PersistenceState,
    pub diagnostics: RuntimeDiagnostics,
}

/// A card-scoped, user-actionable failure: the device accepted the connection and
/// the frame, understood the push, and refused its contents (an undeclared field
/// type, an over-long text value, a value outside the template's declared range).
/// Transport failures are never reported here — those are connection state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardError {
    pub card_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeDiagnostics {
    pub commands_processed: u64,
    pub command_queue_full: u64,
    pub provider_jobs_started: u64,
    pub provider_queue_full: u64,
    pub provider_results_discarded: u64,
    pub subscriber_snapshots_overwritten: u64,
    /// `InterruptDismissed` events the host received but could not apply,
    /// because the arbiter no longer tracks the token they carry (or they
    /// carry none at all).
    ///
    /// Not an error on its own: the common cause is benign, a bounded alert
    /// hold expiring host-side and freeing the slot before the user got round
    /// to tapping the overlay the device is still showing. But it was
    /// previously invisible, and a hardware session spent time on a tap that
    /// looked like it did nothing. A nonzero value here says the host saw the
    /// tap and deliberately declined it, which is a different diagnosis from
    /// the event never arriving at all.
    pub interrupt_dismissals_ignored: u64,
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
    #[serde(default)]
    pub tier: Option<DeviceTier>,
    #[serde(default)]
    pub wifi_state: Option<DeviceWifiState>,
    #[serde(default)]
    pub wifi_rssi: Option<i8>,
    #[serde(default)]
    pub ip: Option<String>,
    #[serde(default)]
    pub last_network_error: Option<String>,
    #[serde(default)]
    pub ota_state: Option<DeviceOtaState>,
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
    Networking,
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
            Self::Networking => protocol::CAPABILITY_NETWORKING,
        }
    }

    /// A name a person can act on. Capability mismatches are reported to the settings
    /// UI, where a raw bitmask ("missing capability bits 0x0000000000000008") tells the
    /// user nothing about what to change or which firmware to install.
    pub const fn label(self) -> &'static str {
        match self {
            Self::CoreWidgets => "core widgets",
            Self::ConfigRotation => "display rotation",
            Self::DashboardLayouts => "dashboard layouts",
            Self::ExtendedTemplates => "extended display templates",
            Self::HostTapActions => "host tap actions",
            Self::AssetTransfer => "icon and font asset transfer",
            Self::FirmwareUpdate => "firmware update",
            Self::Networking => "networking",
        }
    }

    pub fn from_bits(bits: u64) -> Vec<Self> {
        const ALL: [DeviceCapability; 8] = [
            DeviceCapability::CoreWidgets,
            DeviceCapability::ConfigRotation,
            DeviceCapability::DashboardLayouts,
            DeviceCapability::ExtendedTemplates,
            DeviceCapability::HostTapActions,
            DeviceCapability::AssetTransfer,
            DeviceCapability::FirmwareUpdate,
            DeviceCapability::Networking,
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
            | protocol::CAPABILITY_NETWORKING
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceTier {
    Local,
    Networked,
}

impl From<protocol::Tier> for DeviceTier {
    fn from(tier: protocol::Tier) -> Self {
        match tier {
            protocol::Tier::Local => Self::Local,
            protocol::Tier::Networked => Self::Networked,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceWifiState {
    Down,
    Connecting,
    Connected,
    Failed,
}

impl From<protocol::WifiState> for DeviceWifiState {
    fn from(state: protocol::WifiState) -> Self {
        match state {
            protocol::WifiState::Down => Self::Down,
            protocol::WifiState::Connecting => Self::Connecting,
            protocol::WifiState::Connected => Self::Connected,
            protocol::WifiState::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceOtaState {
    Idle,
    Checking,
    Downloading,
    PendingVerify,
    Failed,
}

impl From<protocol::OtaState> for DeviceOtaState {
    fn from(state: protocol::OtaState) -> Self {
        match state {
            protocol::OtaState::Idle => Self::Idle,
            protocol::OtaState::Checking => Self::Checking,
            protocol::OtaState::Downloading => Self::Downloading,
            protocol::OtaState::PendingVerify => Self::PendingVerify,
            protocol::OtaState::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCounters {
    /// Reconnections performed by *this host process*, not by the device.
    ///
    /// Every other counter in this struct is reported by the device in its
    /// `StatusResponse`; this one is the host's own tally, incremented when the
    /// session re-establishes and replays. So it resets when the host restarts
    /// while the device keeps running — reading 0 beside a device that has
    /// plainly reconnected is correct, not a bug. It carries the `host_` prefix
    /// for the same reason `host_dropped_events` does: to say whose number it is
    /// in the JSON, where this comment is not visible.
    pub host_reconnects: u64,
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
    RecoverableError {
        message: String,
    },
    ValidationFailed {
        message: String,
        issues: Vec<crate::ValidationIssue>,
    },
}
