use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Condvar, Mutex, RwLock, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::interrupts::InterruptArbiter;
use crate::pomodoro::Pomodoro;
use chrono::Utc;
use device::{DeviceError, ReceivedEvent, SessionDiagnostics};
use protocol::{
    Ack, AssetBegin, AssetChunk, AssetCommit, AssetRelease, CardConfig, EventAction, EventKind,
    Message, PushScene, StatusResponse, TimeSync, TriggerInterrupt, validate_message,
};

use crate::asset_sync::{AssetSync, AssetSyncError};
use crate::render_negotiation;
use crate::runtime_command::{CommandReply, PomodoroAction, RuntimeCommand, RuntimeError};
use crate::scene_build::{BakedFontMetrics, with_stale_footer};
use crate::scheduler::Scheduler;
use crate::{
    AlertHold, AnalogClockCard, AppConfig, AppSnapshot, CardAlert, CardDataSnapshot, CardError,
    CardErrorKind, CardField, CardFieldValue, CardSettings, ClockCard, ConnectionState,
    DesiredAsset, DeviceCounters, DeviceSnapshot, DeviceTier, DisplayTemplate, PersistenceState,
    PomodoroSnapshot, PomodoroState, ProgressRingCard, RuntimeDiagnostics, RuntimeState,
    build_analog_clock_scene, build_digital_clock_scene, build_progress_ring_scene,
};

mod asset_cache;
mod link;
mod scene;
mod snapshot;
mod sync;
mod timers;

use asset_cache::{prepare_asset_transfer, reconcile_assets};
pub use link::*;
pub use scene::*;
pub use snapshot::*;
// These two export nothing public, so they are imported by name.
use sync::{
    clear_asset_sync_refusals, push_dirty_widgets, send_time_sync, synchronize_full,
    synchronize_pending,
};
use timers::{
    control_pomodoro, flush_interrupts, pomodoro_fields, record_pomodoro_update,
    sync_alert_hold_to_active_interrupt, update_pomodoros,
};

const DEFAULT_RUNTIME_COMMAND_CAPACITY: usize = 16;
const DEFAULT_MAX_SUBSCRIBERS: usize = 8;

/// Receives taps on cards whose host-provided picture may need to change.
pub trait CardTapSink: Send + Sync {
    /// Called on the runtime worker thread. MUST NOT block.
    fn tapped(&self, card_id: &str, source_id: &str);
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeOptions {
    pub command_capacity: usize,
    pub maximum_subscribers: usize,
    pub command_timeout: Duration,
    pub loop_maximum_wait: Duration,
    pub reconnect_interval: Duration,
    pub pomodoro_interval: Duration,
    pub status_interval: Duration,
    pub time_sync_interval: Duration,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            command_capacity: DEFAULT_RUNTIME_COMMAND_CAPACITY,
            maximum_subscribers: DEFAULT_MAX_SUBSCRIBERS,
            command_timeout: Duration::from_secs(5),
            loop_maximum_wait: Duration::from_millis(25),
            reconnect_interval: Duration::from_secs(1),
            pomodoro_interval: Duration::from_secs(1),
            status_interval: Duration::from_secs(2),
            time_sync_interval: Duration::from_hours(1),
        }
    }
}

pub struct RuntimeHandle {
    sender: Arc<SyncSender<RuntimeCommand>>,
    worker: Mutex<Option<RuntimeWorker>>,
    publisher: Arc<SnapshotPublisher>,
    diagnostics: Arc<RuntimeDiagnosticCounters>,
    command_timeout: Duration,
}

struct RuntimeWorker {
    handle: JoinHandle<()>,
    shutdown_requested: bool,
}

impl RuntimeHandle {
    pub fn start(
        config: AppConfig,
        device: Box<dyn RuntimeDevice>,
        options: RuntimeOptions,
    ) -> Result<Self, RuntimeError> {
        Self::start_with_ports(config, device, options, None, None)
    }

    pub fn start_with_image_source_host(
        config: AppConfig,
        device: Box<dyn RuntimeDevice>,
        options: RuntimeOptions,
        image_source_host: Option<Box<dyn ImageSourceHost>>,
    ) -> Result<Self, RuntimeError> {
        Self::start_with_ports(config, device, options, image_source_host, None)
    }

    pub fn start_with_ports(
        config: AppConfig,
        mut device: Box<dyn RuntimeDevice>,
        options: RuntimeOptions,
        image_source_host: Option<Box<dyn ImageSourceHost>>,
        tap_sink: Option<Arc<dyn CardTapSink>>,
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
        let sender = Arc::new(sender);
        let event_sender = Arc::downgrade(&sender);
        device.set_event_waker(Arc::new(move || {
            // A full queue already wakes the worker. Never block the socket
            // reader or replace the event queue's own bounded delivery policy.
            if let Some(sender) = event_sender.upgrade() {
                let _ = sender.try_send(RuntimeCommand::DeviceEventsReady);
            }
        }));
        let worker_publisher = Arc::clone(&publisher);
        let worker_diagnostics = Arc::clone(&diagnostics);
        let worker_inputs = RuntimeWorkerInputs {
            config,
            device,
            image_source_host,
            tap_sink,
        };
        let worker = thread::Builder::new()
            .name("deskmate-runtime".into())
            .spawn(move || {
                run_runtime(
                    worker_inputs,
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
            worker: Mutex::new(Some(RuntimeWorker {
                handle: worker,
                shutdown_requested: false,
            })),
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

    /// Number of device taps dropped because their card was unknown or their
    /// picture card had no host sink.
    pub fn taps_dropped(&self) -> u64 {
        self.diagnostics.taps_dropped.load(Ordering::Relaxed)
    }

    pub fn apply_config(&self, config: AppConfig) -> Result<(), RuntimeError> {
        config
            .compile(1)
            .map_err(|error| RuntimeError::InvalidConfig {
                issues: error.issues,
            })?;
        self.request(|reply| RuntimeCommand::ApplyConfig { config, reply })
    }

    pub fn control_pomodoro(
        &self,
        card_id: impl Into<String>,
        action: PomodoroAction,
    ) -> Result<(), RuntimeError> {
        self.request(|reply| RuntimeCommand::Pomodoro {
            card_id: card_id.into(),
            action,
            reply,
        })
    }

    pub fn push_scene(&self, push: PushScene) -> Result<(), RuntimeError> {
        self.request(|reply| RuntimeCommand::PushScene { push, reply })
    }

    /// Reconciles the host's complete desired asset set after one image source
    /// changes, then queues a face rebuild only when that source is visible.
    pub fn image_source_updated(
        &self,
        source_id: &str,
        digest: [u8; protocol::ASSET_DIGEST_LEN],
    ) -> Result<(), RuntimeError> {
        self.request(|reply| RuntimeCommand::ImageSourceUpdated {
            source_id: source_id.to_owned(),
            digest,
            reply,
        })
    }

    pub fn shutdown(&self) -> Result<(), RuntimeError> {
        let started = Instant::now();
        {
            let mut worker_slot = self
                .worker
                .lock()
                .map_err(|_| RuntimeError::WorkerStopped)?;
            let Some(worker) = worker_slot.as_mut() else {
                return Ok(());
            };
            if worker.handle.is_finished() {
                let worker = worker_slot.take().expect("worker was present");
                drop(worker_slot);
                let requested = worker.shutdown_requested;
                return match worker.handle.join() {
                    Ok(()) if requested => Ok(()),
                    Ok(()) | Err(_) => Err(RuntimeError::WorkerStopped),
                };
            }
            if !worker.shutdown_requested {
                match self.sender.try_send(RuntimeCommand::Shutdown) {
                    Ok(()) => worker.shutdown_requested = true,
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
            }
        }

        loop {
            let finished = {
                let mut worker_slot = self
                    .worker
                    .lock()
                    .map_err(|_| RuntimeError::WorkerStopped)?;
                match worker_slot.as_ref() {
                    None => return Ok(()),
                    Some(worker) if worker.handle.is_finished() => worker_slot.take(),
                    Some(_) => None,
                }
            };
            if let Some(worker) = finished {
                return worker
                    .handle
                    .join()
                    .map_err(|_| RuntimeError::WorkerStopped);
            }

            let Some(remaining) = self.command_timeout.checked_sub(started.elapsed()) else {
                return Err(RuntimeError::ResponseTimeout);
            };
            if remaining.is_zero() {
                return Err(RuntimeError::ResponseTimeout);
            }
            thread::sleep(remaining.min(Duration::from_millis(1)));
        }
    }

    fn request(
        &self,
        command: impl FnOnce(CommandReply) -> RuntimeCommand,
    ) -> Result<(), RuntimeError> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(1);
        let command = command(reply_sender);
        // Chosen before the command is moved into the channel, so the budget
        // always matches the work the command actually performs.
        let budget = if command_drives_a_full_sync(&command) {
            SYNCHRONIZING_COMMAND_TIMEOUT.max(self.command_timeout)
        } else {
            self.command_timeout
        };
        match self.sender.try_send(command) {
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
        match reply_receiver.recv_timeout(budget) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err(RuntimeError::ResponseTimeout),
            Err(RecvTimeoutError::Disconnected) => Err(RuntimeError::WorkerStopped),
        }
    }
}

/// How much longer than the ordinary command budget a synchronizing command
/// gets.
///
/// A command is normally a message or two, and the default budget is short so
/// callers notice a wedged worker quickly. But `ApplyConfig` and
/// `ImageSourceUpdated` drive a full
/// device synchronize, which contains an asset reconcile, which now contains
/// `AssetRelease`'s own twenty-second budget. Five seconds cannot contain
/// twenty, so those two commands reported a timeout for work that was still
/// legitimately in progress and would go on to succeed.
///
/// It stays above the WebSocket caller's 22-second `AssetRelease` wait and
/// under the server's 30-second HTTP timeout; a cross-crate test pins both ends.
pub const SYNCHRONIZING_COMMAND_TIMEOUT: Duration = Duration::from_secs(25);

/// Whether this command drives a full device synchronize, and therefore needs
/// the longer budget above.
fn command_drives_a_full_sync(command: &RuntimeCommand) -> bool {
    matches!(
        command,
        RuntimeCommand::ApplyConfig { .. } | RuntimeCommand::ImageSourceUpdated { .. }
    )
}

impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        let finished = self.worker.lock().ok().and_then(|mut worker_slot| {
            let worker = worker_slot.as_mut()?;
            if !worker.shutdown_requested && self.sender.try_send(RuntimeCommand::Shutdown).is_ok()
            {
                worker.shutdown_requested = true;
            }
            worker
                .handle
                .is_finished()
                .then(|| worker_slot.take().expect("finished worker was present"))
        });
        if let Some(worker) = finished {
            let _ = worker.handle.join();
        }
    }
}

#[allow(clippy::struct_excessive_bools)]
struct WorkerState {
    config: AppConfig,
    runtime: RuntimeState,
    device: DeviceSnapshot,
    latest_fields: BTreeMap<String, Vec<CardField>>,
    /// Card ID -> the exact durable picture face most recently evaluated for
    /// delivery. Comparing this pair with the host detects cadence-driven
    /// digest/staleness changes without retrying an unchanged terminal refusal.
    last_evaluated_picture_face: BTreeMap<String, ([u8; protocol::ASSET_DIGEST_LEN], bool)>,
    image_source_host: Option<Box<dyn ImageSourceHost>>,
    tap_sink: Option<Arc<dyn CardTapSink>>,
    dirty_cards: BTreeSet<String>,
    /// Card ID -> the most recent typed refusal for that card. There is deliberately
    /// one editor-visible slot per card: if data and scene refusals happen before
    /// either recovers, the later refusal replaces the earlier one. A later accepted
    /// push clears the slot only when it is the same kind, and config replacement
    /// clears every slot because all refused payloads belonged to the old revision.
    push_rejections: BTreeMap<String, CardError>,
    pomodoros: BTreeMap<String, Pomodoro>,
    pomodoro_snapshots: BTreeMap<String, PomodoroSnapshot>,
    interrupts: InterruptArbiter,
    active_card: Option<String>,
    active_card_dirty: bool,
    /// The active card needs rebuilding as a scene because a host-owned fact
    /// changed. Consumed once by `push_active_scene`; the only scheduled check
    /// that sets it is an active picture's digest/staleness comparison.
    active_scene_dirty: bool,
    connected: bool,
    ever_connected: bool,
    needs_full_sync: bool,
    /// Latched only after this runtime receives an explicit `WrongTier` refusal.
    /// A Networked status alone cannot establish non-ownership because the server's
    /// WebSocket runtime legitimately drives devices that report that tier.
    ownership_refused: bool,
    next_scene_revision: u32,
    /// Digests the device is known to hold **in either tier**, observed through
    /// `AssetBegin(already-present)` or a successful commit on this runtime. A
    /// scene may only name a digest in here: one naming bytes the device lacks
    /// draws nothing.
    ///
    /// Was `confirmed_durable_assets` until the interactive path started using
    /// the volatile tier. A frame in PSRAM is exactly as resolvable as one in
    /// flash -- `protocol_asset_resolver` checks PSRAM first -- so gating the
    /// scene on durability made the picture wait for the flash write that the
    /// volatile tier exists to avoid. Durable bytes survive a reconnect and
    /// volatile ones do not, which is why this is rebuilt from the keep-set on
    /// every reconcile rather than accumulated.
    confirmed_resident_assets: BTreeSet<[u8; protocol::ASSET_DIGEST_LEN]>,
    /// Assets in the last accepted scene; protect them until another scene lands.
    live_scene_assets: BTreeSet<[u8; protocol::ASSET_DIGEST_LEN]>,
    /// When this runtime last asked the device to reclaim replaced assets.
    /// `None` until the first pass, which always releases.
    last_asset_release: Option<Instant>,
    next_connect: Instant,
    last_published: Option<AppSnapshot>,
}

/// How rarely the device is asked to reclaim replaced assets.
///
/// Reclaiming compacts the flash blob region and costs seconds during which the
/// panel shows its clock, so it must not ride along with every frame. See
/// [`AssetSync::reconcile_releasing`] for why deferring it is safe: the
/// partition is 6 MB, a frame is roughly 35 KB, and the four faces replace 16 an
/// hour. Half an hour of deferral is about eight dead records out of ~170 slots.
const ASSET_RELEASE_INTERVAL: Duration = Duration::from_mins(30);

impl WorkerState {
    /// A hostless worker for tests that do not exercise picture delivery.
    #[cfg(test)]
    fn new(config: AppConfig, now: Instant, scheduler: &mut Scheduler) -> Self {
        Self::new_with_image_source_host(config, now, scheduler, None)
    }

    #[cfg(test)]
    fn new_with_image_source_host(
        config: AppConfig,
        now: Instant,
        scheduler: &mut Scheduler,
        image_source_host: Option<Box<dyn ImageSourceHost>>,
    ) -> Self {
        Self::new_with_ports(config, now, scheduler, image_source_host, None)
    }

    fn new_with_ports(
        config: AppConfig,
        now: Instant,
        scheduler: &mut Scheduler,
        image_source_host: Option<Box<dyn ImageSourceHost>>,
        tap_sink: Option<Arc<dyn CardTapSink>>,
    ) -> Self {
        let mut state = Self {
            config: config.clone(),
            runtime: RuntimeState::Starting,
            device: empty_device(ConnectionState::Connecting),
            latest_fields: BTreeMap::new(),
            last_evaluated_picture_face: BTreeMap::new(),
            image_source_host,
            tap_sink,
            dirty_cards: BTreeSet::new(),
            push_rejections: BTreeMap::new(),
            pomodoros: BTreeMap::new(),
            pomodoro_snapshots: BTreeMap::new(),
            interrupts: InterruptArbiter::default(),
            active_card: None,
            active_card_dirty: false,
            active_scene_dirty: false,
            connected: false,
            ever_connected: false,
            needs_full_sync: true,
            ownership_refused: false,
            next_scene_revision: 0,
            confirmed_resident_assets: BTreeSet::new(),
            live_scene_assets: BTreeSet::new(),
            last_asset_release: None,
            next_connect: now,
            last_published: None,
        };
        state.replace_config(config, now, scheduler);
        state.runtime = RuntimeState::Starting;
        state
    }

    #[allow(clippy::too_many_lines)] // one linear config swap; splitting would scatter its invariants
    fn replace_config(&mut self, config: AppConfig, now: Instant, scheduler: &mut Scheduler) {
        let previous_active_card = self.active_card.clone();
        self.latest_fields.clear();
        let mut previous_pomodoros = std::mem::take(&mut self.pomodoros);
        self.config = config;
        self.dirty_cards.clear();
        self.push_rejections.clear();
        self.pomodoro_snapshots.clear();
        // Owned, not borrowed: `prune_alert_state_for_live_widgets` needs
        // `&mut self`, which cannot coexist with a set still borrowing from
        // `self.config`.
        let live_card_ids: BTreeSet<String> = self
            .config
            .cards
            .iter()
            .map(|card| card.id().to_owned())
            .collect();
        self.prune_alert_state_for_live_widgets(&live_card_ids, scheduler, now);
        self.last_evaluated_picture_face
            .retain(|card_id, _| live_card_ids.contains(card_id));

        for card in &self.config.cards {
            self.latest_fields
                .insert(card.id().to_owned(), card.initial_fields());
        }

        let cards: Vec<CardSettings> = self.config.cards.clone();
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
                CardSettings::Picture { id, .. } => {
                    // A picture card has no fetcher to poll. Its frames arrive
                    // by webhook, pushed by an external producer, so there is no
                    // fetch loop here to schedule and no deadline to arm --
                    // which is also why rotating onto one costs nothing.
                    //
                    // With no host there is additionally nothing holding a
                    // frame, so the compiled placeholder fields are a stand-in
                    // for data rather than data.
                    if self.image_source_host.is_none() {
                        self.latest_fields.remove(id);
                    }
                }
                CardSettings::Clock { .. } => {}
            }
        }
        // Stay on the card the panel is showing when it survives the new
        // configuration, so an unrelated edit does not jump the loop.
        self.active_card = previous_active_card
            .filter(|active| self.config.cards.iter().any(|card| card.id() == active))
            .or_else(|| self.config.cards.first().map(|card| card.id().to_owned()));
        self.active_card_dirty = self.active_card.is_some();
        self.active_scene_dirty = self.active_card.is_some();
        self.rearm_rotation_for_active_screen(scheduler, now);
        self.needs_full_sync = true;
        self.runtime = if self.config.preferences.paused {
            RuntimeState::Paused
        } else {
            RuntimeState::Running
        };
    }

    /// Prunes alert-related state keyed to a widget that is no longer live.
    /// The alert hold is keyed to a specific
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
            .retain_widgets(|card_id| live_widget_ids.contains(card_id));
        let active_token_after_prune = self
            .interrupts
            .active()
            .map(|tracked| tracked.message.token);
        if active_token_before_prune != active_token_after_prune {
            sync_alert_hold_to_active_interrupt(self, scheduler, now);
        }
    }

    /// Dwell is per-card: arm the deadline from whichever card ended up
    /// active (the preserved card if it is still in the loop, otherwise the
    /// first card), not blindly from index 0.
    ///
    /// Called from `replace_config`, i.e. at config install time, which runs
    /// before the device connects. The clock therefore starts on the boot
    /// (or newly-applied) card's dwell immediately, not from first connect;
    /// on a slow first connect the boot card can end up on screen for less
    /// than its configured dwell. Noted, not restructured — connect is
    /// normally fast and this only shortens one card's first showing.
    fn rearm_rotation_for_active_screen(&mut self, scheduler: &mut Scheduler, now: Instant) {
        let active_rotation_index = self
            .active_card
            .as_ref()
            .and_then(|active| card_index(&self.config, active))
            .unwrap_or(0);
        scheduler.set_rotation(current_dwell(&self.config, active_rotation_index), now);
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
        let fields = pomodoro_fields(timer.label(), &update);
        if record_pomodoro_update(self, id, &update, fields) {
            let _ = self.interrupts.schedule_timer_finished(id);
        }
        self.pomodoros.insert(id.into(), timer);
    }

    fn snapshot(&self, diagnostics: &RuntimeDiagnosticCounters) -> AppSnapshot {
        let mut device = self.device.clone();
        device.active_card_id.clone_from(&self.active_card);
        AppSnapshot {
            config: self.config.clone(),
            runtime: self.runtime.clone(),
            device,
            pomodoros: self.pomodoro_snapshots.values().cloned().collect(),
            card_data: self
                .latest_fields
                .iter()
                .map(|(card_id, fields)| CardDataSnapshot::new(card_id, fields))
                .collect(),
            card_errors: self.push_rejections.values().cloned().collect(),
            persistence: PersistenceState::Clean,
            diagnostics: diagnostics.snapshot(),
        }
    }

    fn publish_if_changed(
        &mut self,
        publisher: &SnapshotPublisher,
        diagnostics: &RuntimeDiagnosticCounters,
    ) {
        let snapshot = self.snapshot(diagnostics);
        let changed = self.last_published.as_ref().is_none_or(|last_published| {
            let mut comparable = snapshot.clone();
            // Delivery pressure is reported opportunistically with the next
            // substantive snapshot, but it cannot itself cause another
            // delivery and feed back into this counter forever.
            comparable.diagnostics.subscriber_snapshots_overwritten =
                last_published.diagnostics.subscriber_snapshots_overwritten;
            last_published != &comparable
        });
        if changed {
            publisher.publish(&snapshot);
            self.last_published = Some(snapshot);
        }
    }
}

fn card_index(config: &AppConfig, card_id: &str) -> Option<usize> {
    config.cards.iter().position(|card| card.id() == card_id)
}

fn picture_source<'a>(config: &'a AppConfig, card_id: &str) -> Option<&'a str> {
    config.cards.iter().find_map(|card| match card {
        CardSettings::Picture { id, source_id, .. } if id == card_id => Some(source_id.as_str()),
        _ => None,
    })
}

/// The dwell for the card at `index`, resolved against the document's default.
/// Returns `None` under `CarouselAdvance::Manual` or when fewer than two cards
/// exist, keeping no-op rotation deadlines disarmed.
fn current_dwell(config: &AppConfig, index: usize) -> Option<Duration> {
    if config.cards.len() < 2 {
        return None;
    }
    let default = config.advance.default_dwell_seconds()?;
    let card = config.cards.get(index)?;
    Some(Duration::from_secs(u64::from(
        card.dwell_seconds().unwrap_or(default),
    )))
}

/// Advances the rotation by one step (wrapping) and queues the resulting
/// screen through the existing `active_card_dirty` flush path, then
/// re-arms the deadline from the card just moved to, since dwell is per-card.
/// With fewer than two cards in the loop there is nothing to rotate to, so the
/// deadline is disarmed instead of waking the loop again for a no-op; the next
/// config replace re-evaluates it (see `WorkerState::replace_config`).
fn advance_rotation(state: &mut WorkerState, scheduler: &mut Scheduler, now: Instant) {
    if state.config.cards.len() > 1 {
        let current_index = state
            .active_card
            .as_ref()
            .and_then(|active| card_index(&state.config, active))
            .unwrap_or(0);
        let next_index = (current_index + 1) % state.config.cards.len();
        state.active_card = Some(state.config.cards[next_index].id().to_owned());
        state.active_card_dirty = true;
        state.active_scene_dirty = true;
        scheduler.set_rotation(current_dwell(&state.config, next_index), now);
    } else {
        scheduler.clear_rotation();
    }
}

struct RuntimeWorkerInputs {
    config: AppConfig,
    device: Box<dyn RuntimeDevice>,
    image_source_host: Option<Box<dyn ImageSourceHost>>,
    tap_sink: Option<Arc<dyn CardTapSink>>,
}

fn run_runtime(
    inputs: RuntimeWorkerInputs,
    command_receiver: &Receiver<RuntimeCommand>,
    publisher: &SnapshotPublisher,
    diagnostics: &RuntimeDiagnosticCounters,
    options: RuntimeOptions,
) {
    let RuntimeWorkerInputs {
        config,
        mut device,
        image_source_host,
        tap_sink,
    } = inputs;
    let now = Instant::now();
    let mut scheduler = Scheduler::new(
        now,
        options.pomodoro_interval,
        options.status_interval,
        options.time_sync_interval,
    );
    let mut state =
        WorkerState::new_with_ports(config, now, &mut scheduler, image_source_host, tap_sink);
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
        run_scheduled_work(&mut state, &mut scheduler, device.as_mut(), now, &options);
        state.publish_if_changed(publisher, diagnostics);
    }

    publisher.close();
}

#[allow(clippy::too_many_lines)] // one arm per runtime command
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
        RuntimeCommand::DeviceEventsReady => {}
        RuntimeCommand::ApplyConfig { config, reply } => {
            let now = Instant::now();
            state.replace_config(config, now, scheduler);
            let result = if state.connected && !state.config.preferences.paused {
                synchronize_full(state, scheduler, device, now)
            } else {
                Ok(())
            };
            let _ = reply.send(result);
        }
        RuntimeCommand::Pomodoro {
            card_id,
            action,
            reply,
        } => {
            let result =
                control_pomodoro(state, scheduler, device, &card_id, action, Instant::now());
            let _ = reply.send(result);
        }
        RuntimeCommand::ImageSourceUpdated {
            source_id,
            digest,
            reply,
        } => {
            let result =
                apply_image_source_update(state, device, &source_id, digest, reconnect_interval);
            let _ = reply.send(result);
        }
        RuntimeCommand::PushScene { push, reply } => {
            let result = if state.connected {
                let assets = render_negotiation::analyze_scene(&push.scene).asset_digests;
                device
                    .push_scene(push)
                    .map(|()| {
                        state.live_scene_assets = assets;
                    })
                    .map_err(|error| {
                        runtime_command_device_error(state, &error, reconnect_interval)
                    })
            } else {
                Err(RuntimeError::DeviceDisconnected)
            };
            let _ = reply.send(result);
        }
        RuntimeCommand::Shutdown => return true,
    }
    false
}

/// Whether this reconcile should close with an `AssetRelease`, and remembers
/// the answer when it is yes.
///
/// Reclaiming is housekeeping: nothing waits on it, and it costs the panel
/// seconds of its own clock. Rate-limiting it is what takes flash compaction out
/// of the path between a tap and its picture.
///
/// The cadence alone is not enough once every frame is volatile, which is why
/// [`volatile_pool_is_pressed`] can override it -- see that function.
fn claim_asset_release(state: &mut WorkerState, now: Instant) -> bool {
    let due = state
        .last_asset_release
        .is_none_or(|last| now.saturating_duration_since(last) >= ASSET_RELEASE_INTERVAL);
    if due {
        state.last_asset_release = Some(now);
    }
    due
}

/// Slots left unclaimed when deciding the pool needs reclaiming: one for the
/// incoming replacement every transfer needs, and one so a second card's refresh
/// landing in the same moment is not refused.
const VOLATILE_POOL_HEADROOM: u32 = 2;

/// Whether the device's frame pool is close enough to full that superseded
/// frames must be released now rather than at the next cadence.
///
/// Every refresh of a face yields a new digest, and a committed volatile frame
/// is freed only when an `AssetRelease` keep-set omits it. Four picture cards on
/// a fifteen-minute refresh produce sixteen frames an hour, so a sixteen-slot
/// pool fills in about an hour if nothing reclaims it. The thirty-minute cadence
/// happens to survive that (eight superseded plus four current), but it has no
/// margin: a fifth picture card exhausts the pool between releases and the
/// device starts answering `Busy`.
///
/// So the host watches the pool instead of trusting the arithmetic. The figures
/// come from `StatusResponse` key 32, which is why that key reports occupancy
/// and not just a capacity. A device that does not report it -- any image built
/// before the pool -- leaves this false and keeps the old cadence, which is
/// correct for it: with two slots the host is not staging anything to fill them.
fn volatile_pool_is_pressed(state: &WorkerState) -> bool {
    state.device.volatile_assets.is_some_and(|pool| {
        pool.committed_count + 1 >= pool.slot_capacity.saturating_sub(VOLATILE_POOL_HEADROOM)
    })
}

/// Tell the device which frames are still wanted, so it can free the rest.
///
/// `AssetRelease` is a device-wide KEEP-SET spanning both tiers, so the complete
/// desired set is the only safe thing to send: a partial list reads as "release
/// every digest I left out". Sending it is the only way a superseded volatile
/// frame is ever freed.
///
/// A failure is not reported, and not swallowed either: the cadence is stamped
/// only on success, so a failed pass leaves the pool still pressed and the very
/// next frame update tries again. The picture is already on the glass by the time
/// this runs and nothing waits on the outcome, so turning a delivered frame into
/// a reported error would be a lie about what the owner can see. This crate logs
/// nothing anywhere -- the server owns diagnostics -- so self-correction is the
/// handling rather than a warning nobody would read.
fn reclaim_superseded_frames(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
    desired: &[DesiredAsset],
) {
    if let Ok(keep_set) = reconcile_assets(state, device, desired, true) {
        state.last_asset_release = Some(Instant::now());
        state.confirmed_resident_assets = keep_set.into_iter().collect();
    }
}

/// One authoritative host snapshot, read before the device is touched.
///
/// A stale queued notification must not reconcile a set that no longer contains
/// the digest it names, and an empty or partial set would make `AssetRelease`
/// delete unrelated device-wide assets. Both checks are here rather than at the
/// call site so the order -- read once, validate, then speak to the device -- is
/// not something a later edit can accidentally interleave.
fn desired_assets_for_update(
    state: &mut WorkerState,
    source_id: &str,
    digest: [u8; protocol::ASSET_DIGEST_LEN],
) -> Result<Vec<DesiredAsset>, RuntimeError> {
    let host = state
        .image_source_host
        .as_deref_mut()
        .ok_or_else(|| RuntimeError::ImageSource {
            message: "cannot apply an image update because no image source host is configured"
                .into(),
        })?;
    let desired = host.desired_assets();
    if !desired.iter().any(|asset| asset.digest == digest) {
        return Err(RuntimeError::ImageSource {
            message: format!(
                "image source {source_id:?} is missing from the host's complete desired asset set"
            ),
        });
    }
    let frame = host
        .image_source_frame(source_id)
        .ok_or_else(|| RuntimeError::ImageSource {
            message: format!("image source {source_id:?} has no frame to install"),
        })?;
    if frame.digest != digest {
        return Err(RuntimeError::ImageSource {
            message: format!(
                "image source {source_id:?} now holds a different frame than this update"
            ),
        });
    }
    Ok(desired)
}

fn apply_image_source_update(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
    source_id: &str,
    digest: [u8; protocol::ASSET_DIGEST_LEN],
    reconnect_interval: Duration,
) -> Result<(), RuntimeError> {
    if !state.connected {
        return Err(RuntimeError::DeviceDisconnected);
    }
    let visible_picture_card_id = state.active_card.as_deref().and_then(|active_id| {
        state.config.cards.iter().find_map(|card| match card {
            CardSettings::Picture {
                id,
                source_id: configured_source,
                ..
            } if id == active_id && configured_source == source_id => Some(id.clone()),
            _ => None,
        })
    });

    let desired = desired_assets_for_update(state, source_id, digest)?;

    if state.config.preferences.paused {
        if visible_picture_card_id.is_some() {
            state.active_scene_dirty = true;
        }
        return Ok(());
    }

    // A staged selection already has a confirmed digest on this connection.
    // Even an already-present AssetBegin costs a device round trip; the scene
    // alone is enough. Keep the authoritative host checks above this shortcut.
    if state.confirmed_resident_assets.contains(&digest) {
        clear_asset_sync_refusals(state);
        if visible_picture_card_id.is_some() {
            state.active_scene_dirty = true;
            push_active_scene(state, device, reconnect_interval);
        }
        return Ok(());
    }

    // EVERY picture frame is volatile, not just the one on the glass.
    //
    // A durable pass costs an `AssetCommit` that writes flash and then an
    // `AssetRelease` whose compaction moves every record after the one it just
    // orphaned. Measured on `dev-0005`: 1.53-1.68 s and 10.5 s, against 76-81 ms
    // for the same frame as a `memcpy` into PSRAM (2026-09-28, both tiers on the
    // same board twenty minutes apart). Flash buys a picture frame nothing: the
    // faces re-render every fifteen minutes, every render yields a new digest, so
    // a durable frame is superseded long before its endurance mattered.
    //
    // This used to be the visible card only, because the pool held two frames and
    // a third reservation was refused. It holds sixteen since `v2.2.0-psram`, so
    // the tier fits every picture card at once -- which is what makes the
    // condition here about capability and capacity rather than about what is on
    // screen.
    //
    // A device that does not advertise bit 9, or a digest the host cannot produce,
    // still falls through to the durable path below.
    if state.device.capability_bits() & protocol::CAPABILITY_VOLATILE_ASSETS != 0
        && let Some(frame) = desired.iter().find(|asset| asset.digest == digest)
    {
        match prepare_asset_transfer(state, device, &desired).and_then(|()| {
            AssetSync::transfer_volatile(device, frame, state.device.capability_bits())
        }) {
            Ok(()) => {
                // The device holds it now, in PSRAM. Without this the scene gate
                // would refuse to name the digest and the picture would wait for
                // the durable write -- which is the wait this whole path removes.
                state.confirmed_resident_assets.insert(digest);
                clear_asset_sync_refusals(state);
                // Only the card on screen earns a push. Rebuilding an unrelated
                // face would turn every background refresh into panel traffic.
                if visible_picture_card_id.is_some() {
                    state.active_scene_dirty = true;
                    push_active_scene(state, device, reconnect_interval);
                }
                // Replacement room was secured before the upload. Reconcile
                // other changed views after this scene lands; each pass keeps
                // the last accepted scene alive while reclaiming obsolete frames.
                if volatile_pool_is_pressed(state) {
                    reclaim_superseded_frames(state, device, &desired);
                }
                return Ok(());
            }
            Err(error) => {
                let message = error.to_string();
                state.active_scene_dirty = false;
                // A card error belongs to a card the owner is looking at. A
                // background refusal is the link's business, not this card's.
                if let Some(card_id) = visible_picture_card_id.as_ref() {
                    handle_automatic_asset_error(state, card_id.clone(), error, reconnect_interval);
                }
                return Err(RuntimeError::Device { message });
            }
        }
    }

    // This is deliberately the entire host-owned set. AssetRelease is a
    // device-wide KEEP-SET, so reconciling only this source would delete every
    // other picture digest omitted from the partial list.
    let release = claim_asset_release(state, Instant::now());
    let keep_set = match reconcile_assets(state, device, &desired, release) {
        Ok(keep_set) => keep_set,
        Err(error) => {
            let message = error.to_string();
            if let Some(card_id) = visible_picture_card_id.as_ref() {
                state.active_scene_dirty = false;
                handle_automatic_asset_error(state, card_id.clone(), error, reconnect_interval);
            }
            return Err(RuntimeError::Device { message });
        }
    };
    state.confirmed_resident_assets = keep_set.into_iter().collect();
    clear_asset_sync_refusals(state);

    // Rebuilding an unrelated visible face would turn every background image
    // update into panel traffic. Off-screen frames simply remain resident for
    // the next ordinary rotation onto their card.
    if visible_picture_card_id.is_some() {
        state.active_scene_dirty = true;
        push_active_scene(state, device, reconnect_interval);
    }
    Ok(())
}

fn attempt_connect(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    device: &mut dyn RuntimeDevice,
    now: Instant,
    options: &RuntimeOptions,
) {
    state.confirmed_resident_assets.clear();
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

fn run_scheduled_work(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    device: &mut dyn RuntimeDevice,
    now: Instant,
    options: &RuntimeOptions,
) {
    detect_active_picture_face_change(state);
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
    // `active_card_dirty` and `flush_interrupts`.
    //
    // This only frees the host's arbiter slot
    // and re-sends the saved screen id (see `send_screen` below); the device
    // does not clear the interrupt overlay itself until the user taps it, so
    // host and device interrupt state can diverge until the next tap or a
    // full resync (see `arm_alert_hold`'s doc comment for the full picture).
    if let Some(token) = scheduler.alert_hold_due(now)
        && state.interrupts.dismiss(token).is_ok()
    {
        state.active_card_dirty = true;
        sync_alert_hold_to_active_interrupt(state, scheduler, now);
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

fn detect_active_picture_face_change(state: &mut WorkerState) {
    let Some((card_id, source_id)) = state.active_card.as_deref().and_then(|active_id| {
        state.config.cards.iter().find_map(|card| match card {
            CardSettings::Picture { id, source_id, .. } if id == active_id => {
                Some((id.clone(), source_id.clone()))
            }
            _ => None,
        })
    }) else {
        return;
    };
    let current = state
        .image_source_host
        .as_deref_mut()
        .and_then(|host| host.image_source_frame(&source_id))
        .map(|frame| (frame.digest, frame.stale));
    let evaluated = state.last_evaluated_picture_face.get(&card_id).copied();
    if current != evaluated {
        state.active_scene_dirty = true;
    }
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
                if let Some(index) = card_index(&state.config, &received.event.card_id) {
                    state.active_card = Some(received.event.card_id.clone());
                    // The gesture already changed the physical display. Remember it for
                    // future replay without issuing a redundant activation now.
                    state.active_card_dirty = false;
                    // The gesture selected the device model already, but the new
                    // card still needs its host-built scene laid over that model.
                    state.active_scene_dirty = true;
                    // A manual swipe restarts the dwell from the card just landed on,
                    // rather than letting a soon-to-expire deadline advance early.
                    scheduler.set_rotation(current_dwell(&state.config, index), now);
                }
            }
            (EventKind::Tap, action @ (EventAction::StartPause | EventAction::Reset)) => {
                // Every picture card is lowered to StartPause so the device reports taps at
                // all (see config.rs's wire_config). What the tap MEANS is the host's
                // business: a pomodoro's tap drives its timer, a picture's tap goes to
                // whatever draws it. This file must not learn what that is.
                if let Some(source_id) =
                    picture_source(&state.config, &received.event.card_id).map(str::to_owned)
                {
                    match &state.tap_sink {
                        Some(sink) => sink.tapped(&received.event.card_id, &source_id),
                        None => {
                            diagnostics.taps_dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                } else if card_index(&state.config, &received.event.card_id).is_none() {
                    diagnostics.taps_dropped.fetch_add(1, Ordering::Relaxed);
                } else {
                    let pomodoro_action = match action {
                        EventAction::StartPause => PomodoroAction::Toggle,
                        EventAction::Reset => PomodoroAction::Reset,
                        _ => unreachable!("the match arm only accepts pomodoro tap actions"),
                    };
                    let _ = control_pomodoro(
                        state,
                        scheduler,
                        device,
                        &received.event.card_id,
                        pomodoro_action,
                        now,
                    );
                }
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

#[cfg(test)]
mod tests {
    #[test]
    fn only_the_commands_that_synchronize_get_the_longer_budget() {
        assert!(super::SYNCHRONIZING_COMMAND_TIMEOUT > RuntimeOptions::default().command_timeout);
        let (reply, _receiver) = std::sync::mpsc::sync_channel(1);
        assert!(super::command_drives_a_full_sync(
            &RuntimeCommand::ApplyConfig {
                config: AppConfig::default(),
                reply: reply.clone(),
            }
        ));
        assert!(super::command_drives_a_full_sync(
            &RuntimeCommand::ImageSourceUpdated {
                source_id: "s".into(),
                digest: [0u8; 32],
                reply: reply.clone(),
            }
        ));
        // An explicit scene push is a single request; giving it 25 s would
        // slow down noticing a wedged worker, which is what the short budget
        // is for.
        assert!(!super::command_drives_a_full_sync(
            &RuntimeCommand::PushScene {
                push: PushScene {
                    revision: 1,
                    card_id: "clock".into(),
                    scene: protocol::Scene::default(),
                },
                reply,
            }
        ));
    }
    use super::*;
    use crate::{
        AlertHold, CardAlert, CarouselAdvance, DisplayTemplate, RefreshPolicy, WidgetTapAction,
    };

    #[test]
    fn wrong_tier_sync_result_latches_non_ownership_without_an_error() {
        let now = Instant::now();
        let mut scheduler = fresh_scheduler(now);
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

    // -- Rotation ------------------------------------------------------------
    //
    // These are the pure/instant counterparts of the single real-time rotation test
    // in `tests/runtime.rs`. They call `current_dwell`, `card_index`,
    // `advance_rotation`, and `drain_device_events` directly, so
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
            dwell_seconds: None,
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
            dwell_seconds: None,
        }
    }

    fn rotation_config(cards: Vec<CardSettings>, advance: CarouselAdvance) -> AppConfig {
        AppConfig {
            cards,
            advance,
            ..AppConfig::default()
        }
    }

    fn with_dwell(mut card: CardSettings, dwell: Option<u16>) -> CardSettings {
        match &mut card {
            CardSettings::Clock { dwell_seconds, .. }
            | CardSettings::Pomodoro { dwell_seconds, .. }
            | CardSettings::Picture { dwell_seconds, .. } => *dwell_seconds = dwell,
        }
        card
    }

    /// Two cards, 5s dwell each, with a 45s default that
    /// neither card should ever need (both set an explicit dwell).
    fn timed_two_card_config() -> AppConfig {
        rotation_config(
            vec![
                with_dwell(rotation_clock_card("a"), Some(5)),
                with_dwell(rotation_clock_card("b"), Some(5)),
            ],
            CarouselAdvance::Timed {
                default_dwell_seconds: 45,
            },
        )
    }

    fn timed_three_card_config() -> AppConfig {
        rotation_config(
            vec![
                with_dwell(rotation_clock_card("a"), Some(5)),
                with_dwell(
                    rotation_pomodoro_card(
                        "b",
                        CardAlert::OnTimerFinish {
                            hold: AlertHold::UntilDismissed,
                        },
                    ),
                    Some(7),
                ),
                with_dwell(rotation_clock_card("c"), Some(11)),
            ],
            CarouselAdvance::Timed {
                default_dwell_seconds: 45,
            },
        )
    }

    /// A `RuntimeDevice` stub for unit tests that never connect. Most device
    /// methods are unreachable because these tests keep
    /// `state.connected == false`. `trigger_interrupt` accepts direct
    /// `flush_interrupts` calls used to pin delivery-time alert-hold behavior,
    /// and `try_recv_event` optionally yields one queued event before returning
    /// `None` forever after.
    #[derive(Default)]
    struct StubDevice {
        queued_event: Option<ReceivedEvent>,
        busy_interrupts: u32,
        triggered_interrupts: Vec<TriggerInterrupt>,
    }

    impl RuntimeDevice for StubDevice {
        fn connect(&mut self) -> Result<DeviceConnection, DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn status(&mut self) -> Result<StatusResponse, DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn time_sync(&mut self, _sync: TimeSync) -> Result<(), DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn apply_layout(
            &mut self,
            _brightness: Option<u8>,
            _rotation: u16,
            _cards: Vec<CardConfig>,
        ) -> Result<(), DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn push_timer(
            &mut self,
            _card_id: String,
            _total_ms: u32,
            _remaining_ms: u32,
            _running: bool,
        ) -> Result<(), DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn activate_card(&mut self, _screen_id: String) -> Result<(), DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn push_scene(&mut self, _push: PushScene) -> Result<(), DeviceError> {
            unreachable!("stub device is never connected in these unit tests")
        }
        fn trigger_interrupt(&mut self, interrupt: TriggerInterrupt) -> Result<(), DeviceError> {
            self.triggered_interrupts.push(interrupt);
            if self.busy_interrupts > 0 {
                self.busy_interrupts -= 1;
                return Err(DeviceError::Rejected(protocol::ErrorResponse {
                    code: protocol::ErrorCode::Busy,
                    diagnostic: "display is busy".into(),
                }));
            }
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

    #[test]
    fn push_scene_while_disconnected_does_not_touch_the_device() {
        let now = Instant::now();
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(AppConfig::default(), now, &mut scheduler);
        let mut device = StubDevice::default();
        let (reply, result) = mpsc::sync_channel(1);

        process_command(
            RuntimeCommand::PushScene {
                push: PushScene {
                    card_id: "clock".into(),
                    revision: 1,
                    scene: protocol::Scene::default(),
                },
                reply,
            },
            &mut state,
            &mut scheduler,
            &mut device,
            &RuntimeDiagnosticCounters::default(),
            Duration::from_secs(1),
        );

        assert_eq!(
            result.recv().unwrap(),
            Err(RuntimeError::DeviceDisconnected)
        );
    }

    #[derive(Default)]
    struct ScheduledWorkDevice {
        status_calls: usize,
        time_sync_calls: usize,
        layout_calls: usize,
        asset_begin_failures_remaining: usize,
        /// Every scene the runtime actually pushed, so a negotiation test can
        /// assert the difference between "refused before the wire" and
        /// "pushed".
        scene_pushes: Vec<PushScene>,
        /// Every `AssetRelease` keep-set the device was sent, in order. An
        /// empty keep-set is a destructive full wipe, not a no-op, so a test
        /// has to be able to see that none was sent at all.
        asset_releases: Vec<Vec<[u8; protocol::ASSET_DIGEST_LEN]>>,
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

        fn time_sync(&mut self, _sync: TimeSync) -> Result<(), DeviceError> {
            self.time_sync_calls += 1;
            Ok(())
        }

        fn apply_layout(
            &mut self,
            _brightness: Option<u8>,
            _rotation: u16,
            _cards: Vec<CardConfig>,
        ) -> Result<(), DeviceError> {
            self.layout_calls += 1;
            Ok(())
        }

        fn push_timer(
            &mut self,
            _card_id: String,
            _total_ms: u32,
            _remaining_ms: u32,
            _running: bool,
        ) -> Result<(), DeviceError> {
            Ok(())
        }

        fn activate_card(&mut self, _screen_id: String) -> Result<(), DeviceError> {
            Ok(())
        }

        fn push_scene(&mut self, push: PushScene) -> Result<(), DeviceError> {
            self.scene_pushes.push(push);
            Ok(())
        }

        fn trigger_interrupt(&mut self, _interrupt: TriggerInterrupt) -> Result<(), DeviceError> {
            Ok(())
        }

        fn send_asset_begin(&mut self, _begin: AssetBegin) -> Result<Ack, DeviceError> {
            if self.asset_begin_failures_remaining > 0 {
                self.asset_begin_failures_remaining -= 1;
                return Err(DeviceError::Timeout);
            }
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

        fn send_asset_release(&mut self, release: AssetRelease) -> Result<(), DeviceError> {
            self.asset_releases.push(release.digests);
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
            firmware_version: "scheduled-work-test".into(),
            uptime_ms: 1,
            display_width: 368,
            display_height: 448,
            ..protocol::test_support::sample_status_response()
        }
    }

    struct MutablePictureHost {
        frame: Arc<Mutex<ImageSourceFrame>>,
    }

    fn picture_config() -> AppConfig {
        AppConfig {
            cards: vec![CardSettings::Picture {
                id: "picture-card".into(),
                title: "Picture card".into(),
                source_id: "camera".into(),
                tap_action: WidgetTapAction::None,
                refresh: RefreshPolicy::Manual,
                alert: CardAlert::None,
                dwell_seconds: None,
            }],
            image_sources: vec![crate::config::ImageSource {
                id: "camera".into(),
                name: "Camera".into(),
            }],
            ..AppConfig::default()
        }
    }

    impl ImageSourceHost for MutablePictureHost {
        fn desired_assets(&mut self) -> Vec<DesiredAsset> {
            let frame = self.frame.lock().unwrap().clone();
            vec![DesiredAsset {
                digest: frame.digest,
                kind: protocol::AssetKind::Image,
                bytes: frame.bytes,
            }]
        }

        fn image_source_frame(&mut self, source_id: &str) -> Option<ImageSourceFrame> {
            assert_eq!(source_id, "camera");
            Some(self.frame.lock().unwrap().clone())
        }
    }

    #[test]
    fn picture_negotiation_refusal_records_each_evaluated_identity_once() {
        let now = Instant::now();
        let first_digest = [0x71; protocol::ASSET_DIGEST_LEN];
        let second_digest = [0x72; protocol::ASSET_DIGEST_LEN];
        let frame = Arc::new(Mutex::new(ImageSourceFrame {
            digest: first_digest,
            bytes: Arc::from(&b"picture bytes"[..]),
            stale: false,
        }));
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new_with_image_source_host(
            picture_config(),
            now,
            &mut scheduler,
            Some(Box::new(MutablePictureHost {
                frame: Arc::clone(&frame),
            })),
        );
        state.connected = true;
        state.needs_full_sync = false;
        let mut device = ScheduledWorkDevice::default();

        push_active_scene(&mut state, &mut device, Duration::from_secs(1));
        assert!(
            device.scene_pushes.is_empty(),
            "negotiation refused before I/O"
        );
        assert_eq!(
            state
                .last_evaluated_picture_face
                .get("picture-card")
                .copied(),
            Some((first_digest, false))
        );
        assert!(!state.active_scene_dirty);

        detect_active_picture_face_change(&mut state);
        assert!(
            !state.active_scene_dirty,
            "the unchanged refused candidate must not be evaluated again"
        );

        frame.lock().unwrap().digest = second_digest;
        detect_active_picture_face_change(&mut state);
        assert!(
            state.active_scene_dirty,
            "a changed picture identity must request one new evaluation"
        );
        push_active_scene(&mut state, &mut device, Duration::from_secs(1));
        assert_eq!(
            state
                .last_evaluated_picture_face
                .get("picture-card")
                .copied(),
            Some((second_digest, false))
        );
        assert!(!state.active_scene_dirty);
        assert!(state.push_rejections.contains_key("picture-card"));
    }

    #[test]
    fn a_delayed_image_notification_cannot_cancel_a_pending_transient_scene_retry() {
        let now = Instant::now();
        let digest = [0x73; protocol::ASSET_DIGEST_LEN];
        let frame = Arc::new(Mutex::new(ImageSourceFrame {
            digest,
            bytes: Arc::from(&b"picture bytes"[..]),
            stale: false,
        }));
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new_with_image_source_host(
            picture_config(),
            now,
            &mut scheduler,
            Some(Box::new(MutablePictureHost {
                frame: Arc::clone(&frame),
            })),
        );
        state.connected = true;
        state.needs_full_sync = false;
        state.device.capabilities = vec![crate::DeviceCapability::SceneRender];
        let mut device = ScheduledWorkDevice {
            asset_begin_failures_remaining: 2,
            ..ScheduledWorkDevice::default()
        };

        push_active_scene(&mut state, &mut device, Duration::from_secs(1));
        assert_eq!(
            state
                .last_evaluated_picture_face
                .get("picture-card")
                .copied(),
            Some((digest, false))
        );
        assert!(state.active_scene_dirty, "the automatic retry is pending");

        let error = apply_image_source_update(
            &mut state,
            &mut device,
            "camera",
            digest,
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("AssetBegin"),
            "the command keeps the reconciliation diagnostic"
        );

        push_active_scene(&mut state, &mut device, Duration::from_secs(1));
        assert_eq!(
            device.scene_pushes.len(),
            1,
            "the unchanged frame must recover through the pending automatic retry"
        );
        assert_eq!(
            frame.lock().unwrap().digest,
            digest,
            "the host did not change"
        );
    }

    #[test]
    fn disconnected_scheduled_work_leaves_a_real_wait() {
        let now = Instant::now();
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(AppConfig::default(), now, &mut scheduler);
        let mut device = ScheduledWorkDevice::default();
        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
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
        let mut config =
            rotation_config(vec![rotation_clock_card("clock")], CarouselAdvance::Manual);
        config.preferences.paused = true;
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);
        state.connected = true;
        let mut device = ScheduledWorkDevice::default();
        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
            now,
            &RuntimeOptions::default(),
        );

        assert!(
            scheduler.wait_duration(now, Duration::from_hours(1)) > Duration::ZERO,
            "a paused worker must block instead of spinning on skipped sync deadlines"
        );
    }

    #[test]
    fn reconnect_runs_status_and_time_sync_promptly_after_skipped_work() {
        let disconnected_at = Instant::now();
        let mut scheduler = fresh_scheduler(disconnected_at);
        let mut state = WorkerState::new(AppConfig::default(), disconnected_at, &mut scheduler);
        state.needs_full_sync = false;
        let mut device = ScheduledWorkDevice::default();
        let options = RuntimeOptions::default();

        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
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
            reconnected_at,
            &options,
        );

        assert_eq!(device.status_calls, 1, "status must be prompt on reconnect");
        assert_eq!(
            device.time_sync_calls, 1,
            "time sync must be prompt on reconnect"
        );
    }

    fn navigation_event(card_id: &str) -> ReceivedEvent {
        ReceivedEvent {
            event: protocol::DeviceEvent {
                sequence: 1,
                kind: EventKind::Navigation,
                card_id: card_id.into(),
                action: EventAction::NavigateNext,
                interrupt_token: None,
            },
            missed_before: 0,
        }
    }

    fn interrupt_dismissed_event(card_id: &str, token: u32) -> ReceivedEvent {
        ReceivedEvent {
            event: protocol::DeviceEvent {
                sequence: 1,
                kind: EventKind::InterruptDismissed,
                card_id: card_id.into(),
                action: EventAction::DismissInterrupt,
                interrupt_token: Some(token),
            },
            missed_before: 0,
        }
    }

    // Under `CarouselAdvance::Manual`, no rotation deadline is ever armed;
    // a timed one-card loop also has nowhere to advance. This pins both
    // no-op paths through `current_dwell`.
    #[test]
    fn current_dwell_is_none_when_rotation_cannot_advance() {
        let mut manual_two_card = timed_two_card_config();
        manual_two_card.advance = CarouselAdvance::Manual;
        let timed_single_card = AppConfig {
            advance: CarouselAdvance::Timed {
                default_dwell_seconds: 5,
            },
            ..AppConfig::default()
        };
        for (name, config, index) in [
            ("manual-default-card", AppConfig::default(), 0),
            ("manual-first-of-two", manual_two_card.clone(), 0),
            ("manual-second-of-two", manual_two_card, 1),
            ("timed-single-card", timed_single_card, 0),
        ] {
            assert!(
                current_dwell(&config, index).is_none(),
                "case {name} unexpectedly armed rotation"
            );
        }
    }

    // Dwell is resolved per-card against the carousel default: an explicit
    // dwell wins and an absent one falls back to the default.
    #[test]
    fn current_dwell_resolves_each_cards_own_value_before_falling_back_to_the_default() {
        let config = rotation_config(
            vec![
                with_dwell(rotation_clock_card("explicit"), Some(10)),
                with_dwell(rotation_clock_card("defaulted"), None),
            ],
            CarouselAdvance::Timed {
                default_dwell_seconds: 45,
            },
        );
        assert_eq!(current_dwell(&config, 0), Some(Duration::from_secs(10)));
        assert_eq!(current_dwell(&config, 1), Some(Duration::from_secs(45)));
    }

    // `advance_rotation` walks the card list in order and wraps at the end.
    //
    // Since schema v10 it walks EVERY card: `cards` is the loop, so there is no
    // longer such a thing as a card that raises alerts without ever being shown.
    // Also exercises re-arming with each card's own dwell as the index moves.
    #[test]
    fn advance_rotation_walks_every_card_in_order_and_wraps() {
        let now = Instant::now();
        let mut config = timed_two_card_config();
        config.cards.push(rotation_pomodoro_card(
            "alerting",
            CardAlert::OnTimerFinish {
                hold: AlertHold::UntilDismissed,
            },
        ));
        config.cards.push(rotation_clock_card("last"));
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);
        assert_eq!(state.active_card.as_deref(), Some("a"));

        for expected in ["b", "alerting", "last", "a"] {
            state.active_card_dirty = false;
            advance_rotation(&mut state, &mut scheduler, now);
            assert_eq!(
                state.active_card.as_deref(),
                Some(expected),
                "the loop must reach every card and wrap"
            );
            assert!(state.active_card_dirty);
        }
    }

    // With fewer than two cards there is nothing to rotate to, so
    // `advance_rotation` disarms the deadline instead of activating anything.
    #[test]
    fn advance_rotation_disarms_with_fewer_than_two_in_rotation_cards() {
        let now = Instant::now();
        let config = AppConfig {
            advance: CarouselAdvance::Timed {
                default_dwell_seconds: 5,
            },
            ..AppConfig::default()
        };
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);
        state.active_card_dirty = false;
        advance_rotation(&mut state, &mut scheduler, now);
        assert!(!state.active_card_dirty, "nothing to rotate to");
        assert!(!scheduler.rotation_due(now + Duration::from_hours(1)));
    }

    // A local swipe resolves the reported card's index and re-arms its dwell.
    #[test]
    fn local_swipe_rearms_the_dwell_of_the_card_it_landed_on() {
        let now = Instant::now();
        let config = timed_three_card_config();
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);
        let mut device = StubDevice {
            queued_event: Some(navigation_event("c")),
            ..StubDevice::default()
        };

        drain_device_events(
            &mut state,
            &mut scheduler,
            &mut device,
            &RuntimeDiagnosticCounters::default(),
            now,
        );

        assert_eq!(state.active_card.as_deref(), Some("c"));
        assert!(!scheduler.rotation_due(now + Duration::from_secs(10)));
        assert!(scheduler.rotation_due(now + Duration::from_secs(11)));
    }

    // Rotation advances locally while paused, mirroring how pomodoro ticks are
    // never gated by pause. This keeps local state current for the next sync.
    #[test]
    fn rotation_advances_locally_while_paused_mirroring_pomodoro_ticks() {
        let now = Instant::now();
        let mut config = timed_two_card_config();
        config.preferences.paused = true;
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);
        assert!(state.config.preferences.paused);
        assert!(!state.connected);

        let mut device = StubDevice::default();
        let options = RuntimeOptions::default();

        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
            now + Duration::from_secs(5),
            &options,
        );

        assert_eq!(state.active_card.as_deref(), Some("b"));
        assert!(
            state.active_card_dirty,
            "queued for the device, not yet sent"
        );
    }

    // -- Alert triggers and hold ---------------------------------------------
    //
    // Instant unit tests for the trigger-kind narrowing, the bounded hold
    // deadline, including token-keyed active/pending scoping. Each calls
    // private helpers directly with synthetic `Instant`s, so none of them
    // sleep. The one wall-clock proof
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

    #[test]
    fn mixed_pomodoro_completion_batch_queues_only_the_alerting_timer() {
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
                .map(|tracked| tracked.message.card_id.as_str()),
            Some("loud"),
            "only the OnTimerFinish card's completion schedules an interrupt"
        );
        assert!(state.interrupts.pending().is_none());
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
        let options = RuntimeOptions::default();
        state.active_card_dirty = false;

        // Hold armed at delivery time (60s) + 30s = 90s from `now`. One second
        // before that, the interrupt survives.
        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
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
            now + Duration::from_secs(90),
            &options,
        );
        assert!(
            state.interrupts.active().is_none(),
            "hold expired and auto-dismissed the interrupt"
        );
        assert!(
            state.active_card_dirty,
            "queued the saved carousel screen for resync"
        );
    }

    #[test]
    fn busy_interrupt_delivery_retries_the_identical_message_before_arming_the_hold() {
        let now = Instant::now();
        let config = rotation_config(
            vec![rotation_pomodoro_card(
                "loud",
                CardAlert::OnTimerFinish {
                    hold: AlertHold::Seconds { value: 30 },
                },
            )],
            CarouselAdvance::Manual,
        );
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);
        state.pomodoros.get_mut("loud").unwrap().start(now);
        let delivery_time = now + Duration::from_mins(1);
        update_pomodoros(&mut state, delivery_time);
        let expected = state.interrupts.active().unwrap().message.clone();
        let mut device = StubDevice {
            busy_interrupts: 1,
            ..StubDevice::default()
        };

        flush_interrupts(&mut state, &mut scheduler, &mut device, delivery_time).unwrap();
        let active = state.interrupts.active().unwrap();
        assert_eq!(active.message, expected);
        assert!(!active.acknowledged);
        assert_eq!(
            scheduler.alert_hold_due(delivery_time + Duration::from_hours(1)),
            None
        );

        flush_interrupts(&mut state, &mut scheduler, &mut device, delivery_time).unwrap();
        let active = state.interrupts.active().unwrap();
        assert_eq!(active.message, expected);
        assert!(active.acknowledged);
        assert_eq!(
            device.triggered_interrupts,
            vec![expected.clone(), expected.clone()]
        );
        assert_eq!(
            scheduler.alert_hold_due(delivery_time + Duration::from_secs(30)),
            Some(expected.token)
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
        );
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);
        state.pomodoros.get_mut("loud").unwrap().start(now);
        let delivery_time = now + Duration::from_mins(1);
        update_pomodoros(&mut state, delivery_time);
        assert!(state.interrupts.active().is_some());

        let mut device = StubDevice::default();
        flush_interrupts(&mut state, &mut scheduler, &mut device, delivery_time).unwrap();
        let options = RuntimeOptions::default();

        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
            now + Duration::from_hours(1),
            &options,
        );
        assert!(
            state.interrupts.active().is_some(),
            "AlertHold::UntilDismissed never auto-dismisses"
        );
    }

    // Shared setup for the two active/pending hold-isolation tests: a sticky
    // (`UntilDismissed`) pomodoro alert Active, and a bounded (60s) pomodoro
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
                rotation_pomodoro_card(
                    "upnext",
                    CardAlert::OnTimerFinish {
                        hold: AlertHold::Seconds { value: 60 },
                    },
                ),
            ],
            CarouselAdvance::Manual,
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
                .map(|tracked| tracked.message.card_id.as_str()),
            Some("sticky")
        );

        // A second timer alert arrives while Active is occupied, so it queues Pending.
        state.interrupts.schedule_timer_finished("upnext").unwrap();
        assert_eq!(
            state
                .interrupts
                .pending()
                .map(|tracked| tracked.message.card_id.as_str()),
            Some("upnext"),
            "the Active slot is occupied, so the second timer alert queues Pending"
        );

        (state, scheduler)
    }

    fn stub_work_harness() -> (StubDevice, RuntimeOptions) {
        (StubDevice::default(), RuntimeOptions::default())
    }

    // A Pending alert's deadline must not affect the Active alert. An hour
    // later, both the sticky Active interrupt and the bounded Pending one
    // remain intact because hold deadlines only apply to the Active token.
    #[test]
    fn a_pending_alerts_hold_does_not_cross_talk_with_the_active_interrupt() {
        let now = Instant::now();
        let (mut state, mut scheduler) = sticky_active_and_bounded_pending_alert(now);
        let (mut device, options) = stub_work_harness();

        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
            now + Duration::from_hours(1),
            &options,
        );
        assert_eq!(
            state
                .interrupts
                .active()
                .map(|tracked| tracked.message.card_id.as_str()),
            Some("sticky"),
            "UntilDismissed must not be auto-dismissed by an unrelated Pending interrupt's hold"
        );
        assert_eq!(
            state
                .interrupts
                .pending()
                .map(|tracked| tracked.message.card_id.as_str()),
            Some("upnext"),
            "the timer interrupt is still queued, unharmed"
        );
    }

    // Once the sticky interrupt is dismissed and the Pending timer alert is
    // promoted to Active, its own 60-second hold starts at promotion time.
    #[test]
    fn a_promoted_alert_gets_a_fresh_hold_from_its_own_card() {
        let now = Instant::now();
        let (mut state, mut scheduler) = sticky_active_and_bounded_pending_alert(now);
        let (mut device, options) = stub_work_harness();

        let sticky_token = state.interrupts.active().unwrap().message.token;
        let promotion_time = now + Duration::from_hours(1);
        drain_device_events(
            &mut state,
            &mut scheduler,
            &mut StubDevice {
                queued_event: Some(interrupt_dismissed_event("sticky", sticky_token)),
                ..StubDevice::default()
            },
            &RuntimeDiagnosticCounters::default(),
            promotion_time,
        );
        assert_eq!(
            state
                .interrupts
                .active()
                .map(|tracked| tracked.message.card_id.as_str()),
            Some("upnext"),
            "dismissing the sticky interrupt promotes the queued timer alert"
        );
        flush_interrupts(&mut state, &mut scheduler, &mut device, promotion_time).unwrap();

        run_scheduled_work(
            &mut state,
            &mut scheduler,
            &mut device,
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
            promotion_time + Duration::from_mins(1),
            &options,
        );
        assert!(
            state.interrupts.active().is_none(),
            "the promoted alert's own 60s hold now auto-dismisses it"
        );
    }
}
