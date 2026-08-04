use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use app_core::{
    AppConfig, CalendarRefreshRequest, CalendarRefreshResult, CalendarRefresher, ConnectionState,
    DeviceConnection, PomodoroAction, PomodoroState, RuntimeDevice, RuntimeError, RuntimeHandle,
    RuntimeOptions, RuntimeState, WidgetSettings,
};
use chrono::Utc;
use device::{DeviceError, ReceivedEvent, SessionDiagnostics, TransportError};
use protocol::{
    DeviceEvent, EventAction, EventKind, Field, FieldValue, PROTOCOL_VERSION, ScreenConfig,
    StatusResponse, TimeSync, TriggerInterrupt, WidgetConfig,
};

const FULL_JSON: &str = include_str!("fixtures/full.json");

#[derive(Debug, Clone, PartialEq, Eq)]
enum Operation {
    Connect,
    Status,
    TimeSync,
    ApplyLayout,
    Push(String),
    Activate(String),
    Interrupt(u32),
    ReplayTime,
    ReplayLayout,
    ReplayPush(String),
    ReplayActivate(String),
    ReplayInterrupt(u32),
}

#[derive(Default)]
struct ReplayCache {
    time: bool,
    layout: bool,
    pushes: BTreeMap<String, Vec<Field>>,
    active_screen: Option<String>,
    interrupts: BTreeMap<u32, TriggerInterrupt>,
}

#[derive(Default)]
struct MockState {
    connected: bool,
    connection_count: u64,
    disconnect_on_status: bool,
    reset_on_connect: bool,
    operations: Vec<Operation>,
    events: VecDeque<ReceivedEvent>,
    replay: ReplayCache,
    next_push_gate: Option<Arc<PushGate>>,
}

#[derive(Default)]
struct PushGate {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}

impl PushGate {
    fn wait_until_entered(&self) {
        let state = self.state.lock().unwrap();
        let (state, _) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(1), |(entered, _)| !*entered)
            .unwrap();
        assert!(state.0, "push gate was never entered");
    }

    fn enter_and_wait(&self) {
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        self.changed.notify_all();
        while !state.1 {
            state = self.changed.wait(state).unwrap();
        }
    }

    fn open(&self) {
        let mut state = self.state.lock().unwrap();
        state.1 = true;
        self.changed.notify_all();
    }
}

#[derive(Clone, Default)]
struct MockDeviceControl {
    state: Arc<Mutex<MockState>>,
}

impl MockDeviceControl {
    fn force_disconnect(&self, power_reset: bool) {
        let mut state = self.state.lock().unwrap();
        state.disconnect_on_status = true;
        state.reset_on_connect = power_reset;
    }

    fn push_event(&self, event: DeviceEvent) {
        self.state.lock().unwrap().events.push_back(ReceivedEvent {
            event,
            missed_before: 0,
        });
    }

    fn operations(&self) -> Vec<Operation> {
        self.state.lock().unwrap().operations.clone()
    }

    fn connection_count(&self) -> u64 {
        self.state.lock().unwrap().connection_count
    }

    fn block_next_push(&self) -> Arc<PushGate> {
        let gate = Arc::new(PushGate::default());
        self.state.lock().unwrap().next_push_gate = Some(Arc::clone(&gate));
        gate
    }
}

struct MockDevice {
    control: MockDeviceControl,
}

impl MockDevice {
    fn new(control: MockDeviceControl) -> Self {
        Self { control }
    }

    fn with_connected<R>(
        &self,
        operation: impl FnOnce(&mut MockState) -> R,
    ) -> Result<R, DeviceError> {
        let mut state = self.control.state.lock().unwrap();
        if !state.connected {
            return Err(DeviceError::Transport(TransportError::Disconnected));
        }
        Ok(operation(&mut state))
    }
}

impl RuntimeDevice for MockDevice {
    fn connect(&mut self) -> Result<DeviceConnection, DeviceError> {
        let mut state = self.control.state.lock().unwrap();
        let reconnect = state.connection_count != 0;
        let reset = state.reset_on_connect;
        state.reset_on_connect = false;
        state.connected = true;
        state.connection_count += 1;
        state.operations.push(Operation::Connect);
        if reconnect {
            if state.replay.time {
                state.operations.push(Operation::ReplayTime);
            }
            if reset && state.replay.layout {
                state.operations.push(Operation::ReplayLayout);
                let widget_ids: Vec<String> = state.replay.pushes.keys().cloned().collect();
                for widget_id in widget_ids {
                    state.operations.push(Operation::ReplayPush(widget_id));
                }
            }
            if let Some(screen_id) = state.replay.active_screen.clone() {
                state.operations.push(Operation::ReplayActivate(screen_id));
            }
            let tokens: Vec<u32> = state.replay.interrupts.keys().copied().collect();
            for token in tokens {
                state.operations.push(Operation::ReplayInterrupt(token));
            }
        }
        let uptime_ms = if reset {
            10
        } else {
            state.connection_count * 1_000
        };
        Ok(DeviceConnection {
            port_name: "mock-usb".into(),
            status: status(uptime_ms),
        })
    }

    fn status(&mut self) -> Result<StatusResponse, DeviceError> {
        let mut state = self.control.state.lock().unwrap();
        if state.disconnect_on_status {
            state.disconnect_on_status = false;
            state.connected = false;
            return Err(DeviceError::Transport(TransportError::Disconnected));
        }
        if !state.connected {
            return Err(DeviceError::Transport(TransportError::Disconnected));
        }
        state.operations.push(Operation::Status);
        Ok(status(state.connection_count * 1_000 + 100))
    }

    fn time_sync(&mut self, _sync: TimeSync) -> Result<(), DeviceError> {
        self.with_connected(|state| {
            state.replay.time = true;
            state.operations.push(Operation::TimeSync);
        })
    }

    fn apply_layout(
        &mut self,
        _widgets: Vec<WidgetConfig>,
        _screens: Vec<ScreenConfig>,
    ) -> Result<(), DeviceError> {
        self.with_connected(|state| {
            state.replay.layout = true;
            state.replay.pushes.clear();
            state.replay.interrupts.clear();
            state.operations.push(Operation::ApplyLayout);
        })
    }

    fn push_fields(&mut self, widget_id: String, fields: Vec<Field>) -> Result<(), DeviceError> {
        let gate = self.with_connected(|state| state.next_push_gate.take())?;
        if let Some(gate) = gate {
            gate.enter_and_wait();
        }
        self.with_connected(|state| {
            state.replay.pushes.insert(widget_id.clone(), fields);
            state.operations.push(Operation::Push(widget_id));
        })
    }

    fn activate_screen(&mut self, screen_id: String) -> Result<(), DeviceError> {
        self.with_connected(|state| {
            state.replay.active_screen = Some(screen_id.clone());
            state.operations.push(Operation::Activate(screen_id));
        })
    }

    fn trigger_interrupt(&mut self, interrupt: TriggerInterrupt) -> Result<(), DeviceError> {
        self.with_connected(|state| {
            state
                .replay
                .interrupts
                .insert(interrupt.token, interrupt.clone());
            state.operations.push(Operation::Interrupt(interrupt.token));
        })
    }

    fn try_recv_event(&mut self) -> Option<ReceivedEvent> {
        let mut state = self.control.state.lock().unwrap();
        let event = state.events.pop_front()?;
        if event.event.kind == EventKind::InterruptDismissed
            && let Some(token) = event.event.interrupt_token
        {
            state.replay.interrupts.remove(&token);
        }
        Some(event)
    }

    fn diagnostics(&self) -> SessionDiagnostics {
        let state = self.control.state.lock().unwrap();
        SessionDiagnostics {
            reconnects: state.connection_count.saturating_sub(1),
            ..SessionDiagnostics::default()
        }
    }
}

struct FixedRefresher {
    delay: Duration,
}

impl CalendarRefresher for FixedRefresher {
    fn refresh(&mut self, request: CalendarRefreshRequest) -> CalendarRefreshResult {
        thread::sleep(self.delay);
        CalendarRefreshResult {
            generation: request.generation,
            widget_id: request.widget_id,
            fields: vec![
                Field {
                    key: "title".into(),
                    value: FieldValue::Text(request.title),
                },
                Field {
                    key: "stale".into(),
                    value: FieldValue::Boolean(false),
                },
            ],
            refreshed_at: Some(request.now),
            age: Some(Duration::ZERO),
            stale: false,
            error: None,
        }
    }
}

struct CountingRefresher {
    calls: Arc<AtomicU64>,
}

impl CalendarRefresher for CountingRefresher {
    fn refresh(&mut self, request: CalendarRefreshRequest) -> CalendarRefreshResult {
        self.calls.fetch_add(1, Ordering::Relaxed);
        FixedRefresher {
            delay: Duration::ZERO,
        }
        .refresh(request)
    }
}

#[derive(Default)]
struct LastGoodRefresher {
    last_success: Option<chrono::DateTime<Utc>>,
    calls: u64,
}

impl CalendarRefresher for LastGoodRefresher {
    fn refresh(&mut self, request: CalendarRefreshRequest) -> CalendarRefreshResult {
        self.calls += 1;
        if self.calls == 1 {
            self.last_success = Some(request.now);
            return FixedRefresher {
                delay: Duration::ZERO,
            }
            .refresh(request);
        }
        CalendarRefreshResult {
            generation: request.generation,
            widget_id: request.widget_id,
            fields: vec![
                Field {
                    key: "title".into(),
                    value: FieldValue::Text(request.title),
                },
                Field {
                    key: "stale".into(),
                    value: FieldValue::Boolean(true),
                },
                Field {
                    key: "error".into(),
                    value: FieldValue::Text("offline".into()),
                },
            ],
            refreshed_at: self.last_success,
            age: Some(Duration::from_mins(1)),
            stale: true,
            error: Some("offline".into()),
        }
    }
}

fn status(uptime_ms: u64) -> StatusResponse {
    StatusResponse {
        protocol_version: PROTOCOL_VERSION,
        firmware_version: "mock-m3".into(),
        uptime_ms,
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
    }
}

fn options() -> RuntimeOptions {
    RuntimeOptions {
        command_capacity: 4,
        provider_job_capacity: 1,
        maximum_subscribers: 2,
        command_timeout: Duration::from_secs(1),
        loop_maximum_wait: Duration::from_millis(2),
        reconnect_interval: Duration::from_millis(10),
        pomodoro_interval: Duration::from_millis(10),
        status_interval: Duration::from_millis(10),
        time_sync_interval: Duration::from_hours(1),
        provider_queue_retry: Duration::from_millis(5),
    }
}

fn full_config() -> AppConfig {
    serde_json::from_str(FULL_JSON).unwrap()
}

fn start_runtime(config: AppConfig, control: &MockDeviceControl, delay: Duration) -> RuntimeHandle {
    RuntimeHandle::start(
        config,
        Box::new(MockDevice::new(control.clone())),
        Box::new(FixedRefresher { delay }),
        options(),
    )
    .unwrap()
}

fn wait_for_snapshot(
    runtime: &RuntimeHandle,
    timeout: Duration,
    predicate: impl Fn(&app_core::AppSnapshot) -> bool,
) -> app_core::AppSnapshot {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = runtime.snapshot().unwrap();
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "timed out; latest={snapshot:?}");
        thread::sleep(Duration::from_millis(5));
    }
}

fn wait_for(timeout: Duration, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + timeout;
    while !predicate() {
        assert!(Instant::now() < deadline, "timed out waiting for condition");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn cold_boot_and_two_power_resets_replay_the_complete_owned_state() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
            && snapshot
                .providers
                .first()
                .is_some_and(|provider| provider.last_success_unix_ms.is_some())
    });

    let operations = control.operations();
    let layout = operations
        .iter()
        .position(|operation| *operation == Operation::ApplyLayout)
        .unwrap();
    let time = operations
        .iter()
        .position(|operation| *operation == Operation::TimeSync)
        .unwrap();
    let first_push = operations
        .iter()
        .position(|operation| matches!(operation, Operation::Push(_)))
        .unwrap();
    let activation = operations
        .iter()
        .position(|operation| matches!(operation, Operation::Activate(_)))
        .unwrap();
    assert!(time < layout && layout < first_push && first_push < activation);

    control.force_disconnect(false);
    wait_for(Duration::from_secs(1), || control.connection_count() >= 2);
    let retained = control.operations();
    assert_eq!(
        retained
            .iter()
            .filter(|operation| **operation == Operation::ApplyLayout)
            .count(),
        1
    );
    assert!(!retained.contains(&Operation::ReplayLayout));

    for expected_connections in 3..=4 {
        control.force_disconnect(true);
        wait_for(Duration::from_secs(1), || {
            control.connection_count() >= expected_connections
        });
    }
    let replayed = control.operations();
    assert_eq!(
        replayed
            .iter()
            .filter(|operation| **operation == Operation::ReplayLayout)
            .count(),
        2
    );
    assert!(
        replayed
            .iter()
            .filter(|operation| matches!(operation, Operation::ReplayPush(_)))
            .count()
            >= 6
    );
    runtime.shutdown().unwrap();
}

#[test]
fn provider_delay_does_not_block_commands_and_old_results_are_discarded() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control, Duration::from_millis(150));
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot
            .providers
            .first()
            .is_some_and(|provider| matches!(provider.state, app_core::ProviderState::Refreshing))
    });

    let started = Instant::now();
    runtime
        .control_pomodoro("pomodoro", PomodoroAction::Start)
        .unwrap();
    assert!(started.elapsed() < Duration::from_millis(100));
    runtime.apply_config(AppConfig::default()).unwrap();
    let snapshot = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.diagnostics.provider_results_discarded >= 1
    });
    assert!(snapshot.providers.is_empty());
    runtime.shutdown().unwrap();
}

#[test]
fn pomodoro_events_complete_once_and_dismissed_interrupts_do_not_replay() {
    let control = MockDeviceControl::default();
    let mut config = full_config();
    for widget in &mut config.widgets {
        if let WidgetSettings::Pomodoro {
            duration_seconds, ..
        } = widget
        {
            *duration_seconds = 1;
        }
    }
    let runtime = start_runtime(config, &control, Duration::ZERO);
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });
    control.push_event(DeviceEvent {
        sequence: 1,
        kind: EventKind::Tap,
        widget_id: "pomodoro".into(),
        screen_id: "pomodoro-screen".into(),
        action: EventAction::StartPause,
        interrupt_token: None,
    });
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot
            .pomodoros
            .first()
            .is_some_and(|pomodoro| pomodoro.state == PomodoroState::Running)
    });
    wait_for_snapshot(&runtime, Duration::from_secs(2), |snapshot| {
        snapshot
            .pomodoros
            .first()
            .is_some_and(|pomodoro| pomodoro.state == PomodoroState::Completed)
    });
    wait_for(Duration::from_secs(1), || {
        control.operations().contains(&Operation::Interrupt(1))
    });
    control.push_event(DeviceEvent {
        sequence: 2,
        kind: EventKind::InterruptDismissed,
        widget_id: "pomodoro".into(),
        screen_id: "pomodoro-screen".into(),
        action: EventAction::DismissInterrupt,
        interrupt_token: Some(1),
    });
    thread::sleep(Duration::from_millis(30));
    control.force_disconnect(true);
    wait_for(Duration::from_secs(1), || control.connection_count() >= 2);
    assert!(
        !control
            .operations()
            .contains(&Operation::ReplayInterrupt(1))
    );
    runtime.shutdown().unwrap();
}

#[test]
fn subscribers_are_bounded_and_coalesce_pressure_to_the_latest_snapshot() {
    let control = MockDeviceControl::default();
    let mut runtime_options = options();
    runtime_options.maximum_subscribers = 1;
    let runtime = RuntimeHandle::start(
        full_config(),
        Box::new(MockDevice::new(control)),
        Box::new(FixedRefresher {
            delay: Duration::ZERO,
        }),
        runtime_options,
    )
    .unwrap();
    let subscription = runtime.subscribe().unwrap();
    assert!(matches!(runtime.subscribe(), Err(RuntimeError::QueueFull)));
    runtime.set_paused(true).unwrap();
    runtime.set_paused(false).unwrap();
    runtime.activate_screen("clock-screen").unwrap();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.diagnostics.subscriber_snapshots_overwritten >= 1
            && snapshot.device.active_screen_id.as_deref() == Some("clock-screen")
    });
    let latest = subscription
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert_eq!(
        latest.device.active_screen_id.as_deref(),
        Some("clock-screen")
    );
    assert_eq!(latest.runtime, RuntimeState::Running);
    runtime.shutdown().unwrap();
}

#[test]
fn pause_defers_timer_pushes_and_provider_refresh_until_resume() {
    let control = MockDeviceControl::default();
    let calls = Arc::new(AtomicU64::new(0));
    let runtime = RuntimeHandle::start(
        full_config(),
        Box::new(MockDevice::new(control.clone())),
        Box::new(CountingRefresher {
            calls: Arc::clone(&calls),
        }),
        options(),
    )
    .unwrap();
    wait_for(Duration::from_secs(1), || {
        calls.load(Ordering::Relaxed) >= 1
    });
    let pushes_before = control
        .operations()
        .iter()
        .filter(|operation| **operation == Operation::Push("pomodoro".into()))
        .count();

    runtime.set_paused(true).unwrap();
    runtime
        .control_pomodoro("pomodoro", PomodoroAction::Start)
        .unwrap();
    runtime.refresh_provider("calendar").unwrap();
    thread::sleep(Duration::from_millis(40));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        control
            .operations()
            .iter()
            .filter(|operation| **operation == Operation::Push("pomodoro".into()))
            .count(),
        pushes_before
    );
    assert_eq!(runtime.snapshot().unwrap().runtime, RuntimeState::Paused);

    runtime.set_paused(false).unwrap();
    wait_for(Duration::from_secs(1), || {
        calls.load(Ordering::Relaxed) >= 2
    });
    wait_for(Duration::from_secs(1), || {
        control
            .operations()
            .iter()
            .filter(|operation| **operation == Operation::Push("pomodoro".into()))
            .count()
            > pushes_before
    });
    runtime.shutdown().unwrap();
}

#[test]
fn provider_failure_keeps_last_success_and_projects_stale_state() {
    let control = MockDeviceControl::default();
    let runtime = RuntimeHandle::start(
        full_config(),
        Box::new(MockDevice::new(control)),
        Box::<LastGoodRefresher>::default(),
        options(),
    )
    .unwrap();
    let fresh = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot
            .providers
            .first()
            .is_some_and(|provider| matches!(provider.state, app_core::ProviderState::Fresh))
    });
    let last_success = fresh.providers[0].last_success_unix_ms;
    runtime.refresh_provider("calendar").unwrap();
    let stale = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot
            .providers
            .first()
            .is_some_and(|provider| matches!(provider.state, app_core::ProviderState::Stale { .. }))
    });
    assert_eq!(stale.providers[0].last_success_unix_ms, last_success);
    assert_eq!(stale.providers[0].age_seconds, Some(60));
    runtime.shutdown().unwrap();
}

#[test]
fn command_queue_rejects_pressure_without_growing() {
    let control = MockDeviceControl::default();
    let mut runtime_options = options();
    runtime_options.command_capacity = 1;
    let runtime = Arc::new(
        RuntimeHandle::start(
            full_config(),
            Box::new(MockDevice::new(control.clone())),
            Box::new(FixedRefresher {
                delay: Duration::ZERO,
            }),
            runtime_options,
        )
        .unwrap(),
    );
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });
    let gate = control.block_next_push();
    let first_runtime = Arc::clone(&runtime);
    let first =
        thread::spawn(move || first_runtime.control_pomodoro("pomodoro", PomodoroAction::Start));
    gate.wait_until_entered();
    let second_runtime = Arc::clone(&runtime);
    let second = thread::spawn(move || second_runtime.set_paused(true));
    thread::sleep(Duration::from_millis(20));
    assert_eq!(
        runtime.activate_screen("clock-screen"),
        Err(RuntimeError::QueueFull)
    );
    gate.open();
    first.join().unwrap().unwrap();
    second.join().unwrap().unwrap();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.diagnostics.command_queue_full >= 1
    });
    runtime.shutdown().unwrap();
}

#[test]
fn invalid_commands_do_not_mutate_runtime_state() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(AppConfig::default(), &control, Duration::ZERO);
    let before = runtime.snapshot().unwrap().config;
    assert!(matches!(
        runtime.activate_screen("missing"),
        Err(RuntimeError::UnknownScreen { .. })
    ));
    assert!(matches!(
        runtime.control_pomodoro("missing", PomodoroAction::Reset),
        Err(RuntimeError::UnknownWidget { .. })
    ));
    assert_eq!(runtime.snapshot().unwrap().config, before);
    runtime.shutdown().unwrap();
}
