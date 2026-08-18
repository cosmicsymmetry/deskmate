//! `app-core`'s synchronous device seam over an asynchronously-owned
//! WebSocket.
//!
//! `WebSocketRuntimeDevice` lives on `app-core`'s plain runtime thread. It
//! submits one protocol request and blocks for its typed response. `SocketPeer`
//! lives on Tokio and is the sole owner of WebSocket reads and writes. This
//! channel boundary avoids trying to enter a Tokio runtime from synchronous
//! trait methods and keeps request IDs single-owned.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use app_core::{DeviceConnection, RuntimeDevice};
use axum::extract::ws::{Message as WsMessage, WebSocket};
use device::{DeviceError, ReceivedEvent, SessionDiagnostics, TransportError};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use protocol::{
    Ack, ActivateScreen, ApplyConfig, Field, Message, OtaState, PushData, ScreenConfig,
    StatusResponse, Tier, TimeSync, TriggerInterrupt, WidgetConfig, WifiState,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::time::{Instant, MissedTickBehavior, interval, timeout};

const REQUEST_TIMEOUT: Duration = device::DEFAULT_REQUEST_TIMEOUT;
const RESPONSE_WAIT_TIMEOUT: Duration = Duration::from_secs(4);
const SEND_TIMEOUT: Duration = Duration::from_secs(2);
const PING_INTERVAL: Duration = Duration::from_secs(3);
const IDLE_TIMEOUT: Duration = Duration::from_millis(protocol::LINK_TIMEOUT_MS);
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

/// The synchronous half handed to [`app_core::RuntimeHandle`].
pub struct WebSocketRuntimeDevice {
    device_id: String,
    commands: UnboundedSender<DeviceRequest>,
    events: Receiver<ReceivedEvent>,
    diagnostics: Arc<DiagnosticCounters>,
    connected: bool,
    latest_data_revision: u32,
    latest_config_revision: u32,
    capabilities: u64,
}

/// The async half retained by the upgraded socket task.
pub(crate) struct SocketPeer {
    commands: UnboundedReceiver<DeviceRequest>,
    event_router: EventRouter,
    diagnostics: Arc<DiagnosticCounters>,
    next_request_id: u32,
}

impl WebSocketRuntimeDevice {
    pub(crate) fn channel(device_id: String) -> (Self, SocketPeer) {
        let (command_sender, command_receiver) = tokio::sync::mpsc::unbounded_channel();
        let (event_sender, event_receiver) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let diagnostics = Arc::new(DiagnosticCounters::default());
        let peer = SocketPeer {
            commands: command_receiver,
            event_router: EventRouter {
                sender: event_sender,
                last_seen_sequence: None,
                last_queued_sequence: None,
                diagnostics: Arc::clone(&diagnostics),
            },
            diagnostics: Arc::clone(&diagnostics),
            next_request_id: 1,
        };
        (
            Self {
                device_id,
                commands: command_sender,
                events: event_receiver,
                diagnostics,
                connected: false,
                latest_data_revision: 0,
                latest_config_revision: 0,
                capabilities: 0,
            },
            peer,
        )
    }

    fn request(&self, message: Message) -> Result<Message, DeviceError> {
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.commands
            .send(DeviceRequest {
                message,
                response: response_sender,
            })
            .map_err(|_| DeviceError::Transport(TransportError::Disconnected))?;
        match response_receiver.recv_timeout(RESPONSE_WAIT_TIMEOUT) {
            Ok(response) => response,
            Err(RecvTimeoutError::Timeout) => Err(DeviceError::Timeout),
            Err(RecvTimeoutError::Disconnected) => {
                Err(DeviceError::Transport(TransportError::Disconnected))
            }
        }
    }

    fn require_connected(&self) -> Result<(), DeviceError> {
        if self.connected {
            Ok(())
        } else {
            Err(DeviceError::NoDevice)
        }
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
}

impl RuntimeDevice for WebSocketRuntimeDevice {
    fn connect(&mut self) -> Result<DeviceConnection, DeviceError> {
        let was_connected = self.connected;
        let response = self.request(Message::StatusRequest);
        let status = match response {
            Ok(Message::StatusResponse(status)) => status,
            Ok(_) => return Err(DeviceError::UnexpectedMessage),
            Err(error) => {
                self.connected = false;
                return Err(error);
            }
        };
        self.connected = true;
        self.latest_data_revision = status.latest_revision;
        self.latest_config_revision = status.config_revision;
        self.capabilities = status.capabilities;
        if was_connected {
            self.diagnostics.reconnects.fetch_add(1, Ordering::Relaxed);
        }
        Ok(DeviceConnection {
            port_name: format!("network:{}", self.device_id),
            status,
        })
    }

    fn status(&mut self) -> Result<StatusResponse, DeviceError> {
        self.require_connected()?;
        match self.request(Message::StatusRequest)? {
            Message::StatusResponse(status) => {
                self.capabilities = status.capabilities;
                Ok(status)
            }
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }

    fn time_sync(&mut self, sync: TimeSync) -> Result<(), DeviceError> {
        self.require_connected()?;
        let response = self.request(Message::TimeSync(sync))?;
        Self::require_ack(&response, protocol::TYPE_TIME_SYNC, None)
    }

    fn apply_layout(
        &mut self,
        rotation: u16,
        widgets: Vec<WidgetConfig>,
        screens: Vec<ScreenConfig>,
    ) -> Result<(), DeviceError> {
        self.require_connected()?;
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
        let response = self.request(Message::ApplyConfig(ApplyConfig {
            revision,
            rotation,
            widgets,
            screens,
        }))?;
        Self::require_ack(&response, protocol::TYPE_APPLY_CONFIG, Some(revision))?;
        self.latest_config_revision = revision;
        Ok(())
    }

    fn push_fields(&mut self, widget_id: String, fields: Vec<Field>) -> Result<(), DeviceError> {
        self.require_connected()?;
        let revision = Self::next_revision(self.latest_data_revision)?;
        let response = self.request(Message::PushData(PushData {
            widget_id,
            revision,
            fields,
        }))?;
        Self::require_ack(&response, protocol::TYPE_PUSH_DATA, Some(revision))?;
        self.latest_data_revision = revision;
        Ok(())
    }

    fn activate_screen(&mut self, screen_id: String) -> Result<(), DeviceError> {
        self.require_connected()?;
        let response = self.request(Message::ActivateScreen(ActivateScreen { screen_id }))?;
        Self::require_ack(&response, protocol::TYPE_ACTIVATE_SCREEN, None)
    }

    fn trigger_interrupt(&mut self, interrupt: TriggerInterrupt) -> Result<(), DeviceError> {
        self.require_connected()?;
        let response = self.request(Message::TriggerInterrupt(interrupt))?;
        Self::require_ack(&response, protocol::TYPE_TRIGGER_INTERRUPT, None)
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
                            if !self.handle_binary(
                                &mut sender,
                                &bytes,
                                device_id,
                                &mut pending,
                            ).await {
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
    }

    async fn handle_binary(
        &mut self,
        sender: &mut SplitSink<WebSocket, WsMessage>,
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

            // Retain Task 8's framing probe: a device-originated StatusRequest
            // is not part of normal ownership traffic, but answering it keeps
            // the hostile-frame concatenation test able to observe that both
            // complete frames were decoded. There is no periodic status poller
            // here; app-core owns the real 2-second status cadence.
            if matches!(message, Message::StatusRequest) {
                let response = Message::StatusResponse(server_status());
                let Ok(wire) = protocol::encode_message(frame.request_id, &response) else {
                    return false;
                };
                if send_ws(sender, WsMessage::Binary(wire.into()))
                    .await
                    .is_err()
                {
                    return false;
                }
                continue;
            }

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

fn server_status() -> StatusResponse {
    StatusResponse {
        protocol_version: protocol::PROTOCOL_VERSION,
        max_protocol_version: protocol::MAX_PROTOCOL_VERSION,
        capabilities: 0,
        firmware_version: env!("CARGO_PKG_VERSION").to_owned(),
        uptime_ms: 0,
        free_heap: 0,
        display_width: 0,
        display_height: 0,
        brightness: 0,
        rotation: 0,
        online: true,
        latest_revision: 0,
        valid_frames: 0,
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
        tier: Tier::Local,
        wifi_state: WifiState::Down,
        wifi_rssi: 0,
        ip: String::new(),
        ota_state: OtaState::Idle,
        last_network_error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::SocketPeer;

    #[test]
    fn request_ids_are_nonzero_and_wrap_to_one() {
        let (_device, mut peer) = super::WebSocketRuntimeDevice::channel("dev-1".into());
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
}
