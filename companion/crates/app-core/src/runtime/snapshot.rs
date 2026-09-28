//! Publishing what the runtime knows: diagnostics counters, subscriptions,
//! and the snapshot exposed to companion clients.
// These runtime internals intentionally share the parent module's worker
// types and helpers; a glob keeps that internal seam in one place.
#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Default)]
pub(super) struct RuntimeDiagnosticCounters {
    pub(super) commands_processed: AtomicU64,
    pub(super) command_queue_full: AtomicU64,
    pub(super) subscriber_snapshots_overwritten: AtomicU64,
    pub(super) interrupt_dismissals_ignored: AtomicU64,
    /// Taps the host received and could do nothing with: a card id that resolves to
    /// nothing, or a picture card on a runtime with no tap sink installed. Counted
    /// for the same reason dismissals are -- on hardware, a tap the host discarded
    /// and a tap whose event never arrived look identical without this.
    pub(super) taps_dropped: AtomicU64,
}

impl RuntimeDiagnosticCounters {
    pub(super) fn snapshot(&self) -> RuntimeDiagnostics {
        RuntimeDiagnostics {
            commands_processed: self.commands_processed.load(Ordering::Relaxed),
            command_queue_full: self.command_queue_full.load(Ordering::Relaxed),
            subscriber_snapshots_overwritten: self
                .subscriber_snapshots_overwritten
                .load(Ordering::Relaxed),
            interrupt_dismissals_ignored: self.interrupt_dismissals_ignored.load(Ordering::Relaxed),
            taps_dropped: self.taps_dropped.load(Ordering::Relaxed),
        }
    }
}

pub(super) struct SubscriptionSlot {
    pub(super) state: Mutex<SubscriptionState>,
    pub(super) changed: Condvar,
}

pub(super) struct SubscriptionState {
    pub(super) snapshot: Option<AppSnapshot>,
    pub(super) closed: bool,
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

pub(super) struct SnapshotPublisher {
    pub(super) latest: Arc<RwLock<AppSnapshot>>,
    pub(super) subscribers: Mutex<Vec<Weak<SubscriptionSlot>>>,
    pub(super) maximum_subscribers: usize,
    pub(super) diagnostics: Arc<RuntimeDiagnosticCounters>,
}

impl SnapshotPublisher {
    pub(super) fn subscribe(&self) -> Result<RuntimeSubscription, RuntimeError> {
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

    pub(super) fn publish(&self, snapshot: &AppSnapshot) {
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

    pub(super) fn close(&self) {
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

/// The snapshot a configuration describes before any device has spoken: the
/// cards it declares, one idle pomodoro per pomodoro card, and no device facts.
///
/// Public because the server serves this same shape to the browser companion
/// when the board is not linked -- which is the common case, since the board is
/// normally powered off. Building it here rather than in the server keeps one
/// answer to "what does a snapshot look like before the device speaks", so the
/// offline view and a starting runtime cannot drift apart.
pub fn initial_snapshot(config: &AppConfig, diagnostics: RuntimeDiagnostics) -> AppSnapshot {
    let mut pomodoros = Vec::new();
    for card in &config.cards {
        match card {
            CardSettings::Pomodoro {
                id,
                duration_seconds,
                ..
            } => pomodoros.push(PomodoroSnapshot {
                card_id: id.clone(),
                state: PomodoroState::Idle,
                duration_seconds: *duration_seconds,
                remaining_seconds: *duration_seconds,
            }),
            CardSettings::Picture { .. } | CardSettings::Clock { .. } => {}
        }
    }
    AppSnapshot {
        config: config.clone(),
        runtime: RuntimeState::Starting,
        device: DeviceSnapshot {
            active_card_id: config.cards.first().map(|card| card.id().to_owned()),
            ..empty_device(ConnectionState::Connecting)
        },
        pomodoros,
        card_data: Vec::new(),
        card_errors: Vec::new(),
        persistence: PersistenceState::Clean,
        diagnostics,
    }
}

/// A device snapshot that claims nothing but its connection state. Public for
/// the same reason [`initial_snapshot`] is: the server reports a board it has no
/// link to as disconnected-and-otherwise-unknown, rather than inventing facts.
pub fn empty_device(connection: ConnectionState) -> DeviceSnapshot {
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
        active_card_id: None,
        counters: DeviceCounters::default(),
        asset_store: None,
    }
}
