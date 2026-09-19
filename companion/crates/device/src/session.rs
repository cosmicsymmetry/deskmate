use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use protocol::{
    Deframer, DeviceEvent, Message, NetworkConfig, RequestIdAllocator, StatusResponse,
    decode_message, encode_message, expected_response_type,
};

use crate::{
    ConnectedDevice, DeviceError, SerialTransport, Transport, TransportError, connect,
    message_error, session_state::require_ack,
};

pub const DEFAULT_EVENT_QUEUE_CAPACITY: usize = 32;
const COMMAND_QUEUE_CAPACITY: usize = 8;
const IDLE_READ_PAUSE: Duration = Duration::from_millis(1);

/// How long a caller waits for the session worker's reply, as a multiple of
/// `request_timeout`.
///
/// The worker owes every request an answer within `request_timeout`, because
/// `SessionConnection::transact` enforces that deadline itself. It can only fail
/// to do so when a transport read blocks: `transact` checks the deadline *between*
/// reads, so a read that never returns makes it unreachable. Waiting twice the
/// transaction budget absorbs scheduling jitter and one transaction already in
/// flight ahead of this one, while still bounding the wait -- an unbounded one
/// hands a stuck cable the power to freeze every caller above it.
const REPLY_TIMEOUT_FACTOR: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SessionOptions {
    request_timeout: Duration,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            request_timeout: crate::DEFAULT_REQUEST_TIMEOUT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedEvent {
    pub event: DeviceEvent,
    pub missed_before: u64,
}

enum WorkerCommand<T> {
    Request {
        message: Message,
        response: SyncSender<Result<Message, DeviceError>>,
    },
    Reconnect {
        transport: T,
        response: SyncSender<Result<(), DeviceError>>,
    },
    Shutdown,
}

pub struct DeviceSession<T: Transport + Send + 'static> {
    command_sender: SyncSender<WorkerCommand<T>>,
    capabilities: Arc<AtomicU64>,
    worker: Option<JoinHandle<()>>,
    request_timeout: Duration,
    /// Closed by the worker as it returns. `Drop` waits on this rather than
    /// joining, so a worker still inside a transport read is told apart from one
    /// that has finished.
    finished: Receiver<()>,
    /// Set once a reply misses its deadline. The worker is then blocked in a
    /// transport read the operating system will not interrupt, so the session is
    /// finished: later requests fail immediately instead of each paying the full
    /// wait, and `Drop` stops short of joining a thread that cannot return.
    stalled: AtomicBool,
}

impl<T: Transport + Send + 'static> DeviceSession<T> {
    fn new(transport: T, initial_status: &StatusResponse) -> Self {
        Self::with_options(transport, initial_status, SessionOptions::default())
    }

    fn with_options(
        transport: T,
        initial_status: &StatusResponse,
        options: SessionOptions,
    ) -> Self {
        let (command_sender, command_receiver) = mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
        let capabilities = Arc::new(AtomicU64::new(initial_status.capabilities));
        let connection = SessionConnection {
            transport,
            deframer: Deframer::new(),
            request_ids: RequestIdAllocator::new(),
            request_timeout: options.request_timeout,
        };
        let (finished_sender, finished_receiver) = mpsc::sync_channel::<()>(1);
        let worker = thread::Builder::new()
            .name("deskmate-device-session".into())
            .spawn(move || {
                // Moved in so it is dropped exactly when the worker returns;
                // closing the channel is the signal `Drop` waits for.
                let _finished = finished_sender;
                run_worker(connection, &command_receiver);
            })
            .expect("failed to spawn Deskmate device-session worker");
        Self {
            command_sender,
            capabilities,
            worker: Some(worker),
            request_timeout: options.request_timeout,
            finished: finished_receiver,
            stalled: AtomicBool::new(false),
        }
    }

    fn request(&self, message: Message) -> Result<Message, DeviceError> {
        if self.stalled.load(Ordering::Acquire) {
            return Err(DeviceError::Transport(TransportError::Disconnected));
        }
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.command_sender
            .send(WorkerCommand::Request {
                message,
                response: response_sender,
            })
            .map_err(|_| DeviceError::Transport(TransportError::Disconnected))?;
        match response_receiver.recv_timeout(self.request_timeout * REPLY_TIMEOUT_FACTOR) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                // Reported as a disconnection rather than `Timeout` on purpose.
                // `Timeout` means the device did not answer over a link that is
                // still good, so a caller retries on the same session; this means
                // the session itself will never answer again. The runtime's
                // disconnect classification is what replaces the session and
                // reconnects after a cable disappears mid-read.
                self.stalled.store(true, Ordering::Release);
                Err(DeviceError::Transport(TransportError::Disconnected))
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err(DeviceError::Transport(TransportError::Disconnected))
            }
        }
    }

    fn request_ack(&self, message: Message) -> Result<(), DeviceError> {
        self.request(message).map(|_| ())
    }

    fn capabilities(&self) -> u64 {
        self.capabilities.load(Ordering::Acquire)
    }

    pub fn status(&self) -> Result<StatusResponse, DeviceError> {
        match self.request(Message::StatusRequest)? {
            Message::StatusResponse(status) => {
                self.capabilities
                    .store(status.capabilities, Ordering::Release);
                Ok(status)
            }
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }

    /// Provision the device's network config over USB. The device persists
    /// this and applies the resulting tier on its *next* boot -- it never
    /// hot-swaps ownership mid-session, so this ACK does not mean the device
    /// is networked yet.
    pub fn provision(&self, config: &NetworkConfig) -> Result<(), DeviceError> {
        ensure_capabilities(protocol::CAPABILITY_NETWORKING, self.capabilities())?;
        self.request_ack(Message::NetworkConfig(config.clone()))
    }

    /// Erase the device's persisted network config, returning it to
    /// factory-fresh local tier on its next boot.
    pub fn factory_reset(&self) -> Result<(), DeviceError> {
        ensure_capabilities(protocol::CAPABILITY_NETWORKING, self.capabilities())?;
        self.request_ack(Message::FactoryReset)
    }

    /// Whether this session's worker has stopped answering. Such a session can
    /// never be revived -- `reconnect` hands the new transport to that same
    /// worker -- so a caller holding one must discard it and open another.
    pub fn is_stalled(&self) -> bool {
        self.stalled.load(Ordering::Acquire)
    }

    // Preserve the handoff signature exercised by the unchanged stall regression.
    #[allow(clippy::needless_pass_by_value)]
    fn reconnect(&self, transport: T, initial_status: StatusResponse) -> Result<(), DeviceError> {
        if self.stalled.load(Ordering::Acquire) {
            return Err(DeviceError::Transport(TransportError::Disconnected));
        }
        let capabilities = initial_status.capabilities;
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.command_sender
            .send(WorkerCommand::Reconnect {
                transport,
                response: response_sender,
            })
            .map_err(|_| DeviceError::Transport(TransportError::Disconnected))?;
        let result =
            match response_receiver.recv_timeout(self.request_timeout * REPLY_TIMEOUT_FACTOR) {
                Ok(result) => result,
                // The worker never took the transport, so its capabilities are not
                // this session's -- return before publishing them.
                Err(RecvTimeoutError::Timeout) => {
                    self.stalled.store(true, Ordering::Release);
                    return Err(DeviceError::Transport(TransportError::Disconnected));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(DeviceError::Transport(TransportError::Disconnected));
                }
            };
        // Publish the replacement device's capabilities so every later request is gated against the
        // actual connection rather than the previous firmware.
        self.capabilities.store(capabilities, Ordering::Release);
        result
    }
}

impl<T: Transport + Send + 'static> Drop for DeviceSession<T> {
    fn drop(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        if self.stalled.load(Ordering::Acquire) {
            // The worker is inside a transport read that will not return until
            // the operating system releases it, so both the queued shutdown and
            // the join would block here for as long as that takes. Detach it and
            // let it end on its own so dropping the session cannot wedge the
            // host process after the board is unplugged.
            let _ = self.command_sender.try_send(WorkerCommand::Shutdown);
            return;
        }
        let _ = self.command_sender.send(WorkerCommand::Shutdown);
        // Bounded on purpose. A worker can be blocked inside a transport read the
        // operating system will not interrupt even when no request ever timed out
        // -- an idle session whose cable was pulled -- and `stalled` is only set
        // by a request that missed its deadline. Waiting for the worker to close
        // `finished`, rather than joining it outright, keeps a thread that cannot
        // return from holding up the caller or preventing host shutdown.
        if self
            .finished
            .recv_timeout(self.request_timeout * REPLY_TIMEOUT_FACTOR)
            == Err(RecvTimeoutError::Timeout)
        {
            return;
        }
        let _ = worker.join();
    }
}

pub struct ConnectedSession {
    pub port_name: String,
    pub initial_status: StatusResponse,
    pub session: DeviceSession<SerialTransport>,
}

impl ConnectedSession {
    pub fn reconnect(&mut self, explicit_port: Option<&str>) -> Result<(), DeviceError> {
        let connected = connect(explicit_port)?;
        let port_name = connected.port_name;
        let status = connected.initial_status;
        self.session
            .reconnect(connected.client.into_transport(), status.clone())?;
        self.port_name = port_name;
        self.initial_status = status;
        Ok(())
    }
}

pub fn connect_session(explicit_port: Option<&str>) -> Result<ConnectedSession, DeviceError> {
    let ConnectedDevice {
        port_name,
        initial_status,
        client,
    } = connect(explicit_port)?;
    let session = DeviceSession::new(client.into_transport(), &initial_status);
    Ok(ConnectedSession {
        port_name,
        initial_status,
        session,
    })
}

struct SessionConnection<T> {
    transport: T,
    deframer: Deframer,
    request_ids: RequestIdAllocator,
    request_timeout: Duration,
}

impl<T: Transport> SessionConnection<T> {
    fn write_all(&mut self, wire: &[u8], deadline: Instant) -> Result<(), DeviceError> {
        let mut written = 0;
        while written < wire.len() {
            if Instant::now() >= deadline {
                return Err(DeviceError::Timeout);
            }
            let count = self.transport.write(&wire[written..])?;
            if count == 0 {
                return Err(DeviceError::Transport(TransportError::Disconnected));
            }
            written += count;
        }
        Ok(())
    }

    fn transact(&mut self, request: &Message) -> Result<Message, DeviceError> {
        let expected_type =
            expected_response_type(request.type_id()).ok_or(DeviceError::InvalidRequest)?;
        let request_id = self.request_ids.allocate();
        let wire = encode_message(request_id, request).map_err(message_error)?;
        let deadline = Instant::now() + self.request_timeout;
        self.write_all(&wire, deadline)?;

        let mut chunk = [0_u8; 512];
        loop {
            if Instant::now() >= deadline {
                return Err(DeviceError::Timeout);
            }
            let count = self.transport.read(&mut chunk)?;
            if count == 0 {
                thread::sleep(IDLE_READ_PAUSE);
                continue;
            }
            let mut response = None;
            for framed in self.deframer.push(&chunk[..count]) {
                let frame = match framed {
                    Ok(frame) => frame,
                    Err(error) => {
                        if response.is_none() {
                            return Err(DeviceError::MalformedResponse(error.to_string()));
                        }
                        continue;
                    }
                };
                if frame.request_id == 0 {
                    match decode_message(&frame).map_err(message_error)? {
                        Message::DeviceEvent(_) => {}
                        _ => return Err(DeviceError::UnexpectedMessage),
                    }
                    continue;
                }
                if frame.request_id != request_id {
                    continue;
                }
                let message = decode_message(&frame).map_err(message_error)?;
                if response.is_some() {
                    return Err(DeviceError::UnexpectedMessage);
                }
                if let Message::Error(error) = message {
                    response = Some(Err(DeviceError::Rejected(error)));
                } else if message.type_id() != expected_type {
                    response = Some(Err(DeviceError::UnexpectedMessage));
                } else {
                    response = Some(Ok(message));
                }
            }
            if let Some(response) = response {
                return response;
            }
        }
    }

    fn transact_acked(&mut self, request: &Message) -> Result<Message, DeviceError> {
        let response = self.transact(request)?;
        if matches!(response, Message::Ack(_)) {
            require_ack(&response, request.type_id(), None)?;
        }
        Ok(response)
    }

    fn read_idle(&mut self) -> Result<bool, DeviceError> {
        let mut chunk = [0_u8; 512];
        let count = self.transport.read(&mut chunk)?;
        if count == 0 {
            return Ok(false);
        }
        let _ = self.deframer.push(&chunk[..count]);
        Ok(true)
    }
}

fn ensure_capabilities(required: u64, available: u64) -> Result<(), DeviceError> {
    if available & required == required {
        Ok(())
    } else {
        Err(DeviceError::MissingCapabilities {
            required,
            available,
        })
    }
}

fn run_worker<T: Transport + Send + 'static>(
    mut connection: SessionConnection<T>,
    command_receiver: &Receiver<WorkerCommand<T>>,
) {
    let mut connected = true;
    loop {
        let received = if connected {
            command_receiver.try_recv()
        } else {
            command_receiver
                .recv()
                .map_err(|_| TryRecvError::Disconnected)
        };
        match received {
            Ok(WorkerCommand::Shutdown) | Err(TryRecvError::Disconnected) => break,
            Ok(WorkerCommand::Request { message, response }) => {
                let result = if connected {
                    let result = connection.transact_acked(&message);
                    if matches!(result, Err(DeviceError::Transport(_))) {
                        connected = false;
                    }
                    result
                } else {
                    Err(DeviceError::Transport(TransportError::Disconnected))
                };
                let _ = response.send(result);
                continue;
            }
            Ok(WorkerCommand::Reconnect {
                transport,
                response,
            }) => {
                connection.transport = transport;
                connection.deframer = Deframer::new();
                connected = true;
                let _ = response.send(Ok(()));
                continue;
            }
            Err(TryRecvError::Empty) => {}
        }

        match connection.read_idle() {
            Err(DeviceError::Transport(_)) => connected = false,
            Ok(false) => thread::sleep(IDLE_READ_PAUSE),
            Ok(true) | Err(_) => {}
        }
    }
}

#[cfg(test)]
// Keep the original stall regressions textually intact after narrowing SessionOptions.
#[allow(clippy::needless_update)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use protocol::{
        Ack, EventAction, EventKind, PushTimer, TYPE_APPLY_CONFIG, TYPE_FACTORY_RESET,
        TYPE_NETWORK_CONFIG, TYPE_PUSH_TIMER, TYPE_TIME_SYNC, TimeSync, decode_wire_frame,
    };

    use super::*;

    #[derive(Clone)]
    struct FakeTransport {
        state: Arc<Mutex<FakeState>>,
    }

    struct FakeState {
        status: StatusResponse,
        reads: VecDeque<Result<Vec<u8>, TransportError>>,
        requests: Vec<Message>,
        reply_modes: VecDeque<ReplyMode>,
        written: Vec<u8>,
        maximum_write: usize,
    }

    enum ReplyMode {
        Normal,
        Response(Message),
        RawAck(u8, Option<u8>),
        Fragmented,
        StaleResponseBeforeReply(u32),
        EventBeforeReply(DeviceEvent),
        EventAfterReply(DeviceEvent),
        NoReply,
        Malformed,
        Disconnect,
    }

    impl FakeTransport {
        fn new(status: StatusResponse) -> (Self, Arc<Mutex<FakeState>>) {
            let state = Arc::new(Mutex::new(FakeState {
                status,
                reads: VecDeque::new(),
                requests: Vec::new(),
                reply_modes: VecDeque::new(),
                written: Vec::new(),
                maximum_write: usize::MAX,
            }));
            (
                Self {
                    state: Arc::clone(&state),
                },
                state,
            )
        }
    }

    impl Transport for FakeTransport {
        fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError> {
            let mut state = self.state.lock().unwrap();
            let count = bytes.len().min(state.maximum_write);
            state.written.extend_from_slice(&bytes[..count]);
            if state.written.last() != Some(&0) {
                return Ok(count);
            }
            let wire = std::mem::take(&mut state.written);
            let frame =
                decode_wire_frame(&wire).map_err(|error| TransportError::Io(error.to_string()))?;
            let request =
                decode_message(&frame).map_err(|error| TransportError::Io(error.to_string()))?;
            state.requests.push(request.clone());
            let response = match state.reply_modes.front() {
                Some(ReplyMode::Response(response)) => response.clone(),
                _ => fake_response(&state.status, &request),
            };
            let response_wire = encode_message(frame.request_id, &response)
                .map_err(|error| TransportError::Io(error.to_string()))?;
            match state.reply_modes.pop_front().unwrap_or(ReplyMode::Normal) {
                ReplyMode::Normal | ReplyMode::Response(_) => {
                    state.reads.push_back(Ok(response_wire));
                }
                ReplyMode::RawAck(acknowledged_type, revision) => {
                    let mut frame = decode_wire_frame(&response_wire).unwrap();
                    frame.message_type = protocol::TYPE_ACK;
                    frame.payload = vec![
                        if revision.is_some() { 0xa2 } else { 0xa1 },
                        0,
                        acknowledged_type,
                    ];
                    if let Some(revision) = revision {
                        frame.payload.extend([1, 0x18, revision]);
                    }
                    state
                        .reads
                        .push_back(Ok(protocol::encode_frame(&frame).unwrap()));
                }
                ReplyMode::Fragmented => {
                    let split = response_wire.len() / 2;
                    state.reads.push_back(Ok(response_wire[..split].to_vec()));
                    state.reads.push_back(Ok(response_wire[split..].to_vec()));
                }
                ReplyMode::StaleResponseBeforeReply(stale_request_id) => {
                    let mut coalesced = encode_message(stale_request_id, &response)
                        .map_err(|error| TransportError::Io(error.to_string()))?;
                    coalesced.extend(response_wire);
                    state.reads.push_back(Ok(coalesced));
                }
                ReplyMode::EventBeforeReply(event) => {
                    let mut coalesced = encode_message(0, &Message::DeviceEvent(event))
                        .map_err(|error| TransportError::Io(error.to_string()))?;
                    coalesced.extend(response_wire);
                    state.reads.push_back(Ok(coalesced));
                }
                ReplyMode::EventAfterReply(event) => {
                    let mut coalesced = response_wire;
                    coalesced.extend(
                        encode_message(0, &Message::DeviceEvent(event))
                            .map_err(|error| TransportError::Io(error.to_string()))?,
                    );
                    state.reads.push_back(Ok(coalesced));
                }
                ReplyMode::NoReply => {}
                ReplyMode::Malformed => state.reads.push_back(Ok(vec![2, 1, 0])),
                ReplyMode::Disconnect => {
                    state.reads.push_back(Err(TransportError::Disconnected));
                }
            }
            Ok(count)
        }

        fn read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportError> {
            let item = self.state.lock().unwrap().reads.pop_front();
            match item {
                Some(Ok(chunk)) => {
                    assert!(chunk.len() <= bytes.len());
                    bytes[..chunk.len()].copy_from_slice(&chunk);
                    Ok(chunk.len())
                }
                Some(Err(error)) => Err(error),
                None => Ok(0),
            }
        }
    }

    fn status(latest_revision: u32, config_revision: u32, uptime_ms: u64) -> StatusResponse {
        StatusResponse {
            firmware_version: "deskmate-m2".into(),
            uptime_ms,
            latest_revision,
            config_revision,
            ..protocol::test_support::sample_status_response()
        }
    }

    fn fake_response(status: &StatusResponse, request: &Message) -> Message {
        match request {
            Message::StatusRequest => Message::StatusResponse(status.clone()),
            Message::TimeSync(_) => Message::Ack(Ack {
                acknowledged_type: TYPE_TIME_SYNC,
                revision: None,
                already_present: None,
            }),
            Message::PushTimer(push) => Message::Ack(Ack {
                acknowledged_type: TYPE_PUSH_TIMER,
                revision: Some(push.revision),
                already_present: None,
            }),
            Message::NetworkConfig(_) => Message::Ack(Ack {
                acknowledged_type: TYPE_NETWORK_CONFIG,
                revision: None,
                already_present: None,
            }),
            Message::FactoryReset => Message::Ack(Ack {
                acknowledged_type: TYPE_FACTORY_RESET,
                revision: None,
                already_present: None,
            }),
            _ => panic!("unexpected fake request: {request:?}"),
        }
    }

    fn event(sequence: u64, kind: EventKind, token: Option<u32>) -> DeviceEvent {
        DeviceEvent {
            sequence,
            kind,
            card_id: "timer".into(),
            action: if kind == EventKind::InterruptDismissed {
                EventAction::DismissInterrupt
            } else {
                EventAction::StartPause
            },
            interrupt_token: token,
        }
    }

    fn options() -> SessionOptions {
        SessionOptions {
            request_timeout: Duration::from_millis(250),
        }
    }

    fn network_config() -> NetworkConfig {
        NetworkConfig {
            ssid: "desk-wifi".into(),
            psk: "hunter2".into(),
            server_url: "wss://example.invalid/v1/device/link".into(),
            device_id: "dev-0001".into(),
            token: "placeholder".into(),
            utc_offset_minutes: 240,
            tier: protocol::Tier::Networked,
        }
    }

    #[test]
    fn require_ack_rejects_wrong_type_and_revision() {
        let response = Message::Ack(Ack {
            acknowledged_type: TYPE_PUSH_TIMER,
            revision: Some(7),
            already_present: None,
        });

        assert_eq!(
            require_ack(&response, TYPE_APPLY_CONFIG, Some(7)),
            Err(DeviceError::UnexpectedMessage)
        );
        assert_eq!(
            require_ack(&response, TYPE_PUSH_TIMER, Some(8)),
            Err(DeviceError::UnexpectedMessage)
        );
    }

    #[test]
    fn fragmented_response_and_events_on_both_sides_of_reply_preserve_responses() {
        let (transport, state) = FakeTransport::new(status(7, 3, 100));
        {
            let mut state = state.lock().unwrap();
            state.reply_modes.push_back(ReplyMode::Fragmented);
            state
                .reply_modes
                .push_back(ReplyMode::EventBeforeReply(event(1, EventKind::Tap, None)));
            state
                .reply_modes
                .push_back(ReplyMode::EventAfterReply(event(2, EventKind::Tap, None)));
        }
        let session = DeviceSession::with_options(transport, &status(7, 3, 100), options());
        assert_eq!(session.status().unwrap().latest_revision, 7);
        assert_eq!(session.status().unwrap().latest_revision, 7);
        assert_eq!(session.status().unwrap().latest_revision, 7);
    }

    #[test]
    fn late_response_does_not_fail_the_session_request_waiting_for_its_own_response() {
        let (transport, state) = FakeTransport::new(status(7, 3, 100));
        state
            .lock()
            .unwrap()
            .reply_modes
            .push_back(ReplyMode::StaleResponseBeforeReply(41));
        let session = DeviceSession::with_options(transport, &status(7, 3, 100), options());

        assert_eq!(session.status().unwrap(), status(7, 3, 100));
    }

    #[test]
    fn firmware_without_networking_refuses_cable_operations_before_wire_mutation() {
        let mut without_networking = status(0, 0, 100);
        without_networking.capabilities &= !protocol::CAPABILITY_NETWORKING;
        let (transport, state) = FakeTransport::new(without_networking.clone());
        let session = DeviceSession::with_options(transport, &without_networking, options());
        let network_config = network_config();
        let expected = Err(DeviceError::MissingCapabilities {
            required: protocol::CAPABILITY_NETWORKING,
            available: without_networking.capabilities,
        });
        assert_eq!(
            [session.provision(&network_config), session.factory_reset()],
            [expected.clone(), expected]
        );
        assert!(state.lock().unwrap().requests.iter().all(|request| {
            !matches!(request, Message::NetworkConfig(_) | Message::FactoryReset)
        }));
    }

    #[test]
    fn timeout_and_malformed_response_keep_distinct_error_classes() {
        let (timeout_transport, timeout_state) = FakeTransport::new(status(0, 0, 100));
        timeout_state
            .lock()
            .unwrap()
            .reply_modes
            .push_back(ReplyMode::NoReply);
        let timeout_session = DeviceSession::with_options(
            timeout_transport,
            &status(0, 0, 100),
            SessionOptions {
                request_timeout: Duration::from_millis(20),
            },
        );
        assert_eq!(timeout_session.status(), Err(DeviceError::Timeout));

        let (malformed_transport, malformed_state) = FakeTransport::new(status(0, 0, 100));
        malformed_state
            .lock()
            .unwrap()
            .reply_modes
            .push_back(ReplyMode::Malformed);
        let malformed_session =
            DeviceSession::with_options(malformed_transport, &status(0, 0, 100), options());
        assert!(matches!(
            malformed_session.status(),
            Err(DeviceError::MalformedResponse(_))
        ));
    }

    #[test]
    fn disconnect_during_request_is_distinct_and_later_requests_do_not_write() {
        let (first_transport, first_state) = FakeTransport::new(status(7, 3, 1_000));
        let session = DeviceSession::with_options(first_transport, &status(7, 3, 1_000), options());
        first_state
            .lock()
            .unwrap()
            .reply_modes
            .push_back(ReplyMode::Disconnect);
        assert_eq!(
            session.status(),
            Err(DeviceError::Transport(TransportError::Disconnected))
        );
        let request_count = first_state.lock().unwrap().requests.len();
        assert_eq!(
            session.status(),
            Err(DeviceError::Transport(TransportError::Disconnected))
        );
        assert_eq!(first_state.lock().unwrap().requests.len(), request_count);
    }

    #[test]
    fn reconnect_after_request_disconnect_uses_the_replacement_transport() {
        let (first_transport, first_state) = FakeTransport::new(status(7, 3, 1_000));
        let session = DeviceSession::with_options(first_transport, &status(7, 3, 1_000), options());
        first_state
            .lock()
            .unwrap()
            .reply_modes
            .push_back(ReplyMode::Disconnect);
        assert_eq!(
            session.status(),
            Err(DeviceError::Transport(TransportError::Disconnected))
        );

        let (second_transport, second_state) = FakeTransport::new(status(0, 0, 50));
        session
            .reconnect(second_transport, status(0, 0, 50))
            .unwrap();
        assert_eq!(session.status().unwrap(), status(0, 0, 50));
        assert_eq!(
            first_state.lock().unwrap().requests,
            vec![Message::StatusRequest]
        );
        assert_eq!(
            second_state.lock().unwrap().requests,
            vec![Message::StatusRequest]
        );
    }

    #[test]
    fn provision_sends_the_config_and_accepts_the_ack() {
        let (transport, state) = FakeTransport::new(status(0, 0, 100));
        let session = DeviceSession::with_options(transport, &status(0, 0, 100), options());
        let network_config = network_config();
        session.provision(&network_config).unwrap();
        assert_eq!(
            state.lock().unwrap().requests.last(),
            Some(&Message::NetworkConfig(network_config))
        );
    }

    #[test]
    fn factory_reset_sends_the_request_and_accepts_the_ack() {
        let (transport, state) = FakeTransport::new(status(0, 0, 100));
        let session = DeviceSession::with_options(transport, &status(0, 0, 100), options());
        session.factory_reset().unwrap();
        assert_eq!(
            state.lock().unwrap().requests.last(),
            Some(&Message::FactoryReset)
        );
    }

    // Device-level characterization of the five cable messages sent by the CLI.
    // Exercise DeviceClient and DeviceSession directly; these tests do not invoke
    // CLI dispatch or characterize its output, argument mapping, or exit codes.
    fn device_cable_request(
        transport: FakeTransport,
        initial_status: &StatusResponse,
        request: Message,
    ) -> Result<(), DeviceError> {
        match request {
            Message::StatusRequest => {
                let actual = crate::DeviceClient::new(transport).status()?;
                assert_eq!(&actual, initial_status);
                Ok(())
            }
            Message::TimeSync(sync) => crate::DeviceClient::new(transport)
                .time_sync(sync)
                .map(|_| ()),
            Message::PushTimer(push) => crate::DeviceClient::new(transport)
                .push_timer(push)
                .map(|_| ()),
            Message::NetworkConfig(config) => {
                DeviceSession::with_options(transport, initial_status, options()).provision(&config)
            }
            Message::FactoryReset => {
                DeviceSession::with_options(transport, initial_status, options()).factory_reset()
            }
            other => panic!("not a characterized cable message: {other:?}"),
        }
    }

    fn device_cable_messages() -> Vec<Message> {
        vec![
            Message::StatusRequest,
            Message::TimeSync(TimeSync {
                unix_seconds: 1_789_776_000,
                utc_offset_minutes: 240,
            }),
            Message::PushTimer(PushTimer {
                card_id: "timer".into(),
                revision: 7,
                total_ms: 60_000,
                remaining_ms: 30_000,
                running: true,
            }),
            Message::NetworkConfig(NetworkConfig {
                ssid: "desk-wifi".into(),
                psk: "test-only".into(),
                server_url: "wss://example.invalid/v1/device/link".into(),
                device_id: "dev-test".into(),
                token: "test-only".into(),
                utc_offset_minutes: 240,
                tier: protocol::Tier::Networked,
            }),
            Message::FactoryReset,
        ]
    }

    #[test]
    fn device_cable_messages_preserve_exact_requests_with_partial_io_and_stale_replies() {
        for request in device_cable_messages() {
            for mode in [
                ReplyMode::Fragmented,
                ReplyMode::StaleResponseBeforeReply(41),
            ] {
                let initial_status = status(7, 3, 100);
                let (transport, state) = FakeTransport::new(initial_status.clone());
                {
                    let mut state = state.lock().unwrap();
                    state.maximum_write = 3;
                    state.reply_modes.push_back(mode);
                }
                device_cable_request(transport, &initial_status, request.clone()).unwrap();
                assert_eq!(state.lock().unwrap().requests, vec![request.clone()]);
            }
        }
    }

    #[test]
    fn device_cable_messages_accept_unsolicited_events_before_and_after_the_reply() {
        for request in device_cable_messages() {
            for mode in [
                ReplyMode::EventBeforeReply(event(1, EventKind::Tap, None)),
                ReplyMode::EventAfterReply(event(2, EventKind::Tap, None)),
            ] {
                let initial_status = status(7, 3, 100);
                let (transport, state) = FakeTransport::new(initial_status.clone());
                state.lock().unwrap().reply_modes.push_back(mode);
                device_cable_request(transport, &initial_status, request.clone()).unwrap();
                assert_eq!(state.lock().unwrap().requests, vec![request.clone()]);
            }
        }
    }

    #[test]
    fn device_cable_mutations_require_exact_ack_type_and_revision() {
        for request in device_cable_messages().into_iter().skip(1) {
            let revision = match &request {
                Message::PushTimer(push) => Some(push.revision),
                _ => None,
            };
            for (acknowledged_type, ack_revision) in [
                (protocol::TYPE_ACTIVATE_CARD, None),
                (request.type_id(), Some(99)),
                (request.type_id(), None),
            ] {
                if acknowledged_type == request.type_id() && ack_revision == revision {
                    continue;
                }
                let initial_status = status(7, 3, 100);
                let (transport, state) = FakeTransport::new(initial_status.clone());
                state
                    .lock()
                    .unwrap()
                    .reply_modes
                    .push_back(ReplyMode::RawAck(
                        acknowledged_type,
                        ack_revision.map(|value| u8::try_from(value).unwrap()),
                    ));
                let result = device_cable_request(transport, &initial_status, request.clone());
                if acknowledged_type == request.type_id()
                    && ack_revision.is_some() != revision.is_some()
                {
                    assert!(matches!(result, Err(DeviceError::MalformedResponse(_))));
                } else {
                    assert_eq!(result, Err(DeviceError::UnexpectedMessage));
                }
                assert_eq!(state.lock().unwrap().requests, vec![request.clone()]);
            }
        }
    }

    #[test]
    fn device_cable_messages_preserve_malformed_rejected_and_disconnect_errors() {
        let rejection = protocol::ErrorResponse {
            code: protocol::ErrorCode::Busy,
            diagnostic: "test rejection".into(),
        };
        for request in device_cable_messages() {
            for mode in [
                ReplyMode::Malformed,
                ReplyMode::Disconnect,
                ReplyMode::Response(Message::Error(rejection.clone())),
            ] {
                let expected = match mode {
                    ReplyMode::Disconnect => {
                        Some(DeviceError::Transport(TransportError::Disconnected))
                    }
                    ReplyMode::Response(_) => Some(DeviceError::Rejected(rejection.clone())),
                    _ => None,
                };
                let initial_status = status(7, 3, 100);
                let (transport, state) = FakeTransport::new(initial_status.clone());
                state.lock().unwrap().reply_modes.push_back(mode);
                let result = device_cable_request(transport, &initial_status, request.clone());
                if let Some(expected) = expected {
                    assert_eq!(result, Err(expected));
                } else {
                    assert!(matches!(result, Err(DeviceError::MalformedResponse(_))));
                }
            }
        }
    }

    #[test]
    fn device_cable_administration_without_networking_writes_nothing() {
        for request in device_cable_messages().into_iter().skip(3) {
            let mut initial_status = status(7, 3, 100);
            initial_status.capabilities &= !protocol::CAPABILITY_NETWORKING;
            let (transport, state) = FakeTransport::new(initial_status.clone());
            assert_eq!(
                device_cable_request(transport, &initial_status, request),
                Err(DeviceError::MissingCapabilities {
                    required: protocol::CAPABILITY_NETWORKING,
                    available: initial_status.capabilities,
                })
            );
            assert!(state.lock().unwrap().requests.is_empty());
        }
    }

    #[test]
    fn stalled_request_reports_disconnect_and_subsequent_calls_fail_immediately() {
        let released = Arc::new(AtomicBool::new(false));
        let reading = Arc::new(AtomicBool::new(false));
        let session = DeviceSession::with_options(
            StalledTransport {
                released: Arc::clone(&released),
                reading: Arc::clone(&reading),
            },
            &status(0, 0, 100),
            SessionOptions {
                request_timeout: Duration::from_millis(100),
                ..SessionOptions::default()
            },
        );
        let (cancel, watchdog) = arm_release_watchdog(&released);
        wait_until_blocked_in_read(&reading);
        assert_eq!(
            session.factory_reset(),
            Err(DeviceError::Transport(TransportError::Disconnected))
        );
        assert!(session.is_stalled());
        let started = Instant::now();
        assert_eq!(
            session.factory_reset(),
            Err(DeviceError::Transport(TransportError::Disconnected))
        );
        assert!(started.elapsed() < Duration::from_millis(100));
        drop(session);
        drop(cancel);
        watchdog.join().unwrap();
    }

    /// A transport whose `read` blocks and never returns -- behavior some USB
    /// serial drivers exhibit when the board detaches with a read already in flight.
    /// `SessionConnection::transact` checks its deadline only *between* reads, so
    /// `request_timeout` is unreachable in that state and the worker never answers.
    struct StalledTransport {
        released: Arc<AtomicBool>,
        /// Set as the read begins. A test that acts before the worker is actually
        /// inside the read is testing the wrong thing: the worker would still be
        /// at its command channel and would answer normally.
        reading: Arc<AtomicBool>,
    }

    impl Transport for StalledTransport {
        fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError> {
            Ok(bytes.len())
        }

        fn read(&mut self, _bytes: &mut [u8]) -> Result<usize, TransportError> {
            self.reading.store(true, Ordering::Release);
            while !self.released.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(5));
            }
            Err(TransportError::Disconnected)
        }
    }

    /// Blocks until the session worker is inside `StalledTransport::read`, so a
    /// test acts on a worker that genuinely cannot answer rather than racing it.
    /// Bounded by its own assertion, so it can never hang the suite.
    fn wait_until_blocked_in_read(reading: &AtomicBool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !reading.load(Ordering::Acquire) {
            assert!(
                Instant::now() < deadline,
                "the session worker never reached its transport read"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn arm_release_watchdog(
        released: &Arc<AtomicBool>,
    ) -> (mpsc::Sender<()>, thread::JoinHandle<()>) {
        let (cancel_sender, cancel_receiver) = mpsc::channel();
        let released = Arc::clone(released);
        let watchdog = thread::spawn(move || {
            let _ = cancel_receiver.recv_timeout(Duration::from_secs(2));
            released.store(true, Ordering::Release);
        });
        (cancel_sender, watchdog)
    }

    /// A stalled transport must not be able to wedge the caller. `request` waits on
    /// the worker's reply, and the worker is the very thing a blocking read stops;
    /// without a deadline of its own that wait is unbounded, and one detached cable
    /// freezes every caller above it, including the runtime worker responsible for
    /// subsequent device commands.
    #[test]
    fn a_transport_read_that_never_returns_cannot_wedge_the_caller() {
        let released = Arc::new(AtomicBool::new(false));
        let reading = Arc::new(AtomicBool::new(false));
        let transport = StalledTransport {
            released: Arc::clone(&released),
            reading: Arc::clone(&reading),
        };
        let session = DeviceSession::with_options(
            transport,
            &status(0, 0, 100),
            SessionOptions {
                request_timeout: Duration::from_millis(100),
                ..SessionOptions::default()
            },
        );

        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        let caller = thread::spawn(move || {
            let started = Instant::now();
            let outcome = session.status();
            let _ = done_sender.send((outcome.is_err(), started.elapsed()));
            // Handed back so the session is dropped after the read is released;
            // `Drop` no longer joins a stalled worker (next test), so this only keeps
            // the worker thread from outliving the test.
            session
        });

        let observed = done_receiver.recv_timeout(Duration::from_secs(2));
        released.store(true, Ordering::Release);
        drop(caller.join().expect("the calling thread must not panic"));

        let (failed, elapsed) =
            observed.expect("`request` must answer within its own deadline, but it hung");
        assert!(failed, "a stalled transport must not report success");
        assert!(
            elapsed < Duration::from_secs(2),
            "`request` answered only after {elapsed:?}"
        );
    }

    /// Dropping a stalled session must not join the worker, because the worker is
    /// exactly the thread that cannot return. Otherwise `Drop` waits on the blocked
    /// read and can prevent the host process from exiting until the operating system
    /// finally tears the device node down.
    #[test]
    fn dropping_a_stalled_session_does_not_wait_for_a_thread_that_cannot_return() {
        let released = Arc::new(AtomicBool::new(false));
        let reading = Arc::new(AtomicBool::new(false));
        let session = DeviceSession::with_options(
            StalledTransport {
                released: Arc::clone(&released),
                reading: Arc::clone(&reading),
            },
            &status(0, 0, 100),
            SessionOptions {
                request_timeout: Duration::from_millis(100),
                ..SessionOptions::default()
            },
        );

        // Armed before the first request, not after it: every wait this test can
        // regress into -- the request's and the drop's -- is then released on its
        // own, so a regression fails an assertion instead of hanging the suite.
        let (cancel, watchdog) = arm_release_watchdog(&released);

        assert!(
            session.status().is_err(),
            "a stalled transport must not report success"
        );
        assert!(
            session.status().is_err(),
            "a session already known to be stalled must keep refusing"
        );

        let started = Instant::now();
        drop(session);
        let elapsed = started.elapsed();

        drop(cancel);
        watchdog.join().expect("the watchdog must not panic");
        assert!(
            elapsed < Duration::from_secs(1),
            "dropping a stalled session blocked for {elapsed:?}"
        );
    }

    /// `reconnect` hands a fresh transport to the *existing* worker, so a worker
    /// stuck in a read can never take it -- and waiting for its reply is the same
    /// unbounded wait `request` guards against. The runtime reconnects on a timer
    /// after a disconnection, so leaving this one unbounded would re-wedge it
    /// moments after the first stalled request returned.
    #[test]
    fn reconnecting_a_stalled_session_does_not_wait_on_a_worker_that_cannot_answer() {
        let released = Arc::new(AtomicBool::new(false));
        let reading = Arc::new(AtomicBool::new(false));
        let session = DeviceSession::with_options(
            StalledTransport {
                released: Arc::clone(&released),
                reading: Arc::clone(&reading),
            },
            &status(0, 0, 100),
            SessionOptions {
                request_timeout: Duration::from_millis(100),
                ..SessionOptions::default()
            },
        );
        // Without this the worker may still be at its command channel, take the
        // Reconnect, and answer -- which tests nothing.
        wait_until_blocked_in_read(&reading);
        let (cancel, watchdog) = arm_release_watchdog(&released);

        let started = Instant::now();
        let outcome = session.reconnect(
            StalledTransport {
                released: Arc::clone(&released),
                reading: Arc::clone(&reading),
            },
            status(0, 0, 100),
        );
        let elapsed = started.elapsed();

        drop(cancel);
        watchdog.join().expect("the watchdog must not panic");
        assert!(
            outcome.is_err(),
            "a worker that cannot answer must not report a reconnection"
        );
        assert!(
            elapsed < Duration::from_secs(1),
            "`reconnect` waited {elapsed:?} on a worker that cannot answer"
        );
    }

    /// The same hang, reached without a single request: a session sitting idle
    /// reads the transport between keepalives, so a cable pulled while nothing is
    /// in flight blocks the worker with no request to time out and nothing to set
    /// `stalled`. Dropping it must still not wait on that thread.
    #[test]
    fn dropping_an_idle_session_blocked_in_a_read_does_not_wait_for_it() {
        let released = Arc::new(AtomicBool::new(false));
        let reading = Arc::new(AtomicBool::new(false));
        let session = DeviceSession::with_options(
            StalledTransport {
                released: Arc::clone(&released),
                reading: Arc::clone(&reading),
            },
            &status(0, 0, 100),
            SessionOptions {
                request_timeout: Duration::from_millis(100),
                ..SessionOptions::default()
            },
        );
        wait_until_blocked_in_read(&reading);
        let (cancel, watchdog) = arm_release_watchdog(&released);
        assert!(
            !session.is_stalled(),
            "no request was made, so nothing can have marked this session stalled"
        );

        let started = Instant::now();
        drop(session);
        let elapsed = started.elapsed();

        drop(cancel);
        watchdog.join().expect("the watchdog must not panic");
        assert!(
            elapsed < Duration::from_secs(1),
            "dropping an idle blocked session waited {elapsed:?}"
        );
    }
}
