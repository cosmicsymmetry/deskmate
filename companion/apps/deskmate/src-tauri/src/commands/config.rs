//! Saving and validating a configuration draft, and the device-capability
//! preflight a save has to clear first.
//!
//! Split out of `commands.rs` unchanged on 2026-09-11. The glob keeps name
//! resolution identical to when this was one file.

#[allow(clippy::wildcard_imports)]
use super::*;

pub(super) fn save_and_apply(
    context: ConfigSaveContext,
    mut config: AppConfig,
) -> Result<ConfigApplyResult, IpcError> {
    let _mutation = context
        .mutation_lock
        .lock()
        .map_err(|_| IpcError::Internal {
            message: "desktop mutation lock is unavailable".into(),
        })?;
    let snapshot = context.runtime.snapshot().map_err(IpcError::from)?;
    let settings = context.network_store.load().settings().clone();
    if save_destination(snapshot.device.tier, &settings) != SaveDestination::Local {
        return Err(IpcError::InvalidPayload {
            message: local_save_refusal(&settings).into(),
        });
    }
    merge_command_owned_preferences(&mut config, &snapshot.config);
    let compiled = config.compile(1).map_err(|error| IpcError::Validation {
        message: "configuration requires device features not implemented by this build".into(),
        issues: error.issues,
    })?;
    ensure_device_compatibility(&snapshot.device, compiled.required_capabilities)?;
    let save = persist_config_parts(
        &context.runtime,
        &context.store,
        &context.has_saved_config,
        &config,
    )?;
    context.networked_config.clear_for_local_save()?;
    // Keep a valid saved draft queued even when immediate device application reports
    // a transport failure. RuntimeHandle has already replaced its replay set then.
    context
        .runtime
        .apply_config(config)
        .map_err(IpcError::from)?;
    Ok(ConfigApplyResult { save })
}

pub(super) fn local_save_refusal(settings: &NetworkSettings) -> &'static str {
    if !settings.server_url.is_empty() && settings.device_id.is_empty() {
        "a server endpoint is configured, but no device is paired; enter a device ID or connect over USB to confirm ownership"
    } else {
        "the display is owned by the server; save this configuration to the server instead"
    }
}

pub(super) fn ensure_device_compatibility(
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
pub(super) fn missing_capability_issues(
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
pub(super) fn describe_capabilities(bits: u64) -> String {
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

pub(super) fn persist_config(
    state: &DesktopState,
    config: &AppConfig,
) -> Result<SaveReceipt, IpcError> {
    persist_config_parts(
        &state.runtime,
        &state.store,
        &state.has_saved_config,
        config,
    )
}

pub(super) fn persist_config_parts(
    runtime: &RuntimeHandle,
    store: &ConfigStore,
    has_saved_config: &AtomicBool,
    config: &AppConfig,
) -> Result<SaveReceipt, IpcError> {
    runtime
        .set_persistence_state(app_core::PersistenceState::Saving)
        .map_err(IpcError::from)?;
    match store.save(config) {
        Ok(receipt) => {
            has_saved_config.store(true, Ordering::Release);
            runtime
                .set_persistence_state(app_core::PersistenceState::Clean)
                .map_err(IpcError::from)?;
            Ok(receipt)
        }
        Err(error) => {
            let message = error.to_string();
            let _ = runtime
                .set_persistence_state(app_core::PersistenceState::RecoverableError { message });
            Err(IpcError::from(error))
        }
    }
}

pub(super) fn merge_command_owned_preferences(draft: &mut AppConfig, current: &AppConfig) {
    // Pause and start-at-login have dedicated commands with OS/runtime side effects.
    // Preserve their newest values when a settings draft was opened before a tray or
    // IPC toggle, while still allowing timezone edits through save/apply.
    draft.preferences.paused = current.preferences.paused;
    draft.preferences.autostart = current.preferences.autostart;
}

pub(super) fn autostart_status(
    app: &AppHandle,
    state: &DesktopState,
) -> Result<AutostartStatus, IpcError> {
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

pub(super) fn parse_valid_draft(draft: &DraftPayload) -> Result<AppConfig, IpcError> {
    let config = parse_draft(draft)?;
    config.validate().map_err(|error| IpcError::Validation {
        message: error.to_string(),
        issues: error.issues,
    })?;
    Ok(config)
}

pub(super) fn parse_draft(draft: &DraftPayload) -> Result<AppConfig, IpcError> {
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

#[derive(Clone)]
pub(super) struct ConfigSaveContext {
    pub(super) runtime: Arc<RuntimeHandle>,
    pub(super) store: Arc<ConfigStore>,
    pub(super) network_store: Arc<NetworkSettingsStore>,
    pub(super) networked_config: Arc<NetworkedConfigProjection>,
    pub(super) has_saved_config: Arc<AtomicBool>,
    pub(super) mutation_lock: Arc<Mutex<()>>,
}

impl ConfigSaveContext {
    pub(super) fn from_desktop(state: &DesktopState) -> Self {
        Self {
            runtime: Arc::clone(&state.runtime),
            store: Arc::clone(&state.store),
            network_store: Arc::clone(&state.network_store),
            networked_config: Arc::clone(&state.networked_config),
            has_saved_config: Arc::clone(&state.has_saved_config),
            mutation_lock: Arc::clone(&state.mutation_lock),
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
pub(super) fn validate_draft_for_device(
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
    save_and_apply(ConfigSaveContext::from_desktop(&state), config)
}
