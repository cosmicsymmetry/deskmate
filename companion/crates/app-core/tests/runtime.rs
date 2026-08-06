use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use app_core::{
    AlertHold, AppConfig, CalendarRefreshRequest, CalendarRefreshResult, CalendarRefresher,
    CardAlert, CardField, CardFieldValue, CardPresence, CardSettings, CarouselAdvance,
    CarouselSettings, ConnectionState, DeviceConnection, DisplayOrientation, DisplayTemplate,
    PersistenceState, PomodoroAction, PomodoroState, ProviderRequest, RefreshPolicy, RuntimeDevice,
    RuntimeError, RuntimeHandle, RuntimeOptions, RuntimeState, WidgetTapAction,
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
    ApplyLayout(u16),
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
    latest_interrupt_token: u32,
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

    fn set_latest_interrupt_token(&self, token: u32) {
        self.state.lock().unwrap().latest_interrupt_token = token;
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
        let mut device_status = status(uptime_ms);
        device_status.latest_interrupt_token = state.latest_interrupt_token;
        Ok(DeviceConnection {
            port_name: "mock-usb".into(),
            status: device_status,
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
        let mut device_status = status(state.connection_count * 1_000 + 100);
        device_status.latest_interrupt_token = state.latest_interrupt_token;
        Ok(device_status)
    }

    fn time_sync(&mut self, _sync: TimeSync) -> Result<(), DeviceError> {
        self.with_connected(|state| {
            state.replay.time = true;
            state.operations.push(Operation::TimeSync);
        })
    }

    fn apply_layout(
        &mut self,
        rotation: u16,
        _widgets: Vec<WidgetConfig>,
        _screens: Vec<ScreenConfig>,
    ) -> Result<(), DeviceError> {
        self.with_connected(|state| {
            state.replay.layout = true;
            state.replay.pushes.clear();
            state.replay.interrupts.clear();
            state.operations.push(Operation::ApplyLayout(rotation));
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
            state.latest_interrupt_token = state.latest_interrupt_token.max(interrupt.token);
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
        if event.event.kind == EventKind::Navigation
            && matches!(
                event.event.action,
                EventAction::NavigatePrevious | EventAction::NavigateNext
            )
        {
            state.replay.active_screen = Some(event.event.screen_id.clone());
        }
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

struct MultiProviderRefresher {
    calls: Arc<Mutex<Vec<(String, &'static str)>>>,
    delay: Duration,
}

impl CalendarRefresher for MultiProviderRefresher {
    fn refresh(&mut self, request: CalendarRefreshRequest) -> CalendarRefreshResult {
        let kind = match &request.provider {
            ProviderRequest::Calendar { .. } => "calendar",
            ProviderRequest::Weather { .. } => "weather",
            ProviderRequest::JsonFeed { .. } => "json-feed",
            ProviderRequest::Rss { .. } => "rss",
        };
        self.calls
            .lock()
            .unwrap()
            .push((request.widget_id.clone(), kind));
        thread::sleep(self.delay);
        if kind == "rss" {
            return CalendarRefreshResult {
                generation: request.generation,
                widget_id: request.widget_id,
                fields: vec![Field {
                    key: "title".into(),
                    value: FieldValue::Text(request.title),
                }],
                refreshed_at: None,
                age: None,
                stale: true,
                error: Some("malformed provider data: fixture failure".into()),
            };
        }
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
        max_protocol_version: protocol::MAX_PROTOCOL_VERSION,
        capabilities: protocol::CURRENT_CAPABILITIES,
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
        latest_interrupt_token: 0,
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

fn multi_provider_config() -> AppConfig {
    let mut config = full_config();
    config.cards.push(
        serde_json::from_value(serde_json::json!({
            "kind": "json-feed",
            "id": "json",
            "title": "JSON",
            "url": "https://example.test/metrics.json",
            "mappings": [
                { "field": "row0_title", "path": "$.headline" },
                { "field": "row0_time", "path": "$.time" }
            ],
            "template": { "kind": "row-list" },
            "tap_action": { "kind": "none" },
            "refresh": { "kind": "manual" },
            "presence": { "kind": "in-rotation", "dwell_seconds": null },
            "alert": { "kind": "none" }
        }))
        .unwrap(),
    );
    config.cards.push(
        serde_json::from_value(serde_json::json!({
            "kind": "rss",
            "id": "rss",
            "title": "News",
            "url": "https://example.test/feed.xml",
            "max_items": 5,
            "template": { "kind": "row-list" },
            "tap_action": { "kind": "none" },
            "refresh": { "kind": "manual" },
            "presence": { "kind": "in-rotation", "dwell_seconds": null },
            "alert": { "kind": "none" }
        }))
        .unwrap(),
    );
    config
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
        .position(|operation| *operation == Operation::ApplyLayout(270))
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
            .filter(|operation| **operation == Operation::ApplyLayout(270))
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
fn local_navigation_becomes_the_authoritative_screen_for_reset_replay() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });

    control.push_event(DeviceEvent {
        sequence: 1,
        kind: EventKind::Navigation,
        widget_id: "calendar".into(),
        screen_id: "calendar".into(),
        action: EventAction::NavigateNext,
        interrupt_token: None,
    });
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.active_screen_id.as_deref() == Some("calendar")
    });

    control.force_disconnect(true);
    wait_for(Duration::from_secs(1), || control.connection_count() >= 2);
    wait_for(Duration::from_secs(1), || {
        control
            .operations()
            .iter()
            .rev()
            .any(|operation| *operation == Operation::ReplayActivate("calendar".into()))
    });
    let last_replay_activation = control
        .operations()
        .into_iter()
        .rev()
        .find(|operation| matches!(operation, Operation::ReplayActivate(_)))
        .unwrap();
    assert_eq!(
        last_replay_activation,
        Operation::ReplayActivate("calendar".into())
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
    for widget in &mut config.cards {
        if let CardSettings::Pomodoro {
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
        screen_id: "pomodoro".into(),
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
        screen_id: "pomodoro".into(),
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

// The one wall-clock proof (all other alert-trigger/hold logic is covered by
// instant unit tests in `app-core`'s inline `mod tests`, which call the
// private trigger/eligibility/hold helpers directly) that a bounded
// `AlertHold::Seconds` deadline actually reaches a real device through the
// full `run_runtime` loop: the interrupt fires once, then — with no touch
// dismissal from the device — the hold expires on its own, the host dismisses
// it from its own arbiter, and re-sends `ActivateScreen` for the saved
// carousel screen. `full_config`'s carousel is `Manual`, so the only source
// of a *second* activation here is the hold expiring, not rotation.
//
// This test is named for exactly that and no more: it does **not** prove the
// physical panel actually leaves the interrupt overlay. Per
// `docs/protocol/v1.md`'s `ActivateScreen` section and firmware's
// `protocol_task.c` (`show_carousel_screen` is skipped whenever
// `interrupt_state_active` is non-null), the real device keeps showing the
// interrupt until a tap dismisses it — `ActivateScreen` only changes which
// screen is *saved* to restore to afterward. `MockDevice` in this file has no
// interrupt-takeover model at all: it just records that `ActivateScreen` was
// called. So this test is wiring-only — it exercises the scheduler's
// `alert_hold_due` → `InterruptArbiter::dismiss` → `active_screen_dirty` →
// `send_screen` path through a real threaded runtime loop — not a claim about
// on-device visual behavior. See `arm_alert_hold`'s doc comment in
// `runtime.rs` for the full protocol-v1 limitation this bounded hold lives
// under.
#[test]
fn a_bounded_alert_hold_re_sends_the_saved_carousel_screen_activation() {
    let control = MockDeviceControl::default();
    let mut config = full_config();
    for card in &mut config.cards {
        if let CardSettings::Pomodoro {
            duration_seconds,
            alert,
            ..
        } = card
        {
            *duration_seconds = 1;
            *alert = CardAlert::OnTimerFinish {
                hold: AlertHold::Seconds { value: 5 },
            };
        }
    }
    let runtime = start_runtime(config, &control, Duration::ZERO);
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });
    let activations_before_completion = activated_screen_ids(&control).len();
    control.push_event(DeviceEvent {
        sequence: 1,
        kind: EventKind::Tap,
        widget_id: "pomodoro".into(),
        screen_id: "pomodoro".into(),
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
    // No spontaneous activations from the manual-advance carousel between
    // completion and the interrupt firing.
    assert_eq!(
        activated_screen_ids(&control).len(),
        activations_before_completion
    );

    // No dismissal event from the device: the 5s hold must expire on its own
    // and re-send `ActivateScreen`, which is the only other source of a new
    // activation under `CarouselAdvance::Manual`.
    wait_for(Duration::from_secs(7), || {
        activated_screen_ids(&control).len() > activations_before_completion
    });
    runtime.shutdown().unwrap();
}

#[test]
fn pomodoro_completion_without_an_alert_does_not_schedule_an_interrupt() {
    let control = MockDeviceControl::default();
    let mut config = full_config();
    for card in &mut config.cards {
        if let CardSettings::Pomodoro {
            duration_seconds,
            alert,
            ..
        } = card
        {
            *duration_seconds = 1;
            *alert = CardAlert::None;
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
        screen_id: "pomodoro".into(),
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
    // Give the worker plenty of time to have scheduled an interrupt if the gate were
    // missing or inverted, then confirm it never did.
    thread::sleep(Duration::from_millis(200));
    assert!(
        !control
            .operations()
            .iter()
            .any(|operation| matches!(operation, Operation::Interrupt(_))),
        "a card with alert: none must never trigger a host interrupt on completion"
    );
    runtime.shutdown().unwrap();
}

#[test]
fn app_restart_seeds_interrupt_tokens_from_the_still_powered_device() {
    let control = MockDeviceControl::default();
    control.set_latest_interrupt_token(40);
    let mut config = full_config();
    for widget in &mut config.cards {
        if let CardSettings::Pomodoro {
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
        screen_id: "pomodoro".into(),
        action: EventAction::StartPause,
        interrupt_token: None,
    });
    wait_for(Duration::from_secs(2), || {
        control.operations().contains(&Operation::Interrupt(41))
    });
    assert_eq!(runtime.snapshot().unwrap().runtime, RuntimeState::Running);
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
    runtime.activate_screen("clock").unwrap();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.diagnostics.subscriber_snapshots_overwritten >= 1
            && snapshot.device.active_screen_id.as_deref() == Some("clock")
    });
    let latest = subscription
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert_eq!(latest.device.active_screen_id.as_deref(), Some("clock"));
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
        runtime.activate_screen("clock"),
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
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });
    let before = runtime.snapshot().unwrap().config;
    let apply_count = control
        .operations()
        .iter()
        .filter(|operation| matches!(operation, Operation::ApplyLayout(_)))
        .count();
    assert!(matches!(
        runtime.activate_screen("missing"),
        Err(RuntimeError::UnknownScreen { .. })
    ));
    assert!(matches!(
        runtime.control_pomodoro("missing", PomodoroAction::Reset),
        Err(RuntimeError::UnknownWidget { .. })
    ));
    let unsupported: AppConfig =
        serde_json::from_str(include_str!("fixtures/card-surface.json")).unwrap();
    assert!(matches!(
        runtime.apply_config(unsupported),
        Err(RuntimeError::InvalidConfig { issues })
            if issues.iter().all(|issue| issue.code == app_core::ValidationCode::RequiresCapability)
    ));
    assert_eq!(runtime.snapshot().unwrap().config, before);
    assert_eq!(
        control
            .operations()
            .iter()
            .filter(|operation| matches!(operation, Operation::ApplyLayout(_)))
            .count(),
        apply_count
    );
    runtime.shutdown().unwrap();
}

#[test]
fn manual_refresh_during_in_flight_work_is_coalesced() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control, Duration::from_millis(120));
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot
            .providers
            .first()
            .is_some_and(|provider| matches!(provider.state, app_core::ProviderState::Refreshing))
    });

    runtime.refresh_provider("calendar").unwrap();
    runtime.refresh_provider("calendar").unwrap();
    let fresh = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot
            .providers
            .first()
            .is_some_and(|provider| matches!(provider.state, app_core::ProviderState::Fresh))
    });
    thread::sleep(Duration::from_millis(40));
    assert_eq!(fresh.diagnostics.provider_jobs_started, 1);
    assert_eq!(
        runtime
            .snapshot()
            .unwrap()
            .diagnostics
            .provider_jobs_started,
        1
    );
    runtime.shutdown().unwrap();
}

#[test]
fn all_provider_kinds_share_bounded_scheduling_and_fail_independently() {
    let control = MockDeviceControl::default();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let runtime = RuntimeHandle::start(
        multi_provider_config(),
        Box::new(MockDevice::new(control.clone())),
        Box::new(MultiProviderRefresher {
            calls: Arc::clone(&calls),
            delay: Duration::from_millis(40),
        }),
        options(),
    )
    .unwrap();

    let snapshot = wait_for_snapshot(&runtime, Duration::from_secs(2), |snapshot| {
        snapshot.providers.len() == 3
            && snapshot.providers.iter().all(|provider| {
                matches!(
                    provider.state,
                    app_core::ProviderState::Fresh | app_core::ProviderState::Error { .. }
                )
            })
    });
    assert!(snapshot.providers.iter().any(|provider| {
        provider.widget_id == "rss"
            && matches!(provider.state, app_core::ProviderState::Error { .. })
    }));
    assert!(
        snapshot
            .providers
            .iter()
            .filter(|provider| { matches!(provider.state, app_core::ProviderState::Fresh) })
            .count()
            >= 2
    );
    assert!(snapshot.diagnostics.provider_queue_full >= 1);
    assert!(
        control
            .operations()
            .iter()
            .filter(|operation| **operation == Operation::Status)
            .count()
            >= 2,
        "provider work blocked device keepalives"
    );

    let initial_calls = calls.lock().unwrap().clone();
    assert!(initial_calls.contains(&("calendar".into(), "calendar")));
    assert!(initial_calls.contains(&("json".into(), "json-feed")));
    assert!(initial_calls.contains(&("rss".into(), "rss")));
    assert_eq!(initial_calls.len(), 3);

    runtime.refresh_provider("json").unwrap();
    runtime.refresh_provider("json").unwrap();
    wait_for(Duration::from_secs(1), || calls.lock().unwrap().len() == 4);
    thread::sleep(Duration::from_millis(80));
    assert_eq!(calls.lock().unwrap().len(), 4);
    assert_eq!(calls.lock().unwrap()[3], ("json".into(), "json-feed"));
    runtime.shutdown().unwrap();
}

#[test]
fn unrelated_config_and_preference_edits_preserve_live_timer_and_screen() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });
    runtime
        .control_pomodoro("pomodoro", PomodoroAction::Start)
        .unwrap();
    runtime.activate_screen("calendar").unwrap();
    runtime.set_autostart_preference(false).unwrap();
    let before = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot
            .pomodoros
            .first()
            .is_some_and(|timer| timer.state == PomodoroState::Running)
            && snapshot.device.active_screen_id.as_deref() == Some("calendar")
    });

    let mut edited = before.config;
    let clock = edited
        .cards
        .iter_mut()
        .find(|widget| matches!(widget, CardSettings::Clock { .. }))
        .unwrap();
    if let CardSettings::Clock { title, .. } = clock {
        *title = "Studio".into();
    }
    edited.preferences.orientation = DisplayOrientation::Landscape;
    runtime.apply_config(edited).unwrap();

    let after =
        wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
            snapshot.config.cards.iter().any(
                |widget| matches!(widget, CardSettings::Clock { title, .. } if title == "Studio"),
            )
        });
    assert!(
        after
            .pomodoros
            .first()
            .is_some_and(|timer| timer.state == PomodoroState::Running)
    );
    assert_eq!(after.device.active_screen_id.as_deref(), Some("calendar"));
    assert!(control.operations().contains(&Operation::ApplyLayout(90)));
    runtime.shutdown().unwrap();
}

#[test]
fn persistence_recovery_is_projected_and_cleared_after_a_good_save() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(AppConfig::default(), &control, Duration::ZERO);
    runtime
        .set_persistence_state(PersistenceState::RecoverableError {
            message: "invalid config JSON".into(),
        })
        .unwrap();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        matches!(
            snapshot.persistence,
            PersistenceState::RecoverableError { .. }
        )
    });
    runtime
        .set_persistence_state(PersistenceState::Saving)
        .unwrap();
    runtime
        .set_persistence_state(PersistenceState::Clean)
        .unwrap();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.persistence == PersistenceState::Clean
    });
    runtime.shutdown().unwrap();
}

#[test]
fn app_restart_intentionally_resets_transient_timer_state_to_idle() {
    let config = full_config();
    let first_control = MockDeviceControl::default();
    let first = start_runtime(config.clone(), &first_control, Duration::ZERO);
    first
        .control_pomodoro("pomodoro", PomodoroAction::Start)
        .unwrap();
    wait_for_snapshot(&first, Duration::from_secs(1), |snapshot| {
        snapshot
            .pomodoros
            .first()
            .is_some_and(|timer| timer.state == PomodoroState::Running)
    });
    first.shutdown().unwrap();

    let second_control = MockDeviceControl::default();
    let second = start_runtime(config, &second_control, Duration::ZERO);
    let restarted = wait_for_snapshot(&second, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });
    let timer = restarted.pomodoros.first().unwrap();
    assert_eq!(timer.state, PomodoroState::Idle);
    assert_eq!(timer.remaining_seconds, timer.duration_seconds);
    second.shutdown().unwrap();
}

fn clock_card(id: &str, presence: CardPresence) -> CardSettings {
    CardSettings::Clock {
        id: id.into(),
        title: id.into(),
        show_seconds: true,
        template: DisplayTemplate::DigitalClock,
        tap_action: WidgetTapAction::None,
        refresh: RefreshPolicy::DeviceLocal,
        presence,
        alert: CardAlert::None,
    }
}

fn pomodoro_card(id: &str, presence: CardPresence, alert: CardAlert) -> CardSettings {
    CardSettings::Pomodoro {
        id: id.into(),
        label: id.into(),
        duration_seconds: 60,
        template: DisplayTemplate::ProgressRing,
        tap_action: WidgetTapAction::None,
        refresh: RefreshPolicy::DeviceLocal,
        presence,
        alert,
    }
}

/// The screen IDs sent to the device via `ActivateScreen`, in the order they
/// were sent. The very first entry is always the initial full-sync
/// activation of the boot-time active screen, issued on connect regardless of
/// carousel mode; rotation-triggered activations (if any) follow it.
fn activated_screen_ids(control: &MockDeviceControl) -> Vec<String> {
    control
        .operations()
        .into_iter()
        .filter_map(|operation| match operation {
            Operation::Activate(screen_id) => Some(screen_id),
            _ => None,
        })
        .collect()
}

// Rotation's pure logic (per-card dwell resolution, manual-mode disarming, in-rotation
// ordering/skipping/wrap-around, and the swipe/IPC index-resolution edge cases) is
// covered by instant unit tests in `companion/crates/app-core/src/runtime.rs`'s inline
// `mod tests`, which call `current_dwell`, `rotation_card_ids`, `advance_rotation`,
// `drain_device_events`, and `process_command` directly with synthetic `Instant`s — no
// sleeping required. This file keeps exactly one real-time rotation test: an end-to-end
// wiring proof that `scheduler.rotation_due` firing inside the real `run_runtime` loop
// actually reaches the mock device via `ActivateScreen`, through
// `advance_rotation` -> `active_screen_dirty` -> `send_screen`. One dwell period (the
// validated minimum, 5s) is enough to prove the wiring; it does not re-prove ordering or
// skipping, which the unit tests already pin.
#[test]
fn timed_advance_wiring_reaches_the_device_after_one_dwell() {
    let control = MockDeviceControl::default();
    let config = AppConfig {
        carousel: CarouselSettings {
            advance: CarouselAdvance::Timed {
                default_dwell_seconds: 5,
            },
        },
        cards: vec![
            clock_card(
                "first",
                CardPresence::InRotation {
                    dwell_seconds: Some(5),
                },
            ),
            pomodoro_card(
                "alerting",
                CardPresence::AlertOnly,
                CardAlert::OnTimerFinish {
                    hold: AlertHold::UntilDismissed,
                },
            ),
            clock_card(
                "second",
                CardPresence::InRotation {
                    dwell_seconds: Some(5),
                },
            ),
            clock_card("muted", CardPresence::Off),
        ],
        ..AppConfig::default()
    };
    let runtime = start_runtime(config, &control, Duration::ZERO);
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });
    // The initial full sync activates the first in-rotation card ("first"),
    // independent of rotation.
    wait_for(Duration::from_secs(1), || {
        activated_screen_ids(&control) == ["first"]
    });

    // One dwell later, rotation has walked to "second" (skipping the alert-only and
    // off cards) and the activation reached the mock device.
    wait_for(Duration::from_secs(8), || {
        activated_screen_ids(&control) == ["first", "second"]
    });
    runtime.shutdown().unwrap();
}

/// Proves the `WorkerState::latest_fields` -> `AppSnapshot.card_data` wiring
/// (runtime.rs's `snapshot` method) by driving a real `RuntimeHandle` and
/// reading `card_data` back off `RuntimeHandle::snapshot()`, the same way
/// `all_provider_kinds_share_bounded_scheduling_and_fail_independently`
/// reads `providers`/`diagnostics`. A pomodoro card pushes its fields into
/// `latest_fields` synchronously at config-install time (no device
/// connection or provider round trip needed), which makes the live card's
/// exact field values available on the very first snapshot. A second,
/// otherwise-identical pomodoro card with `CardPresence::Off` is included to
/// prove an off card contributes no entry at all — a stale entry would
/// otherwise render in the settings preview as if it were live.
#[test]
fn card_data_carries_a_live_cards_exact_field_values_and_excludes_off_cards() {
    let control = MockDeviceControl::default();
    let config = AppConfig {
        cards: vec![
            pomodoro_card(
                "on",
                CardPresence::InRotation {
                    dwell_seconds: None,
                },
                CardAlert::None,
            ),
            pomodoro_card("off", CardPresence::Off, CardAlert::None),
        ],
        ..AppConfig::default()
    };
    let runtime = start_runtime(config, &control, Duration::ZERO);

    let snapshot = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.card_data.iter().any(|card| card.card_id == "on")
    });

    let on_card = snapshot
        .card_data
        .iter()
        .find(|card| card.card_id == "on")
        .expect("the live pomodoro card's fields must reach the snapshot");
    assert!(
        on_card.fields.contains(&CardField {
            key: "label".into(),
            value: CardFieldValue::Text { value: "on".into() },
        }),
        "expected the pomodoro's label field in card_data, got {:?}",
        on_card.fields
    );
    assert!(
        on_card.fields.contains(&CardField {
            key: "duration_seconds".into(),
            value: CardFieldValue::Integer { value: 60 },
        }),
        "expected the pomodoro's duration_seconds field in card_data, got {:?}",
        on_card.fields
    );

    assert!(
        !snapshot.card_data.iter().any(|card| card.card_id == "off"),
        "an off-presence card must not contribute an entry to card_data"
    );

    runtime.shutdown().unwrap();
}
