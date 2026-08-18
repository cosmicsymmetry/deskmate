use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use protocol::{
    Ack, ActivateScreen, ApplyConfig, CAPABILITY_CONFIG_ROTATION, CAPABILITY_CORE_WIDGETS,
    Deframer, DeviceEvent, EventAction, EventKind, Field, HeartbeatAck, Message, NetworkConfig,
    PushData, ScreenConfig, StatusResponse, TYPE_ACTIVATE_SCREEN, TYPE_APPLY_CONFIG,
    TYPE_FACTORY_RESET, TYPE_NETWORK_CONFIG, TYPE_PUSH_DATA, TYPE_TIME_SYNC,
    TYPE_TRIGGER_INTERRUPT, TimeSync, TriggerInterrupt, WidgetConfig, decode_message,
    encode_message,
};

use crate::{
    ConnectedDevice, DeviceClient, DeviceError, SerialTransport, Transport, TransportError,
    connect, message_error,
};

pub const DEFAULT_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(3);
pub const DEFAULT_EVENT_QUEUE_CAPACITY: usize = 32;
const COMMAND_QUEUE_CAPACITY: usize = 8;
const IDLE_READ_PAUSE: Duration = Duration::from_millis(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionOptions {
    pub request_timeout: Duration,
    pub keepalive_interval: Duration,
    pub event_queue_capacity: usize,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            request_timeout: crate::DEFAULT_REQUEST_TIMEOUT,
            keepalive_interval: DEFAULT_KEEPALIVE_INTERVAL,
            event_queue_capacity: DEFAULT_EVENT_QUEUE_CAPACITY,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedEvent {
    pub event: DeviceEvent,
    pub missed_before: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionDiagnostics {
    pub keepalives_sent: u64,
    pub reconnects: u64,
    pub duplicate_or_out_of_order_events: u64,
    pub locally_dropped_events: u64,
    pub detected_event_gaps: u64,
    pub malformed_device_frames: u64,
    pub unexpected_device_frames: u64,
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

enum WorkerCommand<T> {
    Request {
        message: Message,
        response: SyncSender<Result<Message, DeviceError>>,
    },
    Reconnect {
        transport: T,
        status: StatusResponse,
        response: SyncSender<Result<(), DeviceError>>,
    },
    Shutdown,
}

#[derive(Clone, Default)]
struct ReplayState {
    time_sync: Option<(TimeSync, Instant)>,
    config: Option<ApplyConfig>,
    pushes: Vec<PushData>,
    active_screen: Option<ActivateScreen>,
    interrupts: Vec<TriggerInterrupt>,
}

pub struct DeviceSession<T: Transport + Send + 'static> {
    command_sender: SyncSender<WorkerCommand<T>>,
    event_receiver: Receiver<ReceivedEvent>,
    latest_data_revision: Arc<AtomicU32>,
    latest_config_revision: Arc<AtomicU32>,
    capabilities: Arc<AtomicU64>,
    diagnostics: Arc<DiagnosticCounters>,
    worker: Option<JoinHandle<()>>,
}

impl<T: Transport + Send + 'static> DeviceSession<T> {
    pub fn new(transport: T, initial_status: &StatusResponse) -> Self {
        Self::with_options(transport, initial_status, SessionOptions::default())
    }

    pub fn with_options(
        transport: T,
        initial_status: &StatusResponse,
        options: SessionOptions,
    ) -> Self {
        let event_capacity = options.event_queue_capacity.max(1);
        let (command_sender, command_receiver) = mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
        let (event_sender, event_receiver) = mpsc::sync_channel(event_capacity);
        let latest_data_revision = Arc::new(AtomicU32::new(initial_status.latest_revision));
        let latest_config_revision = Arc::new(AtomicU32::new(initial_status.config_revision));
        let capabilities = Arc::new(AtomicU64::new(initial_status.capabilities));
        let diagnostics = Arc::new(DiagnosticCounters::default());
        let connection = SessionConnection {
            transport,
            deframer: Deframer::new(),
            next_request_id: 1,
            request_timeout: options.request_timeout,
            keepalive_interval: options.keepalive_interval,
            last_request_finished: Instant::now(),
            event_sender,
            last_seen_event_sequence: None,
            last_queued_event_sequence: None,
            latest_data_revision: Arc::clone(&latest_data_revision),
            latest_config_revision: Arc::clone(&latest_config_revision),
            diagnostics: Arc::clone(&diagnostics),
            replay: ReplayState::default(),
            last_device_uptime_ms: initial_status.uptime_ms,
        };
        let worker = thread::Builder::new()
            .name("deskmate-device-session".into())
            .spawn(move || run_worker(connection, &command_receiver))
            .expect("failed to spawn Deskmate device-session worker");
        Self {
            command_sender,
            event_receiver,
            latest_data_revision,
            latest_config_revision,
            capabilities,
            diagnostics,
            worker: Some(worker),
        }
    }

    fn request(&self, message: Message) -> Result<Message, DeviceError> {
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.command_sender
            .send(WorkerCommand::Request {
                message,
                response: response_sender,
            })
            .map_err(|_| DeviceError::Transport(TransportError::Disconnected))?;
        response_receiver
            .recv()
            .map_err(|_| DeviceError::Transport(TransportError::Disconnected))?
    }

    fn allocate_revision(counter: &AtomicU32) -> Result<u32, DeviceError> {
        let mut current = counter.load(Ordering::Acquire);
        loop {
            let next = current
                .checked_add(1)
                .ok_or(DeviceError::RevisionExhausted)?;
            match counter.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return Ok(next),
                Err(observed) => current = observed,
            }
        }
    }

    pub fn latest_data_revision(&self) -> u32 {
        self.latest_data_revision.load(Ordering::Acquire)
    }

    pub fn latest_config_revision(&self) -> u32 {
        self.latest_config_revision.load(Ordering::Acquire)
    }

    pub fn capabilities(&self) -> u64 {
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

    pub fn time_sync(&self, sync: TimeSync) -> Result<Ack, DeviceError> {
        match self.request(Message::TimeSync(sync))? {
            Message::Ack(ack)
                if ack.acknowledged_type == TYPE_TIME_SYNC && ack.revision.is_none() =>
            {
                Ok(ack)
            }
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }

    pub fn push_data(&self, push: PushData) -> Result<Ack, DeviceError> {
        let revision = push.revision;
        match self.request(Message::PushData(push))? {
            Message::Ack(ack)
                if ack.acknowledged_type == TYPE_PUSH_DATA && ack.revision == Some(revision) =>
            {
                self.latest_data_revision
                    .fetch_max(revision, Ordering::AcqRel);
                Ok(ack)
            }
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }

    pub fn push_fields(
        &self,
        widget_id: impl Into<String>,
        fields: Vec<Field>,
    ) -> Result<Ack, DeviceError> {
        let revision = Self::allocate_revision(&self.latest_data_revision)?;
        self.push_data(PushData {
            widget_id: widget_id.into(),
            revision,
            fields,
        })
    }

    pub fn apply_config(&self, config: ApplyConfig) -> Result<Ack, DeviceError> {
        let required = required_config_capabilities(&config);
        let available = self.capabilities();
        ensure_capabilities(required, available)?;
        let revision = config.revision;
        match self.request(Message::ApplyConfig(config))? {
            Message::Ack(ack)
                if ack.acknowledged_type == TYPE_APPLY_CONFIG && ack.revision == Some(revision) =>
            {
                self.latest_config_revision
                    .fetch_max(revision, Ordering::AcqRel);
                Ok(ack)
            }
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }

    pub fn apply_next_config(
        &self,
        rotation: u16,
        widgets: Vec<WidgetConfig>,
        screens: Vec<ScreenConfig>,
    ) -> Result<Ack, DeviceError> {
        let revision = Self::allocate_revision(&self.latest_config_revision)?;
        self.apply_config(ApplyConfig {
            revision,
            rotation,
            widgets,
            screens,
        })
    }

    pub fn activate_screen(&self, activation: ActivateScreen) -> Result<Ack, DeviceError> {
        match self.request(Message::ActivateScreen(activation))? {
            Message::Ack(ack)
                if ack.acknowledged_type == TYPE_ACTIVATE_SCREEN && ack.revision.is_none() =>
            {
                Ok(ack)
            }
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }

    pub fn trigger_interrupt(&self, interrupt: TriggerInterrupt) -> Result<Ack, DeviceError> {
        match self.request(Message::TriggerInterrupt(interrupt))? {
            Message::Ack(ack)
                if ack.acknowledged_type == TYPE_TRIGGER_INTERRUPT && ack.revision.is_none() =>
            {
                Ok(ack)
            }
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }

    /// Provision the device's network config over USB. The device persists
    /// this and applies the resulting tier on its *next* boot -- it never
    /// hot-swaps ownership mid-session, so this ACK does not mean the device
    /// is networked yet.
    pub fn provision(&self, config: &NetworkConfig) -> Result<Ack, DeviceError> {
        match self.request(Message::NetworkConfig(config.clone()))? {
            Message::Ack(ack)
                if ack.acknowledged_type == TYPE_NETWORK_CONFIG && ack.revision.is_none() =>
            {
                Ok(ack)
            }
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }

    /// Erase the device's persisted network config, returning it to
    /// factory-fresh local tier on its next boot.
    pub fn factory_reset(&self) -> Result<Ack, DeviceError> {
        match self.request(Message::FactoryReset)? {
            Message::Ack(ack)
                if ack.acknowledged_type == TYPE_FACTORY_RESET && ack.revision.is_none() =>
            {
                Ok(ack)
            }
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }

    pub fn heartbeat(&self) -> Result<HeartbeatAck, DeviceError> {
        match self.request(Message::Heartbeat)? {
            Message::HeartbeatAck(ack) => Ok(ack),
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }

    pub fn try_recv_event(&self) -> Option<ReceivedEvent> {
        self.event_receiver.try_recv().ok()
    }

    pub fn recv_event_timeout(
        &self,
        timeout: Duration,
    ) -> Result<Option<ReceivedEvent>, DeviceError> {
        match self.event_receiver.recv_timeout(timeout) {
            Ok(event) => Ok(Some(event)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                Err(DeviceError::Transport(TransportError::Disconnected))
            }
        }
    }

    pub fn diagnostics(&self) -> SessionDiagnostics {
        self.diagnostics.snapshot()
    }

    pub fn reconnect(
        &self,
        transport: T,
        initial_status: StatusResponse,
    ) -> Result<(), DeviceError> {
        let capabilities = initial_status.capabilities;
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.command_sender
            .send(WorkerCommand::Reconnect {
                transport,
                status: initial_status,
                response: response_sender,
            })
            .map_err(|_| DeviceError::Transport(TransportError::Disconnected))?;
        let result = response_receiver
            .recv()
            .map_err(|_| DeviceError::Transport(TransportError::Disconnected))?;
        // The worker has replaced its transport even when replay is rejected. Publish
        // that device's capabilities so every later request is gated against the
        // actual connection rather than the previous firmware.
        self.capabilities.store(capabilities, Ordering::Release);
        result
    }
}

impl<T: Transport + Send + 'static> Drop for DeviceSession<T> {
    fn drop(&mut self) {
        let _ = self.command_sender.send(WorkerCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
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
    next_request_id: u32,
    request_timeout: Duration,
    keepalive_interval: Duration,
    last_request_finished: Instant,
    event_sender: SyncSender<ReceivedEvent>,
    last_seen_event_sequence: Option<u64>,
    last_queued_event_sequence: Option<u64>,
    latest_data_revision: Arc<AtomicU32>,
    latest_config_revision: Arc<AtomicU32>,
    diagnostics: Arc<DiagnosticCounters>,
    replay: ReplayState,
    last_device_uptime_ms: u64,
}

impl<T: Transport> SessionConnection<T> {
    fn allocate_request_id(&mut self) -> u32 {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        if self.next_request_id == 0 {
            self.next_request_id = 1;
        }
        request_id
    }

    fn keepalive_due(&self) -> bool {
        self.last_request_finished.elapsed() >= self.keepalive_interval
    }

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
            DeviceClient::<T>::expected_response(request).ok_or(DeviceError::InvalidRequest)?;
        let request_id = self.allocate_request_id();
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
                        self.diagnostics
                            .malformed_device_frames
                            .fetch_add(1, Ordering::Relaxed);
                        if response.is_none() {
                            return Err(DeviceError::MalformedResponse(error.to_string()));
                        }
                        continue;
                    }
                };
                let message = decode_message(&frame).map_err(message_error)?;
                if frame.request_id == 0 {
                    match message {
                        Message::DeviceEvent(event) => self.route_event(event),
                        _ => return Err(DeviceError::UnexpectedMessage),
                    }
                    continue;
                }
                if frame.request_id != request_id {
                    return Err(DeviceError::UnexpectedRequestId {
                        expected: request_id,
                        received: frame.request_id,
                    });
                }
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
                self.last_request_finished = Instant::now();
                return response;
            }
        }
    }

    fn read_idle(&mut self) -> Result<bool, DeviceError> {
        let mut chunk = [0_u8; 512];
        let count = self.transport.read(&mut chunk)?;
        if count == 0 {
            return Ok(false);
        }
        for framed in self.deframer.push(&chunk[..count]) {
            let Ok(frame) = framed else {
                self.diagnostics
                    .malformed_device_frames
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            };
            match decode_message(&frame) {
                Ok(Message::DeviceEvent(event)) if frame.request_id == 0 => {
                    self.route_event(event);
                }
                _ => {
                    self.diagnostics
                        .unexpected_device_frames
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        Ok(true)
    }

    fn route_event(&mut self, event: DeviceEvent) {
        if self
            .last_seen_event_sequence
            .is_some_and(|sequence| event.sequence <= sequence)
        {
            self.diagnostics
                .duplicate_or_out_of_order_events
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.last_seen_event_sequence = Some(event.sequence);
        if event.kind == EventKind::Navigation
            && matches!(
                event.action,
                EventAction::NavigatePrevious | EventAction::NavigateNext
            )
            && self.replay.config.as_ref().is_some_and(|config| {
                config
                    .screens
                    .iter()
                    .any(|screen| screen.screen_id == event.screen_id)
            })
        {
            self.replay.active_screen = Some(ActivateScreen {
                screen_id: event.screen_id.clone(),
            });
        }
        if event.kind == EventKind::InterruptDismissed
            && let Some(token) = event.interrupt_token
        {
            self.replay
                .interrupts
                .retain(|interrupt| interrupt.token != token);
        }
        let missed_before = self.last_queued_event_sequence.map_or(0, |sequence| {
            event.sequence.saturating_sub(sequence).saturating_sub(1)
        });
        let sequence = event.sequence;
        match self.event_sender.try_send(ReceivedEvent {
            event,
            missed_before,
        }) {
            Ok(()) => {
                self.last_queued_event_sequence = Some(sequence);
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

    fn remember_success(&mut self, request: &Message) {
        match request {
            Message::TimeSync(sync) => self.replay.time_sync = Some((*sync, Instant::now())),
            Message::ApplyConfig(config) => {
                let changes_live_model = self.replay.config.as_ref() != Some(config);
                self.replay.config = Some(config.clone());
                if changes_live_model {
                    self.replay.pushes.clear();
                    self.replay.interrupts.clear();
                    if self.replay.active_screen.as_ref().is_some_and(|active| {
                        !config
                            .screens
                            .iter()
                            .any(|screen| screen.screen_id == active.screen_id)
                    }) {
                        self.replay.active_screen = None;
                    }
                }
                self.latest_config_revision
                    .fetch_max(config.revision, Ordering::AcqRel);
            }
            Message::PushData(push) => {
                self.replay
                    .pushes
                    .retain(|cached| cached.widget_id != push.widget_id);
                self.replay.pushes.push(push.clone());
                self.replay
                    .pushes
                    .sort_unstable_by_key(|cached| cached.revision);
                self.latest_data_revision
                    .fetch_max(push.revision, Ordering::AcqRel);
            }
            Message::ActivateScreen(activation) => {
                self.replay.active_screen = Some(activation.clone());
            }
            Message::TriggerInterrupt(interrupt) => {
                self.replay
                    .interrupts
                    .retain(|cached| cached.token != interrupt.token);
                self.replay.interrupts.push(interrupt.clone());
                self.replay
                    .interrupts
                    .sort_unstable_by_key(|cached| cached.token);
            }
            _ => {}
        }
    }

    fn replay_after_reconnect(&mut self, status: &StatusResponse) -> Result<(), DeviceError> {
        let mut replay = self.replay.clone();
        if let Some(config) = &replay.config {
            ensure_capabilities(required_config_capabilities(config), status.capabilities)?;
        }
        let powered_session_reset = status.uptime_ms < self.last_device_uptime_ms
            || (status.latest_revision == 0
                && status.config_revision == 0
                && (self.latest_data_revision.load(Ordering::Acquire) != 0
                    || self.latest_config_revision.load(Ordering::Acquire) != 0));
        if powered_session_reset {
            self.last_seen_event_sequence = None;
            self.last_queued_event_sequence = None;
        }
        self.last_device_uptime_ms = status.uptime_ms;
        self.latest_data_revision
            .store(status.latest_revision, Ordering::Release);
        self.latest_config_revision
            .store(status.config_revision, Ordering::Release);

        if let Some((mut sync, synchronized_at)) = replay.time_sync {
            if let Ok(elapsed) = i64::try_from(synchronized_at.elapsed().as_secs())
                && let Some(adjusted) = sync.unix_seconds.checked_add(elapsed)
            {
                sync.unix_seconds = adjusted;
            }
            Self::require_ack(
                &self.transact(&Message::TimeSync(sync))?,
                TYPE_TIME_SYNC,
                None,
            )?;
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
                Self::require_ack(
                    &self.transact(&Message::ApplyConfig(config.clone()))?,
                    TYPE_APPLY_CONFIG,
                    Some(config.revision),
                )?;
                config_applied = true;
            }
            self.latest_config_revision
                .store(config.revision, Ordering::Release);
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
            Self::require_ack(
                &self.transact(&Message::PushData(push.clone()))?,
                TYPE_PUSH_DATA,
                Some(push.revision),
            )?;
            data_revision = push.revision;
        }
        self.latest_data_revision
            .store(data_revision, Ordering::Release);

        if let Some(activation) = &replay.active_screen {
            Self::require_ack(
                &self.transact(&Message::ActivateScreen(activation.clone()))?,
                TYPE_ACTIVATE_SCREEN,
                None,
            )?;
        }
        for interrupt in &replay.interrupts {
            Self::require_ack(
                &self.transact(&Message::TriggerInterrupt(interrupt.clone()))?,
                TYPE_TRIGGER_INTERRUPT,
                None,
            )?;
        }
        self.replay = replay;
        Ok(())
    }

    fn require_ack(
        response: &Message,
        acknowledged_type: u8,
        revision: Option<u32>,
    ) -> Result<(), DeviceError> {
        match response {
            Message::Ack(ack)
                if ack.acknowledged_type == acknowledged_type && ack.revision == revision =>
            {
                Ok(())
            }
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }
}

fn required_config_capabilities(config: &ApplyConfig) -> u64 {
    CAPABILITY_CORE_WIDGETS
        | if config.rotation == 270 {
            CAPABILITY_CONFIG_ROTATION
        } else {
            0
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
        match command_receiver.try_recv() {
            Ok(WorkerCommand::Shutdown) | Err(TryRecvError::Disconnected) => break,
            Ok(WorkerCommand::Request { message, response }) => {
                let result = if connected {
                    let result = connection.transact(&message);
                    if result.is_ok() {
                        connection.remember_success(&message);
                    }
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
                status,
                response,
            }) => {
                connection.transport = transport;
                connection.deframer = Deframer::new();
                connection.last_request_finished = Instant::now();
                let result = connection.replay_after_reconnect(&status);
                connected = !matches!(result, Err(DeviceError::Transport(_)));
                if result.is_ok() {
                    connection
                        .diagnostics
                        .reconnects
                        .fetch_add(1, Ordering::Relaxed);
                }
                let _ = response.send(result);
                continue;
            }
            Err(TryRecvError::Empty) => {}
        }

        if !connected {
            match command_receiver.recv() {
                Ok(WorkerCommand::Shutdown) | Err(_) => break,
                Ok(command) => match command {
                    WorkerCommand::Request { response, .. } => {
                        let _ = response
                            .send(Err(DeviceError::Transport(TransportError::Disconnected)));
                    }
                    WorkerCommand::Reconnect {
                        transport,
                        status,
                        response,
                    } => {
                        connection.transport = transport;
                        connection.deframer = Deframer::new();
                        connection.last_request_finished = Instant::now();
                        let result = connection.replay_after_reconnect(&status);
                        connected = !matches!(result, Err(DeviceError::Transport(_)));
                        if result.is_ok() {
                            connection
                                .diagnostics
                                .reconnects
                                .fetch_add(1, Ordering::Relaxed);
                        }
                        let _ = response.send(result);
                    }
                    WorkerCommand::Shutdown => break,
                },
            }
            continue;
        }

        if connection.keepalive_due() {
            let result = connection.transact(&Message::Heartbeat);
            if result.is_ok() {
                connection
                    .diagnostics
                    .keepalives_sent
                    .fetch_add(1, Ordering::Relaxed);
            } else if matches!(result, Err(DeviceError::Transport(_))) {
                connected = false;
            }
            continue;
        }

        match connection.read_idle() {
            Err(DeviceError::Transport(_)) => connected = false,
            Ok(false) => thread::sleep(IDLE_READ_PAUSE),
            Ok(true) | Err(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use protocol::{
        EventAction, InterruptPolicy, PROTOCOL_VERSION, SizeClass, TapAction, TemplateKind,
        decode_wire_frame,
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
    }

    enum ReplyMode {
        Normal,
        Fragmented,
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
            let frame =
                decode_wire_frame(bytes).map_err(|error| TransportError::Io(error.to_string()))?;
            let request =
                decode_message(&frame).map_err(|error| TransportError::Io(error.to_string()))?;
            let mut state = self.state.lock().unwrap();
            state.requests.push(request.clone());
            let response = fake_response(&state.status, &request);
            let response_wire = encode_message(frame.request_id, &response)
                .map_err(|error| TransportError::Io(error.to_string()))?;
            match state.reply_modes.pop_front().unwrap_or(ReplyMode::Normal) {
                ReplyMode::Normal => state.reads.push_back(Ok(response_wire)),
                ReplyMode::Fragmented => {
                    let split = response_wire.len() / 2;
                    state.reads.push_back(Ok(response_wire[..split].to_vec()));
                    state.reads.push_back(Ok(response_wire[split..].to_vec()));
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
            Ok(bytes.len())
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
            protocol_version: PROTOCOL_VERSION,
            max_protocol_version: protocol::MAX_PROTOCOL_VERSION,
            capabilities: protocol::CURRENT_CAPABILITIES,
            firmware_version: "deskmate-m2".into(),
            uptime_ms,
            free_heap: 100_000,
            display_width: 448,
            display_height: 368,
            brightness: 200,
            rotation: 90,
            online: true,
            latest_revision,
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
            config_revision,
            latest_interrupt_token: 0,
            tier: protocol::Tier::Local,
            wifi_state: protocol::WifiState::Down,
            wifi_rssi: 0,
            ip: String::new(),
            ota_state: protocol::OtaState::Idle,
            last_network_error: None,
        }
    }

    fn fake_response(status: &StatusResponse, request: &Message) -> Message {
        match request {
            Message::StatusRequest => Message::StatusResponse(status.clone()),
            Message::Heartbeat => Message::HeartbeatAck(HeartbeatAck {
                uptime_ms: status.uptime_ms,
            }),
            Message::TimeSync(_) => Message::Ack(Ack {
                acknowledged_type: TYPE_TIME_SYNC,
                revision: None,
            }),
            Message::PushData(push) => Message::Ack(Ack {
                acknowledged_type: TYPE_PUSH_DATA,
                revision: Some(push.revision),
            }),
            Message::ApplyConfig(config) => Message::Ack(Ack {
                acknowledged_type: TYPE_APPLY_CONFIG,
                revision: Some(config.revision),
            }),
            Message::ActivateScreen(_) => Message::Ack(Ack {
                acknowledged_type: TYPE_ACTIVATE_SCREEN,
                revision: None,
            }),
            Message::TriggerInterrupt(_) => Message::Ack(Ack {
                acknowledged_type: TYPE_TRIGGER_INTERRUPT,
                revision: None,
            }),
            _ => panic!("unexpected fake request: {request:?}"),
        }
    }

    fn event(sequence: u64, kind: EventKind, token: Option<u32>) -> DeviceEvent {
        DeviceEvent {
            sequence,
            kind,
            widget_id: "timer".into(),
            screen_id: "timer-screen".into(),
            action: if kind == EventKind::InterruptDismissed {
                EventAction::DismissInterrupt
            } else {
                EventAction::StartPause
            },
            interrupt_token: token,
        }
    }

    fn config(revision: u32) -> ApplyConfig {
        ApplyConfig {
            revision,
            rotation: 90,
            widgets: vec![WidgetConfig {
                widget_id: "timer".into(),
                template: TemplateKind::ProgressRing,
                size_class: SizeClass::Standard,
                tap_action: TapAction::StartPause,
                interrupt_policy: InterruptPolicy::Enabled,
            }],
            screens: vec![ScreenConfig {
                screen_id: "timer-screen".into(),
                widget_id: "timer".into(),
            }],
        }
    }

    fn options(keepalive_interval: Duration, event_queue_capacity: usize) -> SessionOptions {
        SessionOptions {
            request_timeout: Duration::from_millis(250),
            keepalive_interval,
            event_queue_capacity,
        }
    }

    fn inject_events(state: &Arc<Mutex<FakeState>>, events: impl IntoIterator<Item = DeviceEvent>) {
        let mut state = state.lock().unwrap();
        for event in events {
            state
                .reads
                .push_back(Ok(encode_message(0, &Message::DeviceEvent(event)).unwrap()));
        }
    }

    fn wait_for(predicate: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_millis(500);
        while !predicate() {
            assert!(Instant::now() < deadline, "condition did not become true");
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn fragmented_response_and_events_on_both_sides_of_reply_are_demultiplexed() {
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
        let session = DeviceSession::with_options(
            transport,
            &status(7, 3, 100),
            options(Duration::from_mins(1), 8),
        );
        assert_eq!(session.status().unwrap().latest_revision, 7);
        assert_eq!(session.status().unwrap().latest_revision, 7);
        assert_eq!(session.status().unwrap().latest_revision, 7);
        assert_eq!(
            session
                .recv_event_timeout(Duration::from_millis(100))
                .unwrap()
                .unwrap()
                .event
                .sequence,
            1
        );
        assert_eq!(
            session
                .recv_event_timeout(Duration::from_millis(100))
                .unwrap()
                .unwrap()
                .event
                .sequence,
            2
        );
    }

    #[test]
    fn duplicate_and_out_of_order_events_are_ignored_and_gaps_are_reported() {
        let (transport, state) = FakeTransport::new(status(0, 0, 100));
        let session = DeviceSession::with_options(
            transport,
            &status(0, 0, 100),
            options(Duration::from_mins(1), 8),
        );
        inject_events(
            &state,
            [
                event(5, EventKind::Tap, None),
                event(5, EventKind::Tap, None),
                event(4, EventKind::Tap, None),
                event(7, EventKind::Tap, None),
            ],
        );
        let first = session
            .recv_event_timeout(Duration::from_millis(100))
            .unwrap()
            .unwrap();
        let second = session
            .recv_event_timeout(Duration::from_millis(100))
            .unwrap()
            .unwrap();
        assert_eq!((first.event.sequence, first.missed_before), (5, 0));
        assert_eq!((second.event.sequence, second.missed_before), (7, 1));
        let diagnostics = session.diagnostics();
        assert_eq!(diagnostics.duplicate_or_out_of_order_events, 2);
        assert_eq!(diagnostics.detected_event_gaps, 1);
    }

    #[test]
    fn bounded_event_queue_drops_newest_and_reports_the_later_gap() {
        let (transport, state) = FakeTransport::new(status(0, 0, 100));
        let session = DeviceSession::with_options(
            transport,
            &status(0, 0, 100),
            options(Duration::from_mins(1), 2),
        );
        inject_events(
            &state,
            [
                event(1, EventKind::Tap, None),
                event(2, EventKind::Tap, None),
                event(3, EventKind::Tap, None),
            ],
        );
        wait_for(|| session.diagnostics().locally_dropped_events == 1);
        assert_eq!(session.try_recv_event().unwrap().event.sequence, 1);
        assert_eq!(session.try_recv_event().unwrap().event.sequence, 2);
        inject_events(&state, [event(4, EventKind::Tap, None)]);
        let recovered = session
            .recv_event_timeout(Duration::from_millis(100))
            .unwrap()
            .unwrap();
        assert_eq!((recovered.event.sequence, recovered.missed_before), (4, 1));
    }

    #[test]
    fn idle_session_emits_keepalives_without_caller_polling() {
        let (transport, state) = FakeTransport::new(status(0, 0, 100));
        let session = DeviceSession::with_options(
            transport,
            &status(0, 0, 100),
            options(Duration::from_millis(20), 8),
        );
        wait_for(|| session.diagnostics().keepalives_sent >= 3);
        let heartbeat_count = state
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|message| matches!(message, Message::Heartbeat))
            .count();
        assert!(heartbeat_count >= 3);
    }

    #[test]
    fn data_revision_starts_above_connect_time_status() {
        let (transport, state) = FakeTransport::new(status(41, 0, 100));
        let session = DeviceSession::with_options(
            transport,
            &status(41, 0, 100),
            options(Duration::from_mins(1), 8),
        );
        let ack = session.push_fields("timer", Vec::new()).unwrap();
        assert_eq!(ack.revision, Some(42));
        assert!(matches!(
            state.lock().unwrap().requests.last(),
            Some(Message::PushData(PushData { revision: 42, .. }))
        ));
    }

    #[test]
    fn legacy_firmware_rejects_rotated_config_before_any_wire_mutation() {
        let mut legacy = status(0, 0, 100);
        legacy.capabilities = protocol::LEGACY_CAPABILITIES;
        let (transport, state) = FakeTransport::new(legacy.clone());
        let session =
            DeviceSession::with_options(transport, &legacy, options(Duration::from_mins(1), 8));
        let mut rotated = config(1);
        rotated.rotation = 270;

        assert_eq!(
            session.apply_config(rotated),
            Err(DeviceError::MissingCapabilities {
                required: protocol::CAPABILITY_CORE_WIDGETS | protocol::CAPABILITY_CONFIG_ROTATION,
                available: protocol::LEGACY_CAPABILITIES,
            })
        );
        assert!(state.lock().unwrap().requests.is_empty());
        assert_eq!(session.latest_config_revision(), 0);
    }

    #[test]
    fn reconnect_preflights_replay_before_time_or_config_mutation() {
        let (transport, _) = FakeTransport::new(status(0, 0, 100));
        let session = DeviceSession::with_options(
            transport,
            &status(0, 0, 100),
            options(Duration::from_mins(1), 8),
        );
        session
            .time_sync(TimeSync {
                unix_seconds: 1_800_000_000,
                utc_offset_minutes: 240,
            })
            .unwrap();
        let mut rotated = config(1);
        rotated.rotation = 270;
        session.apply_config(rotated).unwrap();

        let mut legacy = status(0, 0, 10);
        legacy.capabilities = protocol::LEGACY_CAPABILITIES;
        let (legacy_transport, legacy_state) = FakeTransport::new(legacy.clone());
        assert!(matches!(
            session.reconnect(legacy_transport, legacy),
            Err(DeviceError::MissingCapabilities {
                required,
                available: protocol::LEGACY_CAPABILITIES,
            }) if required
                == protocol::CAPABILITY_CORE_WIDGETS | protocol::CAPABILITY_CONFIG_ROTATION
        ));
        assert!(legacy_state.lock().unwrap().requests.is_empty());
        assert_eq!(session.latest_config_revision(), 1);
        assert_eq!(session.capabilities(), protocol::LEGACY_CAPABILITIES);
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
                keepalive_interval: Duration::from_mins(1),
                event_queue_capacity: 8,
            },
        );
        assert_eq!(timeout_session.status(), Err(DeviceError::Timeout));

        let (malformed_transport, malformed_state) = FakeTransport::new(status(0, 0, 100));
        malformed_state
            .lock()
            .unwrap()
            .reply_modes
            .push_back(ReplyMode::Malformed);
        let malformed_session = DeviceSession::with_options(
            malformed_transport,
            &status(0, 0, 100),
            options(Duration::from_mins(1), 8),
        );
        assert!(matches!(
            malformed_session.status(),
            Err(DeviceError::MalformedResponse(_))
        ));
    }

    #[test]
    fn disconnect_during_request_is_distinct_and_reconnect_replays_state() {
        let (first_transport, first_state) = FakeTransport::new(status(7, 3, 1_000));
        let session = DeviceSession::with_options(
            first_transport,
            &status(7, 3, 1_000),
            options(Duration::from_mins(1), 8),
        );
        session.apply_config(config(4)).unwrap();
        session.push_fields("timer", Vec::new()).unwrap();
        session
            .activate_screen(ActivateScreen {
                screen_id: "timer-screen".into(),
            })
            .unwrap();
        session
            .trigger_interrupt(TriggerInterrupt {
                widget_id: "timer".into(),
                token: 1,
                reason: "done".into(),
            })
            .unwrap();
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
        let replayed = second_state.lock().unwrap().requests.clone();
        assert!(matches!(replayed.first(), Some(Message::ApplyConfig(_))));
        assert!(matches!(replayed.get(1), Some(Message::PushData(_))));
        assert!(matches!(replayed.get(2), Some(Message::ActivateScreen(_))));
        assert!(matches!(
            replayed.get(3),
            Some(Message::TriggerInterrupt(_))
        ));
        assert_eq!(session.latest_config_revision(), 4);
        assert_eq!(session.latest_data_revision(), 8);
        assert_eq!(session.diagnostics().reconnects, 1);
    }

    #[test]
    fn local_navigation_updates_the_screen_replayed_after_power_reset() {
        let (first_transport, first_state) = FakeTransport::new(status(0, 0, 1_000));
        let session = DeviceSession::with_options(
            first_transport,
            &status(0, 0, 1_000),
            options(Duration::from_mins(1), 8),
        );
        let mut layout = config(1);
        layout.widgets.push(WidgetConfig {
            widget_id: "calendar".into(),
            template: TemplateKind::RowList,
            size_class: SizeClass::Standard,
            tap_action: TapAction::None,
            interrupt_policy: InterruptPolicy::Disabled,
        });
        layout.screens.push(ScreenConfig {
            screen_id: "calendar-screen".into(),
            widget_id: "calendar".into(),
        });
        session.apply_config(layout).unwrap();
        session
            .activate_screen(ActivateScreen {
                screen_id: "timer-screen".into(),
            })
            .unwrap();

        inject_events(
            &first_state,
            [DeviceEvent {
                sequence: 1,
                kind: EventKind::Navigation,
                widget_id: "calendar".into(),
                screen_id: "calendar-screen".into(),
                action: EventAction::NavigateNext,
                interrupt_token: None,
            }],
        );
        session.status().unwrap();
        assert_eq!(
            session
                .recv_event_timeout(Duration::from_millis(100))
                .unwrap()
                .unwrap()
                .event
                .screen_id,
            "calendar-screen"
        );

        first_state
            .lock()
            .unwrap()
            .reply_modes
            .push_back(ReplyMode::Disconnect);
        assert!(matches!(
            session.status(),
            Err(DeviceError::Transport(TransportError::Disconnected))
        ));
        let (second_transport, second_state) = FakeTransport::new(status(0, 0, 10));
        session
            .reconnect(second_transport, status(0, 0, 10))
            .unwrap();

        assert!(second_state.lock().unwrap().requests.iter().any(|request| {
            matches!(
                request,
                Message::ActivateScreen(ActivateScreen { screen_id })
                    if screen_id == "calendar-screen"
            )
        }));
    }

    #[test]
    fn same_powered_device_reconnect_does_not_repush_retained_revisions() {
        let (first_transport, _) = FakeTransport::new(status(7, 3, 1_000));
        let session = DeviceSession::with_options(
            first_transport,
            &status(7, 3, 1_000),
            options(Duration::from_mins(1), 8),
        );
        session.apply_config(config(4)).unwrap();
        session.push_fields("timer", Vec::new()).unwrap();

        let (second_transport, second_state) = FakeTransport::new(status(8, 4, 2_000));
        session
            .reconnect(second_transport, status(8, 4, 2_000))
            .unwrap();
        assert!(second_state.lock().unwrap().requests.is_empty());
        assert_eq!(session.latest_data_revision(), 8);
        assert_eq!(session.latest_config_revision(), 4);
    }
}
