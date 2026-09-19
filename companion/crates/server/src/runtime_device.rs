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
use device::{
    DeviceError, ReceivedEvent, ReplayState, SessionDiagnostics, TransportError,
    session_state::{DiagnosticCounters, require_ack},
};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use protocol::{
    Ack, ActivateCard, ApplyConfig, AssetBegin, AssetChunk, AssetCommit, AssetRelease, CardConfig,
    Message, PushScene, PushTimer, RequestIdAllocator, StatusResponse, TimeSync, TriggerInterrupt,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::time::{Instant, MissedTickBehavior, interval, timeout};

/// How long the socket actor waits for a matching protocol response. Keeping
/// this equal to the serial transport's established request deadline gives
/// both implementations the same device-facing failure threshold.
const REQUEST_TIMEOUT: Duration = device::DEFAULT_REQUEST_TIMEOUT;

/// `AssetRelease` is not like the other requests. Its handler runs the
/// mark-dead scan and then COMPACTS the flash blob region, moving every
/// surviving asset's bytes, and flash erase-and-write is slow enough that the
/// ordinary two-second budget is simply the wrong number for it.
///
/// This was observed rather than reasoned about. A picture card's 329,740-byte
/// frame made compaction move far more than the curated fonts ever did, and
/// `AssetRelease` began timing out while the device reported `dropped_responses`,
/// `malformed_frames` and `crc_errors` all at zero -- the reply was late, not
/// lost. The many-round-trip chunk phase, which does no bulk flash work,
/// succeeded in the same pass.
///
/// It stays comfortably under [`IDLE_TIMEOUT`], so a slow compaction cannot be
/// mistaken for a dead peer, and a test pins that ordering.
const ASSET_RELEASE_TIMEOUT: Duration = Duration::from_secs(20);

/// How long this particular request may take. Everything except the one
/// message with a bulk-flash handler keeps the ordinary budget.
fn request_timeout(message: &protocol::Message) -> Duration {
    match message {
        protocol::Message::AssetRelease(_) => ASSET_RELEASE_TIMEOUT,
        _ => REQUEST_TIMEOUT,
    }
}

/// How long the blocking [`RuntimeDevice`] caller waits for the socket actor.
/// This must stay strictly greater than [`REQUEST_TIMEOUT`]: the actor must
/// expire and remove its pending request before the caller can submit another,
/// or a late response could be attributed to the next command.
const RESPONSE_WAIT_TIMEOUT: Duration = Duration::from_secs(4);

/// How much longer the blocking caller waits than the actor does, for the same
/// request. The ordering above is what stops a late response being attributed
/// to the next command, so it has to hold for EVERY message -- including the
/// one with its own longer budget, which is why this is a margin rather than a
/// second fixed number that could drift out of order.
const RESPONSE_WAIT_MARGIN: Duration =
    Duration::from_secs(RESPONSE_WAIT_TIMEOUT.as_secs() - REQUEST_TIMEOUT.as_secs());

/// The blocking caller's budget for this request, always strictly greater than
/// [`request_timeout`] for the same message.
fn response_wait_timeout(message: &protocol::Message) -> Duration {
    request_timeout(message) + RESPONSE_WAIT_MARGIN
}

/// Bounds every WebSocket send, including requests, keepalives, and close
/// frames. A peer that stops reading would otherwise park the socket actor and
/// prevent both response deadlines and idle checks from making progress.
///
/// Chosen conservatively rather than from production latency measurements.
/// `SEND_TIMEOUT < PING_INTERVAL < IDLE_TIMEOUT` must continue to hold.
const SEND_TIMEOUT: Duration = Duration::from_secs(2);

/// How often an otherwise-live link gets a WebSocket ping. It is intentionally
/// below [`IDLE_TIMEOUT`] so dead NAT/WiFi peers are detected before bare TCP's
/// much coarser retransmission timeout.
///
/// Chosen conservatively rather than from production measurements.
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
/// This must sit strictly between a healthy link's silence and
/// [`DEVICE_NETWORK_LINK_TIMEOUT`], and both bounds are load-bearing:
///
/// - Reap too early and the server kills links the device still believes are
///   up. The network transport waits 45 seconds, unlike USB's 10-second link
///   timeout, so the server must use the network value for this relationship.
/// - Reap too late and it is worse, not better: `device_link::handler` refuses
///   a reconnect with 409 while the previous lease is held (`claim_link`), so a
///   value past 45 s would have the device give up, re-dial, and bounce off the
///   server's own stale lease.
///
/// Thirty seconds tolerates ten consecutive missed pongs and still frees the
/// lease 15 seconds before the device abandons the link. Deriving it from the
/// device timeout makes that margin structural; a test pins the full ordering.
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
            replay.observe_event(&event);
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
pub(crate) struct WebSocketRuntimeDevice {
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
        // Computed before the message is moved into the request, and always
        // strictly longer than the actor's own deadline for it.
        let wait_timeout = response_wait_timeout(&message);
        commands
            .send(DeviceRequest {
                message,
                response: response_sender,
            })
            .map_err(|_| DeviceError::Transport(TransportError::Disconnected))?;
        match response_receiver.recv_timeout(wait_timeout) {
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

    fn next_revision(current: u32) -> Result<u32, DeviceError> {
        current.checked_add(1).ok_or(DeviceError::RevisionExhausted)
    }

    fn remember_success(&self, request: &Message) {
        let mut replay = self
            .replay
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        replay.remember_success(request);
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
        // Protocol v2 has no capability a card list can require: the bits that
        // described template rendering and rotation support are gone, and a
        // scene arrives already laid out for the mount. There is nothing to
        // re-check before replaying a config.

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
            require_ack(&response, protocol::TYPE_TIME_SYNC, None)?;
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
                require_ack(
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
                self.request_on_generation(Some(generation), Message::PushTimer(push.clone()))?;
            require_ack(&response, protocol::TYPE_PUSH_TIMER, Some(push.revision))?;
            data_revision = push.revision;
        }
        self.latest_data_revision = data_revision;

        if let Some(activation) = &replay.active_card {
            let (_, response) = self.request_on_generation(
                Some(generation),
                Message::ActivateCard(activation.clone()),
            )?;
            require_ack(&response, protocol::TYPE_ACTIVATE_CARD, None)?;
        }
        for interrupt in &replay.interrupts {
            let (_, response) = self.request_on_generation(
                Some(generation),
                Message::TriggerInterrupt(interrupt.clone()),
            )?;
            require_ack(&response, protocol::TYPE_TRIGGER_INTERRUPT, None)?;
        }

        let mut live = self
            .replay
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        live.commit_replay(replay);
        Ok(())
    }
}

/// This object retains successfully-issued replay state across `connect()`
/// calls because a new socket has a new attachment generation. Ordinary
/// status/data work refuses that generation until app-core runs `connect()`
/// and this implementation has replayed the device model.
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

    fn time_sync(&mut self, sync: TimeSync) -> Result<(), DeviceError> {
        let request = Message::TimeSync(sync);
        let response = self.connected_request(request.clone())?;
        require_ack(&response, protocol::TYPE_TIME_SYNC, None)?;
        self.remember_success(&request);
        Ok(())
    }

    fn apply_layout(&mut self, rotation: u16, cards: Vec<CardConfig>) -> Result<(), DeviceError> {
        if self.connected_generation.is_none() {
            return Err(DeviceError::NoDevice);
        }
        // Protocol v2 retired every capability that described rendering a
        // template, and rotation is no longer one a device can lack: a scene
        // arrives already laid out for the mount. Nothing in a card list is
        // still capability-gated, so there is nothing to check here.
        let revision = Self::next_revision(self.latest_config_revision)?;
        let request = Message::ApplyConfig(ApplyConfig {
            revision,
            rotation,
            cards,
        });
        let response = self.connected_request(request.clone())?;
        require_ack(&response, protocol::TYPE_APPLY_CONFIG, Some(revision))?;
        self.latest_config_revision = revision;
        self.remember_success(&request);
        Ok(())
    }

    fn push_timer(
        &mut self,
        card_id: String,
        total_ms: u32,
        remaining_ms: u32,
        running: bool,
    ) -> Result<(), DeviceError> {
        let revision = Self::next_revision(self.latest_data_revision)?;
        let request = Message::PushTimer(PushTimer {
            card_id,
            revision,
            total_ms,
            remaining_ms,
            running,
        });
        let response = self.connected_request(request.clone())?;
        require_ack(&response, protocol::TYPE_PUSH_TIMER, Some(revision))?;
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
        require_ack(&response, protocol::TYPE_PUSH_SCENE, Some(revision))
    }

    fn activate_card(&mut self, card_id: String) -> Result<(), DeviceError> {
        let request = Message::ActivateCard(ActivateCard { card_id });
        let response = self.connected_request(request.clone())?;
        require_ack(&response, protocol::TYPE_ACTIVATE_CARD, None)?;
        self.remember_success(&request);
        Ok(())
    }

    fn trigger_interrupt(&mut self, interrupt: TriggerInterrupt) -> Result<(), DeviceError> {
        let request = Message::TriggerInterrupt(interrupt);
        let response = self.connected_request(request.clone())?;
        require_ack(&response, protocol::TYPE_TRIGGER_INTERRUPT, None)?;
        self.remember_success(&request);
        Ok(())
    }

    /// Asset transfer is available to server-owned WebSocket devices, so this
    /// is a plain request/reply exactly like `push_timer`. It is not part of
    /// reconnect replay (`remember_success`): `AssetSync` re-derives its own
    /// state from `already_present` on every pass rather than trusting a stale
    /// replay log.
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
        require_ack(&response, protocol::TYPE_ASSET_CHUNK, None)
    }

    fn send_asset_commit(&mut self, commit: AssetCommit) -> Result<(), DeviceError> {
        let response = self.connected_request(Message::AssetCommit(commit))?;
        require_ack(&response, protocol::TYPE_ASSET_COMMIT, None)
    }

    fn send_asset_release(&mut self, release: AssetRelease) -> Result<(), DeviceError> {
        let response = self.connected_request(Message::AssetRelease(release))?;
        require_ack(&response, protocol::TYPE_ASSET_RELEASE, None)
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
                        deadline: Instant::now() + request_timeout(&command.message),
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
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use app_core::RuntimeDevice;
    use device::{DeviceError, ReplayState};
    use protocol::{
        Ack, ActivateCard, ApplyConfig, CardConfig, DeviceEvent, EventAction, EventKind, Message,
        StatusResponse, TapAction, TimeSync, TriggerInterrupt,
    };

    use super::{EventRouter, PendingRequest, SocketPeer};

    fn event_router(
        capacity: usize,
        replay: Arc<Mutex<ReplayState>>,
    ) -> (EventRouter, mpsc::Receiver<device::ReceivedEvent>) {
        let (sender, receiver) = mpsc::sync_channel(capacity);
        let (device, _connector) = super::WebSocketRuntimeDevice::channel("diagnostics".into());
        (
            EventRouter {
                sender,
                last_seen_sequence: None,
                last_queued_sequence: None,
                diagnostics: Arc::clone(&device.diagnostics),
                replay,
            },
            receiver,
        )
    }

    fn navigation(sequence: u64, card_id: &str) -> DeviceEvent {
        DeviceEvent {
            sequence,
            kind: EventKind::Navigation,
            card_id: card_id.into(),
            action: EventAction::NavigateNext,
            interrupt_token: None,
        }
    }

    fn interrupt(token: u32) -> TriggerInterrupt {
        TriggerInterrupt {
            card_id: "timer".into(),
            token,
            reason: format!("interrupt-{token}"),
        }
    }

    fn replay_cards() -> Vec<CardConfig> {
        vec![
            CardConfig {
                card_id: "timer".into(),
                tap_action: TapAction::StartPause,
            },
            CardConfig {
                card_id: "calendar".into(),
                tap_action: TapAction::None,
            },
        ]
    }

    fn sample_scene() -> protocol::PushScene {
        protocol::PushScene {
            card_id: "clock".into(),
            revision: 7,
            scene: protocol::Scene {
                revision: 7,
                background: 0,
                nodes: Vec::new(),
            },
        }
    }

    #[test]
    fn duplicate_navigation_does_not_change_socket_replay_state() {
        let replay = Arc::new(Mutex::new(ReplayState {
            config: Some(ApplyConfig {
                revision: 1,
                rotation: 90,
                cards: replay_cards(),
            }),
            active_card: Some(ActivateCard {
                card_id: "timer".into(),
            }),
            ..ReplayState::default()
        }));
        let (mut router, _events) = event_router(8, Arc::clone(&replay));

        router.route(navigation(2, "calendar"));
        router.route(navigation(2, "timer"));

        assert_eq!(
            replay.lock().unwrap().active_card.as_ref().unwrap().card_id,
            "calendar"
        );
    }

    #[test]
    fn socket_replay_updates_even_when_the_delivery_queue_is_full() {
        let replay = Arc::new(Mutex::new(ReplayState {
            config: Some(ApplyConfig {
                revision: 1,
                rotation: 90,
                cards: replay_cards(),
            }),
            active_card: Some(ActivateCard {
                card_id: "timer".into(),
            }),
            interrupts: vec![interrupt(3), interrupt(5), interrupt(7)],
            ..ReplayState::default()
        }));
        let (mut router, _events) = event_router(1, Arc::clone(&replay));
        router.route(DeviceEvent {
            sequence: 1,
            kind: EventKind::Tap,
            card_id: "timer".into(),
            action: EventAction::StartPause,
            interrupt_token: None,
        });

        router.route(navigation(2, "calendar"));
        router.route(DeviceEvent {
            sequence: 3,
            kind: EventKind::InterruptDismissed,
            card_id: "timer".into(),
            action: EventAction::DismissInterrupt,
            interrupt_token: Some(5),
        });

        let replay = replay.lock().unwrap();
        assert_eq!(replay.active_card.as_ref().unwrap().card_id, "calendar");
        assert_eq!(
            replay
                .interrupts
                .iter()
                .map(|cached| cached.token)
                .collect::<Vec<_>>(),
            vec![3, 7]
        );
    }

    #[test]
    fn websocket_require_ack_rejects_wrong_type_and_revision() {
        let response = Message::Ack(Ack {
            acknowledged_type: protocol::TYPE_PUSH_TIMER,
            revision: Some(7),
            already_present: None,
        });

        assert_eq!(
            super::require_ack(&response, protocol::TYPE_APPLY_CONFIG, Some(7)),
            Err(DeviceError::UnexpectedMessage)
        );
        assert_eq!(
            super::require_ack(&response, protocol::TYPE_PUSH_TIMER, Some(8)),
            Err(DeviceError::UnexpectedMessage)
        );
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
    fn every_message_lets_the_actor_expire_before_the_caller_gives_up() {
        use protocol::{AssetRelease, Message};

        // The invariant that stops a late reply being attributed to the NEXT
        // command has to hold for the long-budget message too, not just the
        // default one -- a 20 s actor deadline behind a 4 s caller wait is
        // exactly the cascade `95ed9eb` fixed.
        for message in [
            Message::StatusRequest,
            Message::AssetRelease(AssetRelease {
                digests: Vec::new(),
            }),
        ] {
            assert!(
                super::response_wait_timeout(&message) > super::request_timeout(&message),
                "actor must expire first for {message:?}"
            );
        }
    }

    #[test]
    fn asset_release_timeout_ladder_reaches_the_http_boundary() {
        let message = Message::AssetRelease(protocol::AssetRelease {
            digests: Vec::new(),
        });

        assert!(
            super::response_wait_timeout(&message)
                < app_core::runtime::SYNCHRONIZING_COMMAND_TIMEOUT
        );
        assert!(app_core::runtime::SYNCHRONIZING_COMMAND_TIMEOUT < crate::REQUEST_TIMEOUT);
    }

    #[test]
    fn asset_release_gets_longer_than_a_normal_request_but_less_than_the_idle_reap() {
        // Its handler compacts the flash blob region, so two seconds is the
        // wrong budget -- but a value past the idle timeout would have the
        // server reap the link while waiting for its own request.
        assert!(super::ASSET_RELEASE_TIMEOUT > super::REQUEST_TIMEOUT);
        assert!(super::ASSET_RELEASE_TIMEOUT < super::IDLE_TIMEOUT);
    }

    #[test]
    fn only_asset_release_gets_the_longer_budget() {
        use protocol::{AssetRelease, Message};

        assert_eq!(
            super::request_timeout(&Message::AssetRelease(AssetRelease {
                digests: Vec::new()
            })),
            super::ASSET_RELEASE_TIMEOUT
        );
        assert_eq!(
            super::request_timeout(&Message::StatusRequest),
            super::REQUEST_TIMEOUT
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

    /// A short burst of dropped replies must not make the server release the
    /// device's ownership lease.
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
        device.apply_layout(90, Vec::new()).expect("initial layout");
        device
            .push_timer("pomodoro".into(), 60_000, 30_000, false)
            .expect("initial timer");
        device
            .activate_card("pomodoro".into())
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
        assert!(matches!(replayed.get(3), Some(Message::PushTimer(_))));
        assert!(matches!(replayed.get(4), Some(Message::ActivateCard(_))));
        assert_eq!(replayed.len(), 5);
    }

    #[test]
    fn reconnect_preserves_navigation_received_during_replay() {
        let replayed = replay_after_concurrent_event(navigation(1, "calendar"));
        assert!(
            replayed.contains(&Message::ActivateCard(ActivateCard {
                card_id: "calendar".into(),
            })),
            "next replay must activate the navigated-to card"
        );
    }

    #[test]
    fn reconnect_preserves_interrupt_dismissal_received_during_replay() {
        let replayed = replay_after_concurrent_event(DeviceEvent {
            sequence: 1,
            kind: EventKind::InterruptDismissed,
            card_id: "timer".into(),
            action: EventAction::DismissInterrupt,
            interrupt_token: Some(5),
        });
        assert!(
            !replayed.iter().any(|message| matches!(message,
                Message::TriggerInterrupt(interrupt) if interrupt.token == 5
            )),
            "next replay must not resend the dismissed interrupt"
        );
    }

    fn replay_after_concurrent_event(event: DeviceEvent) -> Vec<Message> {
        let (mut device, connector) = super::WebSocketRuntimeDevice::channel("dev-1".into());
        let first_actor = spawn_test_actor(connector.attach());
        device.connect().expect("first connect");
        device
            .apply_layout(90, replay_cards())
            .expect("initial layout");
        device
            .push_timer("timer".into(), 60_000, 30_000, false)
            .unwrap();
        device.activate_card("timer".into()).unwrap();
        device.trigger_interrupt(interrupt(5)).unwrap();

        let mut status = sample_status();
        status.config_revision = device.latest_config_revision + 10;
        status.latest_revision = device.latest_data_revision + 10;
        let second_actor =
            spawn_test_actor_with_event(connector.attach(), status.clone(), Some(event));
        device.connect().expect("replay with concurrent event");
        {
            let replay = device.replay.lock().unwrap();
            assert_eq!(
                replay.config.as_ref().unwrap().revision,
                status.config_revision + 1
            );
            assert_eq!(replay.pushes[0].revision, status.latest_revision + 1);
        }

        status.config_revision += 1;
        status.latest_revision += 1;
        let third_actor = spawn_test_actor_with_status(connector.attach(), status);
        device.connect().expect("next replay");
        connector.detach();
        first_actor.join().unwrap();
        second_actor.join().unwrap();
        let replayed = third_actor.join().unwrap();
        assert!(
            !replayed
                .iter()
                .any(|message| matches!(message, Message::ApplyConfig(_) | Message::PushTimer(_))),
            "acknowledged rebased revisions must not be resent"
        );
        replayed
    }

    #[test]
    fn push_scene_delegates_to_the_connected_websocket_transport() {
        let (mut device, connector) = super::WebSocketRuntimeDevice::channel("dev-1".into());
        let actor = spawn_test_actor(connector.attach());
        device.connect().expect("connect");
        let push = sample_scene();

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
        let mut without_scene_render = sample_status();
        without_scene_render.capabilities &= !protocol::CAPABILITY_SCENE_RENDER;
        let actor = spawn_test_actor_with_status(connector.attach(), without_scene_render.clone());
        device.connect().expect("connect");

        assert_eq!(
            device.push_scene(sample_scene()),
            Err(DeviceError::MissingCapabilities {
                required: protocol::CAPABILITY_SCENE_RENDER,
                available: without_scene_render.capabilities,
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
        let mut without_scene_render = sample_status();
        without_scene_render.capabilities &= !protocol::CAPABILITY_SCENE_RENDER;

        let first_actor =
            spawn_test_actor_with_status(connector.attach(), without_scene_render.clone());
        device.connect().expect("scene-incapable connect");
        let push = sample_scene();
        assert!(matches!(
            device.push_scene(push.clone()),
            Err(DeviceError::MissingCapabilities { .. })
        ));

        let second_actor = spawn_test_actor(connector.attach());
        device.connect().expect("scene-capable reconnect");
        device
            .push_scene(push.clone())
            .expect("fresh bit 8 enables scenes");

        let third_actor = spawn_test_actor_with_status(connector.attach(), without_scene_render);
        device.connect().expect("scene-incapable reconnect");
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
        peer: super::SocketPeer,
        status: StatusResponse,
    ) -> std::thread::JoinHandle<Vec<Message>> {
        spawn_test_actor_with_event(peer, status, None)
    }

    fn ack(acknowledged_type: u8, revision: Option<u32>) -> Message {
        Message::Ack(Ack {
            acknowledged_type,
            revision,
            already_present: None,
        })
    }

    fn spawn_test_actor_with_event(
        mut peer: super::SocketPeer,
        status: StatusResponse,
        mut event: Option<DeviceEvent>,
    ) -> std::thread::JoinHandle<Vec<Message>> {
        std::thread::spawn(move || {
            let mut requests = Vec::new();
            while let Some(command) = peer.commands.blocking_recv() {
                let response = match &command.message {
                    Message::StatusRequest => Message::StatusResponse(status.clone()),
                    Message::TimeSync(_) => ack(protocol::TYPE_TIME_SYNC, None),
                    Message::ApplyConfig(config) => {
                        ack(protocol::TYPE_APPLY_CONFIG, Some(config.revision))
                    }
                    Message::PushTimer(push) => ack(protocol::TYPE_PUSH_TIMER, Some(push.revision)),
                    Message::ActivateCard(_) => ack(protocol::TYPE_ACTIVATE_CARD, None),
                    Message::TriggerInterrupt(_) => {
                        // The replay snapshot exists, and its final ACK is still pending.
                        if let Some(event) = event.take() {
                            peer.event_router.route(event);
                        }
                        ack(protocol::TYPE_TRIGGER_INTERRUPT, None)
                    }
                    Message::PushScene(push) => ack(protocol::TYPE_PUSH_SCENE, Some(push.revision)),
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
            firmware_version: "test-device".into(),
            uptime_ms: 1_234,
            free_heap: 5_678,
            brightness: 128,
            valid_frames: 1,
            tier: protocol::Tier::Networked,
            wifi_state: protocol::WifiState::Connected,
            wifi_rssi: -42,
            ip: "192.0.2.10".into(),
            ..protocol::test_support::sample_status_response()
        }
    }
}
