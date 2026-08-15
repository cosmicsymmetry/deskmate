use std::thread::{self, JoinHandle};
use std::time::Duration;

use app_core::{RuntimeError, RuntimeHandle};
use tauri::{AppHandle, Emitter, Manager};

use crate::{DesktopState, MAIN_WINDOW_LABEL};

pub const APP_STATE_EVENT: &str = "app-state";

/// One runtime subscription has a one-snapshot slot. If producers outrun this worker,
/// intermediate snapshots are overwritten while the newest state is retained.
pub fn spawn_state_worker(
    app: &AppHandle,
    runtime: &RuntimeHandle,
) -> Result<JoinHandle<()>, RuntimeError> {
    let subscription = runtime.subscribe()?;
    let app = app.clone();
    thread::Builder::new()
        .name("deskmate-app-state".into())
        .spawn(move || {
            loop {
                match subscription.recv_timeout(Duration::from_secs(1)) {
                    Ok(Some(snapshot)) => {
                        let callback_app = app.clone();
                        let dispatch_app = app.clone();
                        if callback_app
                            .run_on_main_thread(move || {
                                let state = dispatch_app.state::<DesktopState>();
                                let snapshot = state.project_snapshot(snapshot);
                                if state.tray.update(&snapshot.app).is_err() {
                                    eprintln!("cannot update Deskmate tray state");
                                }
                                if dispatch_app
                                    .emit_to(MAIN_WINDOW_LABEL, APP_STATE_EVENT, snapshot)
                                    .is_err()
                                {
                                    eprintln!("cannot project Deskmate app state");
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(None) => {}
                    Err(RuntimeError::WorkerStopped) => break,
                    Err(_) => {
                        eprintln!("Deskmate state subscription failed");
                        break;
                    }
                }
            }
        })
        .map_err(|error| RuntimeError::Device {
            message: format!("cannot start app-state worker: {error}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_state_event_name_is_stable() {
        assert_eq!(APP_STATE_EVENT, "app-state");
    }
}
