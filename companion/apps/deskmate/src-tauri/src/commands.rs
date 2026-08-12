// Tauri injects command state and app handles by value; these signatures are part of
// its command macro contract even when the handler only borrows them internally.
#![allow(clippy::needless_pass_by_value)]

use app_core::{
    AppConfig, AppSnapshot, CardField, CardFieldValue, DisplayOrientation, DisplayTemplate,
    MAX_CONFIG_FILE_BYTES, MAX_ICS_BYTES, MAX_ICS_SOURCE_LEN, MAX_WIDGET_ID_LEN, PomodoroAction,
    RuntimeError, SaveReceipt, StoreError, ValidationIssue, utc_offset_minutes,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;

use crate::{DesktopState, MAIN_WINDOW_LABEL};

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

#[tauri::command]
pub fn get_app_snapshot(state: State<'_, DesktopState>) -> Result<AppSnapshot, IpcError> {
    state.runtime.snapshot().map_err(IpcError::from)
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
) -> Result<AppSnapshot, IpcError> {
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
    state.runtime.snapshot().map_err(IpcError::from)
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

fn autostart_error(error: impl std::fmt::Display) -> IpcError {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    use app_core::{
        AlertHold, AppConfig, AppPreferences, AssetKind, AssetSource, CURRENT_SCHEMA_VERSION,
        CalendarSource, CardAlert, CardDataSnapshot, CardError, CardField, CardFieldValue,
        CardPresence, CardSettings, CarouselAdvance, CarouselSettings, ConnectionState,
        DeviceCapability, DeviceCounters, DeviceSnapshot, DisplayOrientation, DisplayTemplate,
        FirmwareArtifactMetadata, GlyphRange, PersistenceState, PomodoroSnapshot, PomodoroState,
        ProviderSnapshot, ProviderState, RefreshPolicy, RuntimeDiagnostics, RuntimeError,
        RuntimeState, StoreWarning, UpdateChannel, UpdateCheckPolicy, UpdaterSettings,
        ValidationCode, WeatherUnits, WidgetTapAction,
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
        contract_fixtures().snapshot
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
            presence: CardPresence::InRotation {
                dwell_seconds: None,
            },
            alert: CardAlert::None,
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
        let mut config = AppConfig::default();
        config.cards[0] = CardSettings::Clock {
            id: "clock".into(),
            title: "Desk".into(),
            show_seconds: true,
            template: DisplayTemplate::AnalogClock,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::DeviceLocal,
            presence: CardPresence::InRotation {
                dwell_seconds: None,
            },
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
    fn connected_legacy_firmware_is_rejected_during_persistence_preflight() {
        let mut device = contract_fixtures().snapshot.device;
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
        snapshot: AppSnapshot,
        configs: Vec<AppConfig>,
        card_settings: Vec<CardSettings>,
        card_presences: Vec<CardPresence>,
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
            path: "cards[0].presence".into(),
            code: ValidationCode::OutOfRange,
            message: "an alert-only card must configure an alert".into(),
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
                presence: CardPresence::InRotation {
                    dwell_seconds: None,
                },
                alert: CardAlert::None,
            },
            CardSettings::Pomodoro {
                id: "pomodoro".into(),
                label: "Focus".into(),
                duration_seconds: 1_500,
                template: DisplayTemplate::ProgressRing,
                tap_action: WidgetTapAction::StartPause,
                refresh: RefreshPolicy::DeviceLocal,
                presence: CardPresence::InRotation {
                    dwell_seconds: None,
                },
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
                presence: CardPresence::InRotation {
                    dwell_seconds: None,
                },
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
                presence: CardPresence::AlertOnly,
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
                presence: CardPresence::Off,
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
                presence: CardPresence::InRotation {
                    dwell_seconds: Some(20),
                },
                alert: CardAlert::None,
            },
        ]);
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
            carousel: CarouselSettings::default(),
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
        let snapshot = AppSnapshot {
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
                message: "the display refused this card's data (InvalidPayload): invalid push data"
                    .into(),
            }],
            persistence: PersistenceState::RecoverableError {
                message: "disk full".into(),
            },
            diagnostics: RuntimeDiagnostics {
                commands_processed: 1,
                command_queue_full: 2,
                provider_jobs_started: 3,
                provider_queue_full: 4,
                provider_results_discarded: 5,
                subscriber_snapshots_overwritten: 6,
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
            card_presences: vec![
                CardPresence::InRotation {
                    dwell_seconds: None,
                },
                CardPresence::InRotation {
                    dwell_seconds: Some(20),
                },
                CardPresence::AlertOnly,
                CardPresence::Off,
            ],
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
