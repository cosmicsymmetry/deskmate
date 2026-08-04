use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, RwLock, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use chrono::{DateTime, Offset, Utc};
use chrono_tz::Tz;
use device::{ConnectedSession, DeviceError, ReceivedEvent, SessionDiagnostics, connect_session};
use engine::interrupts::InterruptArbiter;
use engine::pomodoro::{Pomodoro, PomodoroState as EnginePomodoroState};
use protocol::{
    ActivateScreen, EventAction, EventKind, Field, ScreenConfig, StatusResponse, TimeSync,
    TriggerInterrupt, WidgetConfig,
};
use providers::Provider;
use providers::ics::{CalendarOptions, IcsProvider, IcsSource};

use crate::commands::{PomodoroAction, RuntimeCommand, RuntimeError};
use crate::scheduler::Scheduler;
use crate::{
    AppConfig, AppSnapshot, CalendarSource, ConnectionState, DeviceCounters, DeviceSnapshot,
    PersistenceState, PomodoroSnapshot, PomodoroState, ProviderSnapshot, ProviderState,
    RuntimeDiagnostics, RuntimeState, WidgetSettings,
};

pub const DEFAULT_RUNTIME_COMMAND_CAPACITY: usize = 16;
pub const DEFAULT_PROVIDER_JOB_CAPACITY: usize = 8;
pub const DEFAULT_MAX_SUBSCRIBERS: usize = 8;

#[derive(Debug, Clone, Copy)]
pub struct RuntimeOptions {
    pub command_capacity: usize,
    pub provider_job_capacity: usize,
    pub maximum_subscribers: usize,
    pub command_timeout: Duration,
    pub loop_maximum_wait: Duration,
    pub reconnect_interval: Duration,
    pub pomodoro_interval: Duration,
    pub status_interval: Duration,
    pub time_sync_interval: Duration,
    pub provider_queue_retry: Duration,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            command_capacity: DEFAULT_RUNTIME_COMMAND_CAPACITY,
            provider_job_capacity: DEFAULT_PROVIDER_JOB_CAPACITY,
            maximum_subscribers: DEFAULT_MAX_SUBSCRIBERS,
            command_timeout: Duration::from_secs(5),
            loop_maximum_wait: Duration::from_millis(25),
            reconnect_interval: Duration::from_secs(1),
            pomodoro_interval: Duration::from_secs(1),
            status_interval: Duration::from_secs(2),
            time_sync_interval: Duration::from_hours(1),
            provider_queue_retry: Duration::from_millis(250),
        }
    }
}

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
    fn time_sync(&mut self, sync: TimeSync) -> Result<(), DeviceError>;
    fn apply_layout(
        &mut self,
        widgets: Vec<WidgetConfig>,
        screens: Vec<ScreenConfig>,
    ) -> Result<(), DeviceError>;
    fn push_fields(&mut self, widget_id: String, fields: Vec<Field>) -> Result<(), DeviceError>;
    fn activate_screen(&mut self, screen_id: String) -> Result<(), DeviceError>;
    fn trigger_interrupt(&mut self, interrupt: TriggerInterrupt) -> Result<(), DeviceError>;
    fn try_recv_event(&mut self) -> Option<ReceivedEvent>;
    fn diagnostics(&self) -> SessionDiagnostics;
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

    fn connected(&self) -> Result<&ConnectedSession, DeviceError> {
        self.connected.as_ref().ok_or(DeviceError::NoDevice)
    }
}

impl RuntimeDevice for SerialRuntimeDevice {
    fn connect(&mut self) -> Result<DeviceConnection, DeviceError> {
        if let Some(connected) = self.connected.as_mut() {
            connected.reconnect(self.explicit_port.as_deref())?;
            return Ok(DeviceConnection {
                port_name: connected.port_name.clone(),
                status: connected.initial_status.clone(),
            });
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

    fn time_sync(&mut self, sync: TimeSync) -> Result<(), DeviceError> {
        self.connected()?.session.time_sync(sync).map(|_| ())
    }

    fn apply_layout(
        &mut self,
        widgets: Vec<WidgetConfig>,
        screens: Vec<ScreenConfig>,
    ) -> Result<(), DeviceError> {
        self.connected()?
            .session
            .apply_next_config(widgets, screens)
            .map(|_| ())
    }

    fn push_fields(&mut self, widget_id: String, fields: Vec<Field>) -> Result<(), DeviceError> {
        self.connected()?
            .session
            .push_fields(widget_id, fields)
            .map(|_| ())
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

#[derive(Debug, Clone)]
pub struct CalendarRefreshRequest {
    pub generation: u64,
    pub widget_id: String,
    pub source: CalendarSource,
    pub timezone: Tz,
    pub title: String,
    pub refresh_interval: Duration,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CalendarRefreshResult {
    pub generation: u64,
    pub widget_id: String,
    pub fields: Vec<Field>,
    pub refreshed_at: Option<DateTime<Utc>>,
    pub age: Option<Duration>,
    pub stale: bool,
    pub error: Option<String>,
}

pub trait CalendarRefresher: Send + 'static {
    fn refresh(&mut self, request: CalendarRefreshRequest) -> CalendarRefreshResult;
}

#[derive(Default)]
pub struct SystemCalendarRefresher {
    providers: HashMap<String, SystemProviderEntry>,
}

struct SystemProviderEntry {
    generation: u64,
    provider: IcsProvider<providers::ics::SystemIcsLoader>,
}

impl CalendarRefresher for SystemCalendarRefresher {
    fn refresh(&mut self, request: CalendarRefreshRequest) -> CalendarRefreshResult {
        let entry = self
            .providers
            .entry(request.widget_id.clone())
            .or_insert_with(|| SystemProviderEntry {
                generation: request.generation,
                provider: system_provider(&request),
            });
        if entry.generation != request.generation {
            *entry = SystemProviderEntry {
                generation: request.generation,
                provider: system_provider(&request),
            };
        }
        let snapshot = entry.provider.refresh(request.now);
        CalendarRefreshResult {
            generation: request.generation,
            widget_id: request.widget_id,
            fields: entry.provider.fields(&snapshot, request.now),
            refreshed_at: snapshot.refreshed_at,
            age: snapshot.age,
            stale: snapshot.stale,
            error: snapshot.error,
        }
    }
}

fn system_provider(
    request: &CalendarRefreshRequest,
) -> IcsProvider<providers::ics::SystemIcsLoader> {
    let source = match &request.source {
        CalendarSource::File(path) => IcsSource::File(PathBuf::from(path)),
        CalendarSource::Url(url) => IcsSource::Url(url.clone()),
    };
    IcsProvider::system(
        source,
        CalendarOptions {
            default_timezone: request.timezone,
            display_timezone: request.timezone,
            refresh_interval: request.refresh_interval,
            title: request.title.clone(),
            ..CalendarOptions::default()
        },
    )
}

struct ProviderWorker {
    sender: Option<SyncSender<CalendarRefreshRequest>>,
    receiver: Receiver<CalendarRefreshResult>,
    worker: Option<JoinHandle<()>>,
}

enum ProviderSubmitError {
    Full,
    Disconnected,
}

impl ProviderWorker {
    fn new(
        mut refresher: Box<dyn CalendarRefresher>,
        capacity: usize,
        diagnostics: Arc<RuntimeDiagnosticCounters>,
    ) -> Self {
        let (sender, request_receiver) = mpsc::sync_channel(capacity.max(1));
        let (result_sender, receiver) = mpsc::sync_channel(capacity.max(1));
        let worker = thread::Builder::new()
            .name("deskmate-provider".into())
            .spawn(move || {
                while let Ok(request) = request_receiver.recv() {
                    let result = refresher.refresh(request);
                    match result_sender.try_send(result) {
                        Ok(()) => {}
                        Err(mpsc::TrySendError::Full(_)) => {
                            diagnostics
                                .provider_results_discarded
                                .fetch_add(1, Ordering::Relaxed);
                        }
                        Err(mpsc::TrySendError::Disconnected(_)) => break,
                    }
                }
            })
            .expect("failed to spawn Deskmate provider worker");
        Self {
            sender: Some(sender),
            receiver,
            worker: Some(worker),
        }
    }

    fn try_submit(&self, request: CalendarRefreshRequest) -> Result<(), ProviderSubmitError> {
        self.sender
            .as_ref()
            .expect("provider sender is present while running")
            .try_send(request)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => ProviderSubmitError::Full,
                mpsc::TrySendError::Disconnected(_) => ProviderSubmitError::Disconnected,
            })
    }

    fn try_recv(&self) -> Result<CalendarRefreshResult, TryRecvError> {
        self.receiver.try_recv()
    }
}

impl Drop for ProviderWorker {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Default)]
struct RuntimeDiagnosticCounters {
    commands_processed: AtomicU64,
    command_queue_full: AtomicU64,
    provider_jobs_started: AtomicU64,
    provider_queue_full: AtomicU64,
    provider_results_discarded: AtomicU64,
    subscriber_snapshots_overwritten: AtomicU64,
}

impl RuntimeDiagnosticCounters {
    fn snapshot(&self) -> RuntimeDiagnostics {
        RuntimeDiagnostics {
            commands_processed: self.commands_processed.load(Ordering::Relaxed),
            command_queue_full: self.command_queue_full.load(Ordering::Relaxed),
            provider_jobs_started: self.provider_jobs_started.load(Ordering::Relaxed),
            provider_queue_full: self.provider_queue_full.load(Ordering::Relaxed),
            provider_results_discarded: self.provider_results_discarded.load(Ordering::Relaxed),
            subscriber_snapshots_overwritten: self
                .subscriber_snapshots_overwritten
                .load(Ordering::Relaxed),
        }
    }
}

struct SubscriptionSlot {
    state: Mutex<SubscriptionState>,
    changed: Condvar,
}

struct SubscriptionState {
    snapshot: Option<AppSnapshot>,
    closed: bool,
}

pub struct RuntimeSubscription {
    slot: Arc<SubscriptionSlot>,
}

impl RuntimeSubscription {
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<AppSnapshot>, RuntimeError> {
        let state = self
            .slot
            .state
            .lock()
            .map_err(|_| RuntimeError::WorkerStopped)?;
        let (mut state, _) = self
            .slot
            .changed
            .wait_timeout_while(state, timeout, |state| {
                state.snapshot.is_none() && !state.closed
            })
            .map_err(|_| RuntimeError::WorkerStopped)?;
        if let Some(snapshot) = state.snapshot.take() {
            Ok(Some(snapshot))
        } else if state.closed {
            Err(RuntimeError::WorkerStopped)
        } else {
            Ok(None)
        }
    }
}

struct SnapshotPublisher {
    latest: Arc<RwLock<AppSnapshot>>,
    subscribers: Mutex<Vec<Weak<SubscriptionSlot>>>,
    maximum_subscribers: usize,
    diagnostics: Arc<RuntimeDiagnosticCounters>,
}

impl SnapshotPublisher {
    fn subscribe(&self) -> Result<RuntimeSubscription, RuntimeError> {
        let snapshot = self
            .latest
            .read()
            .map_err(|_| RuntimeError::WorkerStopped)?
            .clone();
        let mut subscribers = self
            .subscribers
            .lock()
            .map_err(|_| RuntimeError::WorkerStopped)?;
        subscribers.retain(|subscriber| subscriber.strong_count() != 0);
        if subscribers.len() >= self.maximum_subscribers {
            return Err(RuntimeError::QueueFull);
        }
        let slot = Arc::new(SubscriptionSlot {
            state: Mutex::new(SubscriptionState {
                snapshot: Some(snapshot),
                closed: false,
            }),
            changed: Condvar::new(),
        });
        subscribers.push(Arc::downgrade(&slot));
        Ok(RuntimeSubscription { slot })
    }

    fn publish(&self, snapshot: &AppSnapshot) {
        if let Ok(mut latest) = self.latest.write() {
            *latest = snapshot.clone();
        }
        let Ok(mut subscribers) = self.subscribers.lock() else {
            return;
        };
        subscribers.retain(|subscriber| {
            let Some(slot) = subscriber.upgrade() else {
                return false;
            };
            if let Ok(mut state) = slot.state.lock() {
                if state.snapshot.is_some() {
                    self.diagnostics
                        .subscriber_snapshots_overwritten
                        .fetch_add(1, Ordering::Relaxed);
                }
                state.snapshot = Some(snapshot.clone());
                slot.changed.notify_one();
            }
            true
        });
    }

    fn close(&self) {
        let Ok(mut subscribers) = self.subscribers.lock() else {
            return;
        };
        subscribers.retain(|subscriber| {
            let Some(slot) = subscriber.upgrade() else {
                return false;
            };
            if let Ok(mut state) = slot.state.lock() {
                state.closed = true;
                slot.changed.notify_all();
            }
            true
        });
    }
}

pub struct RuntimeHandle {
    sender: SyncSender<RuntimeCommand>,
    worker: Mutex<Option<JoinHandle<()>>>,
    publisher: Arc<SnapshotPublisher>,
    diagnostics: Arc<RuntimeDiagnosticCounters>,
    command_timeout: Duration,
}

impl RuntimeHandle {
    pub fn start_serial(
        config: AppConfig,
        explicit_port: Option<String>,
    ) -> Result<Self, RuntimeError> {
        Self::start(
            config,
            Box::new(SerialRuntimeDevice::new(explicit_port)),
            Box::<SystemCalendarRefresher>::default(),
            RuntimeOptions::default(),
        )
    }

    pub fn start(
        config: AppConfig,
        device: Box<dyn RuntimeDevice>,
        refresher: Box<dyn CalendarRefresher>,
        options: RuntimeOptions,
    ) -> Result<Self, RuntimeError> {
        config
            .validate()
            .map_err(|error| RuntimeError::InvalidConfig {
                issues: error.issues,
            })?;
        let diagnostics = Arc::new(RuntimeDiagnosticCounters::default());
        let initial = initial_snapshot(&config, diagnostics.snapshot());
        let latest = Arc::new(RwLock::new(initial));
        let publisher = Arc::new(SnapshotPublisher {
            latest,
            subscribers: Mutex::new(Vec::new()),
            maximum_subscribers: options.maximum_subscribers.max(1),
            diagnostics: Arc::clone(&diagnostics),
        });
        let (sender, receiver) = mpsc::sync_channel(options.command_capacity.max(1));
        let worker_publisher = Arc::clone(&publisher);
        let worker_diagnostics = Arc::clone(&diagnostics);
        let worker = thread::Builder::new()
            .name("deskmate-runtime".into())
            .spawn(move || {
                run_runtime(
                    config,
                    device,
                    refresher,
                    &receiver,
                    &worker_publisher,
                    &worker_diagnostics,
                    options,
                );
            })
            .map_err(|error| RuntimeError::Device {
                message: format!("cannot start runtime worker: {error}"),
            })?;
        Ok(Self {
            sender,
            worker: Mutex::new(Some(worker)),
            publisher,
            diagnostics,
            command_timeout: options.command_timeout,
        })
    }

    pub fn snapshot(&self) -> Result<AppSnapshot, RuntimeError> {
        self.publisher
            .latest
            .read()
            .map(|snapshot| snapshot.clone())
            .map_err(|_| RuntimeError::WorkerStopped)
    }

    pub fn subscribe(&self) -> Result<RuntimeSubscription, RuntimeError> {
        self.publisher.subscribe()
    }

    pub fn apply_config(&self, config: AppConfig) -> Result<(), RuntimeError> {
        config
            .validate()
            .map_err(|error| RuntimeError::InvalidConfig {
                issues: error.issues,
            })?;
        self.request(|reply| RuntimeCommand::ApplyConfig { config, reply })
    }

    pub fn set_paused(&self, paused: bool) -> Result<(), RuntimeError> {
        self.request(|reply| RuntimeCommand::SetPaused { paused, reply })
    }

    pub fn control_pomodoro(
        &self,
        widget_id: impl Into<String>,
        action: PomodoroAction,
    ) -> Result<(), RuntimeError> {
        self.request(|reply| RuntimeCommand::Pomodoro {
            widget_id: widget_id.into(),
            action,
            reply,
        })
    }

    pub fn refresh_provider(&self, widget_id: impl Into<String>) -> Result<(), RuntimeError> {
        self.request(|reply| RuntimeCommand::RefreshProvider {
            widget_id: widget_id.into(),
            reply,
        })
    }

    pub fn activate_screen(&self, screen_id: impl Into<String>) -> Result<(), RuntimeError> {
        self.request(|reply| RuntimeCommand::ActivateScreen {
            screen_id: screen_id.into(),
            reply,
        })
    }

    pub fn shutdown(&self) -> Result<(), RuntimeError> {
        let has_worker = self
            .worker
            .lock()
            .map_err(|_| RuntimeError::WorkerStopped)?
            .is_some();
        if !has_worker {
            return Ok(());
        }
        let (reply_sender, reply_receiver) = mpsc::sync_channel(1);
        let result = if self
            .sender
            .send(RuntimeCommand::Shutdown {
                reply: reply_sender,
            })
            .is_err()
        {
            Err(RuntimeError::WorkerStopped)
        } else {
            match reply_receiver.recv_timeout(self.command_timeout) {
                Ok(result) => result,
                Err(RecvTimeoutError::Timeout) => Err(RuntimeError::ResponseTimeout),
                Err(RecvTimeoutError::Disconnected) => Err(RuntimeError::WorkerStopped),
            }
        };
        let worker = self
            .worker
            .lock()
            .map_err(|_| RuntimeError::WorkerStopped)?
            .take();
        if let Some(worker) = worker {
            worker.join().map_err(|_| RuntimeError::WorkerStopped)?;
        }
        result
    }

    fn request(
        &self,
        command: impl FnOnce(SyncSender<Result<(), RuntimeError>>) -> RuntimeCommand,
    ) -> Result<(), RuntimeError> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(1);
        match self.sender.try_send(command(reply_sender)) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                self.diagnostics
                    .command_queue_full
                    .fetch_add(1, Ordering::Relaxed);
                return Err(RuntimeError::QueueFull);
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                return Err(RuntimeError::WorkerStopped);
            }
        }
        match reply_receiver.recv_timeout(self.command_timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err(RuntimeError::ResponseTimeout),
            Err(RecvTimeoutError::Disconnected) => Err(RuntimeError::WorkerStopped),
        }
    }
}

impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct CalendarRuntimeState {
    generation: u64,
    in_flight: bool,
    snapshot: ProviderSnapshot,
}

#[allow(clippy::struct_excessive_bools)]
struct WorkerState {
    config: AppConfig,
    runtime: RuntimeState,
    device: DeviceSnapshot,
    persistence: PersistenceState,
    latest_fields: BTreeMap<String, Vec<Field>>,
    dirty_widgets: BTreeSet<String>,
    pomodoros: BTreeMap<String, Pomodoro>,
    pomodoro_snapshots: BTreeMap<String, PomodoroSnapshot>,
    calendars: BTreeMap<String, CalendarRuntimeState>,
    interrupts: InterruptArbiter,
    active_screen: Option<String>,
    active_screen_dirty: bool,
    connected: bool,
    ever_connected: bool,
    needs_full_sync: bool,
    generation: u64,
    next_connect: Instant,
    last_published: Option<AppSnapshot>,
}

impl WorkerState {
    fn new(config: AppConfig, now: Instant, scheduler: &mut Scheduler) -> Self {
        let mut state = Self {
            config: config.clone(),
            runtime: RuntimeState::Starting,
            device: empty_device(ConnectionState::Connecting),
            persistence: PersistenceState::Clean,
            latest_fields: BTreeMap::new(),
            dirty_widgets: BTreeSet::new(),
            pomodoros: BTreeMap::new(),
            pomodoro_snapshots: BTreeMap::new(),
            calendars: BTreeMap::new(),
            interrupts: InterruptArbiter::default(),
            active_screen: None,
            active_screen_dirty: false,
            connected: false,
            ever_connected: false,
            needs_full_sync: true,
            generation: 0,
            next_connect: now,
            last_published: None,
        };
        state.replace_config(config, now, scheduler);
        state.runtime = RuntimeState::Starting;
        state
    }

    fn replace_config(&mut self, config: AppConfig, now: Instant, scheduler: &mut Scheduler) {
        self.generation = self.generation.saturating_add(1);
        self.config = config;
        self.latest_fields.clear();
        self.dirty_widgets.clear();
        self.pomodoros.clear();
        self.pomodoro_snapshots.clear();
        self.calendars.clear();
        self.interrupts.clear_for_config_replacement();

        let compiled = self
            .config
            .compile(1)
            .expect("worker accepts only prevalidated configurations");
        for push in compiled.initial_pushes {
            self.latest_fields.insert(push.widget_id, push.fields);
        }

        let mut calendar_deadlines = Vec::new();
        for widget in &self.config.widgets {
            match widget {
                WidgetSettings::Pomodoro {
                    id,
                    label,
                    duration_seconds,
                    ..
                } => {
                    let timer = Pomodoro::new(label, *duration_seconds)
                        .expect("validated pomodoro duration");
                    self.pomodoro_snapshots.insert(
                        id.clone(),
                        PomodoroSnapshot {
                            widget_id: id.clone(),
                            state: PomodoroState::Idle,
                            duration_seconds: *duration_seconds,
                            remaining_seconds: *duration_seconds,
                        },
                    );
                    self.pomodoros.insert(id.clone(), timer);
                }
                WidgetSettings::Calendar {
                    id,
                    refresh_minutes,
                    ..
                } => {
                    let interval = Duration::from_secs(u64::from(*refresh_minutes) * 60);
                    calendar_deadlines.push((id.clone(), interval));
                    self.calendars.insert(
                        id.clone(),
                        CalendarRuntimeState {
                            generation: self.generation,
                            in_flight: false,
                            snapshot: ProviderSnapshot {
                                widget_id: id.clone(),
                                state: ProviderState::Idle,
                                last_success_unix_ms: None,
                                age_seconds: None,
                            },
                        },
                    );
                }
                WidgetSettings::Clock { .. } => {}
            }
        }
        scheduler.replace_calendars(calendar_deadlines, now);
        self.active_screen = self.config.screens.first().map(|screen| screen.id.clone());
        self.device.active_screen_id.clone_from(&self.active_screen);
        self.active_screen_dirty = self.active_screen.is_some();
        self.needs_full_sync = true;
        self.runtime = if self.config.preferences.paused {
            RuntimeState::Paused
        } else {
            RuntimeState::Running
        };
    }

    fn snapshot(&self, diagnostics: &RuntimeDiagnosticCounters) -> AppSnapshot {
        AppSnapshot {
            config: self.config.clone(),
            runtime: self.runtime.clone(),
            device: self.device.clone(),
            providers: self
                .calendars
                .values()
                .map(|calendar| calendar.snapshot.clone())
                .collect(),
            pomodoros: self.pomodoro_snapshots.values().cloned().collect(),
            persistence: self.persistence.clone(),
            diagnostics: diagnostics.snapshot(),
        }
    }

    fn publish_if_changed(
        &mut self,
        publisher: &SnapshotPublisher,
        diagnostics: &RuntimeDiagnosticCounters,
    ) {
        let snapshot = self.snapshot(diagnostics);
        if self.last_published.as_ref() != Some(&snapshot) {
            publisher.publish(&snapshot);
            self.last_published = Some(snapshot);
        }
    }
}

fn run_runtime(
    config: AppConfig,
    mut device: Box<dyn RuntimeDevice>,
    refresher: Box<dyn CalendarRefresher>,
    command_receiver: &Receiver<RuntimeCommand>,
    publisher: &SnapshotPublisher,
    diagnostics: &RuntimeDiagnosticCounters,
    options: RuntimeOptions,
) {
    let now = Instant::now();
    let mut scheduler = Scheduler::new(
        now,
        options.pomodoro_interval,
        options.status_interval,
        options.time_sync_interval,
    );
    let mut state = WorkerState::new(config, now, &mut scheduler);
    let provider = ProviderWorker::new(
        refresher,
        options.provider_job_capacity,
        Arc::clone(&publisher.diagnostics),
    );
    state.publish_if_changed(publisher, diagnostics);

    let mut shutting_down = false;
    while !shutting_down {
        let wait = scheduler.wait_duration(Instant::now(), options.loop_maximum_wait);
        match command_receiver.recv_timeout(wait) {
            Ok(command) => {
                shutting_down = process_command(
                    command,
                    &mut state,
                    &mut scheduler,
                    device.as_mut(),
                    &provider,
                    diagnostics,
                    &options,
                );
                while !shutting_down {
                    let Ok(command) = command_receiver.try_recv() else {
                        break;
                    };
                    shutting_down = process_command(
                        command,
                        &mut state,
                        &mut scheduler,
                        device.as_mut(),
                        &provider,
                        diagnostics,
                        &options,
                    );
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => shutting_down = true,
        }
        if shutting_down {
            break;
        }

        drain_provider_results(&mut state, &provider, diagnostics);
        let now = Instant::now();
        if !state.connected && now >= state.next_connect {
            attempt_connect(&mut state, device.as_mut(), now, &options);
        }
        drain_device_events(&mut state, device.as_mut(), now);
        run_scheduled_work(
            &mut state,
            &mut scheduler,
            device.as_mut(),
            &provider,
            diagnostics,
            now,
            &options,
        );
        state.publish_if_changed(publisher, diagnostics);
    }

    drop(provider);
    publisher.close();
}

#[allow(clippy::too_many_arguments)]
fn process_command(
    command: RuntimeCommand,
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    device: &mut dyn RuntimeDevice,
    _provider: &ProviderWorker,
    diagnostics: &RuntimeDiagnosticCounters,
    _options: &RuntimeOptions,
) -> bool {
    diagnostics
        .commands_processed
        .fetch_add(1, Ordering::Relaxed);
    match command {
        RuntimeCommand::ApplyConfig { config, reply } => {
            let result = config
                .validate()
                .map_err(|error| RuntimeError::InvalidConfig {
                    issues: error.issues,
                });
            let result = result.and_then(|()| {
                state.replace_config(config, Instant::now(), scheduler);
                if state.connected && !state.config.preferences.paused {
                    synchronize_full(state, device)
                } else {
                    Ok(())
                }
            });
            let _ = reply.send(result);
        }
        RuntimeCommand::SetPaused { paused, reply } => {
            state.config.preferences.paused = paused;
            state.runtime = if paused {
                RuntimeState::Paused
            } else {
                RuntimeState::Running
            };
            let result = if !paused && state.connected {
                synchronize_pending(state, device)
            } else {
                Ok(())
            };
            let _ = reply.send(result);
        }
        RuntimeCommand::Pomodoro {
            widget_id,
            action,
            reply,
        } => {
            let result = control_pomodoro(state, device, &widget_id, action, Instant::now());
            let _ = reply.send(result);
        }
        RuntimeCommand::RefreshProvider { widget_id, reply } => {
            let result = if state.calendars.contains_key(&widget_id) {
                scheduler.schedule_calendar_now(&widget_id, Instant::now());
                Ok(())
            } else {
                Err(RuntimeError::UnknownWidget { widget_id })
            };
            let _ = reply.send(result);
        }
        RuntimeCommand::ActivateScreen { screen_id, reply } => {
            let result = if state
                .config
                .screens
                .iter()
                .any(|screen| screen.id == screen_id)
            {
                state.active_screen = Some(screen_id.clone());
                state.device.active_screen_id = Some(screen_id.clone());
                state.active_screen_dirty = true;
                if state.connected && !state.config.preferences.paused {
                    send_screen(state, device)
                } else {
                    Ok(())
                }
            } else {
                Err(RuntimeError::UnknownScreen { screen_id })
            };
            let _ = reply.send(result);
        }
        RuntimeCommand::Shutdown { reply } => {
            let _ = reply.send(Ok(()));
            return true;
        }
    }
    false
}

fn attempt_connect(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
    now: Instant,
    options: &RuntimeOptions,
) {
    state.device.connection = ConnectionState::Connecting;
    match device.connect() {
        Ok(connection) => {
            state.connected = true;
            state.ever_connected = true;
            update_device_status(state, &connection.port_name, &connection.status, device);
            state.runtime = if state.config.preferences.paused {
                RuntimeState::Paused
            } else {
                RuntimeState::Running
            };
            if !state.config.preferences.paused {
                let result = if state.needs_full_sync {
                    synchronize_full(state, device)
                } else {
                    synchronize_pending(state, device)
                };
                if let Err(error) = result {
                    state.runtime = RuntimeState::Error {
                        message: error.to_string(),
                    };
                }
            }
        }
        Err(error) => {
            mark_disconnected(state, &error, now, options.reconnect_interval);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_scheduled_work(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    device: &mut dyn RuntimeDevice,
    provider: &ProviderWorker,
    diagnostics: &RuntimeDiagnosticCounters,
    now: Instant,
    options: &RuntimeOptions,
) {
    if scheduler.pomodoro_due(now) {
        update_pomodoros(state, now);
    }
    if !state.config.preferences.paused {
        submit_due_providers(
            state,
            scheduler,
            provider,
            diagnostics,
            now,
            options.provider_queue_retry,
        );
    }
    if !state.connected {
        return;
    }
    if scheduler.status_due(now) {
        match device.status() {
            Ok(status) => {
                let port = state.device.port_name.clone().unwrap_or_default();
                update_device_status(state, &port, &status, device);
            }
            Err(error) => {
                handle_device_error(state, &error, now, options.reconnect_interval);
                return;
            }
        }
    }
    if state.config.preferences.paused {
        return;
    }
    if scheduler.time_sync_due(now)
        && let Err(error) = send_time_sync(state, device)
    {
        state.runtime = RuntimeState::Error {
            message: error.to_string(),
        };
    }
    if let Err(error) = synchronize_pending(state, device) {
        state.runtime = RuntimeState::Error {
            message: error.to_string(),
        };
    }
}

fn submit_due_providers(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    provider: &ProviderWorker,
    diagnostics: &RuntimeDiagnosticCounters,
    now: Instant,
    retry: Duration,
) {
    for widget_id in scheduler.due_calendars(now) {
        if state
            .calendars
            .get(&widget_id)
            .is_none_or(|calendar| calendar.in_flight)
        {
            continue;
        }
        let Some(request) = calendar_request(state, &widget_id) else {
            continue;
        };
        match provider.try_submit(request) {
            Ok(()) => {
                if let Some(calendar) = state.calendars.get_mut(&widget_id) {
                    calendar.in_flight = true;
                    calendar.snapshot.state = ProviderState::Refreshing;
                }
                diagnostics
                    .provider_jobs_started
                    .fetch_add(1, Ordering::Relaxed);
                scheduler.calendar_started(&widget_id, now);
            }
            Err(ProviderSubmitError::Full) => {
                diagnostics
                    .provider_queue_full
                    .fetch_add(1, Ordering::Relaxed);
                scheduler.calendar_retry(&widget_id, now, retry);
            }
            Err(ProviderSubmitError::Disconnected) => {
                if let Some(calendar) = state.calendars.get_mut(&widget_id) {
                    calendar.snapshot.state = ProviderState::Error {
                        message: "provider worker stopped".into(),
                    };
                }
            }
        }
    }
}

fn calendar_request(state: &WorkerState, widget_id: &str) -> Option<CalendarRefreshRequest> {
    state.config.widgets.iter().find_map(|widget| match widget {
        WidgetSettings::Calendar {
            id,
            title,
            source,
            refresh_minutes,
            ..
        } if id == widget_id => Some(CalendarRefreshRequest {
            generation: state.generation,
            widget_id: id.clone(),
            source: source.clone(),
            timezone: state.config.preferences.timezone.parse().ok()?,
            title: title.clone(),
            refresh_interval: Duration::from_secs(u64::from(*refresh_minutes) * 60),
            now: Utc::now(),
        }),
        _ => None,
    })
}

fn drain_provider_results(
    state: &mut WorkerState,
    provider: &ProviderWorker,
    diagnostics: &RuntimeDiagnosticCounters,
) {
    while let Ok(result) = provider.try_recv() {
        let Some(calendar) = state.calendars.get_mut(&result.widget_id) else {
            diagnostics
                .provider_results_discarded
                .fetch_add(1, Ordering::Relaxed);
            continue;
        };
        if result.generation != calendar.generation || result.generation != state.generation {
            diagnostics
                .provider_results_discarded
                .fetch_add(1, Ordering::Relaxed);
            continue;
        }
        calendar.in_flight = false;
        calendar.snapshot = ProviderSnapshot {
            widget_id: result.widget_id.clone(),
            state: match (result.stale, &result.error, result.refreshed_at) {
                (false, None, _) => ProviderState::Fresh,
                (true, Some(message), Some(_)) => ProviderState::Stale {
                    message: message.clone(),
                },
                (true, None, _) => ProviderState::Stale {
                    message: "provider data is stale".into(),
                },
                (_, Some(message), None) | (false, Some(message), Some(_)) => {
                    ProviderState::Error {
                        message: message.clone(),
                    }
                }
            },
            last_success_unix_ms: result.refreshed_at.map(|time| time.timestamp_millis()),
            age_seconds: result.age.map(|age| age.as_secs()),
        };
        state
            .latest_fields
            .insert(result.widget_id.clone(), result.fields);
        state.dirty_widgets.insert(result.widget_id);
    }
}

fn update_pomodoros(state: &mut WorkerState, now: Instant) {
    let ids: Vec<String> = state.pomodoros.keys().cloned().collect();
    for widget_id in ids {
        let Some(timer) = state.pomodoros.get_mut(&widget_id) else {
            continue;
        };
        let update = timer.update(now);
        let changed = state.latest_fields.get(&widget_id) != Some(&update.fields);
        if changed {
            state.latest_fields.insert(widget_id.clone(), update.fields);
            state.dirty_widgets.insert(widget_id.clone());
        }
        state.pomodoro_snapshots.insert(
            widget_id.clone(),
            PomodoroSnapshot {
                widget_id: widget_id.clone(),
                state: pomodoro_state(update.state),
                duration_seconds: update.duration_seconds,
                remaining_seconds: update.remaining_seconds,
            },
        );
        if update.completion_interrupt {
            let _ = state.interrupts.schedule(widget_id, "Timer finished");
        }
    }
}

fn control_pomodoro(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
    widget_id: &str,
    action: PomodoroAction,
    now: Instant,
) -> Result<(), RuntimeError> {
    let Some(timer) = state.pomodoros.get_mut(widget_id) else {
        return Err(RuntimeError::UnknownWidget {
            widget_id: widget_id.into(),
        });
    };
    let update = match action {
        PomodoroAction::Start => timer.start(now),
        PomodoroAction::Pause => timer.pause(now),
        PomodoroAction::Toggle => timer.toggle(now),
        PomodoroAction::Reset => timer.reset(now),
    };
    state.latest_fields.insert(widget_id.into(), update.fields);
    state.dirty_widgets.insert(widget_id.into());
    state.pomodoro_snapshots.insert(
        widget_id.into(),
        PomodoroSnapshot {
            widget_id: widget_id.into(),
            state: pomodoro_state(update.state),
            duration_seconds: update.duration_seconds,
            remaining_seconds: update.remaining_seconds,
        },
    );
    if update.completion_interrupt {
        state
            .interrupts
            .schedule(widget_id, "Timer finished")
            .map_err(|error| RuntimeError::Device {
                message: error.to_string(),
            })?;
    }
    if state.connected && !state.config.preferences.paused {
        push_dirty_widgets(state, device)?;
        flush_interrupts(state, device)?;
    }
    Ok(())
}

fn drain_device_events(state: &mut WorkerState, device: &mut dyn RuntimeDevice, now: Instant) {
    while let Some(received) = device.try_recv_event() {
        match (received.event.kind, received.event.action) {
            (EventKind::Tap, EventAction::StartPause) => {
                let _ = control_pomodoro(
                    state,
                    device,
                    &received.event.widget_id,
                    PomodoroAction::Toggle,
                    now,
                );
            }
            (EventKind::Tap, EventAction::Reset) => {
                let _ = control_pomodoro(
                    state,
                    device,
                    &received.event.widget_id,
                    PomodoroAction::Reset,
                    now,
                );
            }
            (EventKind::InterruptDismissed, EventAction::DismissInterrupt) => {
                if let Some(token) = received.event.interrupt_token {
                    let _ = state.interrupts.dismiss(token);
                }
            }
            _ => {}
        }
    }
}

fn synchronize_full(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
) -> Result<(), RuntimeError> {
    state.needs_full_sync = true;
    send_time_sync(state, device)?;
    let compiled = state
        .config
        .compile(1)
        .expect("runtime config remains validated");
    device
        .apply_layout(compiled.layout.widgets, compiled.layout.screens)
        .map_err(|error| device_runtime_error(&error))?;
    state.dirty_widgets = state.latest_fields.keys().cloned().collect();
    push_dirty_widgets(state, device)?;
    state.active_screen_dirty = state.active_screen.is_some();
    send_screen(state, device)?;
    flush_interrupts(state, device)?;
    state.needs_full_sync = false;
    Ok(())
}

fn synchronize_pending(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
) -> Result<(), RuntimeError> {
    if state.needs_full_sync {
        return synchronize_full(state, device);
    }
    push_dirty_widgets(state, device)?;
    send_screen(state, device)?;
    flush_interrupts(state, device)
}

fn push_dirty_widgets(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
) -> Result<(), RuntimeError> {
    let dirty: Vec<String> = state.dirty_widgets.iter().cloned().collect();
    for widget_id in dirty {
        let Some(fields) = state.latest_fields.get(&widget_id).cloned() else {
            state.dirty_widgets.remove(&widget_id);
            continue;
        };
        device
            .push_fields(widget_id.clone(), fields)
            .map_err(|error| device_runtime_error(&error))?;
        state.dirty_widgets.remove(&widget_id);
    }
    Ok(())
}

fn send_screen(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
) -> Result<(), RuntimeError> {
    if !state.active_screen_dirty {
        return Ok(());
    }
    if let Some(screen_id) = state.active_screen.clone() {
        device
            .activate_screen(screen_id)
            .map_err(|error| device_runtime_error(&error))?;
    }
    state.active_screen_dirty = false;
    Ok(())
}

fn flush_interrupts(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
) -> Result<(), RuntimeError> {
    loop {
        let pending = state
            .interrupts
            .active()
            .filter(|tracked| !tracked.acknowledged)
            .or_else(|| {
                state
                    .interrupts
                    .pending()
                    .filter(|tracked| !tracked.acknowledged)
            })
            .map(|tracked| tracked.message.clone());
        let Some(interrupt) = pending else {
            return Ok(());
        };
        match device.trigger_interrupt(interrupt.clone()) {
            Ok(()) => state
                .interrupts
                .acknowledge(interrupt.token)
                .map_err(|error| RuntimeError::Device {
                    message: error.to_string(),
                })?,
            Err(DeviceError::Rejected(error)) if error.code == protocol::ErrorCode::Busy => {
                state
                    .interrupts
                    .mark_busy_for_retry(interrupt.token)
                    .map_err(|error| RuntimeError::Device {
                        message: error.to_string(),
                    })?;
                return Ok(());
            }
            Err(error) => return Err(device_runtime_error(&error)),
        }
    }
}

fn send_time_sync(state: &WorkerState, device: &mut dyn RuntimeDevice) -> Result<(), RuntimeError> {
    let timezone: Tz = state
        .config
        .preferences
        .timezone
        .parse()
        .expect("runtime config timezone remains validated");
    let now = Utc::now();
    let offset_seconds = now
        .with_timezone(&timezone)
        .offset()
        .fix()
        .local_minus_utc();
    let offset_minutes = i16::try_from(offset_seconds / 60).map_err(|_| RuntimeError::Device {
        message: "timezone offset exceeds protocol bounds".into(),
    })?;
    device
        .time_sync(TimeSync {
            unix_seconds: now.timestamp(),
            utc_offset_minutes: offset_minutes,
        })
        .map_err(|error| device_runtime_error(&error))
}

fn handle_device_error(
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

fn mark_disconnected(
    state: &mut WorkerState,
    error: &DeviceError,
    now: Instant,
    reconnect_interval: Duration,
) {
    state.connected = false;
    state.next_connect = now + reconnect_interval;
    state.device.connection = if state.ever_connected {
        ConnectionState::Standalone
    } else {
        ConnectionState::Disconnected {
            reason: Some(error.to_string()),
        }
    };
}

fn is_disconnect(error: &DeviceError) -> bool {
    matches!(error, DeviceError::NoDevice | DeviceError::Transport(_))
}

fn device_runtime_error(error: &DeviceError) -> RuntimeError {
    RuntimeError::Device {
        message: error.to_string(),
    }
}

fn update_device_status(
    state: &mut WorkerState,
    port_name: &str,
    status: &StatusResponse,
    device: &dyn RuntimeDevice,
) {
    let diagnostics = device.diagnostics();
    state.device.connection = ConnectionState::Online;
    state.device.port_name = Some(port_name.into());
    state.device.firmware_version = Some(status.firmware_version.clone());
    state.device.protocol_version = Some(status.protocol_version);
    state.device.uptime_ms = Some(status.uptime_ms);
    state.device.free_heap = Some(status.free_heap);
    state.device.rotation = Some(status.rotation);
    state.device.counters = DeviceCounters {
        reconnects: diagnostics.reconnects,
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
}

fn pomodoro_state(state: EnginePomodoroState) -> PomodoroState {
    match state {
        EnginePomodoroState::Idle => PomodoroState::Idle,
        EnginePomodoroState::Running => PomodoroState::Running,
        EnginePomodoroState::Paused => PomodoroState::Paused,
        EnginePomodoroState::Completed => PomodoroState::Completed,
    }
}

fn initial_snapshot(config: &AppConfig, diagnostics: RuntimeDiagnostics) -> AppSnapshot {
    let mut providers = Vec::new();
    let mut pomodoros = Vec::new();
    for widget in &config.widgets {
        match widget {
            WidgetSettings::Calendar { id, .. } => providers.push(ProviderSnapshot {
                widget_id: id.clone(),
                state: ProviderState::Idle,
                last_success_unix_ms: None,
                age_seconds: None,
            }),
            WidgetSettings::Pomodoro {
                id,
                duration_seconds,
                ..
            } => pomodoros.push(PomodoroSnapshot {
                widget_id: id.clone(),
                state: PomodoroState::Idle,
                duration_seconds: *duration_seconds,
                remaining_seconds: *duration_seconds,
            }),
            WidgetSettings::Clock { .. } => {}
        }
    }
    AppSnapshot {
        config: config.clone(),
        runtime: RuntimeState::Starting,
        device: DeviceSnapshot {
            active_screen_id: config.screens.first().map(|screen| screen.id.clone()),
            ..empty_device(ConnectionState::Connecting)
        },
        providers,
        pomodoros,
        persistence: PersistenceState::Clean,
        diagnostics,
    }
}

fn empty_device(connection: ConnectionState) -> DeviceSnapshot {
    DeviceSnapshot {
        connection,
        port_name: None,
        firmware_version: None,
        protocol_version: None,
        uptime_ms: None,
        free_heap: None,
        rotation: None,
        active_screen_id: None,
        counters: DeviceCounters::default(),
    }
}
