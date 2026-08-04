use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use app_core::{
    AppSnapshot, ConfigStore, ConnectionState, RuntimeError, RuntimeHandle, RuntimeState,
};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{App, AppHandle, Manager, RunEvent, WindowEvent, Wry};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

const CONFIG_FILE_NAME: &str = "config.json";
const MAIN_WINDOW_LABEL: &str = "main";
const TRAY_ID: &str = "deskmate";
const STATUS_ITEM_ID: &str = "device-status";
const PAUSE_ITEM_ID: &str = "pause-pushing";
const OPEN_ITEM_ID: &str = "open-settings";
const AUTOSTART_ITEM_ID: &str = "autostart";
const QUIT_ITEM_ID: &str = "quit";

const TRAY_ONLINE: &[u8] = include_bytes!("../icons/tray-online.png");
const TRAY_OFFLINE: &[u8] = include_bytes!("../icons/tray-offline.png");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayAction {
    TogglePause,
    OpenSettings,
    ToggleAutostart,
    Quit,
}

impl TrayAction {
    fn from_id(id: &str) -> Option<Self> {
        match id {
            PAUSE_ITEM_ID => Some(Self::TogglePause),
            OPEN_ITEM_ID => Some(Self::OpenSettings),
            AUTOSTART_ITEM_ID => Some(Self::ToggleAutostart),
            QUIT_ITEM_ID => Some(Self::Quit),
            _ => None,
        }
    }
}

struct TrayPresentation {
    device_text: &'static str,
    pause_text: &'static str,
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
            pause_text: pause_menu_text(paused),
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

const fn pause_menu_text(paused: bool) -> &'static str {
    if paused {
        "Resume pushing"
    } else {
        "Pause pushing"
    }
}

struct TrayControls {
    status: MenuItem<Wry>,
    pause: MenuItem<Wry>,
    autostart: CheckMenuItem<Wry>,
    tray: TrayIcon<Wry>,
}

impl TrayControls {
    fn update(&self, snapshot: &AppSnapshot) -> tauri::Result<()> {
        let presentation = TrayPresentation::from_snapshot(snapshot);
        self.status.set_text(presentation.device_text)?;
        self.pause.set_text(presentation.pause_text)?;
        self.tray.set_tooltip(Some(presentation.tooltip))?;
        self.tray.set_icon(Some(tray_image(presentation.online)?))?;
        Ok(())
    }
}

struct DesktopState {
    runtime: Arc<RuntimeHandle>,
    store: ConfigStore,
    tray: TrayControls,
    snapshot_worker: Mutex<Option<JoinHandle<()>>>,
    quitting: AtomicBool,
}

impl DesktopState {
    fn toggle_paused(&self) -> Result<(), Box<dyn Error>> {
        let before = self.runtime.snapshot()?;
        let was_paused = before.config.preferences.paused;
        let paused = !was_paused;
        self.runtime.set_paused(paused)?;

        let mut updated = before.config;
        updated.preferences.paused = paused;
        if let Err(error) = self.store.save(&updated) {
            let _ = self.runtime.set_paused(was_paused);
            return Err(Box::new(error));
        }
        Ok(())
    }

    fn toggle_autostart(&self, app: &AppHandle) -> Result<(), Box<dyn Error>> {
        let mut updated = self.runtime.snapshot()?.config;
        let manager = app.autolaunch();
        let was_enabled = manager.is_enabled()?;
        let enabled = !was_enabled;
        if enabled {
            manager.enable()?;
        } else {
            manager.disable()?;
        }

        updated.preferences.autostart = enabled;
        if let Err(error) = self.store.save(&updated) {
            if was_enabled {
                let _ = manager.enable();
            } else {
                let _ = manager.disable();
            }
            return Err(Box::new(error));
        }

        let runtime_result = self.runtime.apply_config(updated);
        let tray_result = self.tray.autostart.set_checked(enabled);
        if let Err(error) = runtime_result {
            // The runtime replaces its in-memory config before attempting device I/O.
            // Keep the valid saved preference queued for reconnect and report the
            // transport error without rolling back the user's OS-level choice.
            return Err(Box::new(error));
        }
        tray_result?;
        Ok(())
    }

    fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::Acquire)
    }

    fn shutdown(&self) {
        if self.quitting.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Err(error) = self.runtime.shutdown() {
            eprintln!("failed to stop Deskmate runtime cleanly: {error}");
        }
        if let Ok(mut worker) = self.snapshot_worker.lock()
            && let Some(worker) = worker.take()
            && worker.join().is_err()
        {
            eprintln!("Deskmate tray-state worker panicked during shutdown");
        }
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
    let pause = MenuItem::with_id(
        app,
        PAUSE_ITEM_ID,
        presentation.pause_text,
        true,
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
            &pause,
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
        pause,
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
        TrayAction::TogglePause => {
            let state = app.state::<DesktopState>();
            if let Err(error) = state.toggle_paused() {
                eprintln!("cannot change pause state: {error}");
            }
        }
        TrayAction::ToggleAutostart => {
            let state = app.state::<DesktopState>();
            if let Err(error) = state.toggle_autostart(app) {
                eprintln!("cannot change autostart: {error}");
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

fn spawn_snapshot_worker(
    app: &AppHandle,
    runtime: &RuntimeHandle,
) -> Result<JoinHandle<()>, RuntimeError> {
    let subscription = runtime.subscribe()?;
    let app = app.clone();
    thread::Builder::new()
        .name("deskmate-tray-state".into())
        .spawn(move || {
            loop {
                match subscription.recv_timeout(Duration::from_secs(1)) {
                    Ok(Some(snapshot)) => {
                        let callback_app = app.clone();
                        let dispatch_app = app.clone();
                        if callback_app
                            .run_on_main_thread(move || {
                                let state = dispatch_app.state::<DesktopState>();
                                if let Err(error) = state.tray.update(&snapshot) {
                                    eprintln!("cannot update Deskmate tray state: {error}");
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(None) => {}
                    Err(RuntimeError::WorkerStopped) => break,
                    Err(error) => {
                        eprintln!("Deskmate state subscription failed: {error}");
                        break;
                    }
                }
            }
        })
        .map_err(|error| RuntimeError::Device {
            message: format!("cannot start tray-state worker: {error}"),
        })
}

fn setup_app(app: &mut App) -> Result<(), Box<dyn Error>> {
    let config_path = app.path().app_data_dir()?.join(CONFIG_FILE_NAME);
    let store = ConfigStore::new(config_path);
    let loaded = store.load();
    if let Some(recovery) = &loaded.recovery {
        eprintln!("using last-good Deskmate configuration: {recovery:?}");
    }

    // The window is configured hidden, so the single owner is running before any
    // settings UI appears. The explicit Tauri app-data path keeps filesystem policy
    // out of app-core.
    let runtime = Arc::new(RuntimeHandle::start_serial(loaded.config, None)?);
    let autostart_enabled = match app.autolaunch().is_enabled() {
        Ok(enabled) => enabled,
        Err(error) => {
            eprintln!("cannot read autostart state: {error}");
            false
        }
    };
    let initial_snapshot = runtime.snapshot()?;
    let tray = create_tray(app, &initial_snapshot, autostart_enabled)?;

    app.manage(DesktopState {
        runtime: Arc::clone(&runtime),
        store,
        tray,
        snapshot_worker: Mutex::new(None),
        quitting: AtomicBool::new(false),
    });

    let worker = spawn_snapshot_worker(app.handle(), &runtime)?;
    app.state::<DesktopState>()
        .snapshot_worker
        .lock()
        .map_err(|_| "tray-state worker lock poisoned")?
        .replace(worker);
    show_settings(app.handle());
    Ok(())
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

    #[test]
    fn tray_ids_map_only_to_supported_actions() {
        assert_eq!(
            TrayAction::from_id(PAUSE_ITEM_ID),
            Some(TrayAction::TogglePause)
        );
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
        assert_eq!(TrayAction::from_id("unknown"), None);
    }

    #[test]
    fn device_and_pause_copy_cover_each_runtime_state() {
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
        assert_eq!(pause_menu_text(false), "Pause pushing");
        assert_eq!(pause_menu_text(true), "Resume pushing");
    }
}
