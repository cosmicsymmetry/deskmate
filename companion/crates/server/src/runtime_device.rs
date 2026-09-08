//! `app-core`'s synchronous device seam over an asynchronously-owned
//! WebSocket.
//!
//! `WebSocketRuntimeDevice` lives on `app-core`'s plain runtime thread. It
//! submits one protocol request and blocks for its typed response. `SocketPeer`
//! lives on Tokio and is the sole owner of WebSocket reads and writes. This
//! channel boundary avoids trying to enter a Tokio runtime from synchronous
//! trait methods and keeps request IDs single-owned.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use app_core::{DeviceConnection, RuntimeDevice};
use axum::extract::ws::{Message as WsMessage, WebSocket};
use device::{DeviceError, ReceivedEvent, SessionDiagnostics, TransportError};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use protocol::{
    Ack, ActivateScreen, ApplyConfig, AssetBegin, AssetChunk, AssetCommit, AssetRelease, ErrorCode,
    ErrorResponse, Field, Message, NetworkConfig, PushData, PushScene, RequestIdAllocator,
    ScreenConfig, StatusResponse, TimeSync, TriggerInterrupt, WidgetConfig,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::time::{Instant, MissedTickBehavior, interval, timeout};

/// How long the socket actor waits for a matching protocol response. Keeping
/// this equal to the serial transport's established request deadline gives
/// both implementations the same device-facing failure threshold.
const REQUEST_TIMEOUT: Duration = device::DEFAULT_REQUEST_TIMEOUT;

/// How long the blocking [`RuntimeDevice`] caller waits for the socket actor.
/// This must stay strictly greater than [`REQUEST_TIMEOUT`]: the actor must
/// expire and remove its pending request before the caller can submit another,
/// or a late response could be attributed to the next command.
const RESPONSE_WAIT_TIMEOUT: Duration = Duration::from_secs(4);

/// Bounds every WebSocket send, including requests, keepalives, and close
/// frames. A peer that stops reading would otherwise park the socket actor and
/// prevent both response deadlines and idle checks from making progress.
///
/// Provisional: like [`PING_INTERVAL`], this is a V2 judgment call to revisit
/// with real traffic in V3. `SEND_TIMEOUT < PING_INTERVAL < IDLE_TIMEOUT` must
/// continue to hold.
const SEND_TIMEOUT: Duration = Duration::from_secs(2);

/// How often an otherwise-live link gets a WebSocket ping. It is intentionally
/// below [`IDLE_TIMEOUT`] so dead NAT/WiFi peers are detected before bare TCP's
/// much coarser retransmission timeout.
///
/// Provisional: sized by judgment for V2, not measurements. Revisit in V3.
const PING_INTERVAL: Duration = Duration::from_secs(3);

/// The device's own link timeout for the NETWORK transport, mirrored from
/// `firmware/main/link/net_link.c`'s `NET_LINK_TIMEOUT_MS`. The firmware owns
/// the value; it is restated here only so [`IDLE_TIMEOUT`]'s relationship to it
/// is checkable by a test rather than by memory. Note the USB transport is a
/// different number: `usb_link.c` uses `PROTOCOL_LINK_TIMEOUT_MS` (10 s) and is
/// symmetric with the host by construction, which is why this asymmetry is
/// specific to the tunnel.
const DEVICE_NETWORK_LINK_TIMEOUT_SECS: u64 = 45;

/// The same value as a [`Duration`], for the ordering tests below. Production
/// code derives [`IDLE_TIMEOUT`] from the seconds constant directly, so this
/// form has no non-test caller.
#[cfg(test)]
const DEVICE_NETWORK_LINK_TIMEOUT: Duration = Duration::from_secs(DEVICE_NETWORK_LINK_TIMEOUT_SECS);

/// How far ahead of the device the server gives up. Wide enough for the device
/// to notice and re-dial into a free lease (the observed reconnect took ~2 s),
/// and what makes [`IDLE_TIMEOUT`] ten missed pongs rather than three.
const IDLE_TIMEOUT_MARGIN_SECS: u64 = 15;

/// Maximum silence from the peer, including absence of a pong.
///
/// This must sit strictly BETWEEN a healthy link's silence and
/// [`DEVICE_NETWORK_LINK_TIMEOUT`], and both bounds are load-bearing:
///
/// - Reap too early and the server kills links the device still believes are
///   up. This previously read `protocol::LINK_TIMEOUT_MS` (10 s), which is
///   right for USB — where both ends use `PROTOCOL_LINK_TIMEOUT_MS` — but the
///   networked device waits 45 s, so the two ends did not agree despite the
///   old comment here claiming they did. Observed on hardware 2026-08-29: the
///   server logged `device link idle timeout, closing` while the device still
///   reported `online: true` with `malformed_frames`/`crc_errors` at 0, and the
///   pair flapped on a ~10 s period (reaped at 10 s, reconnecting in ~2 s).
/// - Reap too late and it is worse, not better: `device_link::handler` refuses
///   a reconnect with 409 while the previous lease is held (`claim_link`), so a
///   value past 45 s would have the device give up, re-dial, and bounce off the
///   server's own stale lease.
///
/// 30 s costs ten consecutive missed pongs before a reap, against the three the
/// old value allowed, and still frees the lease 15 s before the device stops
/// believing in the link. `SEND_TIMEOUT < PING_INTERVAL < IDLE_TIMEOUT <
/// DEVICE_NETWORK_LINK_TIMEOUT` is pinned by a test.
///
/// This does NOT address why replies go missing in the first place: the device
/// gives `esp_websocket_client_send_bin` `PROTOCOL_WRITE_TIMEOUT_MS` (200 ms)
/// to deliver a reply the host waits 2000 ms for, and that budget is what drove
/// `dropped_responses` 1 -> 15 in the same session. Fixing that is a firmware
/// change and buys an OTA re-verification; this constant only stops the server
/// from amplifying it into a link flap.
/// Derived from the device's own timeout rather than written as a bare number,
/// so the ordering this doc argues for is structural and cannot drift back.
const IDLE_TIMEOUT: Duration =
    Duration::from_secs(DEVICE_NETWORK_LINK_TIMEOUT_SECS - IDLE_TIMEOUT_MARGIN_SECS);

/// Bounded handoff from the async socket actor to app-core's synchronous event
/// drain. A full queue drops locally and increments diagnostics rather than
/// allowing an untrusted device to grow memory without limit.
const EVENT_QUEUE_CAPACITY: usize = device::DEFAULT_EVENT_QUEUE_CAPACITY;

struct DeviceRequest {
    message: Message,
    response: SyncSender<Result<Message, DeviceError>>,
}

struct PendingRequest {
    request_id: u32,
    expected_type: u8,
    deadline: Instant,
    response: SyncSender<Result<Message, DeviceError>>,
}

#[derive(Default)]
struct TransportState {
    generation: u64,
    commands: Option<UnboundedSender<DeviceRequest>>,
}

#[derive(Default)]
struct TransportSlot {
    state: Mutex<TransportState>,
}

impl TransportSlot {
    fn attach(&self, commands: UnboundedSender<DeviceRequest>) -> u64 {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.generation = state.generation.wrapping_add(1).max(1);
        state.commands = Some(commands);
        state.generation
    }

    fn current(&self) -> Option<(u64, UnboundedSender<DeviceRequest>)> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .commands
            .as_ref()
            .filter(|commands| !commands.is_closed())
            .map(|commands| (state.generation, commands.clone()))
    }

    fn current_generation(&self) -> Option<u64> {
        self.current().map(|(generation, _)| generation)
    }

    fn detach(&self, generation: u64) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.generation == generation {
            state.commands = None;
        }
    }

    fn detach_current(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .commands = None;
    }
}

#[derive(Default)]
struct DiagnosticCounters {
    keepalives_sent: AtomicU64,
    reconnects: AtomicU64,
    duplicate_or_out_of_order_events: AtomicU64,
    locally_dropped_events: AtomicU64,
    detected_event_gaps: AtomicU64,
    malformed_device_frames: AtomicU64,
    unexpected_device_frames: AtomicU64,
}

impl DiagnosticCounters {
    fn snapshot(&self) -> SessionDiagnostics {
        SessionDiagnostics {
            keepalives_sent: self.keepalives_sent.load(Ordering::Relaxed),
            reconnects: self.reconnects.load(Ordering::Relaxed),
            duplicate_or_out_of_order_events: self
                .duplicate_or_out_of_order_events
                .load(Ordering::Relaxed),
            locally_dropped_events: self.locally_dropped_events.load(Ordering::Relaxed),
            detected_event_gaps: self.detected_event_gaps.load(Ordering::Relaxed),
            malformed_device_frames: self.malformed_device_frames.load(Ordering::Relaxed),
            unexpected_device_frames: self.unexpected_device_frames.load(Ordering::Relaxed),
        }
    }
}

struct EventRouter {
    sender: SyncSender<ReceivedEvent>,
    last_seen_sequence: Option<u64>,
    last_queued_sequence: Option<u64>,
    diagnostics: Arc<DiagnosticCounters>,
    replay: Arc<Mutex<ReplayState>>,
}

impl EventRouter {
    fn route(&mut self, event: protocol::DeviceEvent) {
        if self
            .last_seen_sequence
            .is_some_and(|sequence| event.sequence <= sequence)
        {
            self.diagnostics
                .duplicate_or_out_of_order_events
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.last_seen_sequence = Some(event.sequence);
        {
            let mut replay = self
                .replay
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if event.kind == protocol::EventKind::Navigation
                && matches!(
                    event.action,
                    protocol::EventAction::NavigatePrevious | protocol::EventAction::NavigateNext
                )
                && replay.config.as_ref().is_some_and(|config| {
                    config
                        .screens
                        .iter()
                        .any(|screen| screen.screen_id == event.screen_id)
                })
            {
                replay.active_screen = Some(ActivateScreen {
                    screen_id: event.screen_id.clone(),
                });
            }
            if event.kind == protocol::EventKind::InterruptDismissed
                && let Some(token) = event.interrupt_token
            {
                replay
                    .interrupts
                    .retain(|interrupt| interrupt.token != token);
            }
        }
        let missed_before = self.last_queued_sequence.map_or(0, |sequence| {
            event.sequence.saturating_sub(sequence).saturating_sub(1)
        });
        let sequence = event.sequence;
        match self.sender.try_send(ReceivedEvent {
            event,
            missed_before,
        }) {
            Ok(()) => {
                self.last_queued_sequence = Some(sequence);
                self.diagnostics
                    .detected_event_gaps
                    .fetch_add(missed_before, Ordering::Relaxed);
            }
            Err(mpsc::TrySendError::Full(_)) => {
                self.diagnostics
                    .locally_dropped_events
                    .fetch_add(1, Ordering::Relaxed);
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {}
        }
    }
}

#[derive(Clone, Default)]
struct ReplayState {
    time_sync: Option<(TimeSync, std::time::Instant)>,
    config: Option<ApplyConfig>,
    pushes: Vec<PushData>,
    active_screen: Option<ActivateScreen>,
    interrupts: Vec<TriggerInterrupt>,
}

/// Stable attachment point retained beside a device's long-lived runtime.
/// Each accepted WebSocket installs a fresh actor endpoint into this slot.
#[derive(Clone)]
pub(crate) struct SocketConnector {
    transport: Arc<TransportSlot>,
    event_sender: SyncSender<ReceivedEvent>,
    diagnostics: Arc<DiagnosticCounters>,
    replay: Arc<Mutex<ReplayState>>,
    last_ota_error: Arc<Mutex<Option<String>>>,
}

impl SocketConnector {
    pub(crate) fn attach(&self) -> SocketPeer {
        let (command_sender, command_receiver) = tokio::sync::mpsc::unbounded_channel();
        let generation = self.transport.attach(command_sender);
        SocketPeer {
            commands: command_receiver,
            event_router: EventRouter {
                sender: self.event_sender.clone(),
                last_seen_sequence: None,
                last_queued_sequence: None,
                diagnostics: Arc::clone(&self.diagnostics),
                replay: Arc::clone(&self.replay),
            },
            diagnostics: Arc::clone(&self.diagnostics),
            transport: Arc::clone(&self.transport),
            generation,
            request_ids: RequestIdAllocator::new(),
        }
    }

    pub(crate) fn detach(&self) {
        self.transport.detach_current();
    }

    pub(crate) fn last_ota_error(&self) -> Option<String> {
        self.last_ota_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// The synchronous half handed to [`app_core::RuntimeHandle`].
pub struct WebSocketRuntimeDevice {
    device_id: String,
    transport: Arc<TransportSlot>,
    events: Receiver<ReceivedEvent>,
    diagnostics: Arc<DiagnosticCounters>,
    replay: Arc<Mutex<ReplayState>>,
    last_ota_error: Arc<Mutex<Option<String>>>,
    connected_generation: Option<u64>,
    ever_connected: bool,
    latest_data_revision: u32,
    latest_config_revision: u32,
    capabilities: u64,
}

/// The async half retained by the upgraded socket task.
pub(crate) struct SocketPeer {
    commands: UnboundedReceiver<DeviceRequest>,
    event_router: EventRouter,
    diagnostics: Arc<DiagnosticCounters>,
    transport: Arc<TransportSlot>,
    generation: u64,
    request_ids: RequestIdAllocator,
}

impl WebSocketRuntimeDevice {
    pub(crate) fn channel(device_id: String) -> (Self, SocketConnector) {
        let (event_sender, event_receiver) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let diagnostics = Arc::new(DiagnosticCounters::default());
        let transport = Arc::new(TransportSlot::default());
        let replay = Arc::new(Mutex::new(ReplayState::default()));
        let last_ota_error = Arc::new(Mutex::new(None));
        let connector = SocketConnector {
            transport: Arc::clone(&transport),
            event_sender,
            diagnostics: Arc::clone(&diagnostics),
            replay: Arc::clone(&replay),
            last_ota_error: Arc::clone(&last_ota_error),
        };
        (
            Self {
                device_id,
                transport,
                events: event_receiver,
                diagnostics,
                replay,
                last_ota_error,
                connected_generation: None,
                ever_connected: false,
                latest_data_revision: 0,
                latest_config_revision: 0,
                capabilities: 0,
            },
            connector,
        )
    }

    fn request_on_generation(
        &self,
        expected_generation: Option<u64>,
        message: Message,
    ) -> Result<(u64, Message), DeviceError> {
        let (generation, commands) = self
            .transport
            .current()
            .ok_or(DeviceError::Transport(TransportError::Disconnected))?;
        if expected_generation.is_some_and(|expected| expected != generation) {
            return Err(DeviceError::NoDevice);
        }
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        commands
            .send(DeviceRequest {
                message,
                response: response_sender,
            })
            .map_err(|_| DeviceError::Transport(TransportError::Disconnected))?;
        match response_receiver.recv_timeout(RESPONSE_WAIT_TIMEOUT) {
            Ok(response) => response.map(|message| (generation, message)),
            Err(RecvTimeoutError::Timeout) => Err(DeviceError::Timeout),
            Err(RecvTimeoutError::Disconnected) => {
                Err(DeviceError::Transport(TransportError::Disconnected))
            }
        }
    }

    fn request(&self, message: Message) -> Result<(u64, Message), DeviceError> {
        self.request_on_generation(None, message)
    }

    fn remember_last_ota_error(&self, status: &StatusResponse) {
        self.last_ota_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone_from(&status.last_ota_error);
    }

    fn connected_request(&self, message: Message) -> Result<Message, DeviceError> {
        let generation = self.connected_generation.ok_or(DeviceError::NoDevice)?;
        self.request_on_generation(Some(generation), message)
            .map(|(_, response)| response)
    }

    fn require_ack(
        response: &Message,
        acknowledged_type: u8,
        revision: Option<u32>,
    ) -> Result<(), DeviceError> {
        match response {
            Message::Ack(Ack {
                acknowledged_type: received_type,
                revision: received_revision,
                ..
            }) if *received_type == acknowledged_type && *received_revision == revision => Ok(()),
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }

    fn next_revision(current: u32) -> Result<u32, DeviceError> {
        current.checked_add(1).ok_or(DeviceError::RevisionExhausted)
    }

    fn remember_success(&self, request: &Message) {
        let mut replay = self
            .replay
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match request {
            Message::TimeSync(sync) => replay.time_sync = Some((*sync, std::time::Instant::now())),
            Message::ApplyConfig(config) => {
                let changes_live_model = replay.config.as_ref() != Some(config);
                replay.config = Some(config.clone());
                if changes_live_model {
                    replay.pushes.clear();
                    replay.interrupts.clear();
                    if replay.active_screen.as_ref().is_some_and(|active| {
                        !config
                            .screens
                            .iter()
                            .any(|screen| screen.screen_id == active.screen_id)
                    }) {
                        replay.active_screen = None;
                    }
                }
            }
            Message::PushData(push) => {
                replay
                    .pushes
                    .retain(|cached| cached.widget_id != push.widget_id);
                replay.pushes.push(push.clone());
                replay.pushes.sort_unstable_by_key(|cached| cached.revision);
            }
            Message::ActivateScreen(activation) => {
                replay.active_screen = Some(activation.clone());
            }
            Message::TriggerInterrupt(interrupt) => {
                replay
                    .interrupts
                    .retain(|cached| cached.token != interrupt.token);
                replay.interrupts.push(interrupt.clone());
                replay
                    .interrupts
                    .sort_unstable_by_key(|cached| cached.token);
            }
            _ => {}
        }
    }

    fn replay_after_reconnect(
        &mut self,
        status: &StatusResponse,
        generation: u64,
    ) -> Result<(), DeviceError> {
        let mut replay = self
            .replay
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(config) = &replay.config {
            let required = protocol::CAPABILITY_CORE_WIDGETS
                | if config.rotation == 270 {
                    protocol::CAPABILITY_CONFIG_ROTATION
                } else {
                    0
                };
            if status.capabilities & required != required {
                return Err(DeviceError::MissingCapabilities {
                    required,
                    available: status.capabilities,
                });
            }
        }

        self.latest_data_revision = status.latest_revision;
        self.latest_config_revision = status.config_revision;

        if let Some((mut sync, synchronized_at)) = replay.time_sync {
            if let Ok(elapsed) = i64::try_from(synchronized_at.elapsed().as_secs())
                && let Some(adjusted) = sync.unix_seconds.checked_add(elapsed)
            {
                sync.unix_seconds = adjusted;
            }
            let (_, response) =
                self.request_on_generation(Some(generation), Message::TimeSync(sync))?;
            Self::require_ack(&response, protocol::TYPE_TIME_SYNC, None)?;
        }

        let mut config_applied = false;
        if let Some(config) = replay.config.as_mut() {
            if config.revision != status.config_revision {
                if config.revision < status.config_revision {
                    config.revision = status
                        .config_revision
                        .checked_add(1)
                        .ok_or(DeviceError::RevisionExhausted)?;
                }
                let (_, response) = self.request_on_generation(
                    Some(generation),
                    Message::ApplyConfig(config.clone()),
                )?;
                Self::require_ack(
                    &response,
                    protocol::TYPE_APPLY_CONFIG,
                    Some(config.revision),
                )?;
                config_applied = true;
            }
            self.latest_config_revision = config.revision;
        }

        let mut data_revision = status.latest_revision;
        for push in &mut replay.pushes {
            if !config_applied && push.revision <= data_revision {
                continue;
            }
            if push.revision <= data_revision {
                push.revision = data_revision
                    .checked_add(1)
                    .ok_or(DeviceError::RevisionExhausted)?;
            }
            let (_, response) =
                self.request_on_generation(Some(generation), Message::PushData(push.clone()))?;
            Self::require_ack(&response, protocol::TYPE_PUSH_DATA, Some(push.revision))?;
            data_revision = push.revision;
        }
        self.latest_data_revision = data_revision;

        if let Some(activation) = &replay.active_screen {
            let (_, response) = self.request_on_generation(
                Some(generation),
                Message::ActivateScreen(activation.clone()),
            )?;
            Self::require_ack(&response, protocol::TYPE_ACTIVATE_SCREEN, None)?;
        }
        for interrupt in &replay.interrupts {
            let (_, response) = self.request_on_generation(
                Some(generation),
                Message::TriggerInterrupt(interrupt.clone()),
            )?;
            Self::require_ack(&response, protocol::TYPE_TRIGGER_INTERRUPT, None)?;
        }

        *self
            .replay
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = replay;
        Ok(())
    }
}

/// Like the serial implementation, this object retains successfully-issued
/// replay state across `connect()` calls. A new socket has a new attachment
/// generation, so ordinary status/data work refuses it until app-core runs
/// `connect()` and this implementation has replayed the device model.
impl RuntimeDevice for WebSocketRuntimeDevice {
    fn connect(&mut self) -> Result<DeviceConnection, DeviceError> {
        let response = self.request(Message::StatusRequest);
        let (generation, response) = match response {
            Ok(response) => response,
            Err(error) => {
                self.connected_generation = None;
                return Err(error);
            }
        };
        let Message::StatusResponse(status) = response else {
            return Err(DeviceError::UnexpectedMessage);
        };
        self.remember_last_ota_error(&status);
        self.latest_data_revision = status.latest_revision;
        self.latest_config_revision = status.config_revision;
        self.capabilities = status.capabilities;
        if self.ever_connected {
            if let Err(error) = self.replay_after_reconnect(&status, generation) {
                self.connected_generation = None;
                return Err(error);
            }
            self.diagnostics.reconnects.fetch_add(1, Ordering::Relaxed);
        }
        if self.transport.current_generation() != Some(generation) {
            self.connected_generation = None;
            return Err(DeviceError::Transport(TransportError::Disconnected));
        }
        self.connected_generation = Some(generation);
        self.ever_connected = true;
        Ok(DeviceConnection {
            port_name: format!("network:{}", self.device_id),
            status,
        })
    }

    fn status(&mut self) -> Result<StatusResponse, DeviceError> {
        match self.connected_request(Message::StatusRequest)? {
            Message::StatusResponse(status) => {
                self.remember_last_ota_error(&status);
                self.capabilities = status.capabilities;
                Ok(status)
            }
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }

    fn provision(&mut self, _config: &NetworkConfig) -> Result<(), DeviceError> {
        Err(DeviceError::Rejected(ErrorResponse {
            code: ErrorCode::UnsupportedMessage,
            diagnostic: "provisioning is unsupported on the WebSocket transport".into(),
        }))
    }

    fn factory_reset(&mut self) -> Result<(), DeviceError> {
        Err(DeviceError::Rejected(ErrorResponse {
            code: ErrorCode::UnsupportedMessage,
            diagnostic: "factory reset is unsupported on the WebSocket transport".into(),
        }))
    }

    fn time_sync(&mut self, sync: TimeSync) -> Result<(), DeviceError> {
        let request = Message::TimeSync(sync);
        let response = self.connected_request(request.clone())?;
        Self::require_ack(&response, protocol::TYPE_TIME_SYNC, None)?;
        self.remember_success(&request);
        Ok(())
    }

    fn apply_layout(
        &mut self,
        rotation: u16,
        widgets: Vec<WidgetConfig>,
        screens: Vec<ScreenConfig>,
    ) -> Result<(), DeviceError> {
        if self.connected_generation.is_none() {
            return Err(DeviceError::NoDevice);
        }
        let required = protocol::CAPABILITY_CORE_WIDGETS
            | if rotation == 270 {
                protocol::CAPABILITY_CONFIG_ROTATION
            } else {
                0
            };
        if self.capabilities & required != required {
            return Err(DeviceError::MissingCapabilities {
                required,
                available: self.capabilities,
            });
        }
        let revision = Self::next_revision(self.latest_config_revision)?;
        let request = Message::ApplyConfig(ApplyConfig {
            revision,
            rotation,
            widgets,
            screens,
        });
        let response = self.connected_request(request.clone())?;
        Self::require_ack(&response, protocol::TYPE_APPLY_CONFIG, Some(revision))?;
        self.latest_config_revision = revision;
        self.remember_success(&request);
        Ok(())
    }

    fn push_fields(&mut self, widget_id: String, fields: Vec<Field>) -> Result<(), DeviceError> {
        let revision = Self::next_revision(self.latest_data_revision)?;
        let request = Message::PushData(PushData {
            widget_id,
            revision,
            fields,
        });
        let response = self.connected_request(request.clone())?;
        Self::require_ack(&response, protocol::TYPE_PUSH_DATA, Some(revision))?;
        self.latest_data_revision = revision;
        self.remember_success(&request);
        Ok(())
    }

    fn push_scene(&mut self, push: PushScene) -> Result<(), DeviceError> {
        if self.connected_generation.is_none() {
            return Err(DeviceError::NoDevice);
        }
        let required = protocol::CAPABILITY_SCENE_RENDER;
        if self.capabilities & required != required {
            return Err(DeviceError::MissingCapabilities {
                required,
                available: self.capabilities,
            });
        }
        let revision = push.revision;
        let response = self.connected_request(Message::PushScene(push))?;
        Self::require_ack(&response, protocol::TYPE_PUSH_SCENE, Some(revision))
    }

    fn activate_screen(&mut self, screen_id: String) -> Result<(), DeviceError> {
        let request = Message::ActivateScreen(ActivateScreen { screen_id });
        let response = self.connected_request(request.clone())?;
        Self::require_ack(&response, protocol::TYPE_ACTIVATE_SCREEN, None)?;
        self.remember_success(&request);
        Ok(())
    }

    fn trigger_interrupt(&mut self, interrupt: TriggerInterrupt) -> Result<(), DeviceError> {
        let request = Message::TriggerInterrupt(interrupt);
        let response = self.connected_request(request.clone())?;
        Self::require_ack(&response, protocol::TYPE_TRIGGER_INTERRUPT, None)?;
        self.remember_success(&request);
        Ok(())
    }

    /// Unlike `provision`/`factory_reset`, asset transfer is not cable-only:
    /// the server owning the device over the tunnel is the entire point of
    /// networked tier, so this is a plain request/reply exactly like
    /// `push_fields`. Not part of reconnect replay (`remember_success`) --
    /// `AssetSync` re-derives its own state from `already_present`
    /// on every pass rather than trusting a stale replay log.
    fn send_asset_begin(&mut self, begin: AssetBegin) -> Result<Ack, DeviceError> {
        let response = self.connected_request(Message::AssetBegin(begin))?;
        match response {
            Message::Ack(ack)
                if ack.acknowledged_type == protocol::TYPE_ASSET_BEGIN
                    && ack.revision.is_none()
                    && ack.already_present.is_some() =>
            {
                Ok(ack)
            }
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }

    fn send_asset_chunk(&mut self, chunk: AssetChunk) -> Result<(), DeviceError> {
        let response = self.connected_request(Message::AssetChunk(chunk))?;
        Self::require_ack(&response, protocol::TYPE_ASSET_CHUNK, None)
    }

    fn send_asset_commit(&mut self, commit: AssetCommit) -> Result<(), DeviceError> {
        let response = self.connected_request(Message::AssetCommit(commit))?;
        Self::require_ack(&response, protocol::TYPE_ASSET_COMMIT, None)
    }

    fn send_asset_release(&mut self, release: AssetRelease) -> Result<(), DeviceError> {
        let response = self.connected_request(Message::AssetRelease(release))?;
        Self::require_ack(&response, protocol::TYPE_ASSET_RELEASE, None)
    }

    fn try_recv_event(&mut self) -> Option<ReceivedEvent> {
        self.events.try_recv().ok()
    }

    fn diagnostics(&self) -> SessionDiagnostics {
        self.diagnostics.snapshot()
    }
}

impl SocketPeer {
    pub(crate) async fn run(
        mut self,
        socket: WebSocket,
        device_id: &str,
        last_seen_unix_ms: Arc<AtomicU64>,
    ) {
        let (mut sender, mut receiver) = socket.split();
        let mut pending: Option<PendingRequest> = None;
        let mut last_activity = std::time::Instant::now();
        mark_seen(&last_seen_unix_ms);

        let mut ping_tick = interval(PING_INTERVAL);
        ping_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        ping_tick.tick().await;

        loop {
            let pending_deadline = pending_deadline(pending.as_ref());
            tokio::select! {
                next_message = receiver.next() => {
                    match next_message {
                        None | Some(Err(_) | Ok(WsMessage::Close(_))) => break,
                        Some(Ok(WsMessage::Text(_))) => {
                            tracing::warn!(device_id, "device link received a text frame, closing");
                            close_socket(&mut sender).await;
                            break;
                        }
                        Some(Ok(WsMessage::Binary(bytes))) => {
                            last_activity = std::time::Instant::now();
                            mark_seen(&last_seen_unix_ms);
                            if !self.handle_binary(&bytes, device_id, &mut pending) {
                                close_socket(&mut sender).await;
                                break;
                            }
                        }
                        Some(Ok(WsMessage::Ping(payload))) => {
                            last_activity = std::time::Instant::now();
                            mark_seen(&last_seen_unix_ms);
                            if send_ws(&mut sender, WsMessage::Pong(payload)).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(WsMessage::Pong(_))) => {
                            last_activity = std::time::Instant::now();
                            mark_seen(&last_seen_unix_ms);
                        }
                    }
                }
                command = self.commands.recv(), if pending.is_none() => {
                    let Some(command) = command else {
                        break;
                    };
                    let Some(expected_type) =
                        protocol::expected_response_type(command.message.type_id())
                    else {
                        let _ = command.response.send(Err(DeviceError::InvalidRequest));
                        continue;
                    };
                    let request_id = self.request_ids.allocate();
                    let wire = match protocol::encode_message(request_id, &command.message) {
                        Ok(wire) => wire,
                        Err(error) => {
                            let _ = command.response.send(Err(DeviceError::MalformedResponse(
                                format!("cannot encode host request: {error}"),
                            )));
                            continue;
                        }
                    };
                    if let Err(error) = send_ws(&mut sender, WsMessage::Binary(wire.into())).await {
                        let _ = command.response.send(Err(error));
                        break;
                    }
                    pending = Some(PendingRequest {
                        request_id,
                        expected_type,
                        deadline: Instant::now() + REQUEST_TIMEOUT,
                        response: command.response,
                    });
                }
                () = tokio::time::sleep_until(pending_deadline), if pending.is_some() => {
                    if let Some(expired) = pending.take() {
                        let _ = expired.response.send(Err(DeviceError::Timeout));
                    }
                }
                _ = ping_tick.tick() => {
                    if last_activity.elapsed() > IDLE_TIMEOUT {
                        tracing::warn!(device_id, "device link idle timeout, closing");
                        break;
                    }
                    match send_ws(&mut sender, WsMessage::Ping(Vec::new().into())).await {
                        Ok(()) => {
                            self.diagnostics.keepalives_sent.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(_) => break,
                    }
                }
            }
        }

        if let Some(pending) = pending {
            let _ = pending
                .response
                .send(Err(DeviceError::Transport(TransportError::Disconnected)));
        }
        self.transport.detach(self.generation);
    }

    fn handle_binary(
        &mut self,
        bytes: &[u8],
        device_id: &str,
        pending: &mut Option<PendingRequest>,
    ) -> bool {
        if bytes.is_empty() {
            self.malformed(device_id, "empty binary message");
            return false;
        }

        for wire in bytes.split_inclusive(|byte| *byte == 0) {
            let frame = match protocol::decode_wire_frame(wire) {
                Ok(frame) => frame,
                Err(error) => {
                    self.malformed(device_id, &error.to_string());
                    return false;
                }
            };
            let message = match protocol::decode_message(&frame) {
                Ok(message) => message,
                Err(error) => {
                    self.malformed(device_id, &error.to_string());
                    return false;
                }
            };

            if frame.request_id == 0 {
                if let Message::DeviceEvent(event) = message {
                    self.event_router.route(event);
                } else {
                    self.diagnostics
                        .unexpected_device_frames
                        .fetch_add(1, Ordering::Relaxed);
                }
                continue;
            }

            // Correlate AFTER decoding, never before. Decoding is what rejects a
            // hostile frame and closes the link; screening on the request id
            // first would let an undecodable frame carrying an unmatched id slip
            // past that check entirely, which `hostile_device.rs` catches.
            //
            // Peek before taking: a reply whose id is not the one awaited belongs
            // to a request this host already abandoned, and must not displace the
            // request currently in flight. Taking first is what let one late reply
            // fail the next request, whose own late reply then failed the one
            // after it -- a cascade that ends only when traffic stops. Observed on
            // hardware 2026-08-25 as "expected 472, received 471" widening to
            // "expected 485, received 478". The waiting request keeps its own
            // deadline, so a reply that never arrives still ends as a timeout.
            if pending
                .as_ref()
                .is_none_or(|waiting| waiting.request_id != frame.request_id)
            {
                self.diagnostics
                    .unexpected_device_frames
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let Some(waiting) = pending.take() else {
                continue;
            };
            let response = match message {
                Message::Error(error) => Err(DeviceError::Rejected(error)),
                message if message.type_id() == waiting.expected_type => Ok(message),
                _ => Err(DeviceError::UnexpectedMessage),
            };
            if response.is_err() {
                self.diagnostics
                    .unexpected_device_frames
                    .fetch_add(1, Ordering::Relaxed);
            }
            let _ = waiting.response.send(response);
        }
        true
    }

    fn malformed(&self, device_id: &str, error: &str) {
        self.diagnostics
            .malformed_device_frames
            .fetch_add(1, Ordering::Relaxed);
        tracing::warn!(device_id, error, "device link frame decode failed");
    }
}

fn pending_deadline(pending: Option<&PendingRequest>) -> Instant {
    pending.map_or_else(
        || Instant::now() + Duration::from_hours(24),
        |request| request.deadline,
    )
}

async fn send_ws(
    sender: &mut SplitSink<WebSocket, WsMessage>,
    message: WsMessage,
) -> Result<(), DeviceError> {
    match timeout(SEND_TIMEOUT, sender.send(message)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(DeviceError::Transport(TransportError::Io(
            error.to_string(),
        ))),
        Err(_) => Err(DeviceError::Timeout),
    }
}

async fn close_socket(sender: &mut SplitSink<WebSocket, WsMessage>) {
    let _ = send_ws(sender, WsMessage::Close(None)).await;
}

fn mark_seen(last_seen_unix_ms: &AtomicU64) {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(1);
    last_seen_unix_ms.store(millis.max(1), Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use app_core::RuntimeDevice;
    use device::DeviceError;
    use protocol::{
        Ack, ErrorCode, Field, FieldValue, Message, NetworkConfig, OtaState, StatusResponse, Tier,
        TimeSync, WifiState,
    };

    use super::{PendingRequest, SocketPeer};

    fn network_config() -> NetworkConfig {
        NetworkConfig {
            ssid: "network".into(),
            psk: "passphrase".into(),
            server_url: "wss://deskmate.example/v1/device/link".into(),
            device_id: "dev-0001".into(),
            token: "device-token".into(),
            utc_offset_minutes: 240,
            tier: Tier::Networked,
        }
    }

    #[test]
    fn cable_only_operations_are_typed_as_unsupported_on_websocket() {
        let (mut device, _connector) = super::WebSocketRuntimeDevice::channel("dev-1".into());

        for error in [
            device.provision(&network_config()).unwrap_err(),
            device.factory_reset().unwrap_err(),
        ] {
            assert!(matches!(
                error,
                DeviceError::Rejected(ref rejection)
                    if rejection.code == ErrorCode::UnsupportedMessage
                        && rejection.diagnostic.contains("WebSocket transport")
            ));
        }
    }

    #[test]
    fn remembering_a_later_status_clears_an_older_ota_error() {
        let (device, connector) = super::WebSocketRuntimeDevice::channel("dev-1".into());
        let mut failed = sample_status();
        failed.last_ota_error = Some("download: ESP_FAIL".into());
        device.remember_last_ota_error(&failed);
        assert_eq!(
            connector.last_ota_error().as_deref(),
            Some("download: ESP_FAIL")
        );

        device.remember_last_ota_error(&sample_status());
        assert_eq!(connector.last_ota_error(), None);
    }

    #[test]
    fn socket_peer_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<SocketPeer>();
    }

    #[test]
    fn actor_deadline_expires_before_the_blocking_caller_deadline() {
        assert!(
            super::RESPONSE_WAIT_TIMEOUT > super::REQUEST_TIMEOUT,
            "the actor must clear a pending request before its caller can time out"
        );
    }

    #[test]
    fn late_response_does_not_consume_the_websocket_request_waiting_for_its_own_response() {
        let (device, connector) = super::WebSocketRuntimeDevice::channel("dev-1".into());
        let mut peer = connector.attach();
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        let mut pending = Some(PendingRequest {
            request_id: 2,
            expected_type: protocol::TYPE_STATUS_RESPONSE,
            deadline: tokio::time::Instant::now() + Duration::from_secs(1),
            response: response_sender,
        });
        let mut responses =
            protocol::encode_message(1, &Message::StatusResponse(sample_status())).unwrap();
        responses.extend(
            protocol::encode_message(2, &Message::StatusResponse(sample_status())).unwrap(),
        );

        assert!(peer.handle_binary(&responses, "dev-1", &mut pending));
        assert_eq!(
            response_receiver
                .recv_timeout(Duration::from_millis(50))
                .expect("matching response is delivered")
                .expect("matching response succeeds"),
            Message::StatusResponse(sample_status())
        );
        assert!(pending.is_none());
        assert_eq!(device.diagnostics().unexpected_device_frames, 1);
    }

    #[test]
    fn keepalive_deadlines_preserve_progress_and_idle_detection() {
        assert!(super::SEND_TIMEOUT < super::PING_INTERVAL);
        assert!(super::PING_INTERVAL < super::IDLE_TIMEOUT);
    }

    /// The server must stop believing in a link BEFORE the device does.
    ///
    /// If this inverts, a device that gives up at `NET_LINK_TIMEOUT_MS` and
    /// re-dials arrives while the server still holds the previous lease, and
    /// `device_link::handler` refuses it with 409 ("owner already live"). The
    /// gap also has to be wide enough for the reconnect itself; the observed
    /// reconnect took ~2 s, so a few seconds is not enough margin.
    #[test]
    fn the_server_releases_a_dead_link_before_the_device_redials() {
        assert!(
            super::IDLE_TIMEOUT < super::DEVICE_NETWORK_LINK_TIMEOUT,
            "a server idle timeout at or past the device's own link timeout makes \
             the device re-dial into a held lease and get a 409"
        );
        let margin = super::DEVICE_NETWORK_LINK_TIMEOUT
            .checked_sub(super::IDLE_TIMEOUT)
            .expect("the assertion above pins the ordering");
        assert!(
            margin >= Duration::from_secs(10),
            "leave the device room to notice and reconnect before it gives up; got {margin:?}"
        );
    }

    /// The old value reaped a link after three missed pongs, which is what
    /// flapped against a device dropping replies under a 200 ms write budget.
    #[test]
    fn a_reap_costs_many_consecutive_missed_pongs_not_a_few() {
        let missed = super::IDLE_TIMEOUT.as_secs_f64() / super::PING_INTERVAL.as_secs_f64();
        assert!(
            missed >= 8.0,
            "a reap must survive a burst of dropped replies; got {missed} missed pongs"
        );
    }

    #[test]
    fn detached_transport_fails_immediately_without_queueing_work() {
        let (mut device, connector) = super::WebSocketRuntimeDevice::channel("dev-1".into());
        let peer = connector.attach();
        drop(peer);

        let started = std::time::Instant::now();
        assert!(matches!(
            device.connect(),
            Err(DeviceError::Transport(device::TransportError::Disconnected))
        ));
        assert!(
            started.elapsed() < std::time::Duration::from_millis(100),
            "a detached runtime parked on a request instead of failing cleanly"
        );
    }

    #[test]
    fn a_new_attachment_requires_connect_and_replays_the_device_model() {
        let (mut device, connector) = super::WebSocketRuntimeDevice::channel("dev-1".into());
        let first_actor = spawn_test_actor(connector.attach());
        device.connect().expect("first connect");
        device
            .time_sync(TimeSync {
                unix_seconds: 1_700_000_000,
                utc_offset_minutes: 0,
            })
            .expect("initial time sync");
        device
            .apply_layout(90, Vec::new(), Vec::new())
            .expect("initial layout");
        device
            .push_fields(
                "pomodoro".into(),
                vec![Field {
                    key: "remaining_seconds".into(),
                    value: FieldValue::Integer(30),
                }],
            )
            .expect("initial fields");
        device
            .activate_screen("pomodoro".into())
            .expect("initial activation");

        let second_actor = spawn_test_actor(connector.attach());
        assert!(matches!(device.status(), Err(DeviceError::NoDevice)));
        device.connect().expect("reattach connect and replay");
        connector.detach();

        first_actor.join().expect("first actor joins");
        let replayed = second_actor.join().expect("second actor joins");
        assert!(matches!(replayed.first(), Some(Message::StatusRequest)));
        assert!(matches!(replayed.get(1), Some(Message::TimeSync(_))));
        assert!(matches!(replayed.get(2), Some(Message::ApplyConfig(_))));
        assert!(matches!(replayed.get(3), Some(Message::PushData(_))));
        assert!(matches!(replayed.get(4), Some(Message::ActivateScreen(_))));
        assert_eq!(replayed.len(), 5);
    }

    #[test]
    fn push_scene_delegates_to_the_connected_websocket_transport() {
        let (mut device, connector) = super::WebSocketRuntimeDevice::channel("dev-1".into());
        let actor = spawn_test_actor(connector.attach());
        device.connect().expect("connect");
        let push = protocol::PushScene {
            card_id: "clock".into(),
            revision: 7,
            scene: protocol::Scene {
                revision: 7,
                background: 0,
                nodes: Vec::new(),
            },
        };

        device.push_scene(push.clone()).expect("push scene");
        connector.detach();

        let requests = actor.join().expect("actor joins");
        assert!(matches!(requests.first(), Some(Message::StatusRequest)));
        assert_eq!(requests.get(1), Some(&Message::PushScene(push)));
        assert_eq!(requests.len(), 2);
    }

    #[test]
    fn websocket_firmware_without_scene_render_refuses_before_wire_mutation() {
        let (mut device, connector) = super::WebSocketRuntimeDevice::channel("dev-1".into());
        let mut legacy = sample_status();
        legacy.capabilities &= !protocol::CAPABILITY_SCENE_RENDER;
        let actor = spawn_test_actor_with_status(connector.attach(), legacy.clone());
        device.connect().expect("connect");

        assert_eq!(
            device.push_scene(protocol::PushScene {
                card_id: "clock".into(),
                revision: 7,
                scene: protocol::Scene {
                    revision: 7,
                    background: 0,
                    nodes: Vec::new(),
                },
            }),
            Err(DeviceError::MissingCapabilities {
                required: protocol::CAPABILITY_SCENE_RENDER,
                available: legacy.capabilities,
            })
        );
        connector.detach();

        let requests = actor.join().expect("actor joins");
        assert_eq!(requests, vec![Message::StatusRequest]);
        assert!(
            requests
                .iter()
                .all(|request| !matches!(request, Message::PushScene(_)))
        );
    }

    #[test]
    fn scene_capability_is_refreshed_from_each_websocket_attachment() {
        let (mut device, connector) = super::WebSocketRuntimeDevice::channel("dev-1".into());
        let mut legacy = sample_status();
        legacy.capabilities &= !protocol::CAPABILITY_SCENE_RENDER;

        let first_actor = spawn_test_actor_with_status(connector.attach(), legacy.clone());
        device.connect().expect("legacy connect");
        let push = protocol::PushScene {
            card_id: "clock".into(),
            revision: 7,
            scene: protocol::Scene {
                revision: 7,
                background: 0,
                nodes: Vec::new(),
            },
        };
        assert!(matches!(
            device.push_scene(push.clone()),
            Err(DeviceError::MissingCapabilities { .. })
        ));

        let second_actor = spawn_test_actor(connector.attach());
        device.connect().expect("scene-capable reconnect");
        device
            .push_scene(push.clone())
            .expect("fresh bit 8 enables scenes");

        let third_actor = spawn_test_actor_with_status(connector.attach(), legacy);
        device.connect().expect("legacy reconnect");
        assert!(matches!(
            device.push_scene(push.clone()),
            Err(DeviceError::MissingCapabilities { .. })
        ));
        connector.detach();

        let first = first_actor.join().expect("first actor joins");
        let second = second_actor.join().expect("second actor joins");
        let third = third_actor.join().expect("third actor joins");
        assert_eq!(first, vec![Message::StatusRequest]);
        assert_eq!(
            second,
            vec![Message::StatusRequest, Message::PushScene(push)]
        );
        assert_eq!(third, vec![Message::StatusRequest]);
    }

    fn spawn_test_actor(peer: super::SocketPeer) -> std::thread::JoinHandle<Vec<Message>> {
        spawn_test_actor_with_status(peer, sample_status())
    }

    fn spawn_test_actor_with_status(
        mut peer: super::SocketPeer,
        status: StatusResponse,
    ) -> std::thread::JoinHandle<Vec<Message>> {
        std::thread::spawn(move || {
            let mut requests = Vec::new();
            while let Some(command) = peer.commands.blocking_recv() {
                let response = match &command.message {
                    Message::StatusRequest => Message::StatusResponse(status.clone()),
                    Message::TimeSync(_) => Message::Ack(Ack {
                        acknowledged_type: protocol::TYPE_TIME_SYNC,
                        revision: None,
                        already_present: None,
                    }),
                    Message::ApplyConfig(config) => Message::Ack(Ack {
                        acknowledged_type: protocol::TYPE_APPLY_CONFIG,
                        revision: Some(config.revision),
                        already_present: None,
                    }),
                    Message::PushData(push) => Message::Ack(Ack {
                        acknowledged_type: protocol::TYPE_PUSH_DATA,
                        revision: Some(push.revision),
                        already_present: None,
                    }),
                    Message::ActivateScreen(_) => Message::Ack(Ack {
                        acknowledged_type: protocol::TYPE_ACTIVATE_SCREEN,
                        revision: None,
                        already_present: None,
                    }),
                    Message::PushScene(push) => Message::Ack(Ack {
                        acknowledged_type: protocol::TYPE_PUSH_SCENE,
                        revision: Some(push.revision),
                        already_present: None,
                    }),
                    unexpected => panic!("unexpected test actor request: {unexpected:?}"),
                };
                requests.push(command.message);
                command
                    .response
                    .send(Ok(response))
                    .expect("runtime receives test response");
            }
            peer.transport.detach(peer.generation);
            requests
        })
    }

    fn sample_status() -> StatusResponse {
        StatusResponse {
            protocol_version: protocol::PROTOCOL_VERSION,
            max_protocol_version: protocol::MAX_PROTOCOL_VERSION,
            capabilities: protocol::CAPABILITY_CORE_WIDGETS
                | protocol::CAPABILITY_CONFIG_ROTATION
                | protocol::CAPABILITY_EXTENDED_TEMPLATES
                | protocol::CAPABILITY_SCENE_RENDER,
            firmware_version: "test-device".into(),
            uptime_ms: 1_234,
            free_heap: 5_678,
            display_width: 448,
            display_height: 368,
            brightness: 128,
            rotation: 90,
            online: true,
            latest_revision: 0,
            valid_frames: 1,
            malformed_frames: 0,
            crc_errors: 0,
            overflow_frames: 0,
            dropped_responses: 0,
            rx_dropped_bytes: 0,
            dropped_events: 0,
            event_queue_high_water: 0,
            dropped_ui_commands: 0,
            ui_queue_high_water: 0,
            config_revision: 0,
            latest_interrupt_token: 0,
            tier: Tier::Networked,
            wifi_state: WifiState::Connected,
            wifi_rssi: -42,
            ip: "192.0.2.10".into(),
            ota_state: OtaState::Idle,
            last_network_error: None,
            last_ota_error: None,
        }
    }
}
