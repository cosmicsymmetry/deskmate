use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
use providers::http::SystemHttpClient;
use providers::ics::{CalendarOptions, IcsProvider, IcsSource};
use providers::json_feed::{JsonFeedOptions, JsonFeedProvider, JsonMapping as ProviderJsonMapping};
use providers::rss::{RssOptions, RssProvider};
use providers::weather::{WeatherOptions, WeatherProvider, WeatherUnits as ProviderWeatherUnits};

use crate::commands::{PomodoroAction, RuntimeCommand, RuntimeError};
use crate::scheduler::Scheduler;
use crate::{
    AppConfig, AppSnapshot, CalendarSource, CardSettings, ConnectionState, DeviceCounters,
    DeviceSnapshot, JsonFieldMapping, PersistenceState, PomodoroSnapshot, PomodoroState,
    ProviderSnapshot, ProviderState, RuntimeDiagnostics, RuntimeState, WeatherUnits,
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
        rotation: u16,
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
pub struct ProviderRefreshRequest {
    pub generation: u64,
    pub widget_id: String,
    pub title: String,
    pub provider: ProviderRequest,
    pub active_provider_ids: Vec<String>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderRequest {
    Calendar {
        source: CalendarSource,
        timezone: Tz,
        refresh_interval: Duration,
    },
    Weather {
        location: String,
        units: WeatherUnits,
        refresh_interval: Duration,
    },
    JsonFeed {
        url: String,
        mappings: Vec<JsonFieldMapping>,
        refresh_interval: Duration,
    },
    Rss {
        url: String,
        maximum_items: u8,
        refresh_interval: Duration,
    },
}

#[derive(Debug, Clone)]
pub struct ProviderRefreshResult {
    pub generation: u64,
    pub widget_id: String,
    pub fields: Vec<Field>,
    pub refreshed_at: Option<DateTime<Utc>>,
    pub age: Option<Duration>,
    pub stale: bool,
    pub error: Option<String>,
}

pub trait ProviderRefresher: Send + 'static {
    fn refresh(&mut self, request: ProviderRefreshRequest) -> ProviderRefreshResult;
}

#[derive(Default)]
pub struct SystemProviderRefresher {
    providers: HashMap<String, SystemProviderEntry>,
}

struct SystemProviderEntry {
    title: String,
    request: ProviderRequest,
    provider: SystemProvider,
}

enum SystemProvider {
    Calendar(IcsProvider<providers::ics::SystemIcsLoader>),
    Weather(WeatherProvider<SystemHttpClient>),
    JsonFeed(JsonFeedProvider<SystemHttpClient>),
    Rss(RssProvider<SystemHttpClient>),
}

impl ProviderRefresher for SystemProviderRefresher {
    fn refresh(&mut self, request: ProviderRefreshRequest) -> ProviderRefreshResult {
        self.providers
            .retain(|widget_id, _| request.active_provider_ids.contains(widget_id));
        let needs_replacement = self
            .providers
            .get(&request.widget_id)
            .is_none_or(|entry| entry.title != request.title || entry.request != request.provider);
        if needs_replacement {
            self.providers.insert(
                request.widget_id.clone(),
                SystemProviderEntry {
                    title: request.title.clone(),
                    request: request.provider.clone(),
                    provider: system_provider(&request),
                },
            );
        }
        let entry = self
            .providers
            .get_mut(&request.widget_id)
            .expect("provider entry was inserted above");
        let (fields, refreshed_at, age, stale, error) = match &mut entry.provider {
            SystemProvider::Calendar(provider) => {
                let snapshot = provider.refresh(request.now);
                (
                    provider.fields(&snapshot, request.now),
                    snapshot.refreshed_at,
                    snapshot.age,
                    snapshot.stale,
                    snapshot.error,
                )
            }
            SystemProvider::Weather(provider) => {
                let snapshot = provider.refresh(request.now);
                (
                    provider.fields(&snapshot),
                    snapshot.refreshed_at,
                    snapshot.age,
                    snapshot.stale,
                    snapshot.error,
                )
            }
            SystemProvider::JsonFeed(provider) => {
                let snapshot = provider.refresh(request.now);
                (
                    provider.fields(&snapshot),
                    snapshot.refreshed_at,
                    snapshot.age,
                    snapshot.stale,
                    snapshot.error,
                )
            }
            SystemProvider::Rss(provider) => {
                let snapshot = provider.refresh(request.now);
                (
                    provider.fields(&snapshot),
                    snapshot.refreshed_at,
                    snapshot.age,
                    snapshot.stale,
                    snapshot.error,
                )
            }
        };
        ProviderRefreshResult {
            generation: request.generation,
            widget_id: request.widget_id,
            fields,
            refreshed_at,
            age,
            stale,
            error,
        }
    }
}

fn system_provider(request: &ProviderRefreshRequest) -> SystemProvider {
    match &request.provider {
        ProviderRequest::Calendar {
            source,
            timezone,
            refresh_interval,
        } => {
            let source = match source {
                CalendarSource::File(path) => IcsSource::File(PathBuf::from(path)),
                CalendarSource::Url(url) => IcsSource::Url(url.clone()),
            };
            SystemProvider::Calendar(IcsProvider::system(
                source,
                CalendarOptions {
                    default_timezone: *timezone,
                    display_timezone: *timezone,
                    refresh_interval: *refresh_interval,
                    title: request.title.clone(),
                    ..CalendarOptions::default()
                },
            ))
        }
        ProviderRequest::Weather {
            location,
            units,
            refresh_interval,
        } => SystemProvider::Weather(WeatherProvider::system(WeatherOptions {
            location: location.clone(),
            units: match units {
                WeatherUnits::Metric => ProviderWeatherUnits::Metric,
                WeatherUnits::Imperial => ProviderWeatherUnits::Imperial,
            },
            refresh_interval: *refresh_interval,
            title: request.title.clone(),
        })),
        ProviderRequest::JsonFeed {
            url,
            mappings,
            refresh_interval,
        } => SystemProvider::JsonFeed(JsonFeedProvider::system(JsonFeedOptions {
            url: url.clone(),
            mappings: mappings
                .iter()
                .map(|mapping| ProviderJsonMapping {
                    field: mapping.field.clone(),
                    path: mapping.path.clone(),
                })
                .collect(),
            refresh_interval: *refresh_interval,
            title: request.title.clone(),
        })),
        ProviderRequest::Rss {
            url,
            maximum_items,
            refresh_interval,
        } => SystemProvider::Rss(RssProvider::system(RssOptions {
            url: url.clone(),
            maximum_items: usize::from(*maximum_items),
            refresh_interval: *refresh_interval,
            title: request.title.clone(),
        })),
    }
}

pub type CalendarRefreshRequest = ProviderRefreshRequest;
pub type CalendarRefreshResult = ProviderRefreshResult;
pub use ProviderRefresher as CalendarRefresher;
pub type SystemCalendarRefresher = SystemProviderRefresher;

struct ProviderWorker {
    sender: Option<SyncSender<CalendarRefreshRequest>>,
    receiver: Receiver<CalendarRefreshResult>,
    stopping: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

#[derive(Debug)]
enum ProviderSubmitError {
    Full,
    Disconnected,
}

impl ProviderWorker {
    fn new(mut refresher: Box<dyn CalendarRefresher>, capacity: usize) -> Self {
        let (sender, request_receiver) = mpsc::sync_channel(capacity.max(1));
        // At most one result can exist per in-flight provider, so this channel is
        // already logically bounded by the configured provider count. Dropping a
        // completed result would otherwise leave that provider marked in-flight.
        let (result_sender, receiver) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = Arc::clone(&stopping);
        let worker = thread::Builder::new()
            .name("deskmate-provider".into())
            .spawn(move || {
                while let Ok(request) = request_receiver.recv() {
                    if worker_stopping.load(Ordering::Acquire) {
                        break;
                    }
                    let result = refresher.refresh(request);
                    if result_sender.send(result).is_err() {
                        break;
                    }
                }
            })
            .expect("failed to spawn Deskmate provider worker");
        Self {
            sender: Some(sender),
            receiver,
            stopping,
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
        self.stopping.store(true, Ordering::Release);
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
            .compile(1)
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
            .compile(1)
            .map_err(|error| RuntimeError::InvalidConfig {
                issues: error.issues,
            })?;
        self.request(|reply| RuntimeCommand::ApplyConfig { config, reply })
    }

    pub fn set_paused(&self, paused: bool) -> Result<(), RuntimeError> {
        self.request(|reply| RuntimeCommand::SetPaused { paused, reply })
    }

    pub fn set_autostart_preference(&self, enabled: bool) -> Result<(), RuntimeError> {
        self.request(|reply| RuntimeCommand::SetAutostartPreference { enabled, reply })
    }

    pub fn set_persistence_state(&self, persistence: PersistenceState) -> Result<(), RuntimeError> {
        self.request(|reply| RuntimeCommand::SetPersistenceState { persistence, reply })
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

struct ProviderRuntimeState {
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
    providers: BTreeMap<String, ProviderRuntimeState>,
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
            providers: BTreeMap::new(),
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
        let previous_config = self.config.clone();
        let previous_active_screen = self.active_screen.clone();
        let mut previous_fields = std::mem::take(&mut self.latest_fields);
        let mut previous_pomodoros = std::mem::take(&mut self.pomodoros);
        let mut previous_providers = std::mem::take(&mut self.providers);
        self.generation = self.generation.saturating_add(1);
        self.config = config;
        self.dirty_widgets.clear();
        self.pomodoro_snapshots.clear();
        let live_widget_ids: BTreeSet<&str> = self
            .config
            .cards
            .iter()
            .filter(|card| !card.presence().is_off())
            .map(CardSettings::id)
            .collect();
        self.interrupts
            .retain_widgets(|widget_id| live_widget_ids.contains(widget_id));

        let compiled = self
            .config
            .compile(1)
            .expect("worker accepts only prevalidated configurations");
        for push in compiled.initial_pushes {
            self.latest_fields.insert(push.widget_id, push.fields);
        }

        let mut provider_deadlines = Vec::new();
        let cards: Vec<CardSettings> = self
            .config
            .cards
            .iter()
            .filter(|card| !card.presence().is_off())
            .cloned()
            .collect();
        for card in &cards {
            match card {
                CardSettings::Pomodoro {
                    id,
                    label,
                    duration_seconds,
                    ..
                } => {
                    self.restore_pomodoro(
                        id,
                        label,
                        *duration_seconds,
                        &mut previous_pomodoros,
                        now,
                    );
                }
                CardSettings::Calendar { id, refresh, .. }
                | CardSettings::Weather { id, refresh, .. }
                | CardSettings::JsonFeed { id, refresh, .. }
                | CardSettings::Rss { id, refresh, .. } => {
                    let interval = refresh
                        .interval_minutes()
                        .map(|minutes| Duration::from_secs(u64::from(minutes) * 60));
                    provider_deadlines.push((id.clone(), interval));
                    let unchanged = previous_config
                        .cards
                        .iter()
                        .any(|previous| previous == card);
                    self.restore_provider(
                        id,
                        unchanged,
                        &mut previous_fields,
                        &mut previous_providers,
                    );
                }
                CardSettings::Clock { .. } => {}
            }
        }
        scheduler.replace_providers(provider_deadlines, now);
        self.active_screen = previous_active_screen
            .filter(|active| {
                self.config
                    .cards
                    .iter()
                    .any(|card| card.presence().is_in_rotation() && card.id() == active)
            })
            .or_else(|| {
                self.config
                    .cards
                    .iter()
                    .find(|card| card.presence().is_in_rotation())
                    .map(|card| card.id().to_owned())
            });
        self.device.active_screen_id.clone_from(&self.active_screen);
        self.active_screen_dirty = self.active_screen.is_some();
        self.needs_full_sync = true;
        self.runtime = if self.config.preferences.paused {
            RuntimeState::Paused
        } else {
            RuntimeState::Running
        };
    }

    fn restore_pomodoro(
        &mut self,
        id: &str,
        label: &str,
        duration_seconds: u32,
        previous: &mut BTreeMap<String, Pomodoro>,
        now: Instant,
    ) {
        let mut timer = previous
            .remove(id)
            .filter(|timer| timer.matches_settings(label, duration_seconds))
            .unwrap_or_else(|| {
                Pomodoro::new(label, duration_seconds).expect("validated pomodoro duration")
            });
        let update = timer.update(now);
        self.latest_fields.insert(id.into(), update.fields);
        self.pomodoro_snapshots.insert(
            id.into(),
            PomodoroSnapshot {
                widget_id: id.into(),
                state: pomodoro_state(update.state),
                duration_seconds: update.duration_seconds,
                remaining_seconds: update.remaining_seconds,
            },
        );
        if update.completion_interrupt && card_wants_completion_interrupt(&self.config, id) {
            let _ = self.interrupts.schedule(id, "Timer finished");
        }
        self.pomodoros.insert(id.into(), timer);
    }

    fn restore_provider(
        &mut self,
        id: &str,
        unchanged: bool,
        previous_fields: &mut BTreeMap<String, Vec<Field>>,
        previous_providers: &mut BTreeMap<String, ProviderRuntimeState>,
    ) {
        let provider = if unchanged {
            if let Some(fields) = previous_fields.remove(id) {
                self.latest_fields.insert(id.into(), fields);
            }
            previous_providers.remove(id).map_or_else(
                || self.empty_provider(id),
                |mut provider| {
                    provider.generation = self.generation;
                    provider.in_flight = false;
                    provider
                },
            )
        } else {
            self.empty_provider(id)
        };
        self.providers.insert(id.into(), provider);
    }

    fn empty_provider(&self, id: &str) -> ProviderRuntimeState {
        ProviderRuntimeState {
            generation: self.generation,
            in_flight: false,
            snapshot: ProviderSnapshot {
                widget_id: id.into(),
                state: ProviderState::Idle,
                last_success_unix_ms: None,
                age_seconds: None,
            },
        }
    }

    fn snapshot(&self, diagnostics: &RuntimeDiagnosticCounters) -> AppSnapshot {
        AppSnapshot {
            config: self.config.clone(),
            runtime: self.runtime.clone(),
            device: self.device.clone(),
            providers: self
                .providers
                .values()
                .map(|provider| provider.snapshot.clone())
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
    let provider = ProviderWorker::new(refresher, options.provider_job_capacity);
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
                    diagnostics,
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
                        diagnostics,
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

fn process_command(
    command: RuntimeCommand,
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    device: &mut dyn RuntimeDevice,
    diagnostics: &RuntimeDiagnosticCounters,
) -> bool {
    diagnostics
        .commands_processed
        .fetch_add(1, Ordering::Relaxed);
    match command {
        RuntimeCommand::ApplyConfig { config, reply } => {
            let result =
                config
                    .compile(1)
                    .map(|_| ())
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
        RuntimeCommand::SetAutostartPreference { enabled, reply } => {
            state.config.preferences.autostart = enabled;
            let _ = reply.send(Ok(()));
        }
        RuntimeCommand::SetPersistenceState { persistence, reply } => {
            state.persistence = persistence;
            let _ = reply.send(Ok(()));
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
            let result = if let Some(provider) = state.providers.get(&widget_id) {
                if !provider.in_flight {
                    scheduler.schedule_provider_now(&widget_id, Instant::now());
                }
                Ok(())
            } else {
                Err(RuntimeError::UnknownWidget { widget_id })
            };
            let _ = reply.send(result);
        }
        RuntimeCommand::ActivateScreen { screen_id, reply } => {
            let result = if state
                .config
                .cards
                .iter()
                .any(|card| card.presence().is_in_rotation() && card.id() == screen_id)
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
    for widget_id in scheduler.due_providers(now) {
        if state
            .providers
            .get(&widget_id)
            .is_none_or(|provider| provider.in_flight)
        {
            continue;
        }
        let Some(request) = provider_request(state, &widget_id) else {
            continue;
        };
        match provider.try_submit(request) {
            Ok(()) => {
                if let Some(provider) = state.providers.get_mut(&widget_id) {
                    provider.in_flight = true;
                    provider.snapshot.state = ProviderState::Refreshing;
                }
                diagnostics
                    .provider_jobs_started
                    .fetch_add(1, Ordering::Relaxed);
                scheduler.provider_started(&widget_id, now);
            }
            Err(ProviderSubmitError::Full) => {
                diagnostics
                    .provider_queue_full
                    .fetch_add(1, Ordering::Relaxed);
                scheduler.provider_retry(&widget_id, now, retry);
            }
            Err(ProviderSubmitError::Disconnected) => {
                if let Some(provider) = state.providers.get_mut(&widget_id) {
                    provider.snapshot.state = ProviderState::Error {
                        message: "provider worker stopped".into(),
                    };
                }
            }
        }
    }
}

fn provider_request(state: &WorkerState, widget_id: &str) -> Option<ProviderRefreshRequest> {
    let active_provider_ids: Vec<String> = state.providers.keys().cloned().collect();
    let timezone = state.config.preferences.timezone.parse().ok()?;
    state.config.cards.iter().find_map(|widget| match widget {
        CardSettings::Calendar {
            id,
            title,
            source,
            refresh,
            ..
        } if id == widget_id => Some(ProviderRefreshRequest {
            generation: state.generation,
            widget_id: id.clone(),
            title: title.clone(),
            provider: ProviderRequest::Calendar {
                source: source.clone(),
                timezone,
                refresh_interval: refresh_interval(*refresh),
            },
            active_provider_ids: active_provider_ids.clone(),
            now: Utc::now(),
        }),
        CardSettings::Weather {
            id,
            title,
            location,
            units,
            refresh,
            ..
        } if id == widget_id => Some(ProviderRefreshRequest {
            generation: state.generation,
            widget_id: id.clone(),
            title: title.clone(),
            provider: ProviderRequest::Weather {
                location: location.clone(),
                units: *units,
                refresh_interval: refresh_interval(*refresh),
            },
            active_provider_ids: active_provider_ids.clone(),
            now: Utc::now(),
        }),
        CardSettings::JsonFeed {
            id,
            title,
            url,
            mappings,
            refresh,
            ..
        } if id == widget_id => Some(ProviderRefreshRequest {
            generation: state.generation,
            widget_id: id.clone(),
            title: title.clone(),
            provider: ProviderRequest::JsonFeed {
                url: url.clone(),
                mappings: mappings.clone(),
                refresh_interval: refresh_interval(*refresh),
            },
            active_provider_ids: active_provider_ids.clone(),
            now: Utc::now(),
        }),
        CardSettings::Rss {
            id,
            title,
            url,
            max_items,
            refresh,
            ..
        } if id == widget_id => Some(ProviderRefreshRequest {
            generation: state.generation,
            widget_id: id.clone(),
            title: title.clone(),
            provider: ProviderRequest::Rss {
                url: url.clone(),
                maximum_items: *max_items,
                refresh_interval: refresh_interval(*refresh),
            },
            active_provider_ids: active_provider_ids.clone(),
            now: Utc::now(),
        }),
        _ => None,
    })
}

fn refresh_interval(policy: crate::RefreshPolicy) -> Duration {
    Duration::from_secs(u64::from(policy.interval_minutes().unwrap_or(1_440)) * 60)
}

fn drain_provider_results(
    state: &mut WorkerState,
    provider: &ProviderWorker,
    diagnostics: &RuntimeDiagnosticCounters,
) {
    while let Ok(result) = provider.try_recv() {
        let Some(provider_state) = state.providers.get_mut(&result.widget_id) else {
            diagnostics
                .provider_results_discarded
                .fetch_add(1, Ordering::Relaxed);
            continue;
        };
        if result.generation != provider_state.generation || result.generation != state.generation {
            diagnostics
                .provider_results_discarded
                .fetch_add(1, Ordering::Relaxed);
            continue;
        }
        provider_state.in_flight = false;
        provider_state.snapshot = ProviderSnapshot {
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
        if update.completion_interrupt && card_wants_completion_interrupt(&state.config, &widget_id)
        {
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
    if update.completion_interrupt && card_wants_completion_interrupt(&state.config, widget_id) {
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

/// A pomodoro's completion only becomes a host-triggered interrupt when its
/// card is configured to alert on timer finish. Task 6 narrows this further
/// to `CardAlert::OnTimerFinish` specifically; for now any configured alert
/// is treated as opting in.
fn card_wants_completion_interrupt(config: &AppConfig, widget_id: &str) -> bool {
    config
        .cards
        .iter()
        .find(|card| card.id() == widget_id)
        .is_some_and(|card| !card.alert().is_none())
}

fn drain_device_events(state: &mut WorkerState, device: &mut dyn RuntimeDevice, now: Instant) {
    while let Some(received) = device.try_recv_event() {
        match (received.event.kind, received.event.action) {
            (EventKind::Navigation, EventAction::NavigatePrevious | EventAction::NavigateNext) => {
                if state.config.cards.iter().any(|card| {
                    card.presence().is_in_rotation() && card.id() == received.event.screen_id
                }) {
                    state.active_screen = Some(received.event.screen_id.clone());
                    state
                        .device
                        .active_screen_id
                        .clone_from(&state.active_screen);
                    // The gesture already changed the physical display. Remember it for
                    // future replay without issuing a redundant activation now.
                    state.active_screen_dirty = false;
                }
            }
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
        .apply_layout(
            compiled.layout.rotation,
            compiled.layout.widgets,
            compiled.layout.screens,
        )
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
    // The dedicated field is additive in M3. Revisions provide a safe monotonic floor
    // when talking to an older v1 image that omits it: interrupt tokens and revisions
    // are all nonzero u32 counters, and starting higher is always accepted.
    state.interrupts.advance_latest_token(
        status
            .latest_interrupt_token
            .max(status.latest_revision)
            .max(status.config_revision),
    );
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
    for card in config.cards.iter().filter(|card| !card.presence().is_off()) {
        match card {
            CardSettings::Pomodoro {
                id,
                duration_seconds,
                ..
            } => pomodoros.push(PomodoroSnapshot {
                widget_id: id.clone(),
                state: PomodoroState::Idle,
                duration_seconds: *duration_seconds,
                remaining_seconds: *duration_seconds,
            }),
            CardSettings::Calendar { id, .. }
            | CardSettings::Weather { id, .. }
            | CardSettings::JsonFeed { id, .. }
            | CardSettings::Rss { id, .. } => providers.push(ProviderSnapshot {
                widget_id: id.clone(),
                state: ProviderState::Idle,
                last_success_unix_ms: None,
                age_seconds: None,
            }),
            CardSettings::Clock { .. } => {}
        }
    }
    AppSnapshot {
        config: config.clone(),
        runtime: RuntimeState::Starting,
        device: DeviceSnapshot {
            active_screen_id: config
                .cards
                .iter()
                .find(|card| card.presence().is_in_rotation())
                .map(|card| card.id().to_owned()),
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
        max_protocol_version: None,
        capabilities: Vec::new(),
        unknown_capability_bits: 0,
        uptime_ms: None,
        free_heap: None,
        rotation: None,
        active_screen_id: None,
        counters: DeviceCounters::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ImmediateRefresher {
        completed: mpsc::Sender<()>,
    }

    impl ProviderRefresher for ImmediateRefresher {
        fn refresh(&mut self, request: ProviderRefreshRequest) -> ProviderRefreshResult {
            self.completed.send(()).unwrap();
            ProviderRefreshResult {
                generation: request.generation,
                widget_id: request.widget_id,
                fields: Vec::new(),
                refreshed_at: Some(request.now),
                age: Some(Duration::ZERO),
                stale: false,
                error: None,
            }
        }
    }

    fn provider_request(widget_id: &str) -> ProviderRefreshRequest {
        ProviderRefreshRequest {
            generation: 1,
            widget_id: widget_id.into(),
            title: widget_id.into(),
            provider: ProviderRequest::Calendar {
                source: CalendarSource::File("unused.ics".into()),
                timezone: Tz::UTC,
                refresh_interval: Duration::from_mins(1),
            },
            active_provider_ids: Vec::new(),
            now: Utc::now(),
        }
    }

    #[test]
    fn completed_provider_results_are_never_dropped_under_backpressure() {
        let (completed_sender, completed_receiver) = mpsc::channel();
        let worker = ProviderWorker::new(
            Box::new(ImmediateRefresher {
                completed: completed_sender,
            }),
            1,
        );

        for widget_id in ["one", "two", "three"] {
            worker.try_submit(provider_request(widget_id)).unwrap();
            completed_receiver
                .recv_timeout(Duration::from_secs(1))
                .unwrap();
        }

        let deadline = Instant::now() + Duration::from_secs(1);
        let mut completed_ids = Vec::new();
        while completed_ids.len() < 3 && Instant::now() < deadline {
            match worker.try_recv() {
                Ok(result) => completed_ids.push(result.widget_id),
                Err(TryRecvError::Empty) => thread::yield_now(),
                Err(TryRecvError::Disconnected) => break,
            }
        }
        assert_eq!(completed_ids, ["one", "two", "three"]);
    }
}
