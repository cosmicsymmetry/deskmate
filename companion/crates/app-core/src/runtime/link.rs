//! The device seam: what the runtime may ask of a display, the serial
//! implementation of it, and how a failed request is classified.
//!
//! Named `link` rather than `device` because `device` is a CRATE this module
//! imports; a second `device` in one scope is a trap.
//!
//! The error cluster is here rather than beside the worker loop because every
//! one of its decisions is about a `DeviceError` -- whether the link is gone,
//! whether the owner was refused, what the snapshot should then say.
//!
//! Split out of `runtime.rs` unchanged on 2026-09-11. `use super::*` keeps
//! name resolution identical to when this was one file.

// The glob is what makes this file a MOVE rather than a rewrite: name
// resolution inside it is identical to when all of this lived in one
// `runtime.rs`. Enumerating thirty parent imports would make the split a
// diff nobody can read against the original.
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
    fn connect(&mut self) -> Result<DeviceConnection, DeviceError>;
    fn status(&mut self) -> Result<StatusResponse, DeviceError>;
    fn provision(&mut self, config: &NetworkConfig) -> Result<(), DeviceError>;
    fn factory_reset(&mut self) -> Result<(), DeviceError>;
    fn time_sync(&mut self, sync: TimeSync) -> Result<(), DeviceError>;
    fn apply_layout(
        &mut self,
        rotation: u16,
        widgets: Vec<WidgetConfig>,
        screens: Vec<ScreenConfig>,
    ) -> Result<(), DeviceError>;
    fn push_fields(&mut self, widget_id: String, fields: Vec<Field>) -> Result<(), DeviceError>;
    fn push_scene(&mut self, push: PushScene) -> Result<(), DeviceError>;
    fn activate_screen(&mut self, screen_id: String) -> Result<(), DeviceError>;
    fn trigger_interrupt(&mut self, interrupt: TriggerInterrupt) -> Result<(), DeviceError>;
    /// Reserve (or re-attach to) storage for one asset. Unlike `provision`/
    /// `factory_reset`, this must work on every transport: the server owning
    /// the device over the tunnel is the entire point of networked tier, so
    /// there is no typed unsupported-on-this-transport refusal here. The
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

pub struct SerialRuntimeDevice {
    explicit_port: Option<String>,
    connected: Option<ConnectedSession>,
}

impl SerialRuntimeDevice {
    pub fn new(explicit_port: Option<String>) -> Self {
        Self {
            explicit_port,
            connected: None,
        }
    }

    pub(super) fn connected(&self) -> Result<&ConnectedSession, DeviceError> {
        self.connected.as_ref().ok_or(DeviceError::NoDevice)
    }
}

impl RuntimeDevice for SerialRuntimeDevice {
    fn connect(&mut self) -> Result<DeviceConnection, DeviceError> {
        if let Some(connected) = self.connected.as_mut() {
            match connected.reconnect(self.explicit_port.as_deref()) {
                Ok(()) => {
                    return Ok(DeviceConnection {
                        port_name: connected.port_name.clone(),
                        status: connected.initial_status.clone(),
                    });
                }
                Err(error) => {
                    // A stalled session's worker is blocked inside a transport
                    // read the operating system will not interrupt, and
                    // `reconnect` hands the new transport to that same worker, so
                    // this session can never come back. Discard it and open a
                    // fresh one below; keeping it would leave the display
                    // unreachable for the rest of the run after a single cable
                    // pull. Every other failure keeps the session, as before.
                    if !connected.session.is_stalled() {
                        return Err(error);
                    }
                    self.connected = None;
                }
            }
        }
        let connected = connect_session(self.explicit_port.as_deref())?;
        let result = DeviceConnection {
            port_name: connected.port_name.clone(),
            status: connected.initial_status.clone(),
        };
        self.connected = Some(connected);
        Ok(result)
    }

    fn status(&mut self) -> Result<StatusResponse, DeviceError> {
        self.connected()?.session.status()
    }

    fn provision(&mut self, config: &NetworkConfig) -> Result<(), DeviceError> {
        self.connected()?.session.provision(config).map(|_| ())
    }

    fn factory_reset(&mut self) -> Result<(), DeviceError> {
        self.connected()?.session.factory_reset().map(|_| ())
    }

    fn time_sync(&mut self, sync: TimeSync) -> Result<(), DeviceError> {
        self.connected()?.session.time_sync(sync).map(|_| ())
    }

    fn apply_layout(
        &mut self,
        rotation: u16,
        widgets: Vec<WidgetConfig>,
        screens: Vec<ScreenConfig>,
    ) -> Result<(), DeviceError> {
        self.connected()?
            .session
            .apply_next_config(rotation, widgets, screens)
            .map(|_| ())
    }

    fn push_fields(&mut self, widget_id: String, fields: Vec<Field>) -> Result<(), DeviceError> {
        self.connected()?
            .session
            .push_fields(widget_id, fields)
            .map(|_| ())
    }

    fn push_scene(&mut self, push: PushScene) -> Result<(), DeviceError> {
        self.connected()?.session.push_scene(push).map(|_| ())
    }

    fn activate_screen(&mut self, screen_id: String) -> Result<(), DeviceError> {
        self.connected()?
            .session
            .activate_screen(ActivateScreen { screen_id })
            .map(|_| ())
    }

    fn trigger_interrupt(&mut self, interrupt: TriggerInterrupt) -> Result<(), DeviceError> {
        self.connected()?
            .session
            .trigger_interrupt(interrupt)
            .map(|_| ())
    }

    fn send_asset_begin(&mut self, begin: AssetBegin) -> Result<Ack, DeviceError> {
        self.connected()?.session.asset_begin(begin)
    }

    fn send_asset_chunk(&mut self, chunk: AssetChunk) -> Result<(), DeviceError> {
        self.connected()?.session.asset_chunk(chunk).map(|_| ())
    }

    fn send_asset_commit(&mut self, commit: AssetCommit) -> Result<(), DeviceError> {
        self.connected()?.session.asset_commit(commit).map(|_| ())
    }

    fn send_asset_release(&mut self, release: AssetRelease) -> Result<(), DeviceError> {
        self.connected()?.session.asset_release(release).map(|_| ())
    }

    fn try_recv_event(&mut self) -> Option<ReceivedEvent> {
        self.connected
            .as_ref()
            .and_then(|connected| connected.session.try_recv_event())
    }

    fn diagnostics(&self) -> SessionDiagnostics {
        self.connected
            .as_ref()
            .map_or_else(SessionDiagnostics::default, |connected| {
                connected.session.diagnostics()
            })
    }
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
/// pending host state for a later return to Local and stop issuing USB mutations now.
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
    state.active_scene_dirty = state.active_screen.is_some();
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

pub(super) fn run_runtime_device_command(
    state: &mut WorkerState,
    reconnect_interval: Duration,
    operation: impl FnOnce() -> Result<(), DeviceError>,
) -> Result<(), RuntimeError> {
    if !state.connected {
        return Err(RuntimeError::DeviceDisconnected);
    }
    operation().map_err(|error| runtime_command_device_error(state, &error, reconnect_interval))
}

pub(super) fn reply_to_runtime_device_command(
    reply: &CommandReply,
    state: &mut WorkerState,
    reconnect_interval: Duration,
    operation: impl FnOnce() -> Result<(), DeviceError>,
) {
    let _ = reply.send(run_runtime_device_command(
        state,
        reconnect_interval,
        operation,
    ));
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
    // The dedicated field is additive in M3 and decodes to 0 when absent, so revisions
    // stay as the floor for exactly that case — an older image, or one that has accepted
    // no interrupt yet, where there is nothing better to start from. Applying them
    // unconditionally instead coupled the token counter to unrelated traffic: on the
    // 2026-08-20 hardware gate a delivered interrupt arrived as token 85 rather than 61,
    // the gap being the device's data revision, which read as 24 lost interrupts.
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
