use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, RwLock, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use device::{ConnectedSession, DeviceError, ReceivedEvent, SessionDiagnostics, connect_session};
use engine::interrupts::InterruptArbiter;
use engine::pomodoro::{Pomodoro, PomodoroState as EnginePomodoroState};
use protocol::{
    Ack, ActivateScreen, AssetBegin, AssetChunk, AssetCommit, AssetRelease, EventAction, EventKind,
    Field, FieldValue, Message, NetworkConfig, PushScene, ScreenConfig, StatusResponse, TimeSync,
    TriggerInterrupt, WidgetConfig, validate_message,
};
use providers::Provider;
use providers::http::SystemHttpClient;
use providers::ics::{CalendarOptions, IcsProvider, IcsSource};
use providers::json_feed::{JsonFeedOptions, JsonFeedProvider, JsonMapping as ProviderJsonMapping};
use providers::rss::{RssOptions, RssProvider};
use providers::weather::{WeatherOptions, WeatherProvider, WeatherUnits as ProviderWeatherUnits};

use crate::commands::{CommandReply, PomodoroAction, RuntimeCommand, RuntimeError};
use crate::scheduler::Scheduler;
use crate::{
    AlertHold, AnalogClockCard, AppConfig, AppSnapshot, BakedFontMetrics, BigNumberCard,
    CalendarSource, CardAlert, CardDataSnapshot, CardError, CardErrorKind, CardSettings, ClockCard,
    ConnectionState, DeviceCapability, DeviceCounters, DeviceSnapshot, DeviceTier, DisplayTemplate,
    IconBadgeCard, JsonFieldMapping, PersistenceState, PomodoroSnapshot, PomodoroState,
    ProgressRingCard, ProviderSnapshot, ProviderState, RowListCard, RuntimeDiagnostics,
    RuntimeState, SHIPPED_SCENE_SURFACE_COLOR, SceneDataState, WeatherUnits,
    build_analog_clock_scene, build_big_number_label_scene, build_digital_clock_scene,
    build_icon_badge_text_scene, build_progress_ring_scene, build_row_list_scene,
    with_scene_data_state,
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
    interrupt_dismissals_ignored: AtomicU64,
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
            interrupt_dismissals_ignored: self.interrupt_dismissals_ignored.load(Ordering::Relaxed),
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

    pub fn push_scene(&self, push: PushScene) -> Result<(), RuntimeError> {
        self.request(|reply| RuntimeCommand::PushScene { push, reply })
    }

    /// Provision through the session already owned by the runtime worker. This command never
    /// discovers or opens a serial port; disconnected runtimes fail before touching the device.
    pub fn provision(&self, config: NetworkConfig) -> Result<(), RuntimeError> {
        self.request(|reply| RuntimeCommand::Provision { config, reply })
    }

    /// Factory-reset through the runtime's existing connected session.
    pub fn factory_reset(&self) -> Result<(), RuntimeError> {
        self.request(|reply| RuntimeCommand::FactoryReset { reply })
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
    /// Card ID -> the most recent typed refusal for that card. There is deliberately
    /// one editor-visible slot per card: if data and scene refusals happen before
    /// either recovers, the later refusal replaces the earlier one. A later accepted
    /// push clears the slot only when it is the same kind, and config replacement
    /// clears every slot because all refused payloads belonged to the old revision.
    push_rejections: BTreeMap<String, CardError>,
    pomodoros: BTreeMap<String, Pomodoro>,
    pomodoro_snapshots: BTreeMap<String, PomodoroSnapshot>,
    providers: BTreeMap<String, ProviderRuntimeState>,
    interrupts: InterruptArbiter,
    /// Card ID -> unix-ms start of the event a `CardAlert::BeforeEvent` trigger
    /// last fired for, so repeated provider refreshes inside the same lead
    /// window do not re-fire. See `evaluate_event_alert`.
    armed_event_alerts: BTreeMap<String, i64>,
    active_screen: Option<String>,
    active_screen_dirty: bool,
    /// The active card needs rebuilding as a scene because a host-owned fact
    /// changed. Consumed once by `push_active_scene`; scheduled ticks never set it.
    active_scene_dirty: bool,
    active_rotation_index: usize,
    connected: bool,
    ever_connected: bool,
    needs_full_sync: bool,
    /// Latched only after this runtime receives an explicit `WrongTier` refusal.
    /// A Networked status alone cannot establish non-ownership because the server's
    /// WebSocket runtime legitimately drives devices that report that tier.
    ownership_refused: bool,
    generation: u64,
    next_scene_revision: u32,
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
            push_rejections: BTreeMap::new(),
            pomodoros: BTreeMap::new(),
            pomodoro_snapshots: BTreeMap::new(),
            providers: BTreeMap::new(),
            interrupts: InterruptArbiter::default(),
            armed_event_alerts: BTreeMap::new(),
            active_screen: None,
            active_screen_dirty: false,
            active_scene_dirty: false,
            active_rotation_index: 0,
            connected: false,
            ever_connected: false,
            needs_full_sync: true,
            ownership_refused: false,
            generation: 0,
            next_scene_revision: 0,
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
        self.push_rejections.clear();
        self.pomodoro_snapshots.clear();
        // Owned, not borrowed: `prune_alert_state_for_live_widgets` needs
        // `&mut self`, which cannot coexist with a set still borrowing from
        // `self.config`.
        let live_widget_ids: BTreeSet<String> = self
            .config
            .compiled_card_ids()
            .into_iter()
            .map(str::to_owned)
            .collect();
        self.prune_alert_state_for_live_widgets(&live_widget_ids, scheduler, now);

        let compiled = self
            .config
            .compile(1)
            .expect("worker accepts only prevalidated configurations");
        for push in compiled.initial_pushes {
            self.latest_fields.insert(push.widget_id, push.fields);
        }

        let mut provider_deadlines = Vec::new();
        let compiled_card_ids: BTreeSet<&str> =
            self.config.compiled_card_ids().into_iter().collect();
        let cards: Vec<CardSettings> = self
            .config
            .cards
            .iter()
            .filter(|card| compiled_card_ids.contains(card.id()))
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
        let rotation_card_ids = rotation_card_ids(&self.config);
        // A playlist-ID change deliberately resets to its first entry, even when both playlists share the old card, per the acceptance test.
        self.active_screen = if previous_config.active_playlist_id == self.config.active_playlist_id
        {
            previous_active_screen
                .filter(|active| rotation_card_ids.contains(active))
                .or_else(|| rotation_card_ids.first().cloned())
        } else {
            rotation_card_ids.first().cloned()
        };
        self.device.active_screen_id.clone_from(&self.active_screen);
        self.active_screen_dirty = self.active_screen.is_some();
        self.active_scene_dirty = self.active_screen.is_some();
        self.rearm_rotation_for_active_screen(scheduler, now);
        self.needs_full_sync = true;
        self.runtime = if self.config.preferences.paused {
            RuntimeState::Paused
        } else {
            RuntimeState::Running
        };
    }

    /// Prunes every piece of alert-related state keyed to a widget that is no
    /// longer live: the interrupt arbiter's tracked interrupts (existing,
    /// pre-Task-6 behavior), the calendar dedup map, and the scheduler's
    /// event-alert-check deadlines (Task 6). Also fixes Task 6 review
    /// Critical-1's second half: the alert hold is keyed to a specific
    /// interrupt token (see `sync_alert_hold_to_active_interrupt`'s doc
    /// comment for why), so it must be resynced whenever pruning changes
    /// *who* is active — but left untouched otherwise, so an unrelated config
    /// edit that leaves the active interrupt's widget live does not reset its
    /// in-flight countdown.
    fn prune_alert_state_for_live_widgets(
        &mut self,
        live_widget_ids: &BTreeSet<String>,
        scheduler: &mut Scheduler,
        now: Instant,
    ) {
        let active_token_before_prune = self
            .interrupts
            .active()
            .map(|tracked| tracked.message.token);
        self.interrupts
            .retain_widgets(|widget_id| live_widget_ids.contains(widget_id));
        let active_token_after_prune = self
            .interrupts
            .active()
            .map(|tracked| tracked.message.token);
        if active_token_before_prune != active_token_after_prune {
            sync_alert_hold_to_active_interrupt(self, scheduler, now);
        }
        self.armed_event_alerts
            .retain(|card_id, _| live_widget_ids.contains(card_id.as_str()));
        scheduler.retain_event_alert_checks(|card_id| live_widget_ids.contains(card_id));
    }

    /// Dwell is per-card: arm the deadline from whichever card ended up
    /// active (the preserved screen if it is still in rotation, otherwise the
    /// first in-rotation card), not blindly from index 0.
    ///
    /// Called from `replace_config`, i.e. at config install time, which runs
    /// before the device connects. The clock therefore starts on the boot
    /// (or newly-applied) card's dwell immediately, not from first connect;
    /// on a slow first connect the boot card can end up on screen for less
    /// than its configured dwell. Noted, not restructured — connect is
    /// normally fast and this only shortens one card's first showing.
    fn rearm_rotation_for_active_screen(&mut self, scheduler: &mut Scheduler, now: Instant) {
        let rotation_ids = rotation_card_ids(&self.config);
        self.active_rotation_index = self
            .active_screen
            .as_ref()
            .and_then(|active| rotation_ids.iter().position(|id| id == active))
            .unwrap_or(0);
        scheduler.set_rotation(current_dwell(&self.config, self.active_rotation_index), now);
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
            card_data: self
                .latest_fields
                .iter()
                .map(|(card_id, fields)| CardDataSnapshot::from_protocol(card_id, fields))
                .collect(),
            card_errors: self.push_rejections.values().cloned().collect(),
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

/// Active-playlist card IDs in entry order. Screen IDs equal card IDs (see
/// `AppConfig::compile`), so this doubles as the rotation's screen order.
fn rotation_card_ids(config: &AppConfig) -> Vec<String> {
    config
        .active_playlist()
        .map(|playlist| {
            playlist
                .entries
                .iter()
                .map(|entry| entry.card_id.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// The dwell for the active-playlist entry at `index`, resolved against the
/// playlist's default. Returns `None` under `CarouselAdvance::Manual`, which
/// is what keeps the rotation deadline disarmed in manual mode.
fn current_dwell(config: &AppConfig, index: usize) -> Option<Duration> {
    let playlist = config.active_playlist()?;
    let default = playlist.advance.default_dwell_seconds()?;
    let entry = playlist.entries.get(index)?;
    Some(Duration::from_secs(u64::from(
        entry.dwell_seconds.unwrap_or(default),
    )))
}

/// Advances the rotation by one step (wrapping) and queues the resulting
/// screen through the existing `active_screen_dirty` flush path, then
/// re-arms the deadline from the card just moved to, since dwell is per-card.
/// With fewer than two in-rotation cards there is nothing to rotate to, so the
/// deadline is disarmed instead of waking the loop again for a no-op; the next
/// config replace re-evaluates it (see `WorkerState::replace_config`).
fn advance_rotation(state: &mut WorkerState, scheduler: &mut Scheduler, now: Instant) {
    let ids = rotation_card_ids(&state.config);
    if ids.len() > 1 {
        state.active_rotation_index = (state.active_rotation_index + 1) % ids.len();
        state.active_screen = Some(ids[state.active_rotation_index].clone());
        state
            .device
            .active_screen_id
            .clone_from(&state.active_screen);
        state.active_screen_dirty = true;
        state.active_scene_dirty = true;
        scheduler.set_rotation(
            current_dwell(&state.config, state.active_rotation_index),
            now,
        );
    } else {
        scheduler.clear_rotation();
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
                    options.reconnect_interval,
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
                        options.reconnect_interval,
                    );
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => shutting_down = true,
        }
        if shutting_down {
            break;
        }

        let now = Instant::now();
        drain_provider_results(&mut state, &mut scheduler, &provider, diagnostics, now);
        if !state.connected && now >= state.next_connect {
            attempt_connect(&mut state, &mut scheduler, device.as_mut(), now, &options);
        }
        drain_device_events(
            &mut state,
            &mut scheduler,
            device.as_mut(),
            diagnostics,
            now,
        );
        // Ownership synchronization is complete at this point. Publish that
        // fact before attempting the best-effort render update: a scene is the
        // face drawn by an already-owned device, not a prerequisite for Online.
        state.publish_if_changed(publisher, diagnostics);
        if state.connected && !state.config.preferences.paused {
            push_active_scene(&mut state, device.as_mut(), options.reconnect_interval);
        }
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
    reconnect_interval: Duration,
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
                let now = Instant::now();
                state.replace_config(config, now, scheduler);
                if state.connected && !state.config.preferences.paused {
                    synchronize_full(state, scheduler, device, now)
                } else {
                    Ok(())
                }
            });
            let _ = reply.send(result);
        }
        RuntimeCommand::SetPaused { paused, reply } => {
            let was_paused = state.config.preferences.paused;
            state.config.preferences.paused = paused;
            state.runtime = if paused {
                RuntimeState::Paused
            } else {
                RuntimeState::Running
            };
            if was_paused && !paused {
                let now = Instant::now();
                scheduler.schedule_time_sync_now(now);
                scheduler.schedule_skipped_providers_now(now);
            }
            let result = if !paused && state.connected {
                synchronize_pending(state, scheduler, device, Instant::now())
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
            let result =
                control_pomodoro(state, scheduler, device, &widget_id, action, Instant::now());
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
            let result = activate_screen_command(state, scheduler, device, screen_id);
            let _ = reply.send(result);
        }
        RuntimeCommand::PushScene { push, reply } => {
            reply_to_runtime_device_command(&reply, state, reconnect_interval, || {
                device.push_scene(push)
            });
        }
        RuntimeCommand::Provision { config, reply } => {
            reply_to_runtime_device_command(&reply, state, reconnect_interval, || {
                device.provision(&config)
            });
        }
        RuntimeCommand::FactoryReset { reply } => {
            reply_to_runtime_device_command(&reply, state, reconnect_interval, || {
                device.factory_reset()
            });
        }
        RuntimeCommand::Shutdown { reply } => {
            let _ = reply.send(Ok(()));
            return true;
        }
    }
    false
}

fn activate_screen_command(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    device: &mut dyn RuntimeDevice,
    screen_id: String,
) -> Result<(), RuntimeError> {
    let ids = rotation_card_ids(&state.config);
    let Some(index) = ids.iter().position(|id| *id == screen_id) else {
        return Err(RuntimeError::UnknownScreen { screen_id });
    };
    state.active_screen = Some(screen_id.clone());
    state.device.active_screen_id = Some(screen_id);
    state.active_screen_dirty = true;
    state.active_scene_dirty = true;
    // An explicit activation is a manual override, same as a physical
    // swipe: restart the dwell from the card just landed on instead of
    // advancing early from wherever rotation last left off.
    state.active_rotation_index = index;
    scheduler.set_rotation(current_dwell(&state.config, index), Instant::now());
    if state.connected && !state.config.preferences.paused {
        send_screen(state, device)
    } else {
        Ok(())
    }
}

fn attempt_connect(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    device: &mut dyn RuntimeDevice,
    now: Instant,
    options: &RuntimeOptions,
) {
    state.device.connection = ConnectionState::Connecting;
    match device.connect() {
        Ok(connection) => {
            state.connected = true;
            state.ever_connected = true;
            // Scheduled ticks are consumed even while disconnected so they cannot
            // pin the worker loop. Re-arm them at the transition that makes their
            // I/O possible, preserving the prompt refresh after every reconnect.
            scheduler.schedule_status_now(now);
            scheduler.schedule_time_sync_now(now);
            update_device_status(state, &connection.port_name, &connection.status, device);
            state.runtime = if state.config.preferences.paused {
                RuntimeState::Paused
            } else {
                RuntimeState::Running
            };
            if !state.config.preferences.paused {
                let result = if state.needs_full_sync {
                    synchronize_full(state, scheduler, device, now)
                } else {
                    synchronize_pending(state, scheduler, device, now)
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
    // Deliberate: this sits above the `!paused` guard below, so rotation keeps
    // advancing locally while paused, exactly like pomodoro ticks above. Pausing
    // gates device synchronization (see the guards later in this function and in
    // `synchronize_pending`), not local state. A long pause therefore lets rotation
    // drift, and resuming jumps the panel to wherever it drifted to; this mirrors
    // existing pomodoro behavior rather than introducing a new inconsistency.
    // Pinned by `rotation_advances_locally_while_paused_mirroring_pomodoro_ticks`.
    if scheduler.rotation_due(now) {
        advance_rotation(state, scheduler, now);
    }
    // Same locally-always-advances rationale as rotation above: the hold is a
    // local deadline on the arbiter's active interrupt, independent of device
    // connectivity or pause. Dismissing it just changes local state; the actual
    // device push (if connected) happens through the normal sync path below via
    // `active_screen_dirty` and `flush_interrupts`.
    //
    // BY DESIGN (decided 2026-08-06): this only frees the host's arbiter slot
    // and re-sends the saved screen id (see `send_screen` below); the device
    // does not clear the interrupt overlay itself until the user taps it, so
    // host and device interrupt state can diverge until the next tap or a
    // full resync (see `arm_alert_hold`'s doc comment for the full picture).
    if let Some(token) = scheduler.alert_hold_due(now)
        && state.interrupts.dismiss(token).is_ok()
    {
        state.active_screen_dirty = true;
        sync_alert_hold_to_active_interrupt(state, scheduler, now);
    }
    // Re-evaluates every calendar card's `CardAlert::BeforeEvent` trigger on
    // its own wake, independent of whether a provider refresh happens to land
    // inside the lead window (Task 6 review Important-3: with the migration
    // default 5-minute lead and any refresh interval above that, relying on
    // `apply_provider_result` alone missed most windows — e.g. a 15-minute
    // refresh interval landing at 8 minutes out, then again after the event
    // started, never once lands inside a 5-minute window).
    for card_id in scheduler.due_event_alert_cards(now) {
        if let CardAlert::BeforeEvent { lead_minutes, .. } = card_alert(&state.config, &card_id) {
            let now_unix_ms = Utc::now().timestamp_millis();
            refresh_calendar_alert(state, scheduler, &card_id, lead_minutes, now, now_unix_ms);
        }
    }
    if state.config.preferences.paused {
        scheduler.skip_due_providers(now);
    } else {
        submit_due_providers(
            state,
            scheduler,
            provider,
            diagnostics,
            now,
            options.provider_queue_retry,
        );
    }
    // A deadline in `wait_duration` must be consumed whenever this tick examines
    // it, even if connectivity or pause gates the actual I/O below. Otherwise a
    // skipped deadline remains in the past and `recv_timeout` spins on zero forever.
    let status_due = scheduler.status_due(now);
    let time_sync_due = scheduler.time_sync_due(now);
    if !state.connected {
        return;
    }
    if status_due {
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
    if time_sync_due && let Err(error) = send_time_sync(state, device) {
        state.runtime = RuntimeState::Error {
            message: error.to_string(),
        };
    }
    if let Err(error) = synchronize_pending(state, scheduler, device, now) {
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
    for widget_id in scheduler.take_due_providers(now) {
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
    scheduler: &mut Scheduler,
    provider: &ProviderWorker,
    diagnostics: &RuntimeDiagnosticCounters,
    now: Instant,
) {
    while let Ok(result) = provider.try_recv() {
        apply_provider_result(state, scheduler, diagnostics, now, result);
    }
}

/// Applies one completed provider refresh: projects it into the provider's
/// snapshot/fields, then (calendar cards only) re-evaluates the card's
/// `CardAlert::BeforeEvent` trigger — this is one of two places that
/// happens, the other being the scheduler-driven tick in `run_scheduled_work`
/// that catches the lead window opening between refreshes (see
/// `refresh_calendar_alert`).
fn apply_provider_result(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    diagnostics: &RuntimeDiagnosticCounters,
    now: Instant,
    result: ProviderRefreshResult,
) {
    let Some(provider_state) = state.providers.get_mut(&result.widget_id) else {
        diagnostics
            .provider_results_discarded
            .fetch_add(1, Ordering::Relaxed);
        return;
    };
    if result.generation != provider_state.generation || result.generation != state.generation {
        diagnostics
            .provider_results_discarded
            .fetch_add(1, Ordering::Relaxed);
        return;
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
            (_, Some(message), None) | (false, Some(message), Some(_)) => ProviderState::Error {
                message: message.clone(),
            },
        },
        last_success_unix_ms: result.refreshed_at.map(|time| time.timestamp_millis()),
        age_seconds: result.age.map(|age| age.as_secs()),
    };
    let widget_id = result.widget_id;
    state.latest_fields.insert(widget_id.clone(), result.fields);
    state.dirty_widgets.insert(widget_id.clone());
    if state.active_screen.as_deref() == Some(widget_id.as_str()) {
        // Provider completion is a host fact, including a stale/error-only
        // transition. Rebuild once now; do not infer a refresh cadence here.
        state.active_scene_dirty = true;
    }

    if let CardAlert::BeforeEvent { lead_minutes, .. } = card_alert(&state.config, &widget_id) {
        let now_unix_ms = Utc::now().timestamp_millis();
        refresh_calendar_alert(state, scheduler, &widget_id, lead_minutes, now, now_unix_ms);
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
            let _ = state
                .interrupts
                .schedule(widget_id.clone(), "Timer finished");
        }
    }
}

fn control_pomodoro(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
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
        flush_interrupts(state, scheduler, device, now)?;
    }
    Ok(())
}

/// The configured alert for a card, or `CardAlert::None` if the card is
/// unknown (e.g. it was removed from config between scheduling and firing).
fn card_alert(config: &AppConfig, widget_id: &str) -> CardAlert {
    config
        .cards
        .iter()
        .find(|card| card.id() == widget_id)
        .map_or(CardAlert::None, CardSettings::alert)
}

/// A pomodoro's completion only becomes a host-triggered interrupt when its
/// card is configured to alert specifically on timer finish. `CardAlert::None`
/// never fires, and `CardAlert::BeforeEvent` is a calendar-only trigger (see
/// `is_event_alert_due`) that a pomodoro's completion can never satisfy, even
/// if config validation somehow let it slip through. Config validation
/// forbids that combination (`BeforeEvent` is calendar-only), so this
/// narrowing from Task 2's original "any configured alert" gate is only
/// observable on a config built directly rather than through
/// `AppConfig::compile`/`apply_config` — see
/// `card_wants_completion_interrupt_rejects_before_event_even_on_a_pomodoro_card`,
/// which constructs exactly that and fails under the old, wider gate.
fn card_wants_completion_interrupt(config: &AppConfig, widget_id: &str) -> bool {
    matches!(
        card_alert(config, widget_id),
        CardAlert::OnTimerFinish { .. }
    )
}

/// Arms the scheduler's bounded alert-hold deadline for `token`, but only if
/// `token` actually is the arbiter's *active* interrupt. Delivery of a Pending
/// interrupt does not run a deadline of its own; one is armed for it only once
/// it is promoted to Active (see `sync_alert_hold_to_active_interrupt`). This
/// guard, and keying the hold to
/// a specific token in the first place, is what fixes Task 6 review
/// Critical-1: a single unkeyed, unconditionally-armed hold let scheduling
/// *any* interrupt (including one that landed Pending) clobber the deadline
/// belonging to a *different*, already-Active interrupt — auto-dismissing an
/// `UntilDismissed` alert early, or silently losing a bounded alert's
/// deadline entirely.
///
/// `AlertHold::Seconds` arms an absolute deadline from successful delivery;
/// `AlertHold::UntilDismissed` (or no hold at all) disarms it.
///
/// BY DESIGN (decided 2026-08-06, design spec §3.2): expiring this deadline
/// only frees the host's arbiter slot for the next alert and re-syncs the
/// saved carousel screen id host-side (see the `alert_hold_due` handling in
/// `run_scheduled_work`). It does not clear the panel. Per
/// `docs/protocol/v1.md`'s `ActivateScreen` section and firmware's
/// `protocol_task.c` (`show_carousel_screen` is skipped whenever an interrupt
/// is active), the device yields the overlay on tap alone. Confirmed on
/// hardware — see `docs/hardware/board-notes.md`, card-model §6.
///
/// The consequence to keep in mind: dismissing host-side while the device
/// still shows the overlay leaves host and device interrupt state diverged
/// (the device still expects a tap; a further host-triggered alert can be
/// rejected `Busy` even though the host arbiter believes its slot is free)
/// until the next tap or a full resync. Do not "fix" this by adding a wire
/// dismissal message — that was considered and rejected for v1, because an
/// alert worth interrupting the user is worth acknowledging.
fn arm_alert_hold(scheduler: &mut Scheduler, token: u32, hold: Option<AlertHold>, now: Instant) {
    let deadline = match hold {
        Some(AlertHold::Seconds { value }) => Some(now + Duration::from_secs(u64::from(value))),
        Some(AlertHold::UntilDismissed) | None => None,
    };
    scheduler.set_alert_hold(deadline.map(|deadline| (token, deadline)));
}

/// See `arm_alert_hold`'s doc comment for the Active-only invariant this
/// enforces.
fn arm_alert_hold_if_active(
    interrupts: &InterruptArbiter,
    scheduler: &mut Scheduler,
    token: u32,
    hold: Option<AlertHold>,
    now: Instant,
) {
    if interrupts
        .active()
        .is_some_and(|active| active.message.token == token)
    {
        arm_alert_hold(scheduler, token, hold, now);
    }
}

/// Recomputes the scheduler's alert-hold deadline from scratch against
/// whichever interrupt is currently Active. An already-acknowledged promoted
/// interrupt was delivered while Pending, so its own configured hold starts
/// fresh from `now`; an unacknowledged promoted interrupt has no deadline until
/// `flush_interrupts` delivers it. Callers only invoke this when the active
/// interrupt has actually just changed — after a dismissal promotes (or fails
/// to promote) a Pending interrupt, or after config replacement prunes the
/// previously-active interrupt's widget — never on every call, since
/// recomputing unconditionally would reset an unrelated, still-active
/// interrupt's in-flight countdown on every unrelated event.
fn sync_alert_hold_to_active_interrupt(
    state: &WorkerState,
    scheduler: &mut Scheduler,
    now: Instant,
) {
    match state.interrupts.active() {
        Some(active) if active.acknowledged => {
            let hold = card_alert(&state.config, &active.message.widget_id).hold();
            arm_alert_hold(scheduler, active.message.token, hold, now);
        }
        Some(_) | None => scheduler.set_alert_hold(None),
    }
}

/// Reads a calendar card's machine-readable next-event start
/// (`IcsCalendar::fields`'s `next_start_unix_ms`) out of its latest pushed
/// fields. `0` means "no upcoming event" and is treated as absent, matching
/// the provider's documented sentinel.
fn next_event_start_unix_ms(state: &WorkerState, card_id: &str) -> Option<i64> {
    let fields = state.latest_fields.get(card_id)?;
    fields.iter().find_map(|field| {
        if field.key != "next_start_unix_ms" {
            return None;
        }
        match field.value {
            FieldValue::Integer(unix_ms) if unix_ms != 0 => Some(unix_ms),
            _ => None,
        }
    })
}

/// Read-only eligibility check for a `CardAlert::BeforeEvent` trigger:
/// `Some(start_unix_ms)` when that start is strictly in the future, within
/// `lead_minutes` of `now_unix_ms`, and is not already the start this card
/// last fired for; `None` otherwise (outside the window, no known event, or
/// already armed for this exact start). Does **not** mutate
/// `armed_event_alerts` itself — see `refresh_calendar_alert`, which only
/// records a start as armed after `InterruptArbiter::schedule` actually
/// succeeds. (Task 6 review Important-4: the previous version marked the
/// start armed unconditionally before checking whether `schedule` succeeded,
/// so a transient `Busy` — both arbiter slots already occupied — silently and
/// *permanently* dropped the alert for that event, since the dedup check then
/// always saw it as already armed.)
fn is_event_alert_due(
    state: &WorkerState,
    card_id: &str,
    lead_minutes: u16,
    now_unix_ms: i64,
) -> Option<i64> {
    let start_unix_ms = next_event_start_unix_ms(state, card_id)?;
    let lead_ms = i64::from(lead_minutes) * 60_000;
    let within_window = start_unix_ms > now_unix_ms && start_unix_ms - now_unix_ms <= lead_ms;
    if !within_window {
        return None;
    }
    if state.armed_event_alerts.get(card_id) == Some(&start_unix_ms) {
        return None;
    }
    Some(start_unix_ms)
}

/// The delay in milliseconds (always `> 0`) until this card's lead window is
/// expected to open, based on its currently cached `next_start_unix_ms`, or
/// `None` if there's nothing to wait for — no known future event, or the
/// window is already open/past, both of which the caller already handled
/// synchronously in the same call via `is_event_alert_due`.
fn next_event_alert_check_delay_ms(
    state: &WorkerState,
    card_id: &str,
    lead_minutes: u16,
    now_unix_ms: i64,
) -> Option<i64> {
    let start_unix_ms = next_event_start_unix_ms(state, card_id)?;
    let lead_ms = i64::from(lead_minutes) * 60_000;
    let boundary_unix_ms = start_unix_ms - lead_ms;
    let offset_ms = boundary_unix_ms - now_unix_ms;
    (offset_ms > 0).then_some(offset_ms)
}

/// Re-evaluates a calendar card's `CardAlert::BeforeEvent` trigger against
/// its currently cached next-event start, firing it if eligible, then
/// (re-)schedules the scheduler wake needed to catch the lead window opening
/// even without a fresh provider refresh landing inside it.
///
/// Called from two places: `apply_provider_result`, the instant fresh field
/// data lands, and a scheduler tick in `run_scheduled_work` driven by the
/// deadline this function itself sets. The second call site is the fix for
/// Task 6 review Important-3: relying on provider-result landings alone (as
/// originally specified) meant the trigger could miss its entire lead window
/// whenever no refresh happened to land inside it — e.g. a 15-minute refresh
/// interval against a 5-minute lead window, refreshing 8 minutes before the
/// event and again after it started, never once observes the window.
fn refresh_calendar_alert(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    card_id: &str,
    lead_minutes: u16,
    now: Instant,
    now_unix_ms: i64,
) {
    if let Some(start_unix_ms) = is_event_alert_due(state, card_id, lead_minutes, now_unix_ms)
        && state
            .interrupts
            .schedule(card_id, "Event starting soon")
            .is_ok()
    {
        // Only mark this event start armed once the interrupt actually landed
        // in the arbiter — see `is_event_alert_due`'s doc comment.
        state
            .armed_event_alerts
            .insert(card_id.to_owned(), start_unix_ms);
    }
    let deadline = next_event_alert_check_delay_ms(state, card_id, lead_minutes, now_unix_ms)
        .map(|offset_ms| now + Duration::from_millis(u64::try_from(offset_ms).unwrap_or(0)));
    scheduler.set_event_alert_deadline(card_id, deadline);
}

fn drain_device_events(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    device: &mut dyn RuntimeDevice,
    diagnostics: &RuntimeDiagnosticCounters,
    now: Instant,
) {
    while let Some(received) = device.try_recv_event() {
        match (received.event.kind, received.event.action) {
            (EventKind::Navigation, EventAction::NavigatePrevious | EventAction::NavigateNext) => {
                let ids = rotation_card_ids(&state.config);
                if let Some(index) = ids.iter().position(|id| *id == received.event.screen_id) {
                    state.active_screen = Some(received.event.screen_id.clone());
                    state
                        .device
                        .active_screen_id
                        .clone_from(&state.active_screen);
                    // The gesture already changed the physical display. Remember it for
                    // future replay without issuing a redundant activation now.
                    state.active_screen_dirty = false;
                    // The gesture selected the device model already, but the new
                    // card still needs its host-built scene laid over that model.
                    state.active_scene_dirty = true;
                    // A manual swipe restarts the dwell from the card just landed on,
                    // rather than letting a soon-to-expire deadline advance early.
                    // With a single in-rotation card this still re-arms unconditionally
                    // (current_dwell can return Some for that lone card), so the
                    // deadline fires once more and `advance_rotation` immediately
                    // disarms it again via `clear_rotation` — one harmless extra
                    // wake-and-clear cycle, not a leak.
                    state.active_rotation_index = index;
                    scheduler.set_rotation(current_dwell(&state.config, index), now);
                }
            }
            (EventKind::Tap, EventAction::StartPause) => {
                let _ = control_pomodoro(
                    state,
                    scheduler,
                    device,
                    &received.event.widget_id,
                    PomodoroAction::Toggle,
                    now,
                );
            }
            (EventKind::Tap, EventAction::Reset) => {
                let _ = control_pomodoro(
                    state,
                    scheduler,
                    device,
                    &received.event.widget_id,
                    PomodoroAction::Reset,
                    now,
                );
            }
            (EventKind::InterruptDismissed, EventAction::DismissInterrupt) => {
                let applied = received
                    .event
                    .interrupt_token
                    .is_some_and(|token| state.interrupts.dismiss(token).is_ok());
                if applied {
                    // A dismissal may promote a Pending interrupt to Active; the
                    // hold belongs to a specific token (see `arm_alert_hold`'s
                    // doc comment), so it must be re-armed from whichever
                    // interrupt is active now, not left pointing at the token
                    // that just left.
                    sync_alert_hold_to_active_interrupt(state, scheduler, now);
                } else {
                    // Declining is correct — there is nothing to dismiss — but it
                    // must not be invisible. See the counter's doc comment: on
                    // hardware, a tap the host knowingly declined and a tap whose
                    // event never arrived look identical without this.
                    diagnostics
                        .interrupt_dismissals_ignored
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
            _ => {}
        }
    }
}

fn synchronize_full(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    device: &mut dyn RuntimeDevice,
    now: Instant,
) -> Result<(), RuntimeError> {
    if ownership_was_refused(state) {
        return Ok(());
    }
    state.needs_full_sync = true;
    if !send_time_sync(state, device)? {
        return Ok(());
    }
    let compiled = state
        .config
        .compile(1)
        .expect("runtime config remains validated");
    if !sync_device_result(
        state,
        device.apply_layout(
            compiled.layout.rotation,
            compiled.layout.widgets,
            compiled.layout.screens,
        ),
    )? {
        return Ok(());
    }
    state.dirty_widgets = state.latest_fields.keys().cloned().collect();
    push_dirty_widgets(state, device)?;
    state.active_screen_dirty = state.active_screen.is_some();
    send_screen(state, device)?;
    // `RuntimeCommand::ApplyConfig` waits for this full ownership/model sync
    // before replying. Leave the scene dirty for the worker's separate render
    // phase: activation is the device-model transaction boundary, while drawing
    // the new face is the event-driven consequence of that completed apply.
    flush_interrupts(state, scheduler, device, now)?;
    state.active_scene_dirty = state.active_screen.is_some();
    state.needs_full_sync = false;
    Ok(())
}

fn synchronize_pending(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    device: &mut dyn RuntimeDevice,
    now: Instant,
) -> Result<(), RuntimeError> {
    if ownership_was_refused(state) {
        return Ok(());
    }
    if state.needs_full_sync {
        return synchronize_full(state, scheduler, device, now);
    }
    push_dirty_widgets(state, device)?;
    send_screen(state, device)?;
    flush_interrupts(state, scheduler, device, now)
}

/// Pushes every dirty widget, then leaves the dirty set holding only what still
/// needs sending.
///
/// A push the device *refuses* is not a transport failure and must not be retried:
/// the frame arrived, the device parsed it, and it rejected the contents (an
/// undeclared field type, an over-long text value, a value outside the template's
/// declared range). The identical payload can only be refused again, so retrying it
/// pins the runtime in `RuntimeState::Error` forever and starves every widget queued
/// behind it in the same cycle. Such a push is dropped from the dirty set and
/// recorded as a card-scoped `CardError` the settings UI can show against the card
/// that caused it. `Busy` is the one refusal that *is* transient (the device asked
/// the host to come back later), so it stays dirty for the next cycle. Transport
/// errors still propagate — those must reach the reconnect path.
fn push_dirty_widgets(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
) -> Result<(), RuntimeError> {
    if ownership_was_refused(state) {
        return Ok(());
    }
    let dirty: Vec<String> = state.dirty_widgets.iter().cloned().collect();
    for widget_id in dirty {
        let Some(fields) = state.latest_fields.get(&widget_id).cloned() else {
            state.dirty_widgets.remove(&widget_id);
            continue;
        };
        match device.push_fields(widget_id.clone(), fields) {
            Ok(()) => {
                state.dirty_widgets.remove(&widget_id);
                if state
                    .push_rejections
                    .get(&widget_id)
                    .is_some_and(|error| error.kind == CardErrorKind::DataRefused)
                {
                    state.push_rejections.remove(&widget_id);
                }
            }
            Err(error) if is_wrong_tier(&error) => {
                mark_ownership_refused(state);
                return Ok(());
            }
            Err(DeviceError::Rejected(error)) if error.code == protocol::ErrorCode::Busy => {}
            Err(DeviceError::Rejected(error)) => {
                state.dirty_widgets.remove(&widget_id);
                state.push_rejections.insert(
                    widget_id.clone(),
                    CardError {
                        kind: CardErrorKind::DataRefused,
                        card_id: widget_id,
                        message: format!(
                            "the display refused this card's data ({:?}): {}",
                            error.code, error.diagnostic
                        ),
                    },
                );
            }
            Err(error) => return Err(device_runtime_error(&error)),
        }
    }
    Ok(())
}

fn field_text<'a>(fields: &'a [Field], key: &str) -> &'a str {
    fields
        .iter()
        .find_map(|field| match (&*field.key, &field.value) {
            (candidate, FieldValue::Text(value)) if candidate == key => Some(value.as_str()),
            _ => None,
        })
        .unwrap_or("")
}

fn field_integer(fields: &[Field], key: &str) -> i64 {
    fields
        .iter()
        .find_map(|field| match (&*field.key, &field.value) {
            (candidate, FieldValue::Integer(value)) if candidate == key => Some(*value),
            _ => None,
        })
        .unwrap_or(0)
}

fn field_boolean(fields: &[Field], key: &str) -> bool {
    fields
        .iter()
        .find_map(|field| match (&*field.key, &field.value) {
            (candidate, FieldValue::Boolean(value)) if candidate == key => Some(*value),
            _ => None,
        })
        .unwrap_or(false)
}

fn build_card_scene(
    config: &AppConfig,
    card_id: &str,
    fields: &[Field],
    revision: u32,
) -> Result<PushScene, String> {
    let card = config
        .cards
        .iter()
        .find(|card| card.id() == card_id)
        .ok_or_else(|| format!("card {card_id:?} is not present in the active configuration"))?;
    let metrics = &BakedFontMetrics::SHIPPED;
    let scene = match card.template() {
        DisplayTemplate::DigitalClock => {
            let timezone: Tz = config
                .preferences
                .timezone
                .parse()
                .map_err(|_| "the configured timezone is not recognized".to_owned())?;
            build_digital_clock_scene(
                &ClockCard {
                    revision,
                    show_seconds: field_boolean(fields, "show_seconds"),
                    local_now: Utc::now().with_timezone(&timezone).naive_local(),
                },
                metrics,
            )
        }
        DisplayTemplate::AnalogClock => build_analog_clock_scene(
            &AnalogClockCard {
                revision,
                show_seconds: field_boolean(fields, "show_seconds"),
            },
            metrics,
        ),
        DisplayTemplate::ProgressRing => build_progress_ring_scene(
            &ProgressRingCard {
                revision,
                label: field_text(fields, "label"),
                duration_seconds: field_integer(fields, "duration_seconds"),
            },
            metrics,
        ),
        DisplayTemplate::RowList => build_row_list_scene(
            &RowListCard {
                revision,
                title: field_text(fields, "title"),
                row0_title: field_text(fields, "row0_title"),
                row0_time: field_text(fields, "row0_time"),
                row1_title: field_text(fields, "row1_title"),
                row1_time: field_text(fields, "row1_time"),
                row2_title: field_text(fields, "row2_title"),
                row2_time: field_text(fields, "row2_time"),
                row3_title: field_text(fields, "row3_title"),
                row3_time: field_text(fields, "row3_time"),
                row4_title: field_text(fields, "row4_title"),
                row4_time: field_text(fields, "row4_time"),
            },
            metrics,
        ),
        DisplayTemplate::BigNumberLabel => build_big_number_label_scene(
            &BigNumberCard {
                revision,
                title: field_text(fields, "title"),
                value: field_text(fields, "value"),
                label: field_text(fields, "label"),
            },
            metrics,
        ),
        DisplayTemplate::IconBadgeText { .. } => build_icon_badge_text_scene(
            &IconBadgeCard {
                revision,
                title: field_text(fields, "title"),
                icon: field_text(fields, "icon"),
                badge: field_text(fields, "badge"),
                value: field_text(fields, "value"),
                label: field_text(fields, "label"),
            },
            SHIPPED_SCENE_SURFACE_COLOR,
            metrics,
        ),
    };
    let error = field_text(fields, "error");
    let scene = with_scene_data_state(
        scene,
        SceneDataState {
            stale: field_boolean(fields, "stale"),
            error: (!error.is_empty()).then_some(error),
        },
        metrics,
    );
    let push = PushScene {
        card_id: card_id.to_owned(),
        revision,
        scene,
    };
    validate_message(&Message::PushScene(push.clone()))
        .map_err(|error| format!("the host-built scene is invalid: {error}"))?;
    Ok(push)
}

fn record_scene_refusal(state: &mut WorkerState, card_id: String, message: String) {
    state.push_rejections.insert(
        card_id.clone(),
        CardError {
            kind: CardErrorKind::SceneRefused,
            card_id,
            message,
        },
    );
}

/// Classifies every device-layer outcome for an automatic scene. This match is
/// deliberately exhaustive: a future `DeviceError` variant must choose an explicit
/// retry and visibility policy rather than inheriting silent retry behavior.
fn handle_automatic_scene_error(
    state: &mut WorkerState,
    card_id: String,
    error: DeviceError,
    reconnect_interval: Duration,
) {
    match error {
        error if is_wrong_tier(&error) => mark_ownership_refused(state),
        DeviceError::Rejected(error) if error.code == protocol::ErrorCode::Busy => {
            // Firmware uses Busy for zero-timeout LVGL lock contention and OTA
            // takeover. The scene is still valid and no user edit can fix it;
            // retain the event so the next render phase tries it again.
            state.active_scene_dirty = true;
        }
        DeviceError::Rejected(error) => record_scene_refusal(
            state,
            card_id,
            format!(
                "the display refused this card's scene ({:?}): {}",
                error.code, error.diagnostic
            ),
        ),
        DeviceError::MissingCapabilities {
            required,
            available,
        } => record_scene_refusal(
            state,
            card_id,
            format!(
                "the display no longer advertises declarative scene rendering (required {required:#018x}, available {available:#018x}); reconnect to use its legacy widget renderer"
            ),
        ),
        DeviceError::Timeout
        | DeviceError::Transport(_)
        | DeviceError::MalformedResponse(_)
        | DeviceError::UnexpectedMessage => {
            // Timeouts, transport failures, and malformed/unexpected responses
            // describe the link, not the card. Keep the scene pending and let
            // ordinary status polling decide whether connection state changes.
            state.active_scene_dirty = true;
        }
        error @ DeviceError::NoDevice => {
            // Unlike a dropped response, NoDevice is already a definitive
            // connection observation. Enter the normal reconnect path now.
            mark_disconnected(state, &error, Instant::now(), reconnect_interval);
        }
        error @ (DeviceError::VersionMismatch(_)
        | DeviceError::InvalidRequest
        | DeviceError::RevisionExhausted) => {
            // These are terminal host/session faults, not defects in this card's
            // content and not transient link loss. Make them globally visible and
            // do not retry the identical request forever.
            state.runtime = RuntimeState::Error {
                message: format!("automatic scene delivery failed: {error}"),
            };
        }
    }
}

/// Rebuilds the active card only after a host-owned event marks it dirty.
///
/// The worker calls this in its render phase after publishing ownership state.
/// The dirty bit is consumed before any request and no clock, pomodoro, status, or
/// provider deadline sets it. Device-side bindings keep clock/timer facts moving
/// between these event-driven pushes.
fn push_active_scene(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
    reconnect_interval: Duration,
) {
    if ownership_was_refused(state) || state.needs_full_sync || !state.active_scene_dirty {
        return;
    }
    let Some(card_id) = state.active_screen.clone() else {
        state.active_scene_dirty = false;
        return;
    };
    if !state
        .device
        .capabilities
        .contains(&DeviceCapability::SceneRender)
    {
        state.active_scene_dirty = false;
        if state
            .push_rejections
            .get(&card_id)
            .is_some_and(|error| error.kind == CardErrorKind::SceneRefused)
        {
            state.push_rejections.remove(&card_id);
        }
        return;
    }

    let Some(revision) = state.next_scene_revision.checked_add(1) else {
        state.active_scene_dirty = false;
        record_scene_refusal(
            state,
            card_id,
            "this card cannot be rendered because the scene revision counter is exhausted".into(),
        );
        return;
    };
    state.next_scene_revision = revision;
    let fields = state
        .latest_fields
        .get(&card_id)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let push = match build_card_scene(&state.config, &card_id, fields, revision) {
        Ok(push) => push,
        Err(message) => {
            state.active_scene_dirty = false;
            record_scene_refusal(state, card_id, message);
            return;
        }
    };

    // One best-effort attempt per event. This runs outside ownership sync after
    // the Online snapshot has been published. Any failure belongs to the card's
    // render state; status/ownership traffic independently decides whether the
    // connection itself is still usable.
    state.active_scene_dirty = false;
    match device.push_scene(push) {
        Ok(()) => {
            if state
                .push_rejections
                .get(&card_id)
                .is_some_and(|error| error.kind == CardErrorKind::SceneRefused)
            {
                state.push_rejections.remove(&card_id);
            }
        }
        Err(error) => {
            handle_automatic_scene_error(state, card_id, error, reconnect_interval);
        }
    }
}

fn send_screen(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
) -> Result<(), RuntimeError> {
    if ownership_was_refused(state) {
        return Ok(());
    }
    if !state.active_screen_dirty {
        return Ok(());
    }
    if let Some(screen_id) = state.active_screen.clone()
        && !sync_device_result(state, device.activate_screen(screen_id))?
    {
        return Ok(());
    }
    state.active_screen_dirty = false;
    Ok(())
}

fn flush_interrupts(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    device: &mut dyn RuntimeDevice,
    now: Instant,
) -> Result<(), RuntimeError> {
    if ownership_was_refused(state) {
        return Ok(());
    }
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
            Ok(()) => {
                state
                    .interrupts
                    .acknowledge(interrupt.token)
                    .map_err(|error| RuntimeError::Device {
                        message: error.to_string(),
                    })?;
                arm_alert_hold_if_active(
                    &state.interrupts,
                    scheduler,
                    interrupt.token,
                    card_alert(&state.config, &interrupt.widget_id).hold(),
                    now,
                );
            }
            Err(error) if is_wrong_tier(&error) => {
                mark_ownership_refused(state);
                return Ok(());
            }
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

fn send_time_sync(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
) -> Result<bool, RuntimeError> {
    if ownership_was_refused(state) {
        return Ok(false);
    }
    let now = Utc::now();
    let offset_minutes = crate::config::utc_offset_minutes(&state.config.preferences.timezone, now)
        .map_err(|message| RuntimeError::Device { message })?;
    sync_device_result(
        state,
        device.time_sync(TimeSync {
            unix_seconds: now.timestamp(),
            utc_offset_minutes: offset_minutes,
        }),
    )
}

fn ownership_was_refused(state: &WorkerState) -> bool {
    state.ownership_refused
}

fn is_wrong_tier(error: &DeviceError) -> bool {
    matches!(
        error,
        DeviceError::Rejected(response) if response.code == protocol::ErrorCode::WrongTier
    )
}

/// A `WrongTier` refusal is a fresh ownership observation, not a failed sync. Keep all
/// pending host state for a later return to Local and stop issuing USB mutations now.
fn sync_device_result(
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

fn mark_ownership_refused(state: &mut WorkerState) {
    state.ownership_refused = true;
    state.device.tier = Some(DeviceTier::Networked);
    state.needs_full_sync = true;
    state.runtime = if state.config.preferences.paused {
        RuntimeState::Paused
    } else {
        RuntimeState::Running
    };
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

fn is_disconnect(error: &DeviceError) -> bool {
    matches!(error, DeviceError::NoDevice | DeviceError::Transport(_))
}

fn device_runtime_error(error: &DeviceError) -> RuntimeError {
    RuntimeError::Device {
        message: error.to_string(),
    }
}

fn runtime_command_device_error(
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

fn run_runtime_device_command(
    state: &mut WorkerState,
    reconnect_interval: Duration,
    operation: impl FnOnce() -> Result<(), DeviceError>,
) -> Result<(), RuntimeError> {
    if !state.connected {
        return Err(RuntimeError::DeviceDisconnected);
    }
    operation().map_err(|error| runtime_command_device_error(state, &error, reconnect_interval))
}

fn reply_to_runtime_device_command(
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

fn update_device_status(
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
    let compiled_card_ids: BTreeSet<&str> = config.compiled_card_ids().into_iter().collect();
    for card in config
        .cards
        .iter()
        .filter(|card| compiled_card_ids.contains(card.id()))
    {
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
                .active_playlist()
                .and_then(|playlist| playlist.entries.first())
                .map(|entry| entry.card_id.clone()),
            ..empty_device(ConnectionState::Connecting)
        },
        providers,
        pomodoros,
        card_data: Vec::new(),
        card_errors: Vec::new(),
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
        tier: None,
        wifi_state: None,
        wifi_rssi: None,
        ip: None,
        last_network_error: None,
        ota_state: None,
        active_screen_id: None,
        counters: DeviceCounters::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AlertHold, CardAlert, CarouselAdvance, DisplayTemplate, Playlist, PlaylistEntry,
        RefreshPolicy, WidgetTapAction,
    };

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

    #[test]
    fn wrong_tier_sync_result_latches_non_ownership_without_an_error() {
        let now = Instant::now();
        let mut scheduler = Scheduler::new(
            now,
            Duration::from_hours(1),
            Duration::from_hours(1),
            Duration::from_hours(1),
        );
        let mut state = WorkerState::new(AppConfig::default(), now, &mut scheduler);
        state.device.tier = Some(DeviceTier::Local);
        state.needs_full_sync = false;
        state.runtime = RuntimeState::Error {
            message: "old sync failure".into(),
        };

        let applied = sync_device_result(
            &mut state,
            Err(DeviceError::Rejected(protocol::ErrorResponse {
                code: protocol::ErrorCode::WrongTier,
                diagnostic: "server owns this device".into(),
            })),
        )
        .unwrap();

        assert!(!applied);
        assert_eq!(state.device.tier, Some(DeviceTier::Networked));
        assert!(state.ownership_refused);
        assert!(
            state.needs_full_sync,
            "local recovery still needs a full replay"
        );
        assert_eq!(state.runtime, RuntimeState::Running);
    }

    // -- Rotation (Task 5) ---------------------------------------------------
    //
    // These are the pure/instant counterparts of the single real-time rotation test
    // in `tests/runtime.rs`. They call `current_dwell`, `rotation_card_ids`,
    // `advance_rotation`, `drain_device_events`, and `process_command` directly, so
    // they run in well under a millisecond instead of sleeping out real dwells.

    fn rotation_clock_card(id: &str) -> CardSettings {
        CardSettings::Clock {
            id: id.into(),
            title: id.into(),
            show_seconds: true,
            template: DisplayTemplate::DigitalClock,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
        }
    }

    fn rotation_pomodoro_card(id: &str, alert: CardAlert) -> CardSettings {
        CardSettings::Pomodoro {
            id: id.into(),
            label: id.into(),
            duration_seconds: 60,
            template: DisplayTemplate::ProgressRing,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::DeviceLocal,
            alert,
        }
    }

    fn alert_calendar_card(id: &str, alert: CardAlert) -> CardSettings {
        CardSettings::Calendar {
            id: id.into(),
            title: id.into(),
            source: CalendarSource::File("unused.ics".into()),
            template: DisplayTemplate::RowList,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::Manual,
            alert,
        }
    }

    fn rotation_config(
        cards: Vec<CardSettings>,
        advance: CarouselAdvance,
        entries: &[(&str, Option<u16>)],
    ) -> AppConfig {
        AppConfig {
            cards,
            playlists: vec![Playlist {
                id: "p1".into(),
                name: "P1".into(),
                advance,
                entries: entries
                    .iter()
                    .map(|(card_id, dwell_seconds)| PlaylistEntry {
                        card_id: (*card_id).into(),
                        dwell_seconds: *dwell_seconds,
                    })
                    .collect(),
            }],
            active_playlist_id: "p1".into(),
            ..AppConfig::default()
        }
    }

    /// Two in-rotation cards, 5s dwell each, `Timed` advance with a 45s default that
    /// neither card should ever need (both set an explicit dwell).
    fn timed_two_card_config() -> AppConfig {
        rotation_config(
            vec![rotation_clock_card("a"), rotation_clock_card("b")],
            CarouselAdvance::Timed {
                default_dwell_seconds: 45,
            },
            &[("a", Some(5)), ("b", Some(5))],
        )
    }

    /// Two in-rotation cards with an alert-only card sandwiched between them at the
    /// card-list position that would sit at rotation index 2 if index resolution ever
    /// (incorrectly) walked *all* cards instead of only in-rotation ones. `rotation_card_ids`
    /// is `["a", "c"]`; card "c" sits at list position 2 but rotation index 1.
    fn timed_config_with_alert_only_between_two_in_rotation_cards() -> AppConfig {
        rotation_config(
            vec![
                rotation_clock_card("a"),
                rotation_pomodoro_card(
                    "b",
                    CardAlert::OnTimerFinish {
                        hold: AlertHold::UntilDismissed,
                    },
                ),
                rotation_clock_card("c"),
            ],
            CarouselAdvance::Timed {
                default_dwell_seconds: 45,
            },
            &[("a", Some(5)), ("c", Some(5))],
        )
    }

    /// A `RuntimeDevice` stub for unit tests that never connect: every call other than
    /// Most device methods are unreachable in these tests because they keep
    /// `state.connected == false`. `trigger_interrupt` accepts direct
    /// `flush_interrupts` calls used to pin delivery-time alert-hold behavior,
    /// and `try_recv_event` optionally yields one queued event before returning
    /// `None` forever after.
    #[derive(Default)]
    struct StubDevice {
        queued_event: Option<ReceivedEvent>,
    }

    impl RuntimeDevice for StubDevice {
        fn connect(&mut self) -> Result<DeviceConnection, DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn status(&mut self) -> Result<StatusResponse, DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn provision(&mut self, _config: &NetworkConfig) -> Result<(), DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn factory_reset(&mut self) -> Result<(), DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn time_sync(&mut self, _sync: TimeSync) -> Result<(), DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn apply_layout(
            &mut self,
            _rotation: u16,
            _widgets: Vec<WidgetConfig>,
            _screens: Vec<ScreenConfig>,
        ) -> Result<(), DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn push_fields(
            &mut self,
            _widget_id: String,
            _fields: Vec<Field>,
        ) -> Result<(), DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn activate_screen(&mut self, _screen_id: String) -> Result<(), DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn push_scene(&mut self, _push: PushScene) -> Result<(), DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn trigger_interrupt(&mut self, _interrupt: TriggerInterrupt) -> Result<(), DeviceError> {
            Ok(())
        }
        fn send_asset_begin(&mut self, _begin: AssetBegin) -> Result<Ack, DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn send_asset_chunk(&mut self, _chunk: AssetChunk) -> Result<(), DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn send_asset_commit(&mut self, _commit: AssetCommit) -> Result<(), DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn send_asset_release(&mut self, _release: AssetRelease) -> Result<(), DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn try_recv_event(&mut self) -> Option<ReceivedEvent> {
            self.queued_event.take()
        }
        fn diagnostics(&self) -> SessionDiagnostics {
            SessionDiagnostics::default()
        }
    }

    #[derive(Default)]
    struct ScheduledWorkDevice {
        status_calls: usize,
        time_sync_calls: usize,
    }

    impl RuntimeDevice for ScheduledWorkDevice {
        fn connect(&mut self) -> Result<DeviceConnection, DeviceError> {
            Ok(DeviceConnection {
                port_name: "scheduled-work".into(),
                status: scheduled_work_status(),
            })
        }

        fn status(&mut self) -> Result<StatusResponse, DeviceError> {
            self.status_calls += 1;
            Ok(scheduled_work_status())
        }

        fn provision(&mut self, _config: &NetworkConfig) -> Result<(), DeviceError> {
            Ok(())
        }

        fn factory_reset(&mut self) -> Result<(), DeviceError> {
            Ok(())
        }

        fn time_sync(&mut self, _sync: TimeSync) -> Result<(), DeviceError> {
            self.time_sync_calls += 1;
            Ok(())
        }

        fn apply_layout(
            &mut self,
            _rotation: u16,
            _widgets: Vec<WidgetConfig>,
            _screens: Vec<ScreenConfig>,
        ) -> Result<(), DeviceError> {
            Ok(())
        }

        fn push_fields(
            &mut self,
            _widget_id: String,
            _fields: Vec<Field>,
        ) -> Result<(), DeviceError> {
            Ok(())
        }

        fn activate_screen(&mut self, _screen_id: String) -> Result<(), DeviceError> {
            Ok(())
        }

        fn push_scene(&mut self, _push: PushScene) -> Result<(), DeviceError> {
            Ok(())
        }

        fn trigger_interrupt(&mut self, _interrupt: TriggerInterrupt) -> Result<(), DeviceError> {
            Ok(())
        }

        fn send_asset_begin(&mut self, _begin: AssetBegin) -> Result<Ack, DeviceError> {
            Ok(Ack {
                acknowledged_type: protocol::TYPE_ASSET_BEGIN,
                revision: None,
                already_present: Some(false),
            })
        }

        fn send_asset_chunk(&mut self, _chunk: AssetChunk) -> Result<(), DeviceError> {
            Ok(())
        }

        fn send_asset_commit(&mut self, _commit: AssetCommit) -> Result<(), DeviceError> {
            Ok(())
        }

        fn send_asset_release(&mut self, _release: AssetRelease) -> Result<(), DeviceError> {
            Ok(())
        }

        fn try_recv_event(&mut self) -> Option<ReceivedEvent> {
            None
        }

        fn diagnostics(&self) -> SessionDiagnostics {
            SessionDiagnostics::default()
        }
    }

    fn scheduled_work_status() -> StatusResponse {
        StatusResponse {
            protocol_version: protocol::PROTOCOL_VERSION,
            max_protocol_version: protocol::MAX_PROTOCOL_VERSION,
            capabilities: protocol::CURRENT_CAPABILITIES,
            firmware_version: "scheduled-work-test".into(),
            uptime_ms: 1,
            free_heap: 100_000,
            display_width: 368,
            display_height: 448,
            brightness: 200,
            rotation: 90,
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
            tier: protocol::Tier::Local,
            wifi_state: protocol::WifiState::Down,
            wifi_rssi: 0,
            ip: String::new(),
            ota_state: protocol::OtaState::Idle,
            last_network_error: None,
            last_ota_error: None,
        }
    }

    fn scheduled_work_provider() -> ProviderWorker {
        ProviderWorker::new(
            Box::new(ImmediateRefresher {
                completed: mpsc::channel().0,
            }),
            1,
        )
    }

    #[test]
    fn disconnected_scheduled_work_leaves_a_real_wait() {
        let now = Instant::now();
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(AppConfig::default(), now, &mut scheduler);
        let mut device = ScheduledWorkDevice::default();
        let provider = scheduled_work_provider();

        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
            &provider,
            &RuntimeDiagnosticCounters::default(),
            now,
            &RuntimeOptions::default(),
        );

        assert!(
            scheduler.wait_duration(now, Duration::from_hours(1)) > Duration::ZERO,
            "a disconnected worker must block instead of spinning on a skipped deadline"
        );
    }

    #[test]
    fn paused_scheduled_work_leaves_a_real_wait() {
        let now = Instant::now();
        let mut config = rotation_config(
            vec![alert_calendar_card("calendar", CardAlert::None)],
            CarouselAdvance::Manual,
            &[("calendar", None)],
        );
        config.preferences.paused = true;
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);
        state.connected = true;
        let mut device = ScheduledWorkDevice::default();
        let provider = scheduled_work_provider();

        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
            &provider,
            &RuntimeDiagnosticCounters::default(),
            now,
            &RuntimeOptions::default(),
        );

        assert!(
            scheduler.wait_duration(now, Duration::from_hours(1)) > Duration::ZERO,
            "a paused worker must block instead of spinning on skipped sync or provider deadlines"
        );
    }

    #[test]
    fn reconnect_runs_status_and_time_sync_promptly_after_skipped_work() {
        let disconnected_at = Instant::now();
        let mut scheduler = fresh_scheduler(disconnected_at);
        let mut state = WorkerState::new(AppConfig::default(), disconnected_at, &mut scheduler);
        state.needs_full_sync = false;
        let mut device = ScheduledWorkDevice::default();
        let provider = scheduled_work_provider();
        let options = RuntimeOptions::default();

        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
            &provider,
            &RuntimeDiagnosticCounters::default(),
            disconnected_at,
            &options,
        );

        let reconnected_at = disconnected_at + Duration::from_secs(1);
        attempt_connect(
            &mut state,
            &mut scheduler,
            &mut device,
            reconnected_at,
            &options,
        );
        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
            &provider,
            &RuntimeDiagnosticCounters::default(),
            reconnected_at,
            &options,
        );

        assert_eq!(device.status_calls, 1, "status must be prompt on reconnect");
        assert_eq!(
            device.time_sync_calls, 1,
            "time sync must be prompt on reconnect"
        );
    }

    fn navigation_event(screen_id: &str) -> ReceivedEvent {
        ReceivedEvent {
            event: protocol::DeviceEvent {
                sequence: 1,
                kind: EventKind::Navigation,
                widget_id: screen_id.into(),
                screen_id: screen_id.into(),
                action: EventAction::NavigateNext,
                interrupt_token: None,
            },
            missed_before: 0,
        }
    }

    fn interrupt_dismissed_event(widget_id: &str, token: u32) -> ReceivedEvent {
        ReceivedEvent {
            event: protocol::DeviceEvent {
                sequence: 1,
                kind: EventKind::InterruptDismissed,
                widget_id: widget_id.into(),
                screen_id: widget_id.into(),
                action: EventAction::DismissInterrupt,
                interrupt_token: Some(token),
            },
            missed_before: 0,
        }
    }

    // F1: under `CarouselAdvance::Manual`, no rotation deadline is ever armed. This
    // isolates the exact regression the reviewer flagged: if `current_dwell` stopped
    // propagating `CarouselAdvance::default_dwell_seconds()`'s `None` (e.g. a `?` was
    // dropped and a dwell hardcoded instead), this fails immediately.
    #[test]
    fn current_dwell_is_none_under_manual_advance() {
        let config = AppConfig::default();
        assert!(current_dwell(&config, 0).is_none());
        // Manual disarms regardless of which in-rotation card index is asked about.
        let two_card = timed_two_card_config();
        let mut manual_two_card = two_card;
        manual_two_card.playlists[0].advance = CarouselAdvance::Manual;
        assert!(current_dwell(&manual_two_card, 0).is_none());
        assert!(current_dwell(&manual_two_card, 1).is_none());
    }

    // F2: dwell is resolved per-card against the carousel default, not the other way
    // around. Uses the brief's own example: an explicit dwell wins over the default,
    // and an absent one falls back to it. If `current_dwell` ever ignored the card's own
    // `dwell_seconds` (always returning the carousel default) or ignored the carousel
    // default (returning `None` when the card leaves it unset), this fails.
    #[test]
    fn current_dwell_resolves_each_cards_own_value_before_falling_back_to_the_default() {
        let config = rotation_config(
            vec![
                rotation_clock_card("explicit"),
                rotation_clock_card("defaulted"),
            ],
            CarouselAdvance::Timed {
                default_dwell_seconds: 45,
            },
            &[("explicit", Some(10)), ("defaulted", None)],
        );
        assert_eq!(current_dwell(&config, 0), Some(Duration::from_secs(10)));
        assert_eq!(current_dwell(&config, 1), Some(Duration::from_secs(45)));
    }

    // F2 (continued): `advance_rotation` walks `rotation_card_ids` in order, wraps at
    // the end, and skips alert-only/off cards. Also exercises re-arming with each
    // card's own dwell as the index moves.
    #[test]
    fn advance_rotation_walks_in_rotation_cards_in_order_and_wraps() {
        let now = Instant::now();
        let mut config = timed_two_card_config();
        config.cards.push(rotation_pomodoro_card(
            "alert-only",
            CardAlert::OnTimerFinish {
                hold: AlertHold::UntilDismissed,
            },
        ));
        config.cards.push(rotation_clock_card("off"));
        let mut scheduler = Scheduler::new(
            now,
            Duration::from_hours(1),
            Duration::from_hours(1),
            Duration::from_hours(1),
        );
        let mut state = WorkerState::new(config, now, &mut scheduler);
        assert_eq!(state.active_rotation_index, 0);
        assert_eq!(state.active_screen.as_deref(), Some("a"));

        advance_rotation(&mut state, &mut scheduler, now);
        assert_eq!(state.active_rotation_index, 1);
        assert_eq!(state.active_screen.as_deref(), Some("b"));
        assert!(state.active_screen_dirty);

        state.active_screen_dirty = false;
        advance_rotation(&mut state, &mut scheduler, now);
        assert_eq!(
            state.active_rotation_index, 0,
            "wraps back to the first card"
        );
        assert_eq!(state.active_screen.as_deref(), Some("a"));
        assert!(state.active_screen_dirty);
    }

    // F2 (continued): with fewer than two in-rotation cards there is nothing to
    // rotate to, so `advance_rotation` disarms the deadline instead of activating
    // anything.
    #[test]
    fn advance_rotation_disarms_with_fewer_than_two_in_rotation_cards() {
        let now = Instant::now();
        let mut config = AppConfig::default();
        config.playlists[0].advance = CarouselAdvance::Timed {
            default_dwell_seconds: 5,
        };
        let mut scheduler = Scheduler::new(
            now,
            Duration::from_hours(1),
            Duration::from_hours(1),
            Duration::from_hours(1),
        );
        let mut state = WorkerState::new(config, now, &mut scheduler);
        state.active_screen_dirty = false;
        advance_rotation(&mut state, &mut scheduler, now);
        assert!(!state.active_screen_dirty, "nothing to rotate to");
        assert!(!scheduler.rotation_due(now + Duration::from_hours(1)));
    }

    // F3a: a local swipe resolves the reported screen's index within
    // `rotation_card_ids` (the in-rotation-only order), not its position among all
    // cards, and re-arms the dwell from the card it landed on. The alert-only card "b"
    // sits between "a" and "c" in the card list, so "c" is at list position 2 but
    // rotation index 1 — the exact case that distinguishes correct index resolution
    // from a regression that walks `config.cards` directly.
    #[test]
    fn local_swipe_resolves_the_index_within_rotation_ids_and_rearms_the_dwell() {
        let now = Instant::now();
        let config = timed_config_with_alert_only_between_two_in_rotation_cards();
        let mut scheduler = Scheduler::new(
            now,
            Duration::from_hours(1),
            Duration::from_hours(1),
            Duration::from_hours(1),
        );
        let mut state = WorkerState::new(config, now, &mut scheduler);
        let mut device = StubDevice {
            queued_event: Some(navigation_event("c")),
        };

        drain_device_events(
            &mut state,
            &mut scheduler,
            &mut device,
            &RuntimeDiagnosticCounters::default(),
            now,
        );

        assert_eq!(
            state.active_rotation_index, 1,
            "\"c\" is rotation index 1 (within [\"a\", \"c\"]), not list position 2"
        );
        assert_eq!(state.active_screen.as_deref(), Some("c"));
        // Re-armed from "c"'s own 5s dwell, not left over from "a".
        assert!(!scheduler.rotation_due(now + Duration::from_secs(4)));
        assert!(scheduler.rotation_due(now + Duration::from_secs(5)));
    }

    // F3b: the same index-resolution and re-arm behavior applies to an explicit
    // `RuntimeCommand::ActivateScreen` (the IPC-driven manual activation), which is a
    // manual override just like a swipe.
    #[test]
    fn explicit_activate_command_resolves_the_index_within_rotation_ids_and_rearms_the_dwell() {
        let now = Instant::now();
        let config = timed_config_with_alert_only_between_two_in_rotation_cards();
        let mut scheduler = Scheduler::new(
            now,
            Duration::from_hours(1),
            Duration::from_hours(1),
            Duration::from_hours(1),
        );
        let mut state = WorkerState::new(config, now, &mut scheduler);
        let mut device = StubDevice::default();
        let diagnostics = RuntimeDiagnosticCounters::default();
        let (reply_sender, reply_receiver) = mpsc::sync_channel(1);
        let before = Instant::now();

        let shutting_down = process_command(
            RuntimeCommand::ActivateScreen {
                screen_id: "c".into(),
                reply: reply_sender,
            },
            &mut state,
            &mut scheduler,
            &mut device,
            &diagnostics,
            Duration::from_secs(1),
        );

        assert!(!shutting_down);
        reply_receiver.recv().unwrap().unwrap();
        assert_eq!(
            state.active_rotation_index, 1,
            "\"c\" is rotation index 1 (within [\"a\", \"c\"]), not list position 2"
        );
        assert_eq!(state.active_screen.as_deref(), Some("c"));
        // `process_command`'s `ActivateScreen` arm uses `Instant::now()` internally
        // (there is no injectable clock in this runtime), so assert with a margin
        // around the 5s dwell rather than pinning it to `before` exactly.
        assert!(!scheduler.rotation_due(before + Duration::from_millis(4_500)));
        assert!(scheduler.rotation_due(before + Duration::from_millis(5_500)));
    }

    // F5: pins the deliberate decision (see the comment on the `rotation_due` check in
    // `run_scheduled_work`) that rotation advances locally while paused, mirroring how
    // pomodoro ticks are never gated by pause. This is not new behavior introduced by
    // Task 5's rotation feature; it documents and locks in the existing pattern so a
    // future refactor cannot silently move rotation's guard without a test noticing.
    #[test]
    fn rotation_advances_locally_while_paused_mirroring_pomodoro_ticks() {
        let now = Instant::now();
        let mut config = timed_two_card_config();
        config.preferences.paused = true;
        let mut scheduler = Scheduler::new(
            now,
            Duration::from_hours(1),
            Duration::from_hours(1),
            Duration::from_hours(1),
        );
        let mut state = WorkerState::new(config, now, &mut scheduler);
        assert!(state.config.preferences.paused);
        assert!(!state.connected);

        let mut device = StubDevice::default();
        let diagnostics = RuntimeDiagnosticCounters::default();
        let provider = ProviderWorker::new(
            Box::new(ImmediateRefresher {
                completed: mpsc::channel().0,
            }),
            1,
        );
        let options = RuntimeOptions::default();

        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
            &provider,
            &diagnostics,
            now + Duration::from_secs(5),
            &options,
        );

        assert_eq!(
            state.active_rotation_index, 1,
            "rotation advances locally even while paused and disconnected"
        );
        assert!(
            state.active_screen_dirty,
            "queued for the device, not yet sent"
        );
    }

    // -- Alert triggers and hold (Task 6) ------------------------------------
    //
    // Instant unit tests for the trigger-kind narrowing, the bounded hold
    // deadline (including the token-keyed active/pending scoping fixed by the
    // Task 6 code review's Critical-1), and the calendar `BeforeEvent`
    // trigger's once-per-event-start dedup and tick-driven re-evaluation
    // (Important-3/4). Each calls the private helpers/functions directly with
    // synthetic `Instant`s, so none of them sleep. The one wall-clock proof
    // that the hold deadline reaches a real device via the full `run_runtime`
    // loop lives in `tests/runtime.rs`.

    fn fresh_scheduler(now: Instant) -> Scheduler {
        Scheduler::new(
            now,
            Duration::from_hours(1),
            Duration::from_hours(1),
            Duration::from_hours(1),
        )
    }

    // Task 2 gated a pomodoro's completion interrupt on "any configured alert".
    // Task 6 narrows that to `CardAlert::OnTimerFinish` specifically. This is the
    // positive half of that narrowing (the negative half, `alert: none`, is
    // already pinned by `pomodoro_completion_without_an_alert_does_not_schedule_an_interrupt`
    // in `tests/runtime.rs`): with two pomodoros completing at the same instant,
    // only the `OnTimerFinish` card's completion reaches the interrupt arbiter.
    //
    // NOTE (Task 6 review Critical-2): this test alone does *not* distinguish
    // the narrowing from the old, wider Task 2 gate (`!alert.is_none()`) —
    // `alert: none` vs. `OnTimerFinish` is handled identically by both. See
    // `card_wants_completion_interrupt_rejects_before_event_even_on_a_pomodoro_card`
    // below for the input that actually pins the narrowing.
    #[test]
    fn only_on_timer_finish_alerts_fire_when_a_pomodoro_completes() {
        let now = Instant::now();
        let config = rotation_config(
            vec![
                rotation_pomodoro_card("quiet", CardAlert::None),
                rotation_pomodoro_card(
                    "loud",
                    CardAlert::OnTimerFinish {
                        hold: AlertHold::UntilDismissed,
                    },
                ),
            ],
            CarouselAdvance::Manual,
            &[("quiet", None), ("loud", None)],
        );
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);
        state.pomodoros.get_mut("quiet").unwrap().start(now);
        state.pomodoros.get_mut("loud").unwrap().start(now);

        update_pomodoros(&mut state, now + Duration::from_mins(1));

        assert_eq!(
            state
                .interrupts
                .active()
                .map(|tracked| tracked.message.widget_id.as_str()),
            Some("loud"),
            "only the OnTimerFinish card's completion schedules an interrupt"
        );
        assert!(state.interrupts.pending().is_none());
    }

    // Task 6 review Critical-2: config validation forbids `BeforeEvent` on a
    // pomodoro card, so this exact config could never arrive through
    // `AppConfig::compile`/`apply_config` — it is constructed directly here,
    // bypassing validation, as defence-in-depth. It is the *only* input that
    // actually distinguishes `card_wants_completion_interrupt`'s Task 6 gate
    // (`matches!(_, CardAlert::OnTimerFinish { .. })`) from Task 2's original,
    // wider gate (`!alert.is_none()`), which treated `BeforeEvent` as "wants a
    // completion interrupt" exactly like `OnTimerFinish`. Verified by hand
    // that reverting the gate to `!card_alert(config, widget_id).is_none()`
    // makes this assertion fail (all other app-core tests still pass under
    // that reversion, which is precisely why this dedicated test exists).
    #[test]
    fn card_wants_completion_interrupt_rejects_before_event_even_on_a_pomodoro_card() {
        let config = AppConfig {
            cards: vec![CardSettings::Pomodoro {
                id: "p".into(),
                label: "p".into(),
                duration_seconds: 60,
                template: DisplayTemplate::ProgressRing,
                tap_action: WidgetTapAction::None,
                refresh: RefreshPolicy::DeviceLocal,
                alert: CardAlert::BeforeEvent {
                    lead_minutes: 5,
                    hold: AlertHold::UntilDismissed,
                },
            }],
            ..AppConfig::default()
        };
        assert!(!card_wants_completion_interrupt(&config, "p"));
    }

    // `AlertHold::Seconds` arms an absolute deadline on the scheduler when the
    // interrupt is delivered; once `run_scheduled_work` observes that deadline
    // has passed, it dismisses the active interrupt through the existing
    // dismissal path (`InterruptArbiter::dismiss`) and marks the screen dirty so
    // the device resyncs to the saved carousel screen.
    #[test]
    fn a_bounded_alert_hold_auto_dismisses_the_active_interrupt_when_it_expires() {
        let now = Instant::now();
        let config = rotation_config(
            vec![rotation_pomodoro_card(
                "loud",
                CardAlert::OnTimerFinish {
                    hold: AlertHold::Seconds { value: 30 },
                },
            )],
            CarouselAdvance::Manual,
            &[("loud", None)],
        );
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);
        state.pomodoros.get_mut("loud").unwrap().start(now);
        let delivery_time = now + Duration::from_mins(1);
        update_pomodoros(&mut state, delivery_time);
        assert!(
            state.interrupts.active().is_some(),
            "completion scheduled the interrupt"
        );

        let mut device = StubDevice::default();
        flush_interrupts(&mut state, &mut scheduler, &mut device, delivery_time).unwrap();
        let diagnostics = RuntimeDiagnosticCounters::default();
        let provider = ProviderWorker::new(
            Box::new(ImmediateRefresher {
                completed: mpsc::channel().0,
            }),
            1,
        );
        let options = RuntimeOptions::default();
        state.active_screen_dirty = false;

        // Hold armed at delivery time (60s) + 30s = 90s from `now`. One second
        // before that, the interrupt survives.
        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
            &provider,
            &diagnostics,
            now + Duration::from_secs(89),
            &options,
        );
        assert!(
            state.interrupts.active().is_some(),
            "hold has not expired yet"
        );

        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
            &provider,
            &diagnostics,
            now + Duration::from_secs(90),
            &options,
        );
        assert!(
            state.interrupts.active().is_none(),
            "hold expired and auto-dismissed the interrupt"
        );
        assert!(
            state.active_screen_dirty,
            "queued the saved carousel screen for resync"
        );
    }

    // The counterpart to the bounded-hold test above: `AlertHold::UntilDismissed`
    // arms no deadline at all, so `alert_hold_due` never fires and the interrupt
    // only ever leaves via an explicit device dismissal.
    #[test]
    fn an_until_dismissed_alert_hold_never_auto_dismisses() {
        let now = Instant::now();
        let config = rotation_config(
            vec![rotation_pomodoro_card(
                "loud",
                CardAlert::OnTimerFinish {
                    hold: AlertHold::UntilDismissed,
                },
            )],
            CarouselAdvance::Manual,
            &[("loud", None)],
        );
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);
        state.pomodoros.get_mut("loud").unwrap().start(now);
        let delivery_time = now + Duration::from_mins(1);
        update_pomodoros(&mut state, delivery_time);
        assert!(state.interrupts.active().is_some());

        let mut device = StubDevice::default();
        flush_interrupts(&mut state, &mut scheduler, &mut device, delivery_time).unwrap();
        let diagnostics = RuntimeDiagnosticCounters::default();
        let provider = ProviderWorker::new(
            Box::new(ImmediateRefresher {
                completed: mpsc::channel().0,
            }),
            1,
        );
        let options = RuntimeOptions::default();

        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
            &provider,
            &diagnostics,
            now + Duration::from_hours(1),
            &options,
        );
        assert!(
            state.interrupts.active().is_some(),
            "AlertHold::UntilDismissed never auto-dismisses"
        );
    }

    // Shared setup for the two Critical-1 tests below: a sticky
    // (`UntilDismissed`) pomodoro alert Active, and a bounded (60s) calendar
    // alert queued Pending behind it (the arbiter's Active slot is occupied).
    fn sticky_active_and_bounded_pending_alert(now: Instant) -> (WorkerState, Scheduler) {
        let config = rotation_config(
            vec![
                rotation_pomodoro_card(
                    "sticky",
                    CardAlert::OnTimerFinish {
                        hold: AlertHold::UntilDismissed,
                    },
                ),
                alert_calendar_card(
                    "upnext",
                    CardAlert::BeforeEvent {
                        lead_minutes: 5,
                        hold: AlertHold::Seconds { value: 60 },
                    },
                ),
            ],
            CarouselAdvance::Manual,
            &[("sticky", None), ("upnext", None)],
        );
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);

        // The sticky pomodoro completes first and becomes Active with no
        // deadline (UntilDismissed).
        state.pomodoros.get_mut("sticky").unwrap().start(now);
        let after_completion = now + Duration::from_mins(1);
        update_pomodoros(&mut state, after_completion);
        assert_eq!(
            state
                .interrupts
                .active()
                .map(|tracked| tracked.message.widget_id.as_str()),
            Some("sticky")
        );

        // The calendar alert fires next; the Active slot is occupied, so it
        // queues Pending.
        let now_unix_ms = Utc::now().timestamp_millis();
        state.latest_fields.insert(
            "upnext".into(),
            vec![Field {
                key: "next_start_unix_ms".into(),
                value: FieldValue::Integer(now_unix_ms + 4 * 60_000),
            }],
        );
        refresh_calendar_alert(
            &mut state,
            &mut scheduler,
            "upnext",
            5,
            after_completion,
            now_unix_ms,
        );
        assert_eq!(
            state
                .interrupts
                .pending()
                .map(|tracked| tracked.message.widget_id.as_str()),
            Some("upnext"),
            "the Active slot is occupied, so the calendar alert queues Pending"
        );

        (state, scheduler)
    }

    fn stub_work_harness() -> (
        StubDevice,
        RuntimeDiagnosticCounters,
        ProviderWorker,
        RuntimeOptions,
    ) {
        (
            StubDevice::default(),
            RuntimeDiagnosticCounters::default(),
            ProviderWorker::new(
                Box::new(ImmediateRefresher {
                    completed: mpsc::channel().0,
                }),
                1,
            ),
            RuntimeOptions::default(),
        )
    }

    // Task 6 review Critical-1's reproduction: before the fix, a single
    // unkeyed scheduler deadline was armed unconditionally by whichever
    // interrupt was scheduled *last* — here, the Pending calendar alert's
    // 60s — so `run_scheduled_work` auto-dismissed the sticky pomodoro's
    // *Active* interrupt 60s later, in direct violation of "UntilDismissed
    // never auto-dismisses". This proves the fix: an hour later, both the
    // sticky Active interrupt and the calendar's Pending one are unharmed —
    // the hold only ever runs against whichever interrupt is actually
    // Active.
    #[test]
    fn a_pending_alerts_hold_does_not_cross_talk_with_the_active_interrupt() {
        let now = Instant::now();
        let (mut state, mut scheduler) = sticky_active_and_bounded_pending_alert(now);
        let (mut device, diagnostics, provider, options) = stub_work_harness();

        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
            &provider,
            &diagnostics,
            now + Duration::from_hours(1),
            &options,
        );
        assert_eq!(
            state
                .interrupts
                .active()
                .map(|tracked| tracked.message.widget_id.as_str()),
            Some("sticky"),
            "UntilDismissed must not be auto-dismissed by an unrelated Pending interrupt's hold"
        );
        assert_eq!(
            state
                .interrupts
                .pending()
                .map(|tracked| tracked.message.widget_id.as_str()),
            Some("upnext"),
            "the calendar interrupt is still queued, unharmed"
        );
    }

    // The other half of Critical-1's fix: once the sticky interrupt is
    // dismissed (an on-device tap) and the calendar alert is promoted to
    // Active, its own 60s hold must start fresh from that moment — before
    // the fix, the Pending alert never got a deadline of its own at all, so
    // this promoted interrupt would have run `UntilDismissed`-like forever.
    #[test]
    fn a_promoted_alert_gets_a_fresh_hold_from_its_own_card() {
        let now = Instant::now();
        let (mut state, mut scheduler) = sticky_active_and_bounded_pending_alert(now);
        let (mut device, diagnostics, provider, options) = stub_work_harness();

        let sticky_token = state.interrupts.active().unwrap().message.token;
        let promotion_time = now + Duration::from_hours(1);
        drain_device_events(
            &mut state,
            &mut scheduler,
            &mut StubDevice {
                queued_event: Some(interrupt_dismissed_event("sticky", sticky_token)),
            },
            &RuntimeDiagnosticCounters::default(),
            promotion_time,
        );
        assert_eq!(
            state
                .interrupts
                .active()
                .map(|tracked| tracked.message.widget_id.as_str()),
            Some("upnext"),
            "dismissing the sticky interrupt promotes the queued calendar alert"
        );
        flush_interrupts(&mut state, &mut scheduler, &mut device, promotion_time).unwrap();

        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
            &provider,
            &diagnostics,
            promotion_time + Duration::from_secs(59),
            &options,
        );
        assert!(
            state.interrupts.active().is_some(),
            "the promoted alert's fresh 60s hold has not expired yet"
        );

        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
            &provider,
            &diagnostics,
            promotion_time + Duration::from_mins(1),
            &options,
        );
        assert!(
            state.interrupts.active().is_none(),
            "the promoted alert's own 60s hold now auto-dismisses it"
        );
    }

    // `is_event_alert_due`'s pure eligibility/dedup logic: outside the lead
    // window nothing is eligible; inside it, eligible exactly once per
    // distinct event start (simulating `refresh_calendar_alert`'s caller-side
    // arming after a successful schedule — repeated checks against the same
    // start are not eligible again); a different start re-arms eligibility; a
    // past start or the provider's `0` "no event" sentinel are never eligible.
    #[test]
    fn is_event_alert_due_fires_once_per_distinct_event_start_inside_the_lead_window() {
        let now = Instant::now();
        let config = rotation_config(
            vec![alert_calendar_card(
                "upnext",
                CardAlert::BeforeEvent {
                    lead_minutes: 5,
                    hold: AlertHold::Seconds { value: 60 },
                },
            )],
            CarouselAdvance::Manual,
            &[("upnext", None)],
        );
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);
        let now_unix_ms: i64 = 1_700_000_000_000;

        let set_next_start = |state: &mut WorkerState, offset_ms: i64| {
            state.latest_fields.insert(
                "upnext".into(),
                vec![Field {
                    key: "next_start_unix_ms".into(),
                    value: FieldValue::Integer(now_unix_ms + offset_ms),
                }],
            );
        };

        // Event starts in 6 minutes: outside the 5-minute lead window.
        set_next_start(&mut state, 6 * 60_000);
        assert_eq!(is_event_alert_due(&state, "upnext", 5, now_unix_ms), None);

        // Event starts in 4 minutes: inside the window, eligible.
        set_next_start(&mut state, 4 * 60_000);
        let due = is_event_alert_due(&state, "upnext", 5, now_unix_ms);
        assert_eq!(due, Some(now_unix_ms + 4 * 60_000));
        // Simulate the caller (`refresh_calendar_alert`) marking it armed only
        // after `InterruptArbiter::schedule` succeeds (Important-4).
        state
            .armed_event_alerts
            .insert("upnext".into(), due.unwrap());
        assert_eq!(
            is_event_alert_due(&state, "upnext", 5, now_unix_ms),
            None,
            "already armed for this event start"
        );

        // A different event start (same card) re-arms eligibility.
        set_next_start(&mut state, 4 * 60_000 + 2);
        assert!(is_event_alert_due(&state, "upnext", 5, now_unix_ms).is_some());

        // A start already in the past never fires.
        set_next_start(&mut state, -1);
        assert_eq!(is_event_alert_due(&state, "upnext", 5, now_unix_ms), None);

        // `0` is the provider's "no upcoming event" sentinel and never fires.
        set_next_start(&mut state, -now_unix_ms);
        assert_eq!(is_event_alert_due(&state, "upnext", 5, now_unix_ms), None);
    }

    // Task 6 review Important-4: before the fix, the event start was marked
    // armed *before* checking whether `InterruptArbiter::schedule` actually
    // succeeded, so a transient `Busy` (both arbiter slots already occupied)
    // silently and *permanently* dropped the alert for that event — the dedup
    // check would see it as already armed forever after, even once a slot
    // freed up. This proves the fix: once a slot frees, a later re-evaluation
    // for the *same* event start still fires.
    #[test]
    fn a_calendar_alert_dropped_by_a_busy_arbiter_is_not_permanently_armed() {
        let now = Instant::now();
        let config = rotation_config(
            vec![alert_calendar_card(
                "upnext",
                CardAlert::BeforeEvent {
                    lead_minutes: 5,
                    hold: AlertHold::Seconds { value: 60 },
                },
            )],
            CarouselAdvance::Manual,
            &[("upnext", None)],
        );
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);
        // Occupy both arbiter slots with unrelated interrupts so the calendar
        // alert's own `schedule` call returns `Busy`.
        state.interrupts.schedule("occupant-a", "x").unwrap();
        state.interrupts.schedule("occupant-b", "y").unwrap();

        let now_unix_ms = Utc::now().timestamp_millis();
        state.latest_fields.insert(
            "upnext".into(),
            vec![Field {
                key: "next_start_unix_ms".into(),
                value: FieldValue::Integer(now_unix_ms + 4 * 60_000),
            }],
        );

        refresh_calendar_alert(&mut state, &mut scheduler, "upnext", 5, now, now_unix_ms);
        assert!(
            !state.armed_event_alerts.contains_key("upnext"),
            "a Busy arbiter must not mark the event start armed"
        );

        // Both slots free up (simulating two on-device tap dismissals).
        let first_active = state.interrupts.active().unwrap().message.token;
        assert!(state.interrupts.dismiss(first_active).is_ok());
        let second_active = state.interrupts.active().unwrap().message.token;
        assert!(state.interrupts.dismiss(second_active).is_ok());
        assert!(state.interrupts.active().is_none());

        // The same event start, re-evaluated, now fires.
        refresh_calendar_alert(&mut state, &mut scheduler, "upnext", 5, now, now_unix_ms);
        assert_eq!(
            state
                .interrupts
                .active()
                .map(|tracked| tracked.message.widget_id.as_str()),
            Some("upnext"),
            "once a slot freed, the same event start must still be able to fire"
        );
    }

    // Task 6 review Important-3, pure-logic half: relying solely on
    // provider-result landings to observe the lead window misses it entirely
    // whenever no refresh happens to land inside it (the brief's original
    // design) — a 15-minute refresh interval against a 5-minute lead window
    // can refresh once *before* the window opens and again *after* the event
    // starts, never once landing inside it. This pins the root cause
    // directly: `is_event_alert_due`'s eligibility for the *same* cached
    // event start depends only on `now_unix_ms` (standing in for the wall
    // clock) advancing, not on `latest_fields` changing — ineligible 8
    // minutes out, eligible 3 minutes out, with no field write in between.
    // `now_unix_ms` is a synthetic epoch anchor (not real wall time), like
    // `is_event_alert_due_fires_once_per_distinct_event_start_inside_the_lead_window`
    // above, so this needs no simulated elapsed time and cannot be flaky.
    #[test]
    fn calendar_alert_eligibility_advances_purely_with_the_clock_not_a_fresh_refresh() {
        let now = Instant::now();
        let config = rotation_config(
            vec![alert_calendar_card(
                "upnext",
                CardAlert::BeforeEvent {
                    lead_minutes: 5,
                    hold: AlertHold::Seconds { value: 60 },
                },
            )],
            CarouselAdvance::Manual,
            &[("upnext", None)],
        );
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);
        let base_unix_ms: i64 = 1_700_000_000_000;
        let event_start_unix_ms = base_unix_ms + 8 * 60_000;
        state.latest_fields.insert(
            "upnext".into(),
            vec![Field {
                key: "next_start_unix_ms".into(),
                value: FieldValue::Integer(event_start_unix_ms),
            }],
        );

        // 8 minutes out: outside the 5-minute window.
        assert_eq!(is_event_alert_due(&state, "upnext", 5, base_unix_ms), None);

        // The clock alone advances 3 minutes — no new field data lands. Now 5
        // minutes out: inside the window.
        let three_minutes_later_unix_ms = base_unix_ms + 3 * 60_000;
        assert_eq!(
            is_event_alert_due(&state, "upnext", 5, three_minutes_later_unix_ms),
            Some(event_start_unix_ms)
        );
    }

    // Task 6 review Important-3, wiring half: `run_scheduled_work` must
    // itself dispatch `scheduler.due_event_alert_cards` results through
    // `card_alert` and `refresh_calendar_alert` — not rely on
    // `apply_provider_result` alone, which only runs when a refresh lands.
    // Arms the event-alert deadline directly, as if a prior refresh had
    // already left it behind (rather than going through
    // `refresh_calendar_alert` again here, which would just re-prove the pure
    // half above); no interrupt exists yet. The event start is a real
    // near-future wall-clock time inside the lead window, so whether it fires
    // is decided instantly and deterministically, without needing to
    // simulate elapsed wall-clock time — unlike `Instant`, this codebase has
    // no injectable clock for `Utc::now()` (see e.g. `send_time_sync`, which
    // calls it directly with no test seam either).
    #[test]
    fn run_scheduled_works_tick_handler_reevaluates_calendar_alerts_via_the_scheduler_deadline() {
        let now = Instant::now();
        let config = rotation_config(
            vec![alert_calendar_card(
                "upnext",
                CardAlert::BeforeEvent {
                    lead_minutes: 5,
                    hold: AlertHold::Seconds { value: 60 },
                },
            )],
            CarouselAdvance::Manual,
            &[("upnext", None)],
        );
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);
        assert!(scheduler.pomodoro_due(now));
        assert!(scheduler.status_due(now));
        assert!(scheduler.time_sync_due(now));
        assert_eq!(scheduler.take_due_providers(now), ["upnext"]);

        state.latest_fields.insert(
            "upnext".into(),
            vec![Field {
                key: "next_start_unix_ms".into(),
                value: FieldValue::Integer(Utc::now().timestamp_millis() + 4 * 60_000),
            }],
        );
        // A deadline is armed with no interrupt scheduled yet — exactly the
        // state left behind by a refresh that landed outside the window.
        scheduler.set_event_alert_deadline("upnext", Some(now));
        assert!(state.interrupts.active().is_none());

        let mut device = StubDevice::default();
        let diagnostics = RuntimeDiagnosticCounters::default();
        let provider = ProviderWorker::new(
            Box::new(ImmediateRefresher {
                completed: mpsc::channel().0,
            }),
            1,
        );
        let options = RuntimeOptions::default();
        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
            &provider,
            &diagnostics,
            now,
            &options,
        );

        assert_eq!(
            state
                .interrupts
                .active()
                .map(|tracked| tracked.message.widget_id.as_str()),
            Some("upnext"),
            "the tick handler alone re-evaluated and fired the alert, with no provider refresh involved"
        );
    }

    // Task 6 review Minor-5: this test must be sensitive to the *shape* of the
    // prune, not just to "something got pruned". A surviving card's armed
    // entry is asserted to remain untouched, so replacing the targeted
    // `.retain(...)` with a blanket `.clear()` — which would also pass a
    // prune test that only checks the removed card — fails this one.
    #[test]
    fn armed_event_alerts_are_pruned_only_for_cards_that_leave_the_config() {
        let now = Instant::now();
        let config = rotation_config(
            vec![
                alert_calendar_card(
                    "upnext",
                    CardAlert::BeforeEvent {
                        lead_minutes: 5,
                        hold: AlertHold::Seconds { value: 60 },
                    },
                ),
                alert_calendar_card(
                    "keep",
                    CardAlert::BeforeEvent {
                        lead_minutes: 5,
                        hold: AlertHold::Seconds { value: 60 },
                    },
                ),
            ],
            CarouselAdvance::Manual,
            &[("upnext", None), ("keep", None)],
        );
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config.clone(), now, &mut scheduler);
        state.armed_event_alerts.insert("upnext".into(), 111);
        state.armed_event_alerts.insert("keep".into(), 222);

        let mut replaced = config;
        replaced.cards.retain(|card| card.id() != "upnext");
        replaced.playlists[0]
            .entries
            .retain(|entry| entry.card_id != "upnext");
        state.replace_config(replaced, now, &mut scheduler);

        assert_eq!(
            state.armed_event_alerts.get("keep"),
            Some(&222),
            "a surviving card's armed entry must not be dropped by an unrelated prune"
        );
        assert!(
            !state.armed_event_alerts.contains_key("upnext"),
            "armed alerts for cards no longer in the config must not leak"
        );
    }

    // Wiring proof (instant, no sleeping): a provider result landing through
    // `apply_provider_result` — the function `drain_provider_results` calls in
    // its loop — schedules the interrupt for a calendar card with a
    // `BeforeEvent` alert, but the configured `AlertHold::Seconds` does not arm
    // until `flush_interrupts` delivers it, exactly as for pomodoro completion.
    #[test]
    fn a_landing_provider_result_arms_its_calendar_alert_hold_only_at_delivery() {
        let now = Instant::now();
        let config = rotation_config(
            vec![alert_calendar_card(
                "upnext",
                CardAlert::BeforeEvent {
                    lead_minutes: 5,
                    hold: AlertHold::Seconds { value: 60 },
                },
            )],
            CarouselAdvance::Manual,
            &[("upnext", None)],
        );
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);
        let diagnostics = RuntimeDiagnosticCounters::default();

        let event_start_unix_ms = Utc::now().timestamp_millis() + 4 * 60_000;
        let result = ProviderRefreshResult {
            generation: state.generation,
            widget_id: "upnext".into(),
            fields: vec![Field {
                key: "next_start_unix_ms".into(),
                value: FieldValue::Integer(event_start_unix_ms),
            }],
            refreshed_at: Some(Utc::now()),
            age: Some(Duration::ZERO),
            stale: false,
            error: None,
        };

        apply_provider_result(&mut state, &mut scheduler, &diagnostics, now, result);

        assert_eq!(
            state
                .interrupts
                .active()
                .map(|tracked| tracked.message.widget_id.as_str()),
            Some("upnext"),
            "the landing result's near event start schedules the alert"
        );
        assert_eq!(
            scheduler.alert_hold_due(now + Duration::from_hours(1)),
            None,
            "scheduling alone must not start the hold"
        );

        let mut device = StubDevice::default();
        flush_interrupts(&mut state, &mut scheduler, &mut device, now).unwrap();
        assert_eq!(
            scheduler.alert_hold_due(now + Duration::from_secs(59)),
            None
        );
        assert!(
            scheduler
                .alert_hold_due(now + Duration::from_mins(1))
                .is_some(),
            "hold armed from the card's own AlertHold::Seconds(60)"
        );
    }
}
