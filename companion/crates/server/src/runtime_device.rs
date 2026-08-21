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
    Ack, ActivateScreen, ApplyConfig, ErrorCode, ErrorResponse, Field, Message, NetworkConfig,
    PushData, ScreenConfig, StatusResponse, TimeSync, TriggerInterrupt, WidgetConfig,
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

/// Maximum silence from the peer, including absence of a pong. This mirrors
/// the protocol's declared link timeout so both ends agree when the link has
/// died.
const IDLE_TIMEOUT: Duration = Duration::from_millis(protocol::LINK_TIMEOUT_MS);

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
    latest_status: Arc<Mutex<Option<StatusResponse>>>,
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
            next_request_id: 1,
        }
    }

    pub(crate) fn detach(&self) {
        self.transport.detach_current();
    }

    pub(crate) fn last_ota_error(&self) -> Option<String> {
        self.latest_status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .and_then(|status| status.last_ota_error.clone())
    }
}

/// The synchronous half handed to [`app_core::RuntimeHandle`].
pub struct WebSocketRuntimeDevice {
    device_id: String,
    transport: Arc<TransportSlot>,
    events: Receiver<ReceivedEvent>,
    diagnostics: Arc<DiagnosticCounters>,
    replay: Arc<Mutex<ReplayState>>,
    latest_status: Arc<Mutex<Option<StatusResponse>>>,
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
    next_request_id: u32,
}

impl WebSocketRuntimeDevice {
    pub(crate) fn channel(device_id: String) -> (Self, SocketConnector) {
        let (event_sender, event_receiver) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let diagnostics = Arc::new(DiagnosticCounters::default());
        let transport = Arc::new(TransportSlot::default());
        let replay = Arc::new(Mutex::new(ReplayState::default()));
        let latest_status = Arc::new(Mutex::new(None));
        let connector = SocketConnector {
            transport: Arc::clone(&transport),
            event_sender,
            diagnostics: Arc::clone(&diagnostics),
            replay: Arc::clone(&replay),
            latest_status: Arc::clone(&latest_status),
        };
        (
            Self {
                device_id,
                transport,
                events: event_receiver,
                diagnostics,
                replay,
                latest_status,
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

    fn remember_status(&self, status: &StatusResponse) {
        *self
            .latest_status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(status.clone());
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
        self.remember_status(&status);
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
                self.remember_status(&status);
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
                    let Some(expected_type) = expected_response_type(&command.message) else {
                        let _ = command.response.send(Err(DeviceError::InvalidRequest));
                        continue;
                    };
                    let request_id = self.allocate_request_id();
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

            let Some(waiting) = pending.take() else {
                self.diagnostics
                    .unexpected_device_frames
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            };
            if frame.request_id != waiting.request_id {
                let _ = waiting.response.send(Err(DeviceError::UnexpectedRequestId {
                    expected: waiting.request_id,
                    received: frame.request_id,
                }));
                self.diagnostics
                    .unexpected_device_frames
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            }
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

    fn allocate_request_id(&mut self) -> u32 {
        if self.next_request_id == 0 {
            self.next_request_id = 1;
        }
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        if self.next_request_id == 0 {
            self.next_request_id = 1;
        }
        request_id
    }
}

fn expected_response_type(message: &Message) -> Option<u8> {
    match message {
        Message::StatusRequest => Some(protocol::TYPE_STATUS_RESPONSE),
        Message::TimeSync(_)
        | Message::PushData(_)
        | Message::ApplyConfig(_)
        | Message::ActivateScreen(_)
        | Message::TriggerInterrupt(_)
        | Message::NetworkConfig(_)
        | Message::FactoryReset => Some(protocol::TYPE_ACK),
        Message::Heartbeat => Some(protocol::TYPE_HEARTBEAT_ACK),
        _ => None,
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
    use app_core::RuntimeDevice;
    use device::DeviceError;
    use protocol::{
        Ack, ErrorCode, Field, FieldValue, Message, NetworkConfig, OtaState, StatusResponse, Tier,
        TimeSync, WifiState,
    };

    use super::SocketPeer;

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
    fn request_ids_are_nonzero_and_wrap_to_one() {
        let (_device, connector) = super::WebSocketRuntimeDevice::channel("dev-1".into());
        let mut peer = connector.attach();
        peer.next_request_id = 0;
        assert_eq!(peer.allocate_request_id(), 1);
        assert_eq!(peer.next_request_id, 2);

        peer.next_request_id = u32::MAX;
        assert_eq!(peer.allocate_request_id(), u32::MAX);
        assert_eq!(peer.next_request_id, 1);
        assert_eq!(peer.allocate_request_id(), 1);
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
    fn keepalive_deadlines_preserve_progress_and_idle_detection() {
        assert!(super::SEND_TIMEOUT < super::PING_INTERVAL);
        assert!(super::PING_INTERVAL < super::IDLE_TIMEOUT);
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

    fn spawn_test_actor(mut peer: super::SocketPeer) -> std::thread::JoinHandle<Vec<Message>> {
        std::thread::spawn(move || {
            let mut requests = Vec::new();
            while let Some(command) = peer.commands.blocking_recv() {
                let response = match &command.message {
                    Message::StatusRequest => Message::StatusResponse(sample_status()),
                    Message::TimeSync(_) => Message::Ack(Ack {
                        acknowledged_type: protocol::TYPE_TIME_SYNC,
                        revision: None,
                    }),
                    Message::ApplyConfig(config) => Message::Ack(Ack {
                        acknowledged_type: protocol::TYPE_APPLY_CONFIG,
                        revision: Some(config.revision),
                    }),
                    Message::PushData(push) => Message::Ack(Ack {
                        acknowledged_type: protocol::TYPE_PUSH_DATA,
                        revision: Some(push.revision),
                    }),
                    Message::ActivateScreen(_) => Message::Ack(Ack {
                        acknowledged_type: protocol::TYPE_ACTIVATE_SCREEN,
                        revision: None,
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
                | protocol::CAPABILITY_EXTENDED_TEMPLATES,
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
