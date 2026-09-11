//! The settings window's card preview.
//!
//! It renders the same scene `build_card_scene` would push to the device, so
//! the preview is what the panel shows by construction. A picture card is
//! not drawn here -- its frames live on the server.
//!
//! Split out of `commands.rs` unchanged on 2026-09-11. The glob keeps name
//! resolution identical to when this was one file.

#[allow(clippy::wildcard_imports)]
use super::*;

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
///
/// Picture cards have no `DisplayTemplate`: their frame is pushed by the external
/// producer and is not reconstructed by this simulator.
#[tauri::command]
pub async fn render_card_preview(
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

    if matches!(card, CardSettings::Picture { .. }) {
        return Ok(unrendered_server_frame(PICTURE_PREVIEW_IS_PUSH_ONLY));
    }

    let data = snapshot
        .card_data
        .iter()
        .find(|data| data.card_id == card_id);
    let (fields, sample) = match data {
        Some(data) if !data.fields.is_empty() => (data.fields.clone(), false),
        _ => (Vec::new(), true),
    };

    // One renderer: this is the scene the device would be pushed for this card.
    let scene = app_core::preview_card_scene(&snapshot.config, &card_id, &fields)
        .map_err(|message| IpcError::Internal { message })?;

    let now = chrono::Utc::now();
    let utc_offset_minutes = utc_offset_minutes(&snapshot.config.preferences.timezone, now)
        .map_err(|message| IpcError::Internal { message })?;

    let request = lvgl_sim::scene::SceneRenderRequest {
        scene,
        assets: Vec::new(),
        utc_offset_minutes,
        now_unix_seconds: now.timestamp(),
        timer: preview_timer(&fields),
        fields: Vec::new(),
        orientation: preview_orientation(snapshot.config.preferences.orientation),
    };
    let png = state
        .preview
        .render(request)
        .map_err(|message| IpcError::Internal { message })?;
    Ok(PreviewFrame {
        png_base64: Some(BASE64_STANDARD.encode(png)),
        sample,
        state: None,
    })
}

/// The orientation the settings-window preview renders at.
///
/// Deliberately NOT the configured mounting. `LandscapeFlipped` is the 180 degree
/// mount, and the simulator reproduces it by reversing the finished frame
/// index-by-index (`lvgl-sim/csrc/sim_shim.c`'s `copy_frame_out`) exactly as the
/// firmware's `LV_DISPLAY_ROTATION_270` does. On the panel that flip is cancelled by
/// the physical mounting, so a person always sees an upright face; rendered into a
/// window that is not itself upside down, it is just upside down. The preview's job is
/// to show what the person will see, so it renders upright for both mountings.
///
/// Because the flip is a pure 180 degree rotation of identical content, nothing is
/// lost by this: the upright frame IS the flipped frame, read the way the viewer
/// reads it.
pub(super) fn preview_orientation(configured: DisplayOrientation) -> lvgl_sim::SimOrientation {
    match configured {
        DisplayOrientation::Landscape | DisplayOrientation::LandscapeFlipped => {
            lvgl_sim::SimOrientation::Landscape
        }
    }
}

/// Mirrors the wire mapping `CardSettings::wire_config` uses for `TemplateKind`
/// (`app-core`'s `config.rs`), except targeting `lvgl_sim::SimTemplate` — the two
/// enums are exhaustively 1:1, so this can never fail to map a `DisplayTemplate` the
/// rest of the app accepts; there is no "unknown template" branch to fall back from.
/// The timer a pomodoro scene's `timer.*` bindings resolve against.
///
/// `None` for a card that pushes no timer fields, which is what the simulator
/// expects for a face with no timer binding. The three keys are the ones
/// `firmware/main/link/protocol_task.c` reads by name to build its own snapshot,
/// so the preview and the panel derive the timer from the same inputs.
pub(super) fn preview_timer(fields: &[CardField]) -> Option<lvgl_sim::scene::SceneTimer> {
    let total = preview_field_integer(fields, "duration_seconds")?;
    let remaining = preview_field_integer(fields, "remaining_seconds").unwrap_or(total);
    Some(lvgl_sim::scene::SceneTimer {
        total_ms: preview_seconds_to_ms(total),
        remaining_ms: preview_seconds_to_ms(remaining),
        running: matches!(
            fields
                .iter()
                .find(|field| field.key == "running")
                .map(|field| &field.value),
            Some(CardFieldValue::Boolean { value: true })
        ),
    })
}

pub(super) fn preview_field_integer(fields: &[CardField], key: &str) -> Option<i64> {
    fields
        .iter()
        .find_map(|field| match (&*field.key, &field.value) {
            (candidate, CardFieldValue::Integer { value }) if candidate == key => Some(*value),
            _ => None,
        })
}

pub(super) fn preview_seconds_to_ms(seconds: i64) -> u32 {
    u32::try_from(seconds.max(0))
        .unwrap_or(u32::MAX)
        .saturating_mul(1_000)
}

pub(crate) const PICTURE_PREVIEW_IS_PUSH_ONLY: &str =
    "Picture cards show the last frame pushed by their source";

pub(super) fn unrendered_server_frame(state: &str) -> PreviewFrame {
    PreviewFrame {
        png_base64: None,
        sample: false,
        state: Some(state.to_owned()),
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

    // This preference has no layout effect, so update it without replacing live
    // pomodoro state.
    state
        .runtime
        .set_autostart_preference(enabled)
        .map_err(IpcError::from)?;
    state
        .tray
        .autostart
        .set_checked(enabled)
        .map_err(|error| IpcError::Window {
            message: format!("cannot update settings window: {error}"),
        })?;
    Ok(AutostartStatus {
        enabled,
        preference_enabled: enabled,
    })
}
