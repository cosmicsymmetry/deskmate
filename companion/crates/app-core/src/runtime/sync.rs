//! Getting the device's state to match the host's: the full and incremental
//! synchronize passes, field pushes, screen activation and time sync.
//!
//! Split out of `runtime.rs` unchanged on 2026-09-11. `use super::*` keeps
//! name resolution identical to when this was one file.

// The glob is what makes this file a MOVE rather than a rewrite: name
// resolution inside it is identical to when all of this lived in one
// `runtime.rs`. Enumerating thirty parent imports would make the split a
// diff nobody can read against the original.
#[allow(clippy::wildcard_imports)]
use super::*;

pub(super) fn synchronize_full(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    device: &mut dyn RuntimeDevice,
    now: Instant,
) -> Result<(), RuntimeError> {
    if ownership_was_refused(state) {
        return Ok(());
    }
    state.needs_full_sync = true;
    if !send_time_sync(state, device)? {
        return Ok(());
    }
    let compiled = state
        .config
        .compile(1)
        .expect("runtime config remains validated");
    // Config-declared assets (`CompiledAppConfig.assets` / `AssetSettings`)
    // remain deliberately unwired. `desired_assets()` covers image-source
    // frames only, and AssetRelease is authoritative over the whole device.
    // Mixing the two before config assets have an owner would delete them.
    // An empty desired set is NOT an empty-op reconcile. `AssetRelease`
    // carries the full keep-set, so the device marks every committed record
    // whose digest is absent from it DEAD and compacts it away
    // (`firmware/main/core/asset_store.c`'s `plan_compaction`). An empty list
    // therefore means "wipe every asset you hold", and a server that simply
    // has no picture yet -- a normal state before the first producer push --
    // would issue that wipe on every full synchronize. Skip the pass entirely
    // instead: having nothing to say is not the same as asking for everything
    // to be dropped.
    let asset_result = state.image_source_host.as_mut().and_then(|host| {
        let desired = host.desired_assets();
        if desired.is_empty() {
            return None;
        }
        let digests: Vec<_> = desired.iter().map(|asset| asset.digest).collect();
        Some((
            digests,
            AssetSync::reconcile_with_active_volatile(
                device,
                &desired,
                None,
                state.device.capability_bits(),
            ),
        ))
    });
    match asset_result {
        Some((digests, Ok(_))) => {
            state.confirmed_durable_assets = digests.into_iter().collect();
            clear_asset_sync_refusals(state);
        }
        Some((_, Err(error))) => record_asset_sync_refusals(state, &error.to_string()),
        None => {}
    }
    if !sync_device_result(
        state,
        device.apply_layout(
            compiled.layout.rotation,
            compiled.layout.widgets,
            compiled.layout.screens,
        ),
    )? {
        return Ok(());
    }
    state.dirty_widgets = state.latest_fields.keys().cloned().collect();
    push_dirty_widgets(state, device)?;
    state.active_screen_dirty = state.active_screen.is_some();
    send_screen(state, device)?;
    // `RuntimeCommand::ApplyConfig` waits for this full ownership/model sync
    // before replying. Leave the scene dirty for the worker's separate render
    // phase: activation is the device-model transaction boundary, while drawing
    // the new face is the event-driven consequence of that completed apply.
    flush_interrupts(state, scheduler, device, now)?;
    state.active_scene_dirty = state.active_screen.is_some();
    state.needs_full_sync = false;
    Ok(())
}

const ASSET_SYNC_REFUSAL_PREFIX: &str = "asset reconciliation failed: ";

pub(super) fn record_asset_sync_refusals(state: &mut WorkerState, message: &str) {
    let card_ids: Vec<String> = state
        .config
        .cards
        .iter()
        .filter_map(|card| match card {
            CardSettings::Picture { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect();
    for card_id in card_ids {
        record_scene_refusal(
            state,
            card_id,
            format!("{ASSET_SYNC_REFUSAL_PREFIX}{message}"),
        );
    }
}

pub(super) fn clear_asset_sync_refusals(state: &mut WorkerState) {
    state.push_rejections.retain(|_, error| {
        error.kind != CardErrorKind::SceneRefused
            || !error.message.starts_with(ASSET_SYNC_REFUSAL_PREFIX)
    });
}

pub(super) fn synchronize_pending(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    device: &mut dyn RuntimeDevice,
    now: Instant,
) -> Result<(), RuntimeError> {
    if ownership_was_refused(state) {
        return Ok(());
    }
    if state.needs_full_sync {
        return synchronize_full(state, scheduler, device, now);
    }
    push_dirty_widgets(state, device)?;
    send_screen(state, device)?;
    flush_interrupts(state, scheduler, device, now)
}

/// Pushes every dirty widget, then leaves the dirty set holding only what still
/// needs sending.
///
/// A push the device *refuses* is not a transport failure and must not be retried:
/// the frame arrived, the device parsed it, and it rejected the contents (an
/// undeclared field type, an over-long text value, a value outside the template's
/// declared range). The identical payload can only be refused again, so retrying it
/// pins the runtime in `RuntimeState::Error` forever and starves every widget queued
/// behind it in the same cycle. Such a push is dropped from the dirty set and
/// recorded as a card-scoped `CardError` the settings UI can show against the card
/// that caused it. `Busy` is the one refusal that *is* transient (the device asked
/// the host to come back later), so it stays dirty for the next cycle. Transport
/// errors still propagate — those must reach the reconnect path.
pub(super) fn push_dirty_widgets(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
) -> Result<(), RuntimeError> {
    if ownership_was_refused(state) {
        return Ok(());
    }
    let dirty: Vec<String> = state.dirty_widgets.iter().cloned().collect();
    for widget_id in dirty {
        let Some(fields) = state.latest_fields.get(&widget_id).cloned() else {
            state.dirty_widgets.remove(&widget_id);
            continue;
        };
        match device.push_fields(widget_id.clone(), fields) {
            Ok(()) => {
                state.dirty_widgets.remove(&widget_id);
                if state
                    .push_rejections
                    .get(&widget_id)
                    .is_some_and(|error| error.kind == CardErrorKind::DataRefused)
                {
                    state.push_rejections.remove(&widget_id);
                }
            }
            Err(error) if is_wrong_tier(&error) => {
                mark_ownership_refused(state);
                return Ok(());
            }
            Err(DeviceError::Rejected(error)) if error.code == protocol::ErrorCode::Busy => {}
            Err(DeviceError::Rejected(error)) => {
                state.dirty_widgets.remove(&widget_id);
                state.push_rejections.insert(
                    widget_id.clone(),
                    CardError {
                        kind: CardErrorKind::DataRefused,
                        card_id: widget_id,
                        message: format!(
                            "the display refused this card's data ({:?}): {}",
                            error.code, error.diagnostic
                        ),
                    },
                );
            }
            Err(error) => return Err(device_runtime_error(&error)),
        }
    }
    Ok(())
}

pub(super) fn send_screen(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
) -> Result<(), RuntimeError> {
    if ownership_was_refused(state) {
        return Ok(());
    }
    if !state.active_screen_dirty {
        return Ok(());
    }
    if let Some(screen_id) = state.active_screen.clone()
        && !sync_device_result(state, device.activate_screen(screen_id))?
    {
        return Ok(());
    }
    state.active_screen_dirty = false;
    Ok(())
}

pub(super) fn send_time_sync(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
) -> Result<bool, RuntimeError> {
    if ownership_was_refused(state) {
        return Ok(false);
    }
    let now = Utc::now();
    let offset_minutes = crate::config::utc_offset_minutes(&state.config.preferences.timezone, now)
        .map_err(|message| RuntimeError::Device { message })?;
    sync_device_result(
        state,
        device.time_sync(TimeSync {
            unix_seconds: now.timestamp(),
            utc_offset_minutes: offset_minutes,
        }),
    )
}
