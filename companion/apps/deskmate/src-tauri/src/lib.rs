use std::error::Error;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use app_core::{
    AppConfig, AppSnapshot, ConfigOrigin, ConfigStore, ConnectionState, DeviceTier, LoadOutcome,
    NetworkSettingsStore, PersistenceState, ProviderState, RuntimeHandle, RuntimeState,
    SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE,
};
use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{App, AppHandle, Manager, RunEvent, WindowEvent, Wry};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

mod commands;
mod events;
mod preview;

const CONFIG_FILE_NAME: &str = "config.json";
const NETWORK_SETTINGS_FILE_NAME: &str = "network-settings.json";
const MAIN_WINDOW_LABEL: &str = "main";
const TRAY_ID: &str = "deskmate";
const STATUS_ITEM_ID: &str = "device-status";
const OPEN_ITEM_ID: &str = "open-settings";
const AUTOSTART_ITEM_ID: &str = "autostart";
const QUIT_ITEM_ID: &str = "quit";

const TRAY_ONLINE: &[u8] = include_bytes!("../icons/tray-online.png");
const TRAY_OFFLINE: &[u8] = include_bytes!("../icons/tray-offline.png");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayAction {
    OpenSettings,
    ToggleAutostart,
    Quit,
}

impl TrayAction {
    fn from_id(id: &str) -> Option<Self> {
        match id {
            OPEN_ITEM_ID => Some(Self::OpenSettings),
            AUTOSTART_ITEM_ID => Some(Self::ToggleAutostart),
            QUIT_ITEM_ID => Some(Self::Quit),
            _ => None,
        }
    }
}

struct TrayPresentation {
    device_text: &'static str,
    tooltip: String,
    online: bool,
}

impl TrayPresentation {
    fn from_snapshot(snapshot: &AppSnapshot) -> Self {
        let (device_text, online) = connection_presentation(&snapshot.device.connection);
        let paused =
            snapshot.config.preferences.paused || matches!(snapshot.runtime, RuntimeState::Paused);
        let tooltip = match &snapshot.device.connection {
            ConnectionState::Disconnected {
                reason: Some(reason),
            } => format!("Deskmate - {reason}"),
            _ if paused => "Deskmate - pushing paused".into(),
            _ => device_text.into(),
        };
        Self {
            device_text,
            tooltip,
            online,
        }
    }
}

fn connection_presentation(connection: &ConnectionState) -> (&'static str, bool) {
    match connection {
        ConnectionState::Online => ("Device: Connected", true),
        ConnectionState::Connecting => ("Device: Connecting...", false),
        ConnectionState::Standalone => ("Device: Standalone", false),
        ConnectionState::Disconnected { .. } => ("Device: Disconnected", false),
    }
}

struct TrayControls {
    status: MenuItem<Wry>,
    autostart: CheckMenuItem<Wry>,
    tray: TrayIcon<Wry>,
}

impl TrayControls {
    fn update(&self, snapshot: &AppSnapshot) -> tauri::Result<()> {
        let presentation = TrayPresentation::from_snapshot(snapshot);
        self.status.set_text(presentation.device_text)?;
        self.tray.set_tooltip(Some(presentation.tooltip))?;
        self.tray.set_icon(Some(tray_image(presentation.online)?))?;
        Ok(())
    }
}

#[derive(Default)]
struct NetworkedConfigProjection(Mutex<Option<AppConfig>>);

impl NetworkedConfigProjection {
    fn project(&self, tier: Option<DeviceTier>, config: &mut AppConfig) {
        if !matches!(tier, Some(DeviceTier::Networked)) {
            return;
        }
        if let Ok(networked) = self.0.lock()
            && let Some(networked) = networked.as_ref()
        {
            *config = networked.clone();
        }
    }

    fn replace(&self, config: Option<AppConfig>) -> Result<(), commands::IpcError> {
        *self.0.lock().map_err(|_| commands::IpcError::Internal {
            message: "networked configuration state is unavailable".into(),
        })? = config;
        Ok(())
    }

    fn clear_for_local_save(&self) -> Result<(), commands::IpcError> {
        self.replace(None)
    }
}

struct DesktopSnapshotProjector {
    network_store: Arc<NetworkSettingsStore>,
    networked_config: Arc<NetworkedConfigProjection>,
    last_known_tier: Mutex<Option<DeviceTier>>,
    has_saved_config: Arc<AtomicBool>,
}

impl DesktopSnapshotProjector {
    fn project_snapshot(&self, mut app: AppSnapshot) -> DesktopSnapshot {
        if let Some(tier) = app.device.tier {
            self.remember_device_tier(tier);
        }
        self.networked_config
            .project(app.device.tier, &mut app.config);
        DesktopSnapshot {
            app,
            has_saved_config: self.has_saved_config.load(Ordering::Acquire),
        }
    }

    fn remember_device_tier(&self, tier: DeviceTier) {
        let Ok(mut last_known) = self.last_known_tier.lock() else {
            eprintln!("cannot remember display ownership tier");
            return;
        };
        if *last_known == Some(tier) {
            return;
        }
        let current = self.network_store.load().settings().clone();
        if self
            .network_store
            .save(app_core::NetworkSettingsUpdate::new(
                current.server_url,
                current.device_id,
                Some(tier),
                None,
            ))
            .is_err()
        {
            eprintln!("cannot persist display ownership tier");
            return;
        }
        *last_known = Some(tier);
    }
}

struct DesktopState {
    runtime: Arc<RuntimeHandle>,
    store: Arc<ConfigStore>,
    network_store: Arc<NetworkSettingsStore>,
    networked_config: Arc<NetworkedConfigProjection>,
    snapshot_projector: DesktopSnapshotProjector,
    server_client: ureq::Agent,
    has_saved_config: Arc<AtomicBool>,
    tray: TrayControls,
    snapshot_worker: Mutex<Option<JoinHandle<()>>>,
    mutation_lock: Arc<Mutex<()>>,
    quitting: AtomicBool,
    preview: preview::PreviewHandle,
}

/// The desktop IPC projection adds the one piece of persistence history the runtime
/// intentionally does not own: whether settings have ever existed on disk. Flattening
/// keeps the wire shape compatible with the frontend's single `AppSnapshot` DTO.
#[derive(Debug, Clone, Serialize)]
struct DesktopSnapshot {
    #[serde(flatten)]
    app: AppSnapshot,
    has_saved_config: bool,
}

impl DesktopState {
    fn project_snapshot(&self, app: AppSnapshot) -> DesktopSnapshot {
        self.snapshot_projector.project_snapshot(app)
    }

    fn set_networked_config(&self, config: Option<AppConfig>) -> Result<(), commands::IpcError> {
        self.networked_config.replace(config)
    }

    fn toggle_autostart(&self, app: &AppHandle) -> Result<(), commands::IpcError> {
        let enabled = !app
            .autolaunch()
            .is_enabled()
            .map_err(commands::autostart_error)?;
        commands::set_autostart(app, self, enabled).map(|_| ())
    }

    fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::Acquire)
    }

    fn shutdown(&self) {
        if self.quitting.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Ok(snapshot) = self.runtime.snapshot() {
            report_exit_metrics(&snapshot);
        }
        if let Err(error) = self.runtime.shutdown() {
            eprintln!(
                "failed to stop Deskmate runtime cleanly: {}",
                runtime_error_log_label(&error)
            );
        }
        if let Ok(mut worker) = self.snapshot_worker.lock()
            && let Some(worker) = worker.take()
            && worker.join().is_err()
        {
            eprintln!("Deskmate tray-state worker panicked during shutdown");
        }
    }
}

fn report_exit_metrics(snapshot: &AppSnapshot) {
    let mut provider_fresh = 0usize;
    let mut provider_stale = 0usize;
    let mut provider_error = 0usize;
    for provider in &snapshot.providers {
        match provider.state {
            ProviderState::Fresh => provider_fresh += 1,
            ProviderState::Stale { .. } => provider_stale += 1,
            ProviderState::Error { .. } => provider_error += 1,
            ProviderState::Idle | ProviderState::Refreshing => {}
        }
    }
    eprintln!(
        "Deskmate exit metrics: runtime={} connection={} uptime_ms={:?} free_heap={:?} active_screen_present={} providers=fresh:{provider_fresh},stale:{provider_stale},error:{provider_error} counters={:?} runtime_diagnostics={:?}",
        runtime_log_label(&snapshot.runtime),
        connection_log_label(&snapshot.device.connection),
        snapshot.device.uptime_ms,
        snapshot.device.free_heap,
        snapshot.device.active_screen_id.is_some(),
        snapshot.device.counters,
        snapshot.diagnostics,
    );
}

const fn runtime_log_label(runtime: &RuntimeState) -> &'static str {
    match runtime {
        RuntimeState::Starting => "starting",
        RuntimeState::Running => "running",
        RuntimeState::Paused => "paused",
        RuntimeState::Error { .. } => "error",
    }
}

const fn connection_log_label(connection: &ConnectionState) -> &'static str {
    match connection {
        ConnectionState::Disconnected { .. } => "disconnected",
        ConnectionState::Connecting => "connecting",
        ConnectionState::Online => "online",
        ConnectionState::Standalone => "standalone",
    }
}

const fn runtime_error_log_label(error: &app_core::RuntimeError) -> &'static str {
    match error {
        app_core::RuntimeError::InvalidConfig { .. } => "invalid-config",
        app_core::RuntimeError::QueueFull => "queue-full",
        app_core::RuntimeError::WorkerStopped => "worker-stopped",
        app_core::RuntimeError::ResponseTimeout => "response-timeout",
        app_core::RuntimeError::UnknownWidget { .. } => "unknown-widget",
        app_core::RuntimeError::UnknownScreen { .. } => "unknown-screen",
        app_core::RuntimeError::UnknownCard { .. } => "unknown-card",
        app_core::RuntimeError::NotAPluginCard { .. } => "not-a-plugin-card",
        app_core::RuntimeError::DeviceDisconnected | app_core::RuntimeError::Device { .. } => {
            "device"
        }
        app_core::RuntimeError::Provider { .. } => "provider",
    }
}

fn tray_image(online: bool) -> tauri::Result<Image<'static>> {
    Image::from_bytes(if online { TRAY_ONLINE } else { TRAY_OFFLINE })
}

fn create_tray(
    app: &App,
    snapshot: &AppSnapshot,
    autostart_enabled: bool,
) -> tauri::Result<TrayControls> {
    let presentation = TrayPresentation::from_snapshot(snapshot);
    let status = MenuItem::with_id(
        app,
        STATUS_ITEM_ID,
        presentation.device_text,
        false,
        None::<&str>,
    )?;
    let open = MenuItem::with_id(app, OPEN_ITEM_ID, "Open settings", true, None::<&str>)?;
    let autostart = CheckMenuItem::with_id(
        app,
        AUTOSTART_ITEM_ID,
        "Start at login",
        true,
        autostart_enabled,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, QUIT_ITEM_ID, "Quit Deskmate", true, None::<&str>)?;
    let separator_one = PredefinedMenuItem::separator(app)?;
    let separator_two = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &separator_one,
            &open,
            &autostart,
            &separator_two,
            &quit,
        ],
    )?;

    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .icon(tray_image(presentation.online)?)
        .icon_as_template(true)
        .tooltip(presentation.tooltip)
        .on_menu_event(|app, event| handle_tray_action(app, event.id().as_ref()))
        .build(app)?;

    Ok(TrayControls {
        status,
        autostart,
        tray,
    })
}

fn handle_tray_action(app: &AppHandle, id: &str) {
    let Some(action) = TrayAction::from_id(id) else {
        return;
    };
    match action {
        TrayAction::OpenSettings => show_settings(app),
        TrayAction::ToggleAutostart => {
            let state = app.state::<DesktopState>();
            if let Err(error) = state.toggle_autostart(app) {
                eprintln!("cannot change autostart: {}", error.log_label());
                if let Ok(enabled) = app.autolaunch().is_enabled() {
                    let _ = state.tray.autostart.set_checked(enabled);
                }
            }
        }
        TrayAction::Quit => {
            app.state::<DesktopState>().shutdown();
            app.exit(0);
        }
    }
}

fn show_settings(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

fn setup_app(app: &mut App) -> Result<(), Box<dyn Error>> {
    let config_directory = app.path().app_data_dir()?;
    prepare_config_directory(&config_directory);
    let config_path = config_directory.join(CONFIG_FILE_NAME);
    let store = Arc::new(ConfigStore::new(config_path));
    let network_store = Arc::new(NetworkSettingsStore::new(
        config_directory.join(NETWORK_SETTINGS_FILE_NAME),
    ));
    let network_settings = network_store.load();
    if network_settings.recovery().is_some() {
        eprintln!("saved Deskmate network settings could not be read");
    }
    if network_store.discard_device_token().is_err() {
        eprintln!("saved Deskmate device credential could not be discarded");
    }
    let last_known_tier = network_settings.settings().tier;
    let loaded = store.load();
    let first_run = is_first_run(&loaded);
    let has_saved_config = !first_run;
    let auto_open_settings = first_run;
    let persistence = load_failure_persistence(&loaded);
    if let Some(persistence) = &persistence {
        eprintln!(
            "saved Deskmate configuration was not applied: {}",
            persistence_log_label(persistence)
        );
    }

    // The window is configured hidden, so the single owner is running before any
    // settings UI appears. The explicit Tauri app-data path keeps filesystem policy
    // out of app-core.
    let runtime = Arc::new(RuntimeHandle::start_serial(loaded.into_config(), None)?);
    if let Some(persistence) = persistence {
        runtime.set_persistence_state(persistence)?;
    }
    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or_else(|_| {
        eprintln!("cannot read autostart state");
        false
    });
    let initial_snapshot = runtime.snapshot()?;
    let tray = create_tray(app, &initial_snapshot, autostart_enabled)?;
    let networked_config = Arc::new(NetworkedConfigProjection::default());
    let has_saved_config = Arc::new(AtomicBool::new(has_saved_config));
    let snapshot_projector = DesktopSnapshotProjector {
        network_store: Arc::clone(&network_store),
        networked_config: Arc::clone(&networked_config),
        last_known_tier: Mutex::new(last_known_tier),
        has_saved_config: Arc::clone(&has_saved_config),
    };

    app.manage(DesktopState {
        runtime: Arc::clone(&runtime),
        store,
        network_store,
        networked_config,
        snapshot_projector,
        server_client: server_http_agent(),
        has_saved_config,
        tray,
        snapshot_worker: Mutex::new(None),
        mutation_lock: Arc::new(Mutex::new(())),
        quitting: AtomicBool::new(false),
        // One dedicated thread owns the process-wide `Simulator` for the app's
        // lifetime (see `preview` module docs); nothing else may construct one.
        preview: preview::spawn(),
    });

    let worker = events::spawn_state_worker(app.handle(), &runtime)?;
    app.state::<DesktopState>()
        .snapshot_worker
        .lock()
        .map_err(|_| "tray-state worker lock poisoned")?
        .replace(worker);
    // First-run discoverability only; all other launches remain tray-only.
    if auto_open_settings {
        show_settings(app.handle());
    }
    Ok(())
}

fn prepare_config_directory(path: &Path) {
    if secure_config_directory(path).is_err() {
        // Keep startup available so ConfigStore can surface a typed persistence
        // error. Permission hardening must not turn a recoverable filesystem state
        // into an application panic.
        eprintln!("cannot secure Deskmate configuration directory");
    }
}

fn server_http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        // Validation details live in the 422 response body, so status handling
        // belongs in the typed IPC mapper after the bounded body is read.
        .http_status_as_error(false)
        .build()
        .into()
}

fn secure_config_directory(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Only a clean defaults load means no document existed. Recovery and validation
/// failures still came from a persisted document, so they must not turn an existing
/// installation back into first-run mode.
fn is_first_run(loaded: &LoadOutcome) -> bool {
    matches!(
        loaded,
        LoadOutcome::Loaded {
            origin: ConfigOrigin::Defaults,
            ..
        }
    )
}

fn load_failure_persistence(loaded: &LoadOutcome) -> Option<PersistenceState> {
    match loaded {
        LoadOutcome::Loaded { .. } => None,
        LoadOutcome::Recovered { error, .. } => Some(PersistenceState::RecoverableError {
            message: error.to_string(),
        }),
        LoadOutcome::ValidationFailed { issues, .. } => Some(PersistenceState::ValidationFailed {
            message: SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE.into(),
            issues: issues.clone(),
        }),
    }
}

const fn persistence_log_label(persistence: &PersistenceState) -> &'static str {
    match persistence {
        PersistenceState::Clean => "clean",
        PersistenceState::Saving => "saving",
        PersistenceState::RecoverableError { .. } => "recoverable-error",
        PersistenceState::ValidationFailed { .. } => "validation-failed",
    }
}

pub fn run() {
    let app = tauri::Builder::default()
        // Tauri requires single-instance to be registered before every other plugin.
        .plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _cwd| {
                show_settings(app);
            },
        ))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_app_snapshot,
            commands::validate_config_draft,
            commands::save_apply_config,
            commands::save_server_config,
            commands::get_network_settings,
            commands::set_server_endpoint,
            commands::provision_device,
            commands::factory_reset_device,
            commands::use_local_ownership,
            commands::resume_pushing,
            commands::control_pomodoro,
            commands::refresh_provider,
            commands::choose_ics_file,
            commands::get_autostart_status,
            commands::set_autostart_enabled,
            commands::render_card_preview,
        ])
        .setup(setup_app)
        .build(tauri::generate_context!())
        .expect("failed to build Deskmate desktop app");

    app.run(|app, event| match event {
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } if label == MAIN_WINDOW_LABEL => {
            let state = app.state::<DesktopState>();
            if !state.is_quitting() {
                api.prevent_close();
                if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
                    let _ = window.hide();
                }
            }
        }
        RunEvent::ExitRequested { api, .. } => {
            if !app.state::<DesktopState>().is_quitting() {
                api.prevent_exit();
            }
        }
        RunEvent::Exit => app.state::<DesktopState>().shutdown(),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_core::{AppConfig, ConfigOrigin, StoreError, ValidationCode, ValidationIssue};

    #[test]
    fn tray_ids_map_only_to_supported_actions() {
        assert_eq!(
            TrayAction::from_id(OPEN_ITEM_ID),
            Some(TrayAction::OpenSettings)
        );
        assert_eq!(
            TrayAction::from_id(AUTOSTART_ITEM_ID),
            Some(TrayAction::ToggleAutostart)
        );
        assert_eq!(TrayAction::from_id(QUIT_ITEM_ID), Some(TrayAction::Quit));
        assert_eq!(TrayAction::from_id(STATUS_ITEM_ID), None);
        assert_eq!(TrayAction::from_id("pause-pushing"), None);
        assert_eq!(TrayAction::from_id("unknown"), None);
    }

    #[test]
    fn device_copy_covers_each_connection_state() {
        assert_eq!(
            connection_presentation(&ConnectionState::Online),
            ("Device: Connected", true)
        );
        assert_eq!(
            connection_presentation(&ConnectionState::Connecting),
            ("Device: Connecting...", false)
        );
        assert_eq!(
            connection_presentation(&ConnectionState::Standalone),
            ("Device: Standalone", false)
        );
        assert_eq!(
            connection_presentation(&ConnectionState::Disconnected { reason: None }),
            ("Device: Disconnected", false)
        );
    }

    #[test]
    fn validation_load_failure_remains_typed_in_snapshot_persistence() {
        let issue = ValidationIssue {
            path: "active_playlist_id".into(),
            code: ValidationCode::MissingReference,
            message: "active playlist does not exist".into(),
        };
        let loaded = LoadOutcome::ValidationFailed {
            config: AppConfig::default(),
            origin: ConfigOrigin::Defaults,
            issues: vec![issue.clone()],
        };

        assert_eq!(
            load_failure_persistence(&loaded),
            Some(PersistenceState::ValidationFailed {
                message: SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE.into(),
                issues: vec![issue],
            })
        );

        let recovered = LoadOutcome::Recovered {
            config: AppConfig::default(),
            origin: ConfigOrigin::Defaults,
            error: StoreError::InvalidJson {
                message: "expected value".into(),
            },
        };
        assert!(matches!(
            load_failure_persistence(&recovered),
            Some(PersistenceState::RecoverableError { .. })
        ));
        assert_eq!(
            load_failure_persistence(&LoadOutcome::Loaded {
                config: AppConfig::default(),
                origin: ConfigOrigin::Defaults,
            }),
            None
        );
    }

    #[test]
    fn a_local_save_clears_the_server_projection_and_unknown_tier_never_projects_it() {
        let projection = NetworkedConfigProjection::default();
        let mut server_config = AppConfig::default();
        server_config.preferences.timezone = "Asia/Tbilisi".into();
        projection.replace(Some(server_config.clone())).unwrap();

        let mut unplugged_config = AppConfig::default();
        projection.project(None, &mut unplugged_config);
        assert_eq!(unplugged_config, AppConfig::default());

        projection.clear_for_local_save().unwrap();
        let mut later_networked_config = AppConfig::default();
        projection.project(Some(DeviceTier::Networked), &mut later_networked_config);
        assert_eq!(later_networked_config, AppConfig::default());
    }

    #[test]
    fn projecting_an_observed_tier_persists_it_for_the_next_cable_out_snapshot() {
        use app_core::{DeviceCounters, DeviceSnapshot, NetworkSettingsUpdate};
        use std::time::{SystemTime, UNIX_EPOCH};

        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "deskmate-tier-projection-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let network_store = Arc::new(NetworkSettingsStore::new(
            directory.join("network-settings.json"),
        ));
        network_store
            .save(NetworkSettingsUpdate::new(
                "https://desk.example",
                "desk-1",
                Some(DeviceTier::Local),
                None,
            ))
            .unwrap();
        let projector = DesktopSnapshotProjector {
            network_store: Arc::clone(&network_store),
            networked_config: Arc::new(NetworkedConfigProjection::default()),
            last_known_tier: Mutex::new(Some(DeviceTier::Local)),
            has_saved_config: Arc::new(AtomicBool::new(false)),
        };
        let snapshot = AppSnapshot {
            config: AppConfig::default(),
            runtime: RuntimeState::Running,
            device: DeviceSnapshot {
                connection: ConnectionState::Online,
                port_name: Some("test-port".into()),
                firmware_version: Some("2.0.0".into()),
                protocol_version: Some(1),
                max_protocol_version: Some(1),
                capabilities: Vec::new(),
                unknown_capability_bits: 0,
                uptime_ms: None,
                free_heap: None,
                rotation: None,
                tier: Some(DeviceTier::Networked),
                wifi_state: None,
                wifi_rssi: None,
                ip: None,
                last_network_error: None,
                ota_state: None,
                active_screen_id: None,
                counters: DeviceCounters::default(),
            },
            providers: Vec::new(),
            pomodoros: Vec::new(),
            card_data: Vec::new(),
            card_errors: Vec::new(),
            persistence: PersistenceState::Clean,
            diagnostics: app_core::RuntimeDiagnostics::default(),
        };

        let projected = projector.project_snapshot(snapshot);

        assert_eq!(projected.app.device.tier, Some(DeviceTier::Networked));
        assert_eq!(
            network_store.load().settings().tier,
            Some(DeviceTier::Networked)
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn only_a_missing_settings_document_is_first_run() {
        let defaults = LoadOutcome::Loaded {
            config: AppConfig::default(),
            origin: ConfigOrigin::Defaults,
        };
        let current = LoadOutcome::Loaded {
            config: AppConfig::default(),
            origin: ConfigOrigin::Current,
        };
        let recovered = LoadOutcome::Recovered {
            config: AppConfig::default(),
            origin: ConfigOrigin::Defaults,
            error: StoreError::InvalidJson {
                message: "expected value".into(),
            },
        };
        let validation_failed = LoadOutcome::ValidationFailed {
            config: AppConfig::default(),
            origin: ConfigOrigin::Defaults,
            issues: vec![ValidationIssue {
                path: "active_playlist_id".into(),
                code: ValidationCode::MissingReference,
                message: "active playlist does not exist".into(),
            }],
        };

        assert!(is_first_run(&defaults));
        assert!(!is_first_run(&current));
        assert!(!is_first_run(&recovered));
        assert!(!is_first_run(&validation_failed));
    }

    #[test]
    fn diagnostic_labels_do_not_include_runtime_or_config_messages() {
        let runtime_secret = "calendar: Private appointment";
        let runtime = RuntimeState::Error {
            message: runtime_secret.into(),
        };
        assert_eq!(runtime_log_label(&runtime), "error");
        assert!(!runtime_log_label(&runtime).contains(runtime_secret));
        let runtime_error = app_core::RuntimeError::Device {
            message: runtime_secret.into(),
        };
        assert_eq!(runtime_error_log_label(&runtime_error), "device");
        assert_eq!(
            runtime_error_log_label(&app_core::RuntimeError::DeviceDisconnected),
            "device"
        );
        let ipc_error = commands::IpcError::Device {
            message: runtime_secret.into(),
        };
        assert_eq!(ipc_error.log_label(), "device");

        let config_secret = "private-playlist";
        let persistence = PersistenceState::ValidationFailed {
            message: config_secret.into(),
            issues: vec![ValidationIssue {
                path: "playlists[0].name".into(),
                code: ValidationCode::DuplicateId,
                message: config_secret.into(),
            }],
        };
        assert_eq!(persistence_log_label(&persistence), "validation-failed");
        assert!(!persistence_log_label(&persistence).contains(config_secret));
    }

    #[cfg(unix)]
    #[test]
    fn config_directory_is_user_only() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{SystemTime, UNIX_EPOCH};

        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "deskmate-config-permissions-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();

        prepare_config_directory(&path);

        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::remove_dir(path).unwrap();
    }

    #[test]
    fn config_directory_preparation_failure_is_nonfatal() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let blocking_file = std::env::temp_dir().join(format!(
            "deskmate-config-permissions-blocker-{}-{serial}",
            std::process::id()
        ));
        std::fs::write(&blocking_file, b"not a directory").unwrap();
        let unavailable_directory = blocking_file.join("config");

        prepare_config_directory(&unavailable_directory);

        assert!(!unavailable_directory.exists());
        std::fs::remove_file(blocking_file).unwrap();
    }
}
