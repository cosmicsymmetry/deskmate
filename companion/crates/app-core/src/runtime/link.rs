//! The device seam: what the runtime may ask of a display and how a failed
//! request is classified.
//!
//! Named `link` rather than `device` because `device` is a CRATE this module
//! imports; a second `device` in one scope is a trap.
//!
//! The error cluster is here rather than beside the worker loop because every
//! one of its decisions is about a `DeviceError` -- whether the link is gone,
//! whether the owner was refused, what the snapshot should then say.
// These runtime internals intentionally share the parent module's worker
// types and helpers; a glob keeps that internal seam in one place.
#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceConnection {
    pub port_name: String,
    pub status: StatusResponse,
}

/// Device boundary used by the background runtime. Implementations retain all
/// successfully-issued replay state across `connect` calls.
pub trait RuntimeDevice: Send + 'static {
    /// Optional notification for transports with an independent event reader.
    /// Poll-only transports keep the scheduler's bounded idle wait.
    fn set_event_waker(&mut self, _wake: std::sync::Arc<dyn Fn() + Send + Sync>) {}
    fn connect(&mut self) -> Result<DeviceConnection, DeviceError>;
    fn status(&mut self) -> Result<StatusResponse, DeviceError>;
    fn time_sync(&mut self, sync: TimeSync) -> Result<(), DeviceError>;
    fn apply_layout(&mut self, rotation: u16, cards: Vec<CardConfig>) -> Result<(), DeviceError>;
    /// The timer a card's `timer.*` scene bindings resolve against. Protocol
    /// v2 replaced the generic field push with this.
    fn push_timer(
        &mut self,
        card_id: String,
        total_ms: u32,
        remaining_ms: u32,
        running: bool,
    ) -> Result<(), DeviceError>;
    fn push_scene(&mut self, push: PushScene) -> Result<(), DeviceError>;
    fn activate_card(&mut self, card_id: String) -> Result<(), DeviceError>;
    fn trigger_interrupt(&mut self, interrupt: TriggerInterrupt) -> Result<(), DeviceError>;
    /// Reserve (or re-attach to) storage for one asset. Asset transfer is part
    /// of the runtime seam because server-owned WebSocket devices receive the
    /// same desired asset set as any other runtime device. The
    /// `already_present` flag on the returned `Ack` is the whole inventory
    /// protocol -- a caller that sees `true` sends no chunks.
    fn send_asset_begin(&mut self, begin: AssetBegin) -> Result<Ack, DeviceError>;
    fn send_asset_chunk(&mut self, chunk: AssetChunk) -> Result<(), DeviceError>;
    fn send_asset_commit(&mut self, commit: AssetCommit) -> Result<(), DeviceError>;
    /// Tell the device the full set of digests that should survive. The
    /// device aborts any transfer still in flight, marks committed records
    /// absent from this set dead, and compacts.
    fn send_asset_release(&mut self, release: AssetRelease) -> Result<(), DeviceError>;
    fn try_recv_event(&mut self) -> Option<ReceivedEvent>;
    fn diagnostics(&self) -> SessionDiagnostics;
}

/// What the host knows about one image source right now.
///
/// `bytes` is the decoded canonical blob, carried as an `Arc` so passing this
/// around costs a pointer rather than 330 KB. The admin preview route returns
/// these exact stored bytes instead of rasterizing an image back into itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSourceFrame {
    pub digest: [u8; protocol::ASSET_DIGEST_LEN],
    pub bytes: Arc<[u8]>,
    /// Inferred from the source's own observed push cadence, server-side.
    pub stale: bool,
}

/// Host boundary used by the background runtime for durable picture frames.
pub trait ImageSourceHost: Send + 'static {
    /// Every image source frame that should remain resident on the device,
    /// deduplicated by digest. Called during a full synchronize.
    fn desired_assets(&mut self) -> Vec<DesiredAsset>;

    /// The frame this image source currently holds, or `None` when nothing has
    /// ever been pushed to it.
    ///
    fn image_source_frame(&mut self, source_id: &str) -> Option<ImageSourceFrame>;
}

pub(super) fn ownership_was_refused(state: &WorkerState) -> bool {
    state.ownership_refused
}

pub(super) fn is_wrong_tier(error: &DeviceError) -> bool {
    matches!(
        error,
        DeviceError::Rejected(response) if response.code == protocol::ErrorCode::WrongTier
    )
}

/// A `WrongTier` refusal is a fresh ownership observation, not a failed sync. Keep all
/// pending host state for a later status-confirmed retry and stop issuing device mutations.
pub(super) fn sync_device_result(
    state: &mut WorkerState,
    result: Result<(), DeviceError>,
) -> Result<bool, RuntimeError> {
    match result {
        Ok(()) => Ok(true),
        Err(error) if is_wrong_tier(&error) => {
            mark_ownership_refused(state);
            Ok(false)
        }
        Err(error) => Err(device_runtime_error(&error)),
    }
}

pub(super) fn mark_ownership_refused(state: &mut WorkerState) {
    state.ownership_refused = true;
    state.device.tier = Some(DeviceTier::Networked);
    state.needs_full_sync = true;
    state.runtime = if state.config.preferences.paused {
        RuntimeState::Paused
    } else {
        RuntimeState::Running
    };
}

pub(super) fn handle_device_error(
    state: &mut WorkerState,
    error: &DeviceError,
    now: Instant,
    reconnect_interval: Duration,
) {
    if is_disconnect(error) {
        mark_disconnected(state, error, now, reconnect_interval);
    } else {
        state.runtime = RuntimeState::Error {
            message: error.to_string(),
        };
    }
}

pub(super) fn mark_disconnected(
    state: &mut WorkerState,
    error: &DeviceError,
    now: Instant,
    reconnect_interval: Duration,
) {
    state.connected = false;
    // Capabilities belong to the new attachment, not the retained runtime. A
    // reconnect may follow an OTA in either direction, so force one render-policy
    // decision from the fresh StatusResponse even when all data is otherwise clean.
    state.active_scene_dirty = state.active_card.is_some();
    state.next_connect = now + reconnect_interval;
    state.device.connection = if state.ever_connected {
        ConnectionState::Standalone
    } else {
        ConnectionState::Disconnected {
            reason: Some(error.to_string()),
        }
    };
}

pub(super) fn is_disconnect(error: &DeviceError) -> bool {
    matches!(error, DeviceError::NoDevice | DeviceError::Transport(_))
}

pub(super) fn device_runtime_error(error: &DeviceError) -> RuntimeError {
    RuntimeError::Device {
        message: error.to_string(),
    }
}

pub(super) fn runtime_command_device_error(
    state: &mut WorkerState,
    error: &DeviceError,
    reconnect_interval: Duration,
) -> RuntimeError {
    if is_disconnect(error) {
        mark_disconnected(state, error, Instant::now(), reconnect_interval);
        RuntimeError::DeviceDisconnected
    } else {
        device_runtime_error(error)
    }
}

pub(super) fn update_device_status(
    state: &mut WorkerState,
    port_name: &str,
    status: &StatusResponse,
    device: &dyn RuntimeDevice,
) {
    let diagnostics = device.diagnostics();
    // The device's interrupt counter is the only thing that constrains the next token:
    // firmware rejects `token <= latest_token` as stale, and that `latest_token` counts
    // accepted interrupts alone. `latest_revision` and `config_revision` are the
    // data-push and config counters, which climb with ordinary traffic.
    //
    // The dedicated field decodes to 0 when absent, so revisions remain the
    // floor only for that case — a device with no interrupt counter to report.
    // Applying them unconditionally would couple tokens to unrelated traffic
    // and make ordinary data revisions look like lost interrupts.
    let observed = if status.latest_interrupt_token == 0 {
        status.latest_revision.max(status.config_revision)
    } else {
        status.latest_interrupt_token
    };
    state.interrupts.advance_latest_token(observed);
    state.device.connection = ConnectionState::Online;
    state.device.port_name = Some(port_name.into());
    state.device.firmware_version = Some(status.firmware_version.clone());
    state.device.protocol_version = Some(status.protocol_version);
    state.device.max_protocol_version = Some(status.max_protocol_version);
    state.device.capabilities = crate::DeviceCapability::from_bits(status.capabilities);
    state.device.unknown_capability_bits =
        status.capabilities & !crate::DeviceCapability::known_bits();
    state.device.uptime_ms = Some(status.uptime_ms);
    state.device.free_heap = Some(status.free_heap);
    state.device.rotation = Some(status.rotation);
    state.device.tier = Some(status.tier.into());
    state.device.wifi_state = Some(status.wifi_state.into());
    state.device.wifi_rssi = Some(status.wifi_rssi);
    state.device.ip = Some(status.ip.clone());
    state
        .device
        .last_network_error
        .clone_from(&status.last_network_error);
    state.device.ota_state = Some(status.ota_state.into());
    state.device.asset_store = status.asset_store.map(|stats| crate::DeviceAssetStore {
        used_bytes: stats.used_bytes,
        free_bytes: stats.free_bytes,
        asset_count: stats.asset_count,
    });
    state.device.volatile_assets = status
        .volatile_assets
        .map(|pool| crate::DeviceVolatileAssets {
            committed_count: pool.committed_count,
            slot_capacity: pool.slot_capacity,
            used_bytes: pool.used_bytes,
            psram_free_bytes: pool.psram_free_bytes,
            psram_low_water_bytes: pool.psram_low_water_bytes,
        });
    state.device.counters = DeviceCounters {
        host_reconnects: diagnostics.reconnects,
        valid_frames: status.valid_frames,
        malformed_frames: status.malformed_frames,
        crc_errors: status.crc_errors,
        overflow_frames: status.overflow_frames,
        dropped_responses: status.dropped_responses,
        rx_dropped_bytes: status.rx_dropped_bytes,
        dropped_events: status.dropped_events,
        event_queue_high_water: status.event_queue_high_water,
        dropped_ui_commands: status.dropped_ui_commands,
        ui_queue_high_water: status.ui_queue_high_water,
        host_dropped_events: diagnostics.locally_dropped_events,
        detected_event_gaps: diagnostics.detected_event_gaps,
    };
    if state.ownership_refused && matches!(state.device.tier, Some(DeviceTier::Local)) {
        state.ownership_refused = false;
        state.needs_full_sync = true;
    }
}
