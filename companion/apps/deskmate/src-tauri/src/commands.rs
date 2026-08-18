// Tauri injects command state and app handles by value; these signatures are part of
// its command macro contract even when the handler only borrows them internally.
#![allow(clippy::needless_pass_by_value)]

use app_core::{
    AppConfig, CardField, CardFieldValue, DisplayOrientation, DisplayTemplate,
    MAX_CONFIG_FILE_BYTES, MAX_ICS_BYTES, MAX_ICS_SOURCE_LEN, MAX_WIDGET_ID_LEN, NetworkConfig,
    NetworkSettings, NetworkSettingsStoreError, NetworkSettingsUpdate, PomodoroAction,
    ProvisioningTier, RuntimeError, SaveReceipt, StoreError, ValidationIssue, utc_offset_minutes,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;

use crate::{DesktopSnapshot, DesktopState, MAIN_WINDOW_LABEL};

// These are the protocol-v1 NetworkConfig bounds. app-core deliberately exposes the
// command type, not protocol implementation constants, so the shell repeats them at
// its untrusted IPC edge and the runtime remains the final encoder-side authority.
const MAX_SSID_BYTES: usize = 32;
const MAX_PASSPHRASE_BYTES: usize = 64;
const MAX_SERVER_URL_BYTES: usize = 128;
const MAX_DEVICE_ID_BYTES: usize = 32;
const MAX_DEVICE_TOKEN_BYTES: usize = 128;
const MAX_SERVER_ERROR_BYTES: usize = 64 * 1_024;

/// The draft travels as a bounded JSON envelope so an IPC caller cannot make serde
/// allocate an arbitrarily deep application document before domain validation runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftPayload {
    pub json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetTarget {
    pub widget_id: String,
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

/// Secret-bearing IPC inputs intentionally implement neither `Debug` nor `Serialize`.
/// Their only outbound projection is `NetworkSettings`, which contains public fields.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerEndpointRequest {
    server_url: String,
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
    server_url: String,
    device_id: String,
    admin_token: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum ServerErrorBody {
    InvalidConfig { issues: Vec<ValidationIssue> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutostartStatus {
    pub enabled: bool,
    pub preference_enabled: bool,
}

/// A rendered card preview: the exact PNG bytes the firmware's own template
/// renderer produced, base64-encoded for the typed IPC boundary (the webview never
/// receives anything besides these bytes — see the module docs on `preview`).
/// `sample` is set when the card has never published data (the runtime holds no
/// `CardDataSnapshot` for it): the request still renders, with an empty field set,
/// so the image is the firmware's own unconfigured appearance for that template
/// rather than an invented placeholder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewFrame {
    pub png_base64: String,
    pub sample: bool,
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
    NotFound {
        message: String,
    },
    Device {
        message: String,
    },
    Provider {
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
            | Self::NotFound { message }
            | Self::Device { message }
            | Self::Provider { message }
            | Self::Autostart { message }
            | Self::Window { message }
            | Self::Internal { message } => formatter.write_str(message),
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
            Self::NotFound { .. } => "not-found",
            Self::Device { .. } => "device",
            Self::Provider { .. } => "provider",
            Self::Autostart { .. } => "autostart",
            Self::Window { .. } => "window",
            Self::Internal { .. } => "internal",
        }
    }
}

#[tauri::command]
pub fn get_app_snapshot(state: State<'_, DesktopState>) -> Result<DesktopSnapshot, IpcError> {
    let snapshot = state.runtime.snapshot().map_err(IpcError::from)?;
    Ok(state.project_snapshot(snapshot))
}

#[tauri::command]
pub fn validate_config_draft(
    state: State<'_, DesktopState>,
    draft: DraftPayload,
) -> Result<DraftValidation, IpcError> {
    // The connected device's capabilities are half of what Save checks, so draft
    // validation must see them too — see `validate_draft_for_device`.
    let snapshot = state.runtime.snapshot().map_err(IpcError::from)?;
    validate_draft_for_device(&draft, &snapshot.device)
}

/// Reports exactly the issues `save_and_apply` would, so the settings UI can never
/// call a draft valid that Save then rejects.
///
/// That means both halves of Save's preflight: `compile()` (a strict superset of
/// `validate()` — it also catches a card with no `wire_config()` lowering) AND the
/// connected device's capability check. Reporting only the first was the narrower
/// contract this exists to prevent: a draft using a template the connected firmware
/// cannot render read as valid right up until Save refused it, with an error attached
/// to no field. The revision value only matters for `revision == 0` rejection, so any
/// nonzero placeholder is fine for a draft that is never actually applied.
fn validate_draft_for_device(
    draft: &DraftPayload,
    device: &app_core::DeviceSnapshot,
) -> Result<DraftValidation, IpcError> {
    let config = parse_draft(draft)?;
    let issues = match config.compile(1) {
        Ok(compiled) => missing_capability_issues(device, compiled.required_capabilities),
        Err(error) => error.issues,
    };
    Ok(DraftValidation {
        valid: issues.is_empty(),
        issues,
    })
}

#[tauri::command]
pub fn save_apply_config(
    state: State<'_, DesktopState>,
    draft: DraftPayload,
) -> Result<ConfigApplyResult, IpcError> {
    let config = parse_valid_draft(&draft)?;
    save_and_apply(&state, config)
}

#[tauri::command]
pub fn get_network_settings(state: State<'_, DesktopState>) -> NetworkSettings {
    state.network_store.load().settings().clone()
}

#[tauri::command]
pub fn set_server_endpoint(
    state: State<'_, DesktopState>,
    request: ServerEndpointRequest,
) -> Result<NetworkSettings, IpcError> {
    validate_server_url(&request.server_url)?;
    validate_secret(&request.admin_token, 4_096, "admin token")?;
    let current = state.network_store.load().settings().clone();
    let settings = NetworkSettings {
        server_url: request.server_url,
        device_id: current.device_id,
    };
    state
        .network_store
        .save(NetworkSettingsUpdate::new(
            settings.server_url.clone(),
            settings.device_id.clone(),
            Some(request.admin_token),
            None,
        ))
        .map_err(IpcError::from)?;
    Ok(settings)
}

#[tauri::command]
pub fn provision_device(
    state: State<'_, DesktopState>,
    request: ProvisionDeviceRequest,
) -> Result<NetworkSettings, IpcError> {
    validate_network_config_request(&request)?;
    let snapshot = state.runtime.snapshot().map_err(IpcError::from)?;
    let utc_offset_minutes =
        utc_offset_minutes(&snapshot.config.preferences.timezone, chrono::Utc::now()).map_err(
            |message| IpcError::Validation {
                message,
                issues: vec![ValidationIssue {
                    path: "preferences.timezone".into(),
                    code: app_core::ValidationCode::InvalidTimezone,
                    message: "Choose a valid display timezone before provisioning.".into(),
                }],
            },
        )?;
    let tier = match request.tier {
        app_core::DeviceTier::Local => ProvisioningTier::Local,
        app_core::DeviceTier::Networked => ProvisioningTier::Networked,
    };
    let public_settings = NetworkSettings {
        server_url: request.server_url.clone(),
        device_id: request.device_id.clone(),
    };
    let config = NetworkConfig {
        ssid: request.ssid,
        psk: request.passphrase,
        server_url: request.server_url,
        device_id: request.device_id,
        token: request.device_token.clone(),
        utc_offset_minutes,
        tier,
    };

    // Persist first: if the cable disappears, the write-only credential is still
    // available to a retry without ever being returned to the renderer.
    state
        .network_store
        .save(NetworkSettingsUpdate::new(
            public_settings.server_url.clone(),
            public_settings.device_id.clone(),
            None,
            Some(request.device_token),
        ))
        .map_err(IpcError::from)?;
    state.runtime.provision(config).map_err(IpcError::from)?;
    if matches!(tier, ProvisioningTier::Local) {
        state.set_networked_config(None)?;
    }
    Ok(public_settings)
}

#[tauri::command]
pub fn factory_reset_device(state: State<'_, DesktopState>) -> Result<(), IpcError> {
    state.runtime.factory_reset().map_err(IpcError::from)?;
    let current = state.network_store.load().settings().clone();
    state
        .network_store
        .save(NetworkSettingsUpdate::new(
            current.server_url,
            current.device_id,
            None,
            Some(String::new()),
        ))
        .map_err(IpcError::from)?;
    state.set_networked_config(None)
}

#[tauri::command]
pub async fn save_server_config(
    state: State<'_, DesktopState>,
    request: ServerConfigRequest,
) -> Result<ConfigApplyResult, IpcError> {
    let config = parse_valid_draft(&request.draft)?;
    let snapshot = state.runtime.snapshot().map_err(IpcError::from)?;
    if !matches!(snapshot.device.tier, Some(app_core::DeviceTier::Networked)) {
        return Err(IpcError::InvalidPayload {
            message: "the connected display is not owned by the server".into(),
        });
    }
    validate_secret(&request.admin_token, 4_096, "admin token")?;
    let url = server_config_url(&request.server_url, &request.device_id)?.to_string();
    let body = serde_json::to_vec(&config).map_err(|_| IpcError::Internal {
        message: "configuration could not be serialized for the server".into(),
    })?;
    let agent = state.server_client.clone();
    let admin_token = request.admin_token;
    let (status, response_body) = tauri::async_runtime::spawn_blocking(move || {
        put_server_config(&agent, &url, &admin_token, &body)
    })
    .await
    .map_err(|_| IpcError::RuntimeUnavailable {
        message: "the server request worker stopped unexpectedly".into(),
    })??;
    if !(200..300).contains(&status) {
        return Err(server_failure(status, &response_body));
    }

    // The server is authoritative in networked tier. Keep a local authoring mirror,
    // but deliberately do not call RuntimeHandle::apply_config: that would put a
    // config write onto the restricted USB link and challenge the server's ownership.
    let save = persist_config(&state, &config)?;
    state.set_networked_config(Some(config))?;
    Ok(ConfigApplyResult { save })
}

#[tauri::command]
pub fn set_pushing_paused(state: State<'_, DesktopState>, paused: bool) -> Result<(), IpcError> {
    set_paused(&state, paused)
}

#[tauri::command]
pub fn control_pomodoro(
    state: State<'_, DesktopState>,
    target: WidgetTarget,
    action: PomodoroAction,
) -> Result<(), IpcError> {
    validate_target(&target.widget_id, MAX_WIDGET_ID_LEN, "widget ID")?;
    state
        .runtime
        .control_pomodoro(target.widget_id, action)
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn refresh_provider(
    state: State<'_, DesktopState>,
    target: WidgetTarget,
) -> Result<(), IpcError> {
    validate_target(&target.widget_id, MAX_WIDGET_ID_LEN, "widget ID")?;
    state
        .runtime
        .refresh_provider(target.widget_id)
        .map_err(IpcError::from)
}

#[tauri::command]
pub async fn choose_ics_file(app: AppHandle) -> Result<Option<String>, IpcError> {
    let Some(file) = app
        .dialog()
        .file()
        .set_title("Choose an iCalendar file")
        .add_filter("iCalendar", &["ics", "ical"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|error| IpcError::InvalidPayload {
        message: format!("the selected calendar source is not a local file: {error}"),
    })?;
    let metadata = std::fs::metadata(&path).map_err(|error| IpcError::Provider {
        message: format!("cannot read the selected calendar file: {error}"),
    })?;
    if !metadata.is_file() {
        return Err(IpcError::InvalidPayload {
            message: "the selected calendar source is not a file".into(),
        });
    }
    validate_ics_file_size(metadata.len())?;
    let value = path
        .to_str()
        .ok_or_else(|| IpcError::InvalidPayload {
            message: "the selected calendar path is not valid UTF-8".into(),
        })?
        .to_owned();
    validate_target(&value, MAX_ICS_SOURCE_LEN, "ICS file path")?;
    Ok(Some(value))
}

fn validate_ics_file_size(length: u64) -> Result<(), IpcError> {
    if length > MAX_ICS_BYTES as u64 {
        return Err(IpcError::PayloadTooLarge {
            message: "the selected calendar file exceeds the 1 MB limit".into(),
            maximum_bytes: MAX_ICS_BYTES,
        });
    }
    Ok(())
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

#[tauri::command]
pub fn set_settings_window_visible(
    app: AppHandle,
    state: State<'_, DesktopState>,
    visible: bool,
) -> Result<DesktopSnapshot, IpcError> {
    let window = app
        .get_webview_window(MAIN_WINDOW_LABEL)
        .ok_or_else(|| IpcError::Window {
            message: "settings window is unavailable".into(),
        })?;
    if visible {
        window.unminimize().map_err(window_error)?;
        window.show().map_err(window_error)?;
        window.set_focus().map_err(window_error)?;
    } else {
        window.hide().map_err(window_error)?;
    }
    let snapshot = state.runtime.snapshot().map_err(IpcError::from)?;
    Ok(state.project_snapshot(snapshot))
}

/// Renders one card exactly as the firmware's own template would: the same
/// `SimTemplate` `wire_config` would compile it to, the same last-good field values
/// the device holds (`AppSnapshot.card_data`), the same timezone offset the device's
/// clock is synced to, and the currently configured mounting orientation.
///
/// A card with no published data yet (no `CardDataSnapshot` entry, or one with an
/// empty field set) renders with an empty field vector instead of inventing sample
/// text: the firmware's own per-template defaults show through, which is the
/// device's actual unconfigured appearance. `sample` tells the caller this happened
/// so the settings UI can badge it, without the renderer itself lying about what it
/// drew.
#[tauri::command]
pub fn render_card_preview(
    state: State<'_, DesktopState>,
    card_id: String,
) -> Result<PreviewFrame, IpcError> {
    validate_target(&card_id, MAX_WIDGET_ID_LEN, "card ID")?;
    let snapshot = state.runtime.snapshot().map_err(IpcError::from)?;
    let card = snapshot
        .config
        .cards
        .iter()
        .find(|card| card.id() == card_id)
        .ok_or_else(|| IpcError::NotFound {
            message: format!("no card with id {card_id:?}"),
        })?;
    let template = sim_template(card.template());

    let data = snapshot
        .card_data
        .iter()
        .find(|data| data.card_id == card_id);
    let (fields, sample) = match data {
        Some(data) if !data.fields.is_empty() => {
            (data.fields.iter().map(sim_field).collect(), false)
        }
        _ => (Vec::new(), true),
    };

    let now = chrono::Utc::now();
    let utc_offset_minutes = utc_offset_minutes(&snapshot.config.preferences.timezone, now)
        .map_err(|message| IpcError::Internal { message })?;
    let orientation = match snapshot.config.preferences.orientation {
        DisplayOrientation::Landscape => lvgl_sim::SimOrientation::Landscape,
        DisplayOrientation::LandscapeFlipped => lvgl_sim::SimOrientation::LandscapeFlipped,
    };

    let request = lvgl_sim::RenderRequest {
        template,
        fields,
        utc_offset_minutes,
        now_unix_seconds: now.timestamp(),
        orientation,
    };
    let png = state
        .preview
        .render(request)
        .map_err(|message| IpcError::Internal { message })?;
    Ok(PreviewFrame {
        png_base64: BASE64_STANDARD.encode(png),
        sample,
    })
}

/// Mirrors the wire mapping `CardSettings::wire_config` uses for `TemplateKind`
/// (`app-core`'s `config.rs`), except targeting `lvgl_sim::SimTemplate` — the two
/// enums are exhaustively 1:1, so this can never fail to map a `DisplayTemplate` the
/// rest of the app accepts; there is no "unknown template" branch to fall back from.
fn sim_template(template: &DisplayTemplate) -> lvgl_sim::SimTemplate {
    match template {
        DisplayTemplate::DigitalClock => lvgl_sim::SimTemplate::DigitalClock,
        DisplayTemplate::ProgressRing => lvgl_sim::SimTemplate::ProgressRing,
        DisplayTemplate::RowList => lvgl_sim::SimTemplate::RowList,
        DisplayTemplate::AnalogClock => lvgl_sim::SimTemplate::AnalogClock,
        DisplayTemplate::BigNumberLabel => lvgl_sim::SimTemplate::BigNumberLabel,
        DisplayTemplate::IconBadgeText { .. } => lvgl_sim::SimTemplate::IconBadgeText,
    }
}

fn sim_field(field: &CardField) -> lvgl_sim::SimField {
    let value = match &field.value {
        CardFieldValue::Text { value } => lvgl_sim::SimFieldValue::Text(value.clone()),
        CardFieldValue::Integer { value } => lvgl_sim::SimFieldValue::Integer(*value),
        CardFieldValue::Boolean { value } => lvgl_sim::SimFieldValue::Boolean(*value),
    };
    lvgl_sim::SimField {
        name: field.key.clone(),
        value,
    }
}

pub(crate) fn set_paused(state: &DesktopState, paused: bool) -> Result<(), IpcError> {
    let _mutation = state.mutation_lock.lock().map_err(|_| IpcError::Internal {
        message: "desktop mutation lock is unavailable".into(),
    })?;
    let mut config = state.runtime.snapshot().map_err(IpcError::from)?.config;
    if config.preferences.paused == paused {
        return Ok(());
    }
    config.preferences.paused = paused;
    persist_config(state, &config)?;
    // If the worker cannot accept this change, the valid saved preference remains
    // authoritative on the next app start rather than being silently discarded.
    state.runtime.set_paused(paused).map_err(IpcError::from)
}

pub(crate) fn set_autostart(
    app: &AppHandle,
    state: &DesktopState,
    enabled: bool,
) -> Result<AutostartStatus, IpcError> {
    let _mutation = state.mutation_lock.lock().map_err(|_| IpcError::Internal {
        message: "desktop mutation lock is unavailable".into(),
    })?;
    let manager = app.autolaunch();
    let was_enabled = manager.is_enabled().map_err(autostart_error)?;
    let changed_os = was_enabled != enabled;
    if changed_os {
        if enabled {
            manager.enable().map_err(autostart_error)?;
        } else {
            manager.disable().map_err(autostart_error)?;
        }
    }

    let mut config = state.runtime.snapshot().map_err(IpcError::from)?.config;
    config.preferences.autostart = enabled;
    if let Err(error) = persist_config(state, &config) {
        if changed_os {
            if was_enabled {
                let _ = manager.enable();
            } else {
                let _ = manager.disable();
            }
        }
        return Err(error);
    }

    // This preference has no layout or provider effect, so update it without replacing
    // live pomodoro/provider state.
    state
        .runtime
        .set_autostart_preference(enabled)
        .map_err(IpcError::from)?;
    state
        .tray
        .autostart
        .set_checked(enabled)
        .map_err(window_error)?;
    Ok(AutostartStatus {
        enabled,
        preference_enabled: enabled,
    })
}

fn save_and_apply(
    state: &DesktopState,
    mut config: AppConfig,
) -> Result<ConfigApplyResult, IpcError> {
    let _mutation = state.mutation_lock.lock().map_err(|_| IpcError::Internal {
        message: "desktop mutation lock is unavailable".into(),
    })?;
    let snapshot = state.runtime.snapshot().map_err(IpcError::from)?;
    merge_command_owned_preferences(&mut config, &snapshot.config);
    let compiled = config.compile(1).map_err(|error| IpcError::Validation {
        message: "configuration requires device features not implemented by this build".into(),
        issues: error.issues,
    })?;
    ensure_device_compatibility(&snapshot.device, compiled.required_capabilities)?;
    let save = persist_config(state, &config)?;
    // Keep a valid saved draft queued even when immediate device application reports
    // a transport failure. RuntimeHandle has already replaced its replay set then.
    state.runtime.apply_config(config).map_err(IpcError::from)?;
    Ok(ConfigApplyResult { save })
}

fn ensure_device_compatibility(
    device: &app_core::DeviceSnapshot,
    required_capabilities: u64,
) -> Result<(), IpcError> {
    let issues = missing_capability_issues(device, required_capabilities);
    if issues.is_empty() {
        return Ok(());
    }
    Err(IpcError::Validation {
        message: "the connected firmware cannot apply this configuration".into(),
        issues,
    })
}

/// The capability issues a configuration would raise against `device`, or an empty
/// list when it is compatible (or when no device is online to check against — an
/// offline draft is saved for the next connection and gated then).
fn missing_capability_issues(
    device: &app_core::DeviceSnapshot,
    required_capabilities: u64,
) -> Vec<ValidationIssue> {
    if !matches!(device.connection, app_core::ConnectionState::Online)
        || device.protocol_version.is_none()
    {
        return Vec::new();
    }
    let available = device.capability_bits();
    if available & required_capabilities == required_capabilities {
        return Vec::new();
    }
    let missing = required_capabilities & !available;
    vec![ValidationIssue {
        path: "device.capabilities".into(),
        code: app_core::ValidationCode::RequiresCapability,
        message: format!(
            "the connected firmware does not support {}. Update the firmware, or remove the cards and settings that need it.",
            describe_capabilities(missing)
        ),
    }]
}

/// Names capabilities the way the person reading the settings window needs them. The
/// raw bitmask this used to print ("missing capability bits 0x0000000000000008") named
/// nothing the user could act on.
fn describe_capabilities(bits: u64) -> String {
    let mut names: Vec<&'static str> = app_core::DeviceCapability::from_bits(bits)
        .into_iter()
        .map(app_core::DeviceCapability::label)
        .collect();
    // A bit this build does not know about can only be reported as itself.
    if bits & !app_core::DeviceCapability::known_bits() != 0 {
        names.push("an unrecognized device feature");
    }
    match names.len() {
        0 => "the features this configuration needs".to_owned(),
        1 => names[0].to_owned(),
        _ => {
            let last = names.pop().unwrap_or_default();
            format!("{} and {last}", names.join(", "))
        }
    }
}

fn persist_config(state: &DesktopState, config: &AppConfig) -> Result<SaveReceipt, IpcError> {
    state
        .runtime
        .set_persistence_state(app_core::PersistenceState::Saving)
        .map_err(IpcError::from)?;
    match state.store.save(config) {
        Ok(receipt) => {
            state.mark_config_saved();
            state
                .runtime
                .set_persistence_state(app_core::PersistenceState::Clean)
                .map_err(IpcError::from)?;
            Ok(receipt)
        }
        Err(error) => {
            let message = error.to_string();
            let _ = state
                .runtime
                .set_persistence_state(app_core::PersistenceState::RecoverableError { message });
            Err(IpcError::from(error))
        }
    }
}

fn merge_command_owned_preferences(draft: &mut AppConfig, current: &AppConfig) {
    // Pause and start-at-login have dedicated commands with OS/runtime side effects.
    // Preserve their newest values when a settings draft was opened before a tray or
    // IPC toggle, while still allowing timezone edits through save/apply.
    draft.preferences.paused = current.preferences.paused;
    draft.preferences.autostart = current.preferences.autostart;
}

fn autostart_status(app: &AppHandle, state: &DesktopState) -> Result<AutostartStatus, IpcError> {
    let enabled = app.autolaunch().is_enabled().map_err(autostart_error)?;
    let preference_enabled = state
        .runtime
        .snapshot()
        .map_err(IpcError::from)?
        .config
        .preferences
        .autostart;
    Ok(AutostartStatus {
        enabled,
        preference_enabled,
    })
}

fn parse_valid_draft(draft: &DraftPayload) -> Result<AppConfig, IpcError> {
    let config = parse_draft(draft)?;
    config.validate().map_err(|error| IpcError::Validation {
        message: error.to_string(),
        issues: error.issues,
    })?;
    Ok(config)
}

fn parse_draft(draft: &DraftPayload) -> Result<AppConfig, IpcError> {
    let length = draft.json.len();
    if length > MAX_CONFIG_FILE_BYTES {
        return Err(IpcError::PayloadTooLarge {
            message: format!(
                "configuration draft exceeds the {MAX_CONFIG_FILE_BYTES}-byte IPC limit"
            ),
            maximum_bytes: MAX_CONFIG_FILE_BYTES,
        });
    }
    serde_json::from_str(&draft.json).map_err(|error| IpcError::InvalidPayload {
        message: format!("configuration draft is not valid: {error}"),
    })
}

fn validate_network_config_request(request: &ProvisionDeviceRequest) -> Result<(), IpcError> {
    validate_bounded(&request.ssid, MAX_SSID_BYTES, "WiFi network")?;
    validate_bounded(&request.passphrase, MAX_PASSPHRASE_BYTES, "WiFi passphrase")?;
    validate_bounded(&request.server_url, MAX_SERVER_URL_BYTES, "server URL")?;
    validate_bounded(&request.device_id, MAX_DEVICE_ID_BYTES, "device ID")?;
    validate_bounded(
        &request.device_token,
        MAX_DEVICE_TOKEN_BYTES,
        "device token",
    )?;
    if matches!(request.tier, app_core::DeviceTier::Networked) {
        validate_target(&request.ssid, MAX_SSID_BYTES, "WiFi network")?;
        validate_server_url(&request.server_url)?;
        validate_target(&request.device_id, MAX_DEVICE_ID_BYTES, "device ID")?;
        validate_secret(
            &request.device_token,
            MAX_DEVICE_TOKEN_BYTES,
            "device token",
        )?;
    }
    Ok(())
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

fn validate_server_url(value: &str) -> Result<url::Url, IpcError> {
    validate_target(value, MAX_SERVER_URL_BYTES, "server URL")?;
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

fn server_config_url(server_url: &str, device_id: &str) -> Result<url::Url, IpcError> {
    validate_target(device_id, MAX_DEVICE_ID_BYTES, "device ID")?;
    let mut url = validate_server_url(server_url)?;
    url.set_query(None);
    url.set_fragment(None);
    let mut segments = url
        .path_segments_mut()
        .map_err(|()| IpcError::InvalidPayload {
            message: "server URL cannot be used as a base URL".into(),
        })?;
    segments
        .pop_if_empty()
        .push("v1")
        .push("devices")
        .push(device_id)
        .push("config");
    drop(segments);
    Ok(url)
}

fn put_server_config(
    agent: &ureq::Agent,
    url: &str,
    admin_token: &str,
    body: &[u8],
) -> Result<(u16, Vec<u8>), IpcError> {
    let mut response = agent
        .put(url)
        .header("Authorization", format!("Bearer {admin_token}"))
        .content_type("application/json")
        .send(body)
        .map_err(|_| IpcError::RuntimeUnavailable {
            message: "the configured server could not be reached".into(),
        })?;
    let status = response.status().as_u16();
    let response_body = if status == 422 {
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_SERVER_ERROR_BYTES.saturating_add(1) as u64)
            .read_to_vec()
            .map_err(|_| IpcError::RuntimeUnavailable {
                message: "the server returned an unreadable error response".into(),
            })?;
        if body.len() > MAX_SERVER_ERROR_BYTES {
            return Err(IpcError::RuntimeUnavailable {
                message: "the server returned an oversized error response".into(),
            });
        }
        body
    } else {
        Vec::new()
    };
    Ok((status, response_body))
}

fn server_failure(status: u16, body: &[u8]) -> IpcError {
    match status {
        401 => IpcError::InvalidPayload {
            message: "the server rejected the admin token".into(),
        },
        404 => IpcError::NotFound {
            message: "the configured device was not found on the server".into(),
        },
        422 => match serde_json::from_slice::<ServerErrorBody>(body) {
            Ok(ServerErrorBody::InvalidConfig { issues }) => IpcError::Validation {
                message: format!(
                    "the server rejected this configuration with {} validation issue(s)",
                    issues.len()
                ),
                issues,
            },
            Err(_) => IpcError::RuntimeUnavailable {
                message: "the server rejected the configuration without validation details".into(),
            },
        },
        _ => IpcError::RuntimeUnavailable {
            message: format!("the server rejected the configuration (HTTP {status})"),
        },
    }
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

fn window_error(error: impl std::fmt::Display) -> IpcError {
    IpcError::Window {
        message: format!("cannot update settings window: {error}"),
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
            RuntimeError::UnknownWidget { .. } | RuntimeError::UnknownScreen { .. } => {
                Self::NotFound {
                    message: error.to_string(),
                }
            }
            RuntimeError::DeviceDisconnected => Self::Device {
                message: "device is disconnected".into(),
            },
            RuntimeError::Device { message } => Self::Device { message },
            RuntimeError::Provider { message } => Self::Provider { message },
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
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    use app_core::{
        AlertHold, AppConfig, AppPreferences, AppSnapshot, AssetKind, AssetSource,
        CURRENT_SCHEMA_VERSION, CalendarSource, CardAlert, CardDataSnapshot, CardError, CardField,
        CardFieldValue, CardSettings, CarouselAdvance, ConnectionState, DeviceCapability,
        DeviceCounters, DeviceSnapshot, DisplayOrientation, DisplayTemplate,
        FirmwareArtifactMetadata, GlyphRange, PersistenceState, Playlist, PlaylistEntry,
        PomodoroSnapshot, PomodoroState, ProviderSnapshot, ProviderState, RefreshPolicy,
        RuntimeDiagnostics, RuntimeError, RuntimeState, SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE,
        StoreWarning, UpdateChannel, UpdateCheckPolicy, UpdaterSettings, ValidationCode,
        WeatherUnits, WidgetTapAction,
    };
    use serde::Serialize;

    /// Mirrors `CardSettings::wire_config`'s `TemplateKind` mapping (`app-core`'s
    /// `config.rs`) 1:1, for every `DisplayTemplate` variant the app can construct.
    /// A future template variant that forgets to extend this match is a compile
    /// error, not a silent "unknown template" fallback — see `sim_template`'s docs.
    #[test]
    fn sim_template_mirrors_the_wire_mapping_for_every_display_template() {
        assert_eq!(
            sim_template(&DisplayTemplate::DigitalClock),
            lvgl_sim::SimTemplate::DigitalClock
        );
        assert_eq!(
            sim_template(&DisplayTemplate::ProgressRing),
            lvgl_sim::SimTemplate::ProgressRing
        );
        assert_eq!(
            sim_template(&DisplayTemplate::RowList),
            lvgl_sim::SimTemplate::RowList
        );
        assert_eq!(
            sim_template(&DisplayTemplate::AnalogClock),
            lvgl_sim::SimTemplate::AnalogClock
        );
        assert_eq!(
            sim_template(&DisplayTemplate::BigNumberLabel),
            lvgl_sim::SimTemplate::BigNumberLabel
        );
        assert_eq!(
            sim_template(&DisplayTemplate::IconBadgeText {
                icon_asset_id: Some("weather-icons".into())
            }),
            lvgl_sim::SimTemplate::IconBadgeText
        );
    }

    #[test]
    fn sim_field_carries_the_key_and_maps_every_value_kind() {
        let text = sim_field(&CardField {
            key: "title".into(),
            value: CardFieldValue::Text {
                value: "Desk".into(),
            },
        });
        assert_eq!(text.name, "title");
        assert_eq!(text.value, lvgl_sim::SimFieldValue::Text("Desk".into()));

        let integer = sim_field(&CardField {
            key: "remaining_seconds".into(),
            value: CardFieldValue::Integer { value: 900 },
        });
        assert_eq!(integer.name, "remaining_seconds");
        assert_eq!(integer.value, lvgl_sim::SimFieldValue::Integer(900));

        let boolean = sim_field(&CardField {
            key: "stale".into(),
            value: CardFieldValue::Boolean { value: true },
        });
        assert_eq!(boolean.name, "stale");
        assert_eq!(boolean.value, lvgl_sim::SimFieldValue::Boolean(true));
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
        config.cards[0] = CardSettings::Weather {
            id: "weather".into(),
            title: "Weather".into(),
            location: "Tbilisi".into(),
            units: WeatherUnits::Metric,
            template: DisplayTemplate::RowList,
            tap_action: WidgetTapAction::Dismiss,
            refresh: RefreshPolicy::Interval { minutes: 30 },
            alert: CardAlert::None,
        };
        config.playlists[0].entries[0].card_id = "weather".into();
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
        let mut config = AppConfig::default();
        config.cards[0] = CardSettings::Clock {
            id: "clock".into(),
            title: "Desk".into(),
            show_seconds: true,
            template: DisplayTemplate::AnalogClock,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
        };
        let draft = DraftPayload {
            json: serde_json::to_string(&config).unwrap(),
        };
        let required = config.compile(1).unwrap().required_capabilities;
        assert_ne!(
            required & DeviceCapability::ExtendedTemplates.bit(),
            0,
            "fixture must need the extended-templates capability"
        );

        // Offline: nothing to gate against, so the draft is reported as valid.
        let mut device = offline_device().device;
        assert!(validate_draft_for_device(&draft, &device).unwrap().valid);

        // Online, but on firmware without the capability: the same issue Save raises.
        device.connection = ConnectionState::Online;
        device.protocol_version = Some(1);
        device.capabilities = vec![DeviceCapability::CoreWidgets];
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
            describe_capabilities(DeviceCapability::ExtendedTemplates.bit()),
            "extended display templates"
        );
        assert_eq!(
            describe_capabilities(
                DeviceCapability::ConfigRotation.bit() | DeviceCapability::AssetTransfer.bit()
            ),
            "display rotation and icon and font asset transfer"
        );
        assert_eq!(
            describe_capabilities(DeviceCapability::ExtendedTemplates.bit() | 1 << 63),
            "extended display templates and an unrecognized device feature"
        );

        let mut device = offline_device().device;
        device.connection = ConnectionState::Online;
        device.protocol_version = Some(1);
        device.capabilities = vec![DeviceCapability::CoreWidgets];
        device.unknown_capability_bits = 0;
        let issues = missing_capability_issues(
            &device,
            DeviceCapability::CoreWidgets.bit() | DeviceCapability::ExtendedTemplates.bit(),
        );
        assert!(
            issues[0].message.contains("extended display templates"),
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
        assert!(validate_target("clock", MAX_WIDGET_ID_LEN, "widget ID").is_ok());
        assert!(matches!(
            validate_target(" ", MAX_WIDGET_ID_LEN, "widget ID"),
            Err(IpcError::InvalidPayload { .. })
        ));
        assert!(matches!(
            validate_target(
                &"x".repeat(MAX_WIDGET_ID_LEN + 1),
                MAX_WIDGET_ID_LEN,
                "widget ID"
            ),
            Err(IpcError::InvalidPayload { .. })
        ));
    }

    #[test]
    fn calendar_file_picker_enforces_the_provider_size_limit() {
        assert!(validate_ics_file_size(MAX_ICS_BYTES as u64).is_ok());
        assert!(matches!(
            validate_ics_file_size(MAX_ICS_BYTES as u64 + 1),
            Err(IpcError::PayloadTooLarge { maximum_bytes, .. })
                if maximum_bytes == MAX_ICS_BYTES
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
            IpcError::from(RuntimeError::UnknownWidget {
                widget_id: "missing".into()
            }),
            IpcError::NotFound { .. }
        ));
        assert!(matches!(
            IpcError::from(RuntimeError::Provider {
                message: "offline".into()
            }),
            IpcError::Provider { .. }
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
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = br#"{"kind":"invalid-config","issues":[{"path":"cards","code":"empty","message":"Add a card."}]}"#;
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
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
        device.protocol_version = Some(1);
        device.capabilities = vec![DeviceCapability::CoreWidgets];
        device.unknown_capability_bits = 0;
        let required = DeviceCapability::CoreWidgets.bit() | DeviceCapability::ConfigRotation.bit();

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
        playlists: Vec<Playlist>,
        playlist_entries: Vec<PlaylistEntry>,
        card_alerts: Vec<CardAlert>,
        alert_holds: Vec<AlertHold>,
        carousel_advances: Vec<CarouselAdvance>,
        calendar_sources: Vec<CalendarSource>,
        display_templates: Vec<DisplayTemplate>,
        tap_actions: Vec<WidgetTapAction>,
        refresh_policies: Vec<RefreshPolicy>,
        weather_units: Vec<WeatherUnits>,
        asset_sources: Vec<AssetSource>,
        asset_kinds: Vec<AssetKind>,
        update_channels: Vec<UpdateChannel>,
        update_check_policies: Vec<UpdateCheckPolicy>,
        firmware_artifacts: Vec<FirmwareArtifactMetadata>,
        display_orientations: Vec<DisplayOrientation>,
        device_capabilities: Vec<DeviceCapability>,
        runtime_states: Vec<RuntimeState>,
        connection_states: Vec<ConnectionState>,
        provider_states: Vec<ProviderState>,
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

    #[allow(clippy::too_many_lines)]
    fn contract_fixtures() -> ContractFixtures {
        let issue = ValidationIssue {
            path: "active_playlist_id".into(),
            code: ValidationCode::MissingReference,
            message: "active playlist does not exist".into(),
        };
        let file_source = CalendarSource::File("/tmp/calendar.ics".into());
        let url_source = CalendarSource::Url("https://example.test/calendar.ics".into());
        let cards = vec![
            CardSettings::Clock {
                id: "clock".into(),
                title: "Desk".into(),
                show_seconds: true,
                template: DisplayTemplate::DigitalClock,
                tap_action: WidgetTapAction::None,
                refresh: RefreshPolicy::DeviceLocal,
                alert: CardAlert::None,
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
            },
            CardSettings::Calendar {
                id: "calendar".into(),
                title: "Next".into(),
                source: file_source.clone(),
                template: DisplayTemplate::RowList,
                tap_action: WidgetTapAction::None,
                refresh: RefreshPolicy::Interval { minutes: 15 },
                alert: CardAlert::BeforeEvent {
                    lead_minutes: 5,
                    hold: AlertHold::Seconds { value: 60 },
                },
            },
        ];
        let mut all_card_settings = cards.clone();
        all_card_settings.extend([
            CardSettings::Weather {
                id: "weather".into(),
                title: "Weather".into(),
                location: "Tbilisi".into(),
                units: WeatherUnits::Metric,
                template: DisplayTemplate::IconBadgeText {
                    icon_asset_id: Some("weather-icons".into()),
                },
                tap_action: WidgetTapAction::OpenUrl {
                    url: "https://example.test/weather".into(),
                },
                refresh: RefreshPolicy::Interval { minutes: 30 },
                alert: CardAlert::None,
            },
            CardSettings::JsonFeed {
                id: "json".into(),
                title: "Metric".into(),
                url: "https://example.test/metric.json".into(),
                mappings: vec![app_core::JsonFieldMapping {
                    field: "value".into(),
                    path: "$.current.value".into(),
                }],
                template: DisplayTemplate::BigNumberLabel,
                tap_action: WidgetTapAction::OpenApplication {
                    application_id: "com.example.metrics".into(),
                },
                refresh: RefreshPolicy::Manual,
                alert: CardAlert::None,
            },
            CardSettings::Rss {
                id: "news".into(),
                title: "News".into(),
                url: "https://example.test/feed.xml".into(),
                max_items: 3,
                template: DisplayTemplate::RowList,
                tap_action: WidgetTapAction::Dismiss,
                refresh: RefreshPolicy::Interval { minutes: 15 },
                alert: CardAlert::None,
            },
        ]);
        let playlist_entries = vec![
            PlaylistEntry {
                card_id: "clock".into(),
                dwell_seconds: None,
            },
            PlaylistEntry {
                card_id: "pomodoro".into(),
                dwell_seconds: Some(20),
            },
            PlaylistEntry {
                card_id: "calendar".into(),
                dwell_seconds: None,
            },
        ];
        let playlists = vec![
            Playlist {
                id: "workday".into(),
                name: "Workday".into(),
                advance: CarouselAdvance::Timed {
                    default_dwell_seconds: 30,
                },
                entries: playlist_entries.clone(),
            },
            Playlist {
                id: "manual".into(),
                name: "Manual".into(),
                advance: CarouselAdvance::Manual,
                entries: vec![playlist_entries[0].clone()],
            },
        ];
        let config = AppConfig {
            schema_version: CURRENT_SCHEMA_VERSION,
            preferences: AppPreferences {
                timezone: "Asia/Tbilisi".into(),
                autostart: true,
                paused: false,
                orientation: DisplayOrientation::LandscapeFlipped,
            },
            cards: cards.clone(),
            assets: Vec::new(),
            playlists: playlists.clone(),
            active_playlist_id: "workday".into(),
            updater: UpdaterSettings::default(),
        };
        let card_data = vec![CardDataSnapshot {
            card_id: "calendar".into(),
            fields: vec![
                CardField {
                    key: "row0_title".into(),
                    value: CardFieldValue::Text {
                        value: "Design review".into(),
                    },
                },
                CardField {
                    key: "next_start_unix_ms".into(),
                    value: CardFieldValue::Integer {
                        value: 1_787_000_000_000,
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
                    protocol_version: Some(1),
                    max_protocol_version: Some(1),
                    capabilities: vec![DeviceCapability::CoreWidgets],
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
                    active_screen_id: Some("clock".into()),
                    counters: DeviceCounters {
                        reconnects: 1,
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
                providers: vec![ProviderSnapshot {
                    widget_id: "calendar".into(),
                    state: ProviderState::Stale {
                        message: "offline".into(),
                    },
                    last_success_unix_ms: Some(1_786_000_000_000),
                    age_seconds: Some(120),
                }],
                pomodoros: vec![PomodoroSnapshot {
                    widget_id: "pomodoro".into(),
                    state: PomodoroState::Running,
                    duration_seconds: 1_500,
                    remaining_seconds: 900,
                }],
                card_data: card_data.clone(),
                card_errors: vec![CardError {
                    card_id: "json".into(),
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
                    provider_jobs_started: 3,
                    provider_queue_full: 4,
                    provider_results_discarded: 5,
                    subscriber_snapshots_overwritten: 6,
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
        let provider_states = vec![
            ProviderState::Idle,
            ProviderState::Refreshing,
            ProviderState::Fresh,
            ProviderState::Stale {
                message: "stale".into(),
            },
            ProviderState::Error {
                message: "error".into(),
            },
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
            ValidationCode::MissingScreen,
            ValidationCode::DuplicateReference,
            ValidationCode::UnsupportedSize,
            ValidationCode::OutOfRange,
            ValidationCode::InvalidTimezone,
            ValidationCode::InvalidSource,
            ValidationCode::InvalidComposition,
            ValidationCode::Overlap,
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
            IpcError::NotFound {
                message: "missing".into(),
            },
            IpcError::Device {
                message: "device".into(),
            },
            IpcError::Provider {
                message: "provider".into(),
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
        ];

        ContractFixtures {
            snapshot,
            configs: vec![config],
            card_settings: all_card_settings,
            playlists,
            playlist_entries,
            card_alerts: vec![
                CardAlert::None,
                CardAlert::OnTimerFinish {
                    hold: AlertHold::UntilDismissed,
                },
                CardAlert::BeforeEvent {
                    lead_minutes: 5,
                    hold: AlertHold::Seconds { value: 60 },
                },
            ],
            alert_holds: vec![AlertHold::UntilDismissed, AlertHold::Seconds { value: 60 }],
            carousel_advances: vec![
                CarouselAdvance::Manual,
                CarouselAdvance::Timed {
                    default_dwell_seconds: 20,
                },
            ],
            calendar_sources: vec![file_source, url_source],
            display_templates: vec![
                DisplayTemplate::DigitalClock,
                DisplayTemplate::AnalogClock,
                DisplayTemplate::ProgressRing,
                DisplayTemplate::RowList,
                DisplayTemplate::BigNumberLabel,
                DisplayTemplate::IconBadgeText {
                    icon_asset_id: Some("weather-icons".into()),
                },
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
            weather_units: vec![WeatherUnits::Metric, WeatherUnits::Imperial],
            asset_sources: vec![AssetSource::File("/tmp/weather-icons.bin".into())],
            asset_kinds: vec![
                AssetKind::Icon {
                    width: 32,
                    height: 32,
                },
                AssetKind::Font {
                    pixel_size: 18,
                    glyph_ranges: vec![GlyphRange {
                        start: 0x20,
                        end: 0x7e,
                    }],
                },
            ],
            update_channels: vec![
                UpdateChannel::Stable,
                UpdateChannel::Beta,
                UpdateChannel::Manual,
            ],
            update_check_policies: vec![UpdateCheckPolicy::Disabled, UpdateCheckPolicy::Notify],
            firmware_artifacts: vec![FirmwareArtifactMetadata {
                version: "1.0.0".into(),
                model: "waveshare-1.8".into(),
                byte_length: 524_288,
                sha256_hex: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                    .into(),
                signing_key_id: "deskmate-release-1".into(),
                signature_base64: format!("{}==", "A".repeat(86)),
            }],
            display_orientations: vec![
                DisplayOrientation::Landscape,
                DisplayOrientation::LandscapeFlipped,
            ],
            device_capabilities: vec![
                DeviceCapability::CoreWidgets,
                DeviceCapability::ConfigRotation,
                DeviceCapability::DashboardLayouts,
                DeviceCapability::ExtendedTemplates,
                DeviceCapability::HostTapActions,
                DeviceCapability::AssetTransfer,
                DeviceCapability::FirmwareUpdate,
                DeviceCapability::Networking,
            ],
            runtime_states,
            connection_states,
            provider_states,
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
                png_base64: "iVBORw0KGgo=".into(),
                sample: true,
            },
        }
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
