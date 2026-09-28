use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::AppConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSnapshot {
    pub config: AppConfig,
    pub runtime: RuntimeState,
    pub device: DeviceSnapshot,
    pub pomodoros: Vec<PomodoroSnapshot>,
    pub card_data: Vec<CardDataSnapshot>,
    /// Cards whose last data push was refused, or whose complete scene could not be
    /// built or rendered exactly. Retrying an identical payload can only fail again,
    /// so the runtime records the typed actionable failure here instead of looping.
    /// See `CardError`.
    pub card_errors: Vec<CardError>,
    pub persistence: PersistenceState,
    pub diagnostics: RuntimeDiagnostics,
}

/// A card-scoped, user-actionable failure. Transport failures are never reported
/// here — those are connection state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardError {
    pub kind: CardErrorKind,
    pub card_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CardErrorKind {
    /// The display rejected a `PushTimer` for this card. The name is protocol v1's
    /// (`PushData`); it is kept because the serialized `data-refused` is part of
    /// the browser contract.
    DataRefused,
    /// The host could not build, or the display could not render, the complete scene.
    SceneRefused,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeDiagnostics {
    pub commands_processed: u64,
    pub command_queue_full: u64,
    pub subscriber_snapshots_overwritten: u64,
    /// `InterruptDismissed` events the host received but could not apply,
    /// because the arbiter no longer tracks the token they carry (or they
    /// carry none at all).
    ///
    /// Not an error on its own: the common cause is benign, a bounded alert
    /// hold expiring host-side and freeing the slot before the user got round
    /// to tapping the overlay the device is still showing. A nonzero value
    /// distinguishes a tap the host deliberately declined from an event that
    /// never arrived.
    pub interrupt_dismissals_ignored: u64,
    /// Taps the host received for a card it routed nowhere: the card id resolves
    /// to no card in the current config, or it names a picture and no card-tap
    /// sink is installed.
    ///
    /// Here for the same reason as the counter above, and learned the same way.
    /// A tap that reaches the host and goes nowhere looks exactly like a tap
    /// whose event never left the panel. On 2026-09-24 that was the question
    /// asked of `dev-0005` and it could not be answered: the counter existed and
    /// was being incremented, but it stopped at this crate's boundary instead of
    /// travelling in the snapshot with every other counter here.
    pub taps_dropped: u64,
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
    pub active_card_id: Option<String>,
    pub counters: DeviceCounters,
    #[serde(default)]
    pub asset_store: Option<DeviceAssetStore>,
}

/// What the device says about its **durable flash** asset store, from key 31 of
/// every `StatusResponse`. `None` means the device is not reporting it, which is
/// what a store that never formatted does.
///
/// These are flash numbers and not PSRAM ones, so they do not measure the
/// volatile tier a picture frame is headed for. What they answer is whether the
/// flash is still being written for frames at all: the durable store has never
/// held anything else, because every font the host emits is baked and a config
/// declares no assets. `used_bytes` holding still across refreshes is the
/// observable end of the wear this project has been paying.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAssetStore {
    pub used_bytes: u32,
    pub free_bytes: u32,
    pub asset_count: u32,
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
    AssetTransfer,
    FirmwareUpdate,
    Networking,
    SceneRender,
    VolatileAssets,
    DurableAssetEncoding,
}

impl DeviceCapability {
    const ALL: [Self; 6] = [
        Self::AssetTransfer,
        Self::FirmwareUpdate,
        Self::Networking,
        Self::SceneRender,
        Self::VolatileAssets,
        Self::DurableAssetEncoding,
    ];

    pub const fn bit(self) -> u64 {
        match self {
            Self::AssetTransfer => protocol::CAPABILITY_ASSET_TRANSFER,
            Self::FirmwareUpdate => protocol::CAPABILITY_FIRMWARE_UPDATE,
            Self::Networking => protocol::CAPABILITY_NETWORKING,
            Self::SceneRender => protocol::CAPABILITY_SCENE_RENDER,
            Self::VolatileAssets => protocol::CAPABILITY_VOLATILE_ASSETS,
            Self::DurableAssetEncoding => protocol::CAPABILITY_DURABLE_ASSET_ENCODING,
        }
    }

    /// A name a person can act on. Capability mismatches are reported to the settings
    /// UI, where a raw bitmask ("missing capability bits 0x0000000000000008") tells the
    /// user nothing about what to change or which firmware to install.
    pub const fn label(self) -> &'static str {
        match self {
            Self::AssetTransfer => "icon and font asset transfer",
            Self::FirmwareUpdate => "firmware update",
            Self::Networking => "networking",
            Self::SceneRender => "declarative scene rendering",
            Self::VolatileAssets => "volatile raster assets",
            Self::DurableAssetEncoding => "durable raster asset encoding",
        }
    }

    pub fn from_bits(bits: u64) -> Vec<Self> {
        Self::ALL
            .into_iter()
            .filter(|capability| bits & capability.bit() != 0)
            .collect()
    }

    pub const fn known_bits() -> u64 {
        let mut bits = 0;
        let mut index = 0;
        while index < Self::ALL.len() {
            bits |= Self::ALL[index].bit();
            index += 1;
        }
        bits
    }
}

#[cfg(test)]
mod device_capability_tests {
    use super::DeviceCapability;

    #[test]
    fn every_current_firmware_capability_has_a_host_name() {
        let unnamed_bits = protocol::CURRENT_CAPABILITIES & !DeviceCapability::known_bits();

        assert_eq!(
            unnamed_bits, 0,
            "current firmware advertises capability bits the host cannot name: {unnamed_bits:#018x}"
        );
    }

    #[test]
    fn scene_render_capability_has_a_person_facing_name_and_round_trips() {
        assert_eq!(
            DeviceCapability::SceneRender.bit(),
            protocol::CAPABILITY_SCENE_RENDER
        );
        assert_eq!(
            DeviceCapability::SceneRender.label(),
            "declarative scene rendering"
        );
        assert_eq!(
            DeviceCapability::from_bits(protocol::CAPABILITY_SCENE_RENDER),
            vec![DeviceCapability::SceneRender]
        );
    }

    #[test]
    fn volatile_assets_capability_has_a_name_bit_and_serde_contract() {
        assert_eq!(
            DeviceCapability::VolatileAssets.bit(),
            protocol::CAPABILITY_VOLATILE_ASSETS
        );
        assert_eq!(
            DeviceCapability::from_bits(protocol::CAPABILITY_VOLATILE_ASSETS),
            vec![DeviceCapability::VolatileAssets]
        );
        assert_eq!(
            serde_json::to_string(&DeviceCapability::VolatileAssets).unwrap(),
            "\"volatile-assets\""
        );
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
pub struct PomodoroSnapshot {
    pub card_id: String,
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

/// One live value a card's scene builder reads. Protocol v2 removed the wire's
/// generic field bag, so this is now purely a HOST-side type: it is what the
/// runtime keeps per card, what the settings UI renders, and what
/// `build_card_scene` reads.
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
    pub(crate) fn new(card_id: &str, fields: &[CardField]) -> Self {
        Self {
            card_id: card_id.to_owned(),
            fields: fields.to_vec(),
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
