// Tauri injects command state and app handles by value; these signatures are part of
// its command macro contract even when the handler only borrows them internally.
#![allow(clippy::needless_pass_by_value)]

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use app_core::{
    AdminConfigErrorBody, AppConfig, CardField, CardFieldValue, CardSettings, ConfigStore,
    DisplayOrientation, MAX_CARD_ID_LEN, MAX_CONFIG_FILE_BYTES, MAX_DEVICE_ID_LEN,
    MAX_DEVICE_TOKEN_LEN, MAX_PSK_LEN, MAX_SERVER_URL_LEN, MAX_SSID_LEN, NetworkConfig,
    NetworkSettings, NetworkSettingsStore, NetworkSettingsStoreError, NetworkSettingsUpdate,
    PomodoroAction, ProvisioningTier, RuntimeError, RuntimeHandle, SaveReceipt, StoreError,
    ValidationIssue, utc_offset_minutes,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

use crate::{DesktopSnapshot, DesktopState, NetworkedConfigProjection};

pub(crate) const MAX_SERVER_ERROR_BYTES: usize = 64 * 1_024;
const MAX_IMAGE_SOURCE_RESPONSE_BYTES: usize = 64 * 1_024;
const MAX_FACE_FIELDS: usize = 16;
const MAX_FACE_FIELD_KEY_BYTES: usize = 64;
const MAX_FACE_FIELD_VALUE_BYTES: usize = 2_048;

mod config;
mod network;
mod preview;

pub use config::*;
pub use network::*;
pub use preview::*;

/// The draft travels as a bounded JSON envelope so an IPC caller cannot make serde
/// allocate an arbitrarily deep application document before domain validation runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftPayload {
    pub json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardTarget {
    pub card_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftValidation {
    pub valid: bool,
    pub issues: Vec<ValidationIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigApplyResult {
    pub save: SaveReceipt,
}

#[derive(Clone)]
struct ProvisionContext {
    runtime: Arc<RuntimeHandle>,
    network_store: Arc<NetworkSettingsStore>,
    networked_config: Arc<NetworkedConfigProjection>,
}

impl ProvisionContext {
    fn from_desktop(state: &DesktopState) -> Self {
        Self {
            runtime: Arc::clone(&state.runtime),
            network_store: Arc::clone(&state.network_store),
            networked_config: Arc::clone(&state.networked_config),
        }
    }
}

/// Everything a read-only server call needs, cloned out of `DesktopState` so the
/// blocking half can move onto a worker thread. No config, no runtime, no locks:
/// these calls never mutate anything the desktop owns.
#[derive(Clone)]
struct ServerQueryContext {
    agent: ureq::Agent,
    network_store: Arc<NetworkSettingsStore>,
}

impl ServerQueryContext {
    fn from_desktop(state: &DesktopState) -> Self {
        Self {
            agent: state.server_client.clone(),
            network_store: Arc::clone(&state.network_store),
        }
    }

    /// The store lends the token for one call and never returns it; a missing token
    /// is a typed instruction, not a transport failure.
    fn with_admin_token<T>(
        &self,
        operation: impl FnOnce(&str) -> Result<T, IpcError>,
    ) -> Result<T, IpcError> {
        self.network_store
            .with_admin_token(operation)
            .map_err(IpcError::from)?
            .ok_or_else(|| IpcError::InvalidPayload {
                message: "Enter the admin token in Network setup before reading server state."
                    .into(),
            })?
    }
}

struct ServerSaveContext {
    config: ConfigSaveContext,
    server_client: ureq::Agent,
}

/// Secret-bearing IPC inputs intentionally implement neither `Debug` nor `Serialize`.
/// Their only outbound projection is `NetworkSettings`, which contains public fields.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerEndpointRequest {
    server_url: String,
    device_id: String,
    admin_token: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvisionDeviceRequest {
    ssid: String,
    passphrase: String,
    server_url: String,
    device_id: String,
    device_token: String,
    tier: app_core::DeviceTier,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfigRequest {
    draft: DraftPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutostartStatus {
    pub enabled: bool,
    pub preference_enabled: bool,
}

/// A rendered card preview. `png_base64` is `None` when there is nothing to draw
/// and `state` then names why, in the server's own words: the stage prints that
/// sentence rather than "Preview unavailable", which stays reserved for a real
/// transport failure. Built-in cards always set `png_base64` and never `state`.
///
/// `sample` is set when the card has never published data (the runtime holds no
/// `CardDataSnapshot` for it): the request still renders, with an empty field set,
/// so the image is the firmware's own unconfigured appearance for that template
/// rather than an invented placeholder. It therefore only ever accompanies a real
/// frame -- a card with no frame is not a sample of anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewFrame {
    pub png_base64: Option<String>,
    pub sample: bool,
    pub state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "category", rename_all = "kebab-case")]
pub enum IpcError {
    InvalidPayload {
        message: String,
    },
    PayloadTooLarge {
        message: String,
        maximum_bytes: usize,
    },
    Validation {
        message: String,
        issues: Vec<ValidationIssue>,
    },
    Persistence {
        message: String,
    },
    RuntimeBusy {
        message: String,
    },
    RuntimeUnavailable {
        message: String,
    },
    /// The server was reached, answered, and said something this app cannot read.
    ///
    /// Distinct from `RuntimeUnavailable` because the difference is the whole
    /// message: "couldn't reach the server" is simply false here. The reachable
    /// cause is the documented server-then-Mac rollout -- a server built before
    /// this app answers an additive route without the keys this app's DTOs
    /// require -- so the window's sentence for it means "update the server",
    /// never "check your network".
    IncompatibleServer {
        message: String,
    },
    NotFound {
        message: String,
    },
    Device {
        message: String,
    },
    Autostart {
        message: String,
    },
    Window {
        message: String,
    },
    Internal {
        message: String,
    },
    /// A typed, visible refusal for input this build genuinely cannot act on yet --
    /// mirrors runtime.rs's `CardErrorKind::SceneRefused` for the same reason: say so
    /// explicitly rather than pretending to render something, or miscategorizing the
    /// refusal as an unexpected internal error.
    Unsupported {
        message: String,
    },
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPayload { message }
            | Self::PayloadTooLarge { message, .. }
            | Self::Validation { message, .. }
            | Self::Persistence { message }
            | Self::RuntimeBusy { message }
            | Self::RuntimeUnavailable { message }
            | Self::IncompatibleServer { message }
            | Self::NotFound { message }
            | Self::Device { message }
            | Self::Autostart { message }
            | Self::Window { message }
            | Self::Internal { message }
            | Self::Unsupported { message } => formatter.write_str(message),
        }
    }
}

impl std::error::Error for IpcError {}

impl IpcError {
    pub(crate) const fn log_label(&self) -> &'static str {
        match self {
            Self::InvalidPayload { .. } => "invalid-payload",
            Self::PayloadTooLarge { .. } => "payload-too-large",
            Self::Validation { .. } => "validation",
            Self::Persistence { .. } => "persistence",
            Self::RuntimeBusy { .. } => "runtime-busy",
            Self::RuntimeUnavailable { .. } => "runtime-unavailable",
            Self::IncompatibleServer { .. } => "incompatible-server",
            Self::NotFound { .. } => "not-found",
            Self::Device { .. } => "device",
            Self::Autostart { .. } => "autostart",
            Self::Window { .. } => "window",
            Self::Internal { .. } => "internal",
            Self::Unsupported { .. } => "unsupported",
        }
    }

    fn map_message(mut self, map: impl FnOnce(String) -> String) -> Self {
        let message = match &mut self {
            Self::InvalidPayload { message }
            | Self::PayloadTooLarge { message, .. }
            | Self::Validation { message, .. }
            | Self::Persistence { message }
            | Self::RuntimeBusy { message }
            | Self::RuntimeUnavailable { message }
            | Self::IncompatibleServer { message }
            | Self::NotFound { message }
            | Self::Device { message }
            | Self::Autostart { message }
            | Self::Window { message }
            | Self::Internal { message }
            | Self::Unsupported { message } => message,
        };
        *message = map(std::mem::take(message));
        self
    }
}

#[tauri::command]
pub fn get_network_settings(state: State<'_, DesktopState>) -> NetworkSettings {
    state.network_store.load().settings().clone()
}

#[tauri::command]
pub fn resume_pushing(state: State<'_, DesktopState>) -> Result<(), IpcError> {
    set_paused(&state, false)
}

#[tauri::command]
pub fn control_pomodoro(
    state: State<'_, DesktopState>,
    target: CardTarget,
    action: PomodoroAction,
) -> Result<(), IpcError> {
    validate_target(&target.card_id, MAX_CARD_ID_LEN, "card ID")?;
    state
        .runtime
        .control_pomodoro(target.card_id, action)
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn get_autostart_status(
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<AutostartStatus, IpcError> {
    autostart_status(&app, &state)
}

#[tauri::command]
pub fn set_autostart_enabled(
    app: AppHandle,
    state: State<'_, DesktopState>,
    enabled: bool,
) -> Result<AutostartStatus, IpcError> {
    set_autostart(&app, &state, enabled)
}

fn validate_bounded(value: &str, maximum: usize, label: &str) -> Result<(), IpcError> {
    if value.len() > maximum {
        return Err(IpcError::InvalidPayload {
            message: format!("{label} must be at most {maximum} UTF-8 bytes"),
        });
    }
    Ok(())
}

fn validate_secret(value: &str, maximum: usize, label: &str) -> Result<(), IpcError> {
    if value.is_empty() {
        return Err(IpcError::InvalidPayload {
            message: format!("{label} must not be empty"),
        });
    }
    if value.contains(['\r', '\n']) {
        return Err(IpcError::InvalidPayload {
            message: format!("{label} contains an invalid line break"),
        });
    }
    validate_bounded(value, maximum, label)
}

pub(crate) fn validate_server_url(value: &str) -> Result<url::Url, IpcError> {
    validate_target(value, MAX_SERVER_URL_LEN, "server URL")?;
    let url = url::Url::parse(value).map_err(|_| IpcError::InvalidPayload {
        message: "server URL must be an absolute HTTP or HTTPS URL".into(),
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(IpcError::InvalidPayload {
            message: "server URL must be an absolute HTTP or HTTPS URL without credentials".into(),
        });
    }
    Ok(url)
}

fn validate_target(value: &str, maximum: usize, label: &str) -> Result<(), IpcError> {
    if value.trim().is_empty() {
        return Err(IpcError::InvalidPayload {
            message: format!("{label} must not be empty"),
        });
    }
    if value.len() > maximum {
        return Err(IpcError::InvalidPayload {
            message: format!("{label} must be at most {maximum} UTF-8 bytes"),
        });
    }
    Ok(())
}

pub(crate) fn autostart_error(error: impl std::fmt::Display) -> IpcError {
    IpcError::Autostart {
        message: format!("cannot update start-at-login: {error}"),
    }
}

impl From<RuntimeError> for IpcError {
    fn from(error: RuntimeError) -> Self {
        match error {
            RuntimeError::InvalidConfig { issues } => Self::Validation {
                message: format!("configuration has {} validation issue(s)", issues.len()),
                issues,
            },
            RuntimeError::QueueFull => Self::RuntimeBusy {
                message: "the background runtime is busy; try again".into(),
            },
            RuntimeError::WorkerStopped | RuntimeError::ResponseTimeout => {
                Self::RuntimeUnavailable {
                    message: error.to_string(),
                }
            }
            RuntimeError::UnknownCard { .. } => Self::NotFound {
                message: error.to_string(),
            },
            RuntimeError::DeviceDisconnected => Self::Device {
                message: "device is disconnected".into(),
            },
            RuntimeError::Device { message } => Self::Device { message },
            RuntimeError::ImageSource { message } => Self::RuntimeUnavailable { message },
        }
    }
}

impl From<StoreError> for IpcError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Validation { issues } => Self::Validation {
                message: format!("configuration has {} validation issue(s)", issues.len()),
                issues,
            },
            other => Self::Persistence {
                message: other.to_string(),
            },
        }
    }
}

impl From<NetworkSettingsStoreError> for IpcError {
    fn from(error: NetworkSettingsStoreError) -> Self {
        Self::Persistence {
            message: error.to_string(),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A private directory named for the test that owns it, so parallel runs of these
    /// store-backed tests cannot collide on one path.
    fn scratch_directory(label: &str) -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};

        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("deskmate-{label}-{}-{serial}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        directory
    }
    use std::fs;
    use std::path::Path;

    use app_core::{
        AlertHold, AppConfig, AppPreferences, AppSnapshot, AssetKind, AssetSource,
        CURRENT_SCHEMA_VERSION, CardAlert, CardDataSnapshot, CardError, CardErrorKind, CardField,
        CardFieldValue, CardSettings, CarouselAdvance, ConnectionState, DeviceCapability,
        DeviceCounters, DeviceSnapshot, DisplayOrientation, DisplayTemplate, IconGlyphMapping,
        PersistenceState, PomodoroSnapshot, PomodoroState, RefreshPolicy, RuntimeDiagnostics,
        RuntimeError, RuntimeState, SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE, StoreWarning,
        UpdateChannel, UpdateCheckPolicy, UpdaterSettings, ValidationCode, WidgetTapAction,
    };
    use serde::Serialize;

    pub(crate) fn read_http_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
        use std::io::Read as _;

        let mut request = Vec::new();
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let mut chunk = [0_u8; 1_024];
            let length = stream.read(&mut chunk).unwrap();
            assert_ne!(length, 0, "request ended before its HTTP headers");
            request.extend_from_slice(&chunk[..length]);
        }
        let header_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        while request.len() < header_end + content_length {
            let mut chunk = [0_u8; 1_024];
            let length = stream.read(&mut chunk).unwrap();
            assert_ne!(length, 0, "request ended before its HTTP body");
            request.extend_from_slice(&chunk[..length]);
        }
        request
    }

    /// The preview shows what a person standing at the panel sees, which is upright
    /// at BOTH mountings -- the 180 degree mount is cancelled by the mounting itself.
    /// Restoring the pass-through this replaced (returning `LandscapeFlipped` for the
    /// flipped mounting) puts an upside-down clock in the settings window, and fails
    /// here.
    /// The preview's timer comes from the same three pushed keys
    /// `firmware/main/link/protocol_task.c` reads to build the device's own
    /// snapshot, so a pomodoro face is posed identically in both places.
    #[test]
    fn preview_timer_is_derived_from_the_keys_the_device_reads() {
        let integer = |key: &str, value: i64| CardField {
            key: key.to_owned(),
            value: CardFieldValue::Integer { value },
        };

        assert_eq!(preview_timer(&[]), None, "no timer fields means no timer");

        let timer = preview_timer(&[
            integer("duration_seconds", 1_500),
            integer("remaining_seconds", 300),
            CardField {
                key: "running".into(),
                value: CardFieldValue::Boolean { value: true },
            },
        ])
        .expect("a pomodoro pushes a timer");
        assert_eq!(timer.total_ms, 1_500_000);
        assert_eq!(timer.remaining_ms, 300_000);
        assert!(timer.running);

        // remaining_seconds absent means "not started", i.e. a full timer,
        // never a zero one -- a zero would draw a finished ring.
        let unstarted =
            preview_timer(&[integer("duration_seconds", 60)]).expect("duration alone is a timer");
        assert_eq!(unstarted.remaining_ms, 60_000);
        assert!(!unstarted.running);

        // A negative value cannot reach the wire, but must not wrap if it does.
        let negative = preview_timer(&[integer("duration_seconds", -5)]).expect("still a timer");
        assert_eq!(negative.total_ms, 0);
    }

    #[test]
    fn the_preview_renders_upright_whichever_way_the_panel_is_mounted() {
        for configured in [
            DisplayOrientation::Landscape,
            DisplayOrientation::LandscapeFlipped,
        ] {
            assert_eq!(
                preview_orientation(configured),
                lvgl_sim::SimOrientation::Landscape,
                "preview orientation for {configured:?} must be upright"
            );
        }
    }

    #[test]
    fn draft_parser_bounds_and_strictly_decodes_the_envelope() {
        let default_json = serde_json::to_string(&AppConfig::default()).unwrap();
        assert_eq!(
            parse_valid_draft(&DraftPayload { json: default_json }).unwrap(),
            AppConfig::default()
        );

        let oversized = DraftPayload {
            json: "x".repeat(MAX_CONFIG_FILE_BYTES + 1),
        };
        assert!(matches!(
            parse_draft(&oversized),
            Err(IpcError::PayloadTooLarge { maximum_bytes, .. })
                if maximum_bytes == MAX_CONFIG_FILE_BYTES
        ));

        let unknown_field = DraftPayload {
            json: r#"{"schema_version":1,"preferences":{"timezone":"UTC","autostart":false,"paused":false},"widgets":[],"screens":[],"unexpected":true}"#.into(),
        };
        assert!(matches!(
            parse_draft(&unknown_field),
            Err(IpcError::InvalidPayload { .. })
        ));
    }

    /// A device snapshot with no connection, so draft validation exercises only the
    /// configuration's own issues.
    fn offline_device() -> AppSnapshot {
        contract_fixtures().snapshot.app
    }

    #[test]
    fn validation_is_non_mutating_and_returns_stable_issues() {
        let mut invalid = AppConfig::default();
        invalid.preferences.timezone = "Not/AZone".into();
        let result = validate_draft_for_device(
            &DraftPayload {
                json: serde_json::to_string(&invalid).unwrap(),
            },
            &offline_device().device,
        )
        .unwrap();
        assert!(!result.valid);
        assert_eq!(result.issues.len(), 1);
        assert_eq!(result.issues[0].code, ValidationCode::InvalidTimezone);
    }

    /// A draft can pass `validate()` cleanly (every field within its bounds) yet still
    /// be unsaveable because one of its fields has no `wire_config()` mapping. Before
    /// this fix, `validate_config_draft` ran only `validate()` and reported such a
    /// draft valid, so Save's separate `compile()` call was the first place the
    /// failure ever surfaced — with an error attached to no field, in a session where
    /// every other edit was now blocked too. `validate_config_draft` must catch this
    /// itself, on the card's own path, exactly like the save path does.
    ///
    /// `template` no longer fits this shape — `wire_config()` now lowers every
    /// `DisplayTemplate` (M4 task 3/8) — so this uses `tap_action: Dismiss`, which
    /// `wire_config()` still cannot lower (host tap actions are a later capability),
    /// to keep exercising the same "validates but cannot compile" gap.
    #[test]
    fn draft_validation_catches_a_field_with_no_wire_mapping_like_save_does() {
        let mut config = AppConfig::default();
        config.cards[0] = CardSettings::Clock {
            id: "clock".into(),
            title: "Desk".into(),
            show_seconds: true,
            template: DisplayTemplate::DigitalClock,
            tap_action: WidgetTapAction::Dismiss,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
            dwell_seconds: None,
        };
        assert!(config.validate().is_ok(), "fixture must validate cleanly");

        let result = validate_draft_for_device(
            &DraftPayload {
                json: serde_json::to_string(&config).unwrap(),
            },
            &offline_device().device,
        )
        .unwrap();

        assert!(!result.valid);
        assert!(result.issues.iter().any(|issue| {
            issue.path == "cards[0]" && issue.code == ValidationCode::RequiresCapability
        }));
    }

    /// Final-review finding: `validate_config_draft` compiled the draft but never saw
    /// the connected device, so a configuration the attached firmware cannot render
    /// read as valid until Save's `ensure_device_compatibility` refused it. Draft
    /// validation must report the same issue Save would, on the same path.
    #[test]
    fn draft_validation_reports_the_capability_gap_save_would_reject() {
        // Asset transfer is the one capability a configuration can still
        // require of a device: protocol v2 retired every bit that described
        // rendering a template.
        let config = AppConfig {
            assets: vec![app_core::AssetSettings {
                id: "face".into(),
                source: app_core::AssetSource::File("/tmp/face.ttf".into()),
                kind: app_core::AssetKind::Font,
                maximum_bytes: 1_024,
            }],
            ..AppConfig::default()
        };
        let draft = DraftPayload {
            json: serde_json::to_string(&config).unwrap(),
        };
        let required = config.compile(1).unwrap().required_capabilities;
        assert_ne!(
            required & DeviceCapability::AssetTransfer.bit(),
            0,
            "fixture must need the asset-transfer capability"
        );

        // Offline: nothing to gate against, so the draft is reported as valid.
        let mut device = offline_device().device;
        assert!(validate_draft_for_device(&draft, &device).unwrap().valid);

        // Online, but on firmware without the capability: the same issue Save raises.
        device.connection = ConnectionState::Online;
        device.protocol_version = Some(2);
        device.capabilities = vec![DeviceCapability::SceneRender];
        device.unknown_capability_bits = 0;

        let result = validate_draft_for_device(&draft, &device).unwrap();
        assert!(!result.valid);
        assert_eq!(result.issues.len(), 1);
        assert_eq!(result.issues[0].path, "device.capabilities");
        assert_eq!(result.issues[0].code, ValidationCode::RequiresCapability);
        let IpcError::Validation {
            issues: save_issues,
            ..
        } = ensure_device_compatibility(&device, required).unwrap_err()
        else {
            panic!("save must reject an incompatible configuration as a validation error");
        };
        assert_eq!(
            result.issues, save_issues,
            "draft validation and save must report the identical issue"
        );
    }

    /// The capability gap is reported by name, not as a bitmask: "missing capability
    /// bits 0x0000000000000008" told the user nothing they could act on.
    #[test]
    fn capability_gaps_are_named_in_plain_language() {
        assert_eq!(
            describe_capabilities(DeviceCapability::AssetTransfer.bit()),
            describe_capabilities(DeviceCapability::AssetTransfer.bit())
        );
        assert_eq!(
            describe_capabilities(
                DeviceCapability::SceneRender.bit() | DeviceCapability::AssetTransfer.bit()
            ),
            "icon and font asset transfer and declarative scene rendering"
        );
        assert_eq!(
            describe_capabilities(DeviceCapability::SceneRender.bit() | 1 << 63),
            "declarative scene rendering and an unrecognized device feature"
        );

        let mut device = offline_device().device;
        device.connection = ConnectionState::Online;
        device.protocol_version = Some(2);
        device.capabilities = vec![DeviceCapability::SceneRender];
        device.unknown_capability_bits = 0;
        let issues = missing_capability_issues(
            &device,
            DeviceCapability::SceneRender.bit() | DeviceCapability::AssetTransfer.bit(),
        );
        assert!(
            issues[0].message.contains("asset transfer"),
            "got {:?}",
            issues[0].message
        );
        assert!(
            !issues[0].message.contains("0x"),
            "no bitmask may reach the user: {:?}",
            issues[0].message
        );
    }

    #[test]
    fn action_targets_are_bounded_before_runtime_dispatch() {
        assert!(validate_target("clock", MAX_CARD_ID_LEN, "card ID").is_ok());
        assert!(matches!(
            validate_target(" ", MAX_CARD_ID_LEN, "card ID"),
            Err(IpcError::InvalidPayload { .. })
        ));
        assert!(matches!(
            validate_target(&"x".repeat(MAX_CARD_ID_LEN + 1), MAX_CARD_ID_LEN, "card ID"),
            Err(IpcError::InvalidPayload { .. })
        ));
    }

    #[test]
    fn runtime_errors_map_to_stable_ipc_categories() {
        assert!(matches!(
            IpcError::from(RuntimeError::QueueFull),
            IpcError::RuntimeBusy { .. }
        ));
        assert!(matches!(
            IpcError::from(RuntimeError::WorkerStopped),
            IpcError::RuntimeUnavailable { .. }
        ));
        assert!(matches!(
            IpcError::from(RuntimeError::UnknownCard {
                card_id: "missing".into()
            }),
            IpcError::NotFound { .. }
        ));
        assert!(matches!(
            IpcError::from(RuntimeError::ImageSource {
                message: "offline".into()
            }),
            IpcError::RuntimeUnavailable { .. }
        ));
        assert!(matches!(
            IpcError::from(RuntimeError::DeviceDisconnected),
            IpcError::Device { .. }
        ));
    }

    #[test]
    fn stale_drafts_cannot_overwrite_command_owned_preferences() {
        let mut draft = AppConfig::default();
        draft.preferences.timezone = "Asia/Tbilisi".into();
        let mut current = AppConfig::default();
        current.preferences.paused = true;
        current.preferences.autostart = true;

        merge_command_owned_preferences(&mut draft, &current);

        assert_eq!(draft.preferences.timezone, "Asia/Tbilisi");
        assert!(draft.preferences.paused);
        assert!(draft.preferences.autostart);
    }

    #[test]
    fn server_config_url_preserves_the_base_path_and_encodes_the_device_id() {
        let url = server_config_url("https://desk.example/base/?discard=yes", "desk one").unwrap();
        assert_eq!(
            url.as_str(),
            "https://desk.example/base/v1/devices/desk%20one/config"
        );
    }

    #[test]
    fn operator_server_base_derives_the_secure_device_link_url() {
        assert_eq!(
            device_link_url("https://desk.example/base?discard=yes#fragment").unwrap(),
            "wss://desk.example/v1/device/link"
        );
        assert_eq!(
            device_link_url("https://[2001:db8::1]:8443").unwrap(),
            "wss://[2001:db8::1]:8443/v1/device/link"
        );
        let insecure = device_link_url("http://desk.example").unwrap_err();
        assert!(matches!(insecure, IpcError::InvalidPayload { .. }));
        assert!(insecure.to_string().contains("HTTPS"));
    }

    #[test]
    fn save_destination_follows_live_persisted_and_legacy_ownership() {
        use app_core::DeviceTier::{Local, Networked};

        let settings = |server_url: &str, device_id: &str, tier| NetworkSettings {
            server_url: server_url.into(),
            device_id: device_id.into(),
            tier,
        };
        let cases = [
            (
                Some(Networked),
                settings("", "", Some(Local)),
                SaveDestination::Server,
            ),
            (
                Some(Local),
                settings("https://desk.example", "desk-1", Some(Networked)),
                SaveDestination::Local,
            ),
            (
                None,
                settings("", "", Some(Networked)),
                SaveDestination::Server,
            ),
            (
                None,
                settings("https://desk.example", "desk-1", Some(Networked)),
                SaveDestination::Server,
            ),
            (None, settings("", "", Some(Local)), SaveDestination::Local),
            (
                None,
                settings("https://desk.example", "desk-1", Some(Local)),
                SaveDestination::Local,
            ),
            (None, settings("", "", None), SaveDestination::Local),
            (
                None,
                settings("https://desk.example", "", None),
                SaveDestination::Server,
            ),
            (None, settings("", "desk-1", None), SaveDestination::Server),
            (
                None,
                settings("https://desk.example", "desk-1", None),
                SaveDestination::Server,
            ),
        ];

        for (live_tier, settings, expected) in cases {
            assert_eq!(save_destination(live_tier, &settings), expected);
        }
    }

    /// A Mac that has only ever *observed* a networked board knows its tier but not
    /// its id: `remember_device_tier` persists the tier alone. Pairing cannot supply
    /// the id either, because that needs a plaintext device token the server keeps
    /// only as a digest. Saving server access is therefore the one path left, so the
    /// id typed beside the URL has to survive it.
    #[test]
    fn saving_server_access_persists_the_typed_device_id() {
        let directory = scratch_directory("server-access-device-id");
        let network_store = NetworkSettingsStore::new(directory.join("network-settings.json"));
        network_store
            .save(NetworkSettingsUpdate::new(
                "",
                "",
                Some(app_core::DeviceTier::Networked),
                None,
            ))
            .unwrap();

        let settings = set_server_endpoint_with_context(
            &network_store,
            ServerEndpointRequest {
                server_url: "https://desk.example".into(),
                device_id: "dev-0003".into(),
                admin_token: "admin-secret".into(),
            },
        )
        .unwrap();

        assert_eq!(settings.device_id, "dev-0003");
        assert_eq!(
            NetworkSettingsStore::new(directory.join("network-settings.json"))
                .load()
                .settings()
                .device_id,
            "dev-0003",
            "the typed device id never reached the settings file"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    /// Saving an endpoint before a device is paired is a supported flow -- it is what
    /// `use_local_ownership` exists to recover from -- so a blank box must mean "leave
    /// the id alone" rather than "erase the id I already have".
    #[test]
    fn saving_server_access_keeps_the_stored_device_id_when_the_box_is_blank() {
        let directory = scratch_directory("server-access-blank-device-id");
        let network_store = NetworkSettingsStore::new(directory.join("network-settings.json"));
        network_store
            .save(NetworkSettingsUpdate::new(
                "https://old.example",
                "dev-0003",
                Some(app_core::DeviceTier::Networked),
                None,
            ))
            .unwrap();

        let settings = set_server_endpoint_with_context(
            &network_store,
            ServerEndpointRequest {
                server_url: "https://desk.example".into(),
                device_id: "   ".into(),
                admin_token: "admin-secret".into(),
            },
        )
        .expect("a blank device id must not fail the save");

        assert_eq!(settings.device_id, "dev-0003");
        assert_eq!(settings.server_url, "https://desk.example");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn actual_local_save_refuses_persisted_network_ownership_before_writing() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "deskmate-local-ownership-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let config_path = directory.join("config.json");
        let network_store = Arc::new(NetworkSettingsStore::new(
            directory.join("network-settings.json"),
        ));
        network_store
            .save(NetworkSettingsUpdate::new(
                "https://desk.example",
                "desk-1",
                Some(app_core::DeviceTier::Networked),
                Some("admin-secret".into()),
            ))
            .unwrap();
        let runtime = Arc::new(
            RuntimeHandle::start_serial(
                AppConfig::default(),
                Some("/definitely/not/a/deskmate-test-port".into()),
            )
            .unwrap(),
        );
        let context = ConfigSaveContext {
            runtime: Arc::clone(&runtime),
            store: Arc::new(ConfigStore::new(&config_path)),
            network_store,
            networked_config: Arc::new(NetworkedConfigProjection::default()),
            has_saved_config: Arc::new(AtomicBool::new(false)),
            mutation_lock: Arc::new(Mutex::new(())),
        };

        let result = save_and_apply(context, AppConfig::default());
        runtime.shutdown().unwrap();

        assert!(matches!(result, Err(IpcError::InvalidPayload { .. })));
        assert!(
            !config_path.exists(),
            "the refused local draft reached ConfigStore"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_provision_does_not_record_an_ownership_transfer() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "deskmate-failed-provision-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let network_store = Arc::new(NetworkSettingsStore::new(
            directory.join("network-settings.json"),
        ));
        network_store
            .save(NetworkSettingsUpdate::new(
                "https://old.example",
                "old-device",
                Some(app_core::DeviceTier::Local),
                None,
            ))
            .unwrap();
        let runtime = Arc::new(
            RuntimeHandle::start_serial(
                AppConfig::default(),
                Some("/definitely/not/a/deskmate-test-port".into()),
            )
            .unwrap(),
        );
        let context = ProvisionContext {
            runtime: Arc::clone(&runtime),
            network_store: Arc::clone(&network_store),
            networked_config: Arc::new(NetworkedConfigProjection::default()),
        };
        let request = ProvisionDeviceRequest {
            ssid: "home-network".into(),
            passphrase: "wifi-secret".into(),
            server_url: "https://desk.example".into(),
            device_id: "desk-1".into(),
            device_token: "device-secret".into(),
            tier: app_core::DeviceTier::Networked,
        };

        let result = provision_device_with_context(context, request);
        runtime.shutdown().unwrap();

        assert!(matches!(result, Err(IpcError::Device { .. })));
        assert_eq!(
            network_store.load().settings(),
            &NetworkSettings {
                server_url: "https://old.example".into(),
                device_id: "old-device".into(),
                tier: Some(app_core::DeviceTier::Local),
            },
            "a rejected device command changed the saved ownership"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn local_override_recovers_saved_routing_without_a_device_command() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "deskmate-local-override-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let network_store = NetworkSettingsStore::new(directory.join("network-settings.json"));
        network_store
            .save(NetworkSettingsUpdate::new(
                "https://desk.example",
                "desk-1",
                Some(app_core::DeviceTier::Networked),
                Some("admin-secret".into()),
            ))
            .unwrap();
        let projection = NetworkedConfigProjection::default();
        let mut stale_server_config = AppConfig::default();
        stale_server_config.preferences.timezone = "Asia/Tbilisi".into();
        projection.replace(Some(stale_server_config)).unwrap();

        let settings = use_local_ownership_with_context(&network_store, &projection).unwrap();

        assert_eq!(settings.tier, Some(app_core::DeviceTier::Local));
        assert_eq!(
            network_store.load().settings().tier,
            Some(app_core::DeviceTier::Local)
        );
        let mut projected = AppConfig::default();
        projection.project(app_core::DeviceTier::Networked, &mut projected);
        assert_eq!(projected, AppConfig::default());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn incomplete_server_identity_refusal_does_not_claim_server_ownership() {
        let settings = NetworkSettings {
            server_url: "https://desk.example".into(),
            device_id: String::new(),
            tier: None,
        };

        assert_eq!(
            local_save_refusal(&settings),
            "a server endpoint is configured, but no device is paired; enter a device ID or connect over USB to confirm ownership"
        );
    }

    #[test]
    fn a_remote_failure_after_mirroring_names_both_destination_results() {
        let error = server_error_after_local_save(IpcError::Validation {
            message: "the server rejected this configuration".into(),
            issues: vec![ValidationIssue {
                path: "cards[0].title".into(),
                code: ValidationCode::Empty,
                message: "Choose a title.".into(),
            }],
        });

        assert!(matches!(
            error,
            IpcError::Validation { message, issues }
                if message.contains("saved on this Mac")
                    && message.contains("server destination did not succeed")
                    && issues.len() == 1
        ));
    }

    #[test]
    fn server_request_runs_after_the_mutation_lock_is_released() {
        use std::io::Write;
        use std::net::TcpListener;
        use std::time::{SystemTime, UNIX_EPOCH};

        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "deskmate-server-lock-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let config_path = directory.join("config.json");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let network_store = Arc::new(NetworkSettingsStore::new(
            directory.join("network-settings.json"),
        ));
        network_store
            .save(NetworkSettingsUpdate::new(
                format!("http://{address}"),
                "desk-1",
                Some(app_core::DeviceTier::Networked),
                Some("admin-secret".into()),
            ))
            .unwrap();
        let runtime = Arc::new(
            RuntimeHandle::start_serial(
                AppConfig::default(),
                Some("/definitely/not/a/deskmate-test-port".into()),
            )
            .unwrap(),
        );
        let mutation_lock = Arc::new(Mutex::new(()));
        let context = ServerSaveContext {
            config: ConfigSaveContext {
                runtime: Arc::clone(&runtime),
                store: Arc::new(ConfigStore::new(&config_path)),
                network_store,
                networked_config: Arc::new(NetworkedConfigProjection::default()),
                has_saved_config: Arc::new(AtomicBool::new(false)),
                mutation_lock: Arc::clone(&mutation_lock),
            },
            server_client: crate::server_http_agent(),
        };
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            assert!(
                mutation_lock.try_lock().is_ok(),
                "the server request started while the desktop mutation lock was held"
            );
            let request = read_http_request(&mut stream);
            assert!(String::from_utf8_lossy(&request).starts_with("PUT "));
            stream
                .write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });

        let result = save_server_config_blocking(context, AppConfig::default());
        server.join().unwrap();
        runtime.shutdown().unwrap();

        assert!(
            config_path.exists(),
            "the local authoring mirror was not saved"
        );
        assert!(matches!(
            result,
            Err(IpcError::RuntimeUnavailable { message })
                if message.contains("saved on this Mac")
                    && message.contains("server destination did not succeed")
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn server_invalid_config_is_the_existing_validation_ipc_category() {
        let error = server_failure(
            422,
            br#"{"kind":"invalid-config","issues":[{"path":"cards[0].title","code":"empty","message":"Choose a title."}]}"#,
        );
        assert!(matches!(
            &error,
            IpcError::Validation { message, issues }
                if issues.len() == 1
                    && issues[0].path == "cards[0].title"
                    && message.contains("server rejected")
                    && !message.contains("last working")
        ));
        assert_eq!(error.log_label(), "validation");

        let malformed = server_failure(422, b"not-json");
        assert!(matches!(malformed, IpcError::RuntimeUnavailable { .. }));
        assert!(!matches!(malformed, IpcError::Device { .. }));
    }

    #[test]
    fn server_transport_keeps_the_bounded_422_validation_body() {
        use std::io::Write;
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = br#"{"kind":"invalid-config","issues":[{"path":"cards","code":"empty","message":"Add a card."}]}"#;
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            let request = String::from_utf8_lossy(&request);
            assert!(request.starts_with("PUT /v1/devices/desk-1/config "));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer admin-secret")
            );
            write!(
                stream,
                "HTTP/1.1 422 Unprocessable Entity\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });

        let (status, response_body) = put_server_config(
            &crate::server_http_agent(),
            &format!("http://{address}/v1/devices/desk-1/config"),
            "admin-secret",
            b"{}",
        )
        .unwrap();
        server.join().unwrap();
        assert_eq!(status, 422);
        assert!(matches!(
            server_failure(status, &response_body),
            IpcError::Validation { issues, .. }
                if issues.len() == 1 && issues[0].message == "Add a card."
        ));
    }

    #[test]
    fn networked_provisioning_requires_public_identity_and_write_only_token() {
        let request = ProvisionDeviceRequest {
            ssid: "home".into(),
            passphrase: String::new(),
            server_url: "https://desk.example".into(),
            device_id: "desk-1".into(),
            device_token: "device-secret".into(),
            tier: app_core::DeviceTier::Networked,
        };
        assert!(validate_network_config_request(&request).is_ok());

        let missing = ProvisionDeviceRequest {
            device_token: String::new(),
            ..request
        };
        let error = validate_network_config_request(&missing).unwrap_err();
        assert!(matches!(error, IpcError::InvalidPayload { .. }));
        assert!(!error.to_string().contains("device-secret"));
    }

    #[test]
    fn connected_legacy_firmware_is_rejected_during_persistence_preflight() {
        let mut device = contract_fixtures().snapshot.app.device;
        device.connection = ConnectionState::Online;
        device.protocol_version = Some(2);
        device.capabilities = vec![DeviceCapability::SceneRender];
        device.unknown_capability_bits = 0;
        let required = DeviceCapability::SceneRender.bit() | DeviceCapability::AssetTransfer.bit();

        let error = ensure_device_compatibility(&device, required).unwrap_err();
        assert!(matches!(
            error,
            IpcError::Validation { issues, .. }
                if issues.len() == 1
                    && issues[0].path == "device.capabilities"
                    && issues[0].code == ValidationCode::RequiresCapability
        ));

        device.connection = ConnectionState::Standalone;
        assert!(ensure_device_compatibility(&device, required).is_ok());
    }

    #[derive(Serialize)]
    struct ContractFixtures {
        snapshot: DesktopSnapshot,
        configs: Vec<AppConfig>,
        card_settings: Vec<CardSettings>,
        card_alerts: Vec<CardAlert>,
        alert_holds: Vec<AlertHold>,
        carousel_advances: Vec<CarouselAdvance>,
        display_templates: Vec<DisplayTemplate>,
        tap_actions: Vec<WidgetTapAction>,
        refresh_policies: Vec<RefreshPolicy>,
        asset_sources: Vec<AssetSource>,
        asset_kinds: Vec<AssetKind>,
        update_channels: Vec<UpdateChannel>,
        update_check_policies: Vec<UpdateCheckPolicy>,
        display_orientations: Vec<DisplayOrientation>,
        device_capabilities: Vec<DeviceCapability>,
        runtime_states: Vec<RuntimeState>,
        connection_states: Vec<ConnectionState>,
        pomodoro_states: Vec<PomodoroState>,
        card_data: Vec<CardDataSnapshot>,
        persistence_states: Vec<PersistenceState>,
        validation_codes: Vec<ValidationCode>,
        pomodoro_actions: Vec<PomodoroAction>,
        errors: Vec<IpcError>,
        draft_validation: DraftValidation,
        config_apply_result: ConfigApplyResult,
        autostart_status: AutostartStatus,
        preview_frame: PreviewFrame,
    }

    fn contract_card_kind(card: &CardSettings) -> &'static str {
        match card {
            CardSettings::Clock { .. } => "clock",
            CardSettings::Pomodoro { .. } => "pomodoro",
            CardSettings::Picture { .. } => "picture",
        }
    }

    #[allow(clippy::too_many_lines)]
    fn contract_fixtures() -> ContractFixtures {
        let issue = ValidationIssue {
            path: "active_playlist_id".into(),
            code: ValidationCode::MissingReference,
            message: "active playlist does not exist".into(),
        };
        let cards = vec![
            CardSettings::Clock {
                id: "clock".into(),
                title: "Desk".into(),
                show_seconds: true,
                template: DisplayTemplate::DigitalClock,
                tap_action: WidgetTapAction::None,
                refresh: RefreshPolicy::DeviceLocal,
                alert: CardAlert::None,
                dwell_seconds: None,
            },
            CardSettings::Pomodoro {
                id: "pomodoro".into(),
                label: "Focus".into(),
                duration_seconds: 1_500,
                template: DisplayTemplate::ProgressRing,
                tap_action: WidgetTapAction::StartPause,
                refresh: RefreshPolicy::DeviceLocal,
                alert: CardAlert::OnTimerFinish {
                    hold: AlertHold::UntilDismissed,
                },
                dwell_seconds: None,
            },
            CardSettings::Picture {
                id: "limits-picture".into(),
                title: "Limits".into(),
                source_id: "limits".into(),
                tap_action: WidgetTapAction::None,
                refresh: RefreshPolicy::Manual,
                alert: CardAlert::None,
                dwell_seconds: None,
            },
        ];
        let all_card_settings = cards.clone();
        let config = AppConfig {
            schema_version: CURRENT_SCHEMA_VERSION,
            preferences: AppPreferences {
                timezone: "Asia/Tbilisi".into(),
                autostart: true,
                paused: false,
                orientation: DisplayOrientation::LandscapeFlipped,
            },
            cards: cards.clone(),
            image_sources: vec![app_core::config::ImageSource {
                id: "limits".into(),
                name: "Claude limits".into(),
            }],
            assets: Vec::new(),
            advance: CarouselAdvance::Timed {
                default_dwell_seconds: 30,
            },
            updater: UpdaterSettings::default(),
        };
        let card_data = vec![CardDataSnapshot {
            card_id: "limits-picture".into(),
            fields: vec![
                CardField {
                    key: "summary".into(),
                    value: CardFieldValue::Text {
                        value: "42 · Good".into(),
                    },
                },
                CardField {
                    key: "stale".into(),
                    value: CardFieldValue::Boolean { value: false },
                },
            ],
        }];
        let snapshot = DesktopSnapshot {
            has_saved_config: true,
            app: AppSnapshot {
                config: config.clone(),
                runtime: RuntimeState::Error {
                    message: "example runtime error".into(),
                },
                device: DeviceSnapshot {
                    connection: ConnectionState::Disconnected {
                        reason: Some("USB disconnected".into()),
                    },
                    port_name: Some("/dev/cu.usbmodem1".into()),
                    firmware_version: Some("1.0.0".into()),
                    protocol_version: Some(2),
                    max_protocol_version: Some(2),
                    capabilities: vec![DeviceCapability::SceneRender],
                    unknown_capability_bits: 0,
                    uptime_ms: Some(42),
                    free_heap: Some(123_456),
                    rotation: Some(90),
                    tier: None,
                    wifi_state: None,
                    wifi_rssi: None,
                    ip: None,
                    last_network_error: None,
                    ota_state: None,
                    active_card_id: Some("clock".into()),
                    counters: DeviceCounters {
                        host_reconnects: 1,
                        valid_frames: 2,
                        malformed_frames: 3,
                        crc_errors: 4,
                        overflow_frames: 5,
                        dropped_responses: 6,
                        rx_dropped_bytes: 7,
                        dropped_events: 8,
                        event_queue_high_water: 9,
                        dropped_ui_commands: 10,
                        ui_queue_high_water: 11,
                        host_dropped_events: 12,
                        detected_event_gaps: 13,
                    },
                },
                pomodoros: vec![PomodoroSnapshot {
                    card_id: "pomodoro".into(),
                    state: PomodoroState::Running,
                    duration_seconds: 1_500,
                    remaining_seconds: 900,
                }],
                card_data: card_data.clone(),
                card_errors: vec![CardError {
                    kind: CardErrorKind::DataRefused,
                    card_id: "limits-picture".into(),
                    message:
                        "the display refused this card's data (InvalidPayload): invalid push data"
                            .into(),
                }],
                persistence: PersistenceState::ValidationFailed {
                    message: SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE.into(),
                    issues: vec![issue.clone()],
                },
                diagnostics: RuntimeDiagnostics {
                    commands_processed: 1,
                    command_queue_full: 2,
                    subscriber_snapshots_overwritten: 6,
                    interrupt_dismissals_ignored: 7,
                },
            },
        };
        let runtime_states = vec![
            RuntimeState::Starting,
            RuntimeState::Running,
            RuntimeState::Paused,
            RuntimeState::Error {
                message: "error".into(),
            },
        ];
        let connection_states = vec![
            ConnectionState::Disconnected { reason: None },
            ConnectionState::Connecting,
            ConnectionState::Online,
            ConnectionState::Standalone,
        ];
        let pomodoro_states = vec![
            PomodoroState::Idle,
            PomodoroState::Running,
            PomodoroState::Paused,
            PomodoroState::Completed,
        ];
        let persistence_states = vec![
            PersistenceState::Clean,
            PersistenceState::Saving,
            PersistenceState::RecoverableError {
                message: "error".into(),
            },
            PersistenceState::ValidationFailed {
                message: SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE.into(),
                issues: vec![issue.clone()],
            },
        ];
        let validation_codes = vec![
            ValidationCode::UnsupportedVersion,
            ValidationCode::Empty,
            ValidationCode::TooLong,
            ValidationCode::TooMany,
            ValidationCode::DuplicateId,
            ValidationCode::MissingReference,
            ValidationCode::OutOfRange,
            ValidationCode::InvalidTimezone,
            ValidationCode::InvalidSource,
            ValidationCode::InvalidComposition,
            ValidationCode::TooLarge,
            ValidationCode::RequiresCapability,
        ];
        let pomodoro_actions = vec![
            PomodoroAction::Start,
            PomodoroAction::Pause,
            PomodoroAction::Toggle,
            PomodoroAction::Reset,
        ];
        let errors = vec![
            IpcError::InvalidPayload {
                message: "invalid".into(),
            },
            IpcError::PayloadTooLarge {
                message: "large".into(),
                maximum_bytes: MAX_CONFIG_FILE_BYTES,
            },
            IpcError::Validation {
                message: "validation".into(),
                issues: vec![issue.clone()],
            },
            IpcError::Persistence {
                message: "persistence".into(),
            },
            IpcError::RuntimeBusy {
                message: "busy".into(),
            },
            IpcError::RuntimeUnavailable {
                message: "unavailable".into(),
            },
            IpcError::IncompatibleServer {
                message: "incompatible".into(),
            },
            IpcError::NotFound {
                message: "missing".into(),
            },
            IpcError::Device {
                message: "device".into(),
            },
            IpcError::Autostart {
                message: "autostart".into(),
            },
            IpcError::Window {
                message: "window".into(),
            },
            IpcError::Internal {
                message: "internal".into(),
            },
            IpcError::Unsupported {
                message: "unsupported".into(),
            },
        ];

        ContractFixtures {
            snapshot,
            configs: vec![config],
            card_settings: all_card_settings,
            card_alerts: vec![
                CardAlert::None,
                CardAlert::OnTimerFinish {
                    hold: AlertHold::UntilDismissed,
                },
            ],
            alert_holds: vec![AlertHold::UntilDismissed, AlertHold::Seconds { value: 60 }],
            carousel_advances: vec![
                CarouselAdvance::Manual,
                CarouselAdvance::Timed {
                    default_dwell_seconds: 20,
                },
            ],
            display_templates: vec![
                DisplayTemplate::DigitalClock,
                DisplayTemplate::AnalogClock,
                DisplayTemplate::ProgressRing,
            ],
            tap_actions: vec![
                WidgetTapAction::None,
                WidgetTapAction::StartPause,
                WidgetTapAction::Reset,
                WidgetTapAction::Dismiss,
                WidgetTapAction::OpenUrl {
                    url: "https://example.test/action".into(),
                },
                WidgetTapAction::OpenApplication {
                    application_id: "com.example.app".into(),
                },
            ],
            refresh_policies: vec![
                RefreshPolicy::DeviceLocal,
                RefreshPolicy::Manual,
                RefreshPolicy::Interval { minutes: 15 },
            ],
            asset_sources: vec![AssetSource::File("/tmp/status-icons.ttf".into())],
            asset_kinds: vec![
                AssetKind::Font,
                AssetKind::IconFont {
                    glyphs: vec![IconGlyphMapping {
                        name: "cloud-rain".into(),
                        codepoint: 0xf729,
                    }],
                },
                AssetKind::Image,
            ],
            update_channels: vec![
                UpdateChannel::Stable,
                UpdateChannel::Beta,
                UpdateChannel::Manual,
            ],
            update_check_policies: vec![UpdateCheckPolicy::Disabled, UpdateCheckPolicy::Notify],
            display_orientations: vec![
                DisplayOrientation::Landscape,
                DisplayOrientation::LandscapeFlipped,
            ],
            device_capabilities: DeviceCapability::from_bits(DeviceCapability::known_bits()),
            runtime_states,
            connection_states,
            pomodoro_states,
            card_data,
            persistence_states,
            validation_codes,
            pomodoro_actions,
            errors,
            draft_validation: DraftValidation {
                valid: false,
                issues: vec![issue],
            },
            config_apply_result: ConfigApplyResult {
                save: SaveReceipt {
                    generation: 7,
                    warning: Some(StoreWarning::Io {
                        operation: "sync config directory".into(),
                        message: "example warning".into(),
                    }),
                },
            },
            autostart_status: AutostartStatus {
                enabled: true,
                preference_enabled: false,
            },
            preview_frame: PreviewFrame {
                png_base64: Some("iVBORw0KGgo=".into()),
                sample: true,
                state: None,
            },
        }
    }

    #[test]
    fn contract_fixture_covers_every_card_settings_variant() {
        let kinds = contract_fixtures()
            .card_settings
            .iter()
            .map(contract_card_kind)
            .collect::<Vec<_>>();
        assert_eq!(kinds, ["clock", "pomodoro", "picture"]);
    }

    #[test]
    fn a_picture_preview_explains_that_the_source_owns_the_frame() {
        assert_eq!(
            unrendered_server_frame(PICTURE_PREVIEW_IS_PUSH_ONLY),
            PreviewFrame {
                png_base64: None,
                sample: false,
                state: Some("Picture cards show the last frame pushed by their source".into()),
            }
        );
    }

    /// Builds a `ServerQueryContext` pointed at a loopback listener with a stored
    /// admin token, in the tier the caller names.
    fn server_query_fixture(
        label: &str,
        tier: app_core::DeviceTier,
    ) -> (
        ServerQueryContext,
        std::net::TcpListener,
        std::path::PathBuf,
    ) {
        let directory = scratch_directory(label);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let network_store = Arc::new(NetworkSettingsStore::new(
            directory.join("network-settings.json"),
        ));
        network_store
            .save(NetworkSettingsUpdate::new(
                format!("http://{address}"),
                "desk-1",
                Some(tier),
                Some("admin-secret".into()),
            ))
            .unwrap();
        let context = ServerQueryContext {
            agent: crate::server_http_agent(),
            network_store,
        };
        (context, listener, directory)
    }

    fn answer_once(
        listener: std::net::TcpListener,
        status: &'static str,
        body: impl Into<String>,
    ) -> std::thread::JoinHandle<String> {
        use std::io::Write as _;

        let body = body.into();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = String::from_utf8_lossy(&read_http_request(&mut stream)).to_string();
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            request
        })
    }

    #[test]
    fn minting_a_picture_source_posts_its_name_and_returns_one_time_access() {
        let (context, listener, directory) =
            server_query_fixture("mint-picture-source", app_core::DeviceTier::Networked);
        let token = "11".repeat(32);
        let server = answer_once(
            listener,
            "200 OK",
            format!(r#"{{"id":"picture-source","token":"{token}"}}"#),
        );

        let minted = mint_server_image_source(&context, "Picture", None).unwrap();

        let request = server.join().unwrap();
        let normalized = request.to_ascii_lowercase();
        assert!(request.starts_with("POST /v1/images "));
        assert!(normalized.contains("authorization: bearer admin-secret\r\n"));
        assert!(normalized.contains("content-type: application/json\r\n"));
        assert!(request.ends_with(r#"{"name":"Picture"}"#));
        assert_eq!(minted.source_id, "picture-source");
        assert_eq!(minted.token, token);
        assert!(
            minted
                .push_url
                .ends_with(&format!("/v1/images/{}", minted.token))
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn listing_image_sources_gets_server_descriptors_with_admin_auth() {
        let (context, listener, directory) =
            server_query_fixture("list-picture-sources", app_core::DeviceTier::Networked);
        let server = answer_once(
            listener,
            "200 OK",
            r#"[{"id":"image-a205","name":"External","face":null},{"id":"image-a977","name":"Managed","face":{"kind":"server-face","label":"Managed face","fields":[{"key":"place","label":"Place","type":"text","value":"Dubai","placeholder":"Dubai"}]}}]"#,
        );

        let sources = list_server_image_sources(&context).expect("list sources");

        let request = server.join().unwrap();
        let normalized = request.to_ascii_lowercase();
        assert!(request.starts_with("GET /v1/images "));
        assert!(normalized.contains("authorization: bearer admin-secret\r\n"));
        assert_eq!(sources.len(), 2);
        assert!(sources[0].face.is_none());
        assert_eq!(sources[1].face.as_ref().unwrap().fields.len(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn updating_image_source_settings_puts_only_descriptor_field_values() {
        let (context, listener, directory) =
            server_query_fixture("update-picture-source", app_core::DeviceTier::Networked);
        let server = answer_once(
            listener,
            "200 OK",
            r#"{"kind":"server-face","label":"Managed face","fields":[{"key":"place","label":"Place","type":"text","value":"Berlin","placeholder":"Dubai"}]}"#,
        );
        let request = UpdateImageSourceFaceRequest {
            source_id: "image-a977".into(),
            fields: BTreeMap::from([("place".into(), "Berlin".into())]),
        };

        let descriptor =
            update_server_image_source_face(&context, request).expect("update settings");

        let request = server.join().unwrap();
        let normalized = request.to_ascii_lowercase();
        assert!(request.starts_with("PUT /v1/images/image-a977/face "));
        assert!(normalized.contains("authorization: bearer admin-secret\r\n"));
        assert!(normalized.contains("content-type: application/json\r\n"));
        assert!(request.ends_with(r#"{"fields":{"place":"Berlin"}}"#));
        assert_eq!(descriptor.label, "Managed face");
        fs::remove_dir_all(directory).unwrap();
    }

    fn typescript_contract_source() -> String {
        let json = serde_json::to_string_pretty(&contract_fixtures()).unwrap();
        format!(
            "// Generated by the Rust IPC contract test; edit the DTOs, not this fixture.\n\
             import type {{ IpcContractFixtures }} from \"./types\";\n\n\
             export const ipcContractFixtures = {json} as const satisfies IpcContractFixtures;\n"
        )
    }

    #[test]
    fn typescript_contract_fixture_stays_in_sync() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/lib/types.contract.ts");
        let checked = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "cannot read checked TypeScript contract {}: {error}",
                path.display()
            )
        });
        assert_eq!(checked, typescript_contract_source());
    }

    #[test]
    #[ignore = "prints the checked TypeScript fixture for intentional regeneration"]
    fn print_typescript_contract_fixture() {
        print!("{}", typescript_contract_source());
    }
}
