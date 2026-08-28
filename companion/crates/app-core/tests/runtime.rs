use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use app_core::{
    AlertHold, AppConfig, CalendarRefreshRequest, CalendarRefreshResult, CalendarRefresher,
    CardAlert, CardErrorKind, CardField, CardFieldValue, CardSettings, CarouselAdvance,
    ConnectionState, DeviceCapability, DeviceConnection, DeviceOtaState, DeviceTier,
    DeviceWifiState, DisplayOrientation, DisplayTemplate, NetworkConfig, PersistenceState,
    Playlist, PlaylistEntry, PomodoroAction, PomodoroState, ProviderRequest, ProvisioningTier,
    RefreshPolicy, RuntimeDevice, RuntimeError, RuntimeHandle, RuntimeOptions, RuntimeState,
    WidgetTapAction,
};
use chrono::Utc;
use device::{DeviceError, ReceivedEvent, SessionDiagnostics, TransportError};
use protocol::{
    Ack, AssetBegin, AssetChunk, AssetCommit, AssetRelease, DeviceEvent, ErrorCode, ErrorResponse,
    EventAction, EventKind, Field, FieldValue, PROTOCOL_VERSION, PushScene, Scene, SceneNode,
    SceneValue, ScreenConfig, StatusResponse, TimeSync, TriggerInterrupt, WidgetConfig,
};

const FULL_JSON: &str = include_str!("fixtures/full.json");

#[derive(Debug, Clone, PartialEq, Eq)]
enum Operation {
    Connect,
    Provision,
    FactoryReset,
    Status,
    TimeSync,
    ApplyLayout(u16),
    Push(String),
    PushScene(PushScene),
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

#[derive(Default, PartialEq, Eq)]
enum MockPower {
    #[default]
    Powered,
    Unpowered,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InjectedDisconnect {
    Status,
    Provision,
}

#[derive(Default)]
struct MockState {
    connected: bool,
    power: MockPower,
    connection_count: u64,
    provision_attempts: u64,
    injected_disconnect: Option<InjectedDisconnect>,
    reset_on_connect: bool,
    operations: Vec<Operation>,
    events: VecDeque<ReceivedEvent>,
    replay: ReplayCache,
    latest_interrupt_token: u32,
    status_override: Option<StatusResponse>,
    next_push_gate: Option<Arc<PushGate>>,
    next_scene_gate: Option<Arc<PushGate>>,
    next_apply_layout_error: Option<DeviceError>,
    scene_errors: VecDeque<DeviceError>,
    /// Widgets whose pushes the device understands and refuses, exactly as real
    /// firmware does for a field the widget's template does not declare.
    refused_pushes: BTreeSet<String>,
    /// Cards whose scene the device understands but cannot render exactly.
    refused_scenes: BTreeSet<String>,
    /// Simulates the tier changing between the last status response and a host sync.
    /// The rejection also changes later status responses to Networked, as hardware
    /// does after accepting provisioning and rebooting into server ownership.
    reject_time_sync_as_wrong_tier: bool,
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
        state.injected_disconnect = Some(InjectedDisconnect::Status);
        state.reset_on_connect = power_reset;
    }

    fn power_off(&self) {
        let mut state = self.state.lock().unwrap();
        state.connected = false;
        state.power = MockPower::Unpowered;
    }

    fn power_on(&self) {
        let mut state = self.state.lock().unwrap();
        state.power = MockPower::Powered;
        state.reset_on_connect = true;
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

    fn provision_attempts(&self) -> u64 {
        self.state.lock().unwrap().provision_attempts
    }

    fn disconnect_during_next_provision(&self) {
        self.state.lock().unwrap().injected_disconnect = Some(InjectedDisconnect::Provision);
    }

    fn block_next_push(&self) -> Arc<PushGate> {
        let gate = Arc::new(PushGate::default());
        self.state.lock().unwrap().next_push_gate = Some(Arc::clone(&gate));
        gate
    }

    fn block_next_scene(&self) -> Arc<PushGate> {
        let gate = Arc::new(PushGate::default());
        self.state.lock().unwrap().next_scene_gate = Some(Arc::clone(&gate));
        gate
    }

    fn fail_next_apply_layout_with(&self, error: DeviceError) {
        self.state.lock().unwrap().next_apply_layout_error = Some(error);
    }

    fn fail_next_scene_with(&self, error: DeviceError) {
        self.state.lock().unwrap().scene_errors.push_back(error);
    }

    fn set_latest_interrupt_token(&self, token: u32) {
        self.state.lock().unwrap().latest_interrupt_token = token;
    }

    fn set_status(&self, status: StatusResponse) {
        self.state.lock().unwrap().status_override = Some(status);
    }

    fn refuse_pushes_for(&self, widget_id: &str) {
        self.state
            .lock()
            .unwrap()
            .refused_pushes
            .insert(widget_id.to_owned());
    }

    fn refuse_scenes_for(&self, card_id: &str) {
        self.state
            .lock()
            .unwrap()
            .refused_scenes
            .insert(card_id.to_owned());
    }

    fn reject_time_sync_as_wrong_tier(&self) {
        self.state.lock().unwrap().reject_time_sync_as_wrong_tier = true;
    }

    fn allow_time_sync(&self) {
        self.state.lock().unwrap().reject_time_sync_as_wrong_tier = false;
    }

    fn time_sync_attempts(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .operations
            .iter()
            .filter(|operation| **operation == Operation::TimeSync)
            .count()
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
        if state.power == MockPower::Unpowered {
            return Err(DeviceError::Transport(TransportError::Disconnected));
        }
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
        let mut device_status = state
            .status_override
            .clone()
            .unwrap_or_else(|| status(uptime_ms));
        device_status.uptime_ms = uptime_ms;
        device_status.latest_interrupt_token = state.latest_interrupt_token;
        Ok(DeviceConnection {
            port_name: "mock-usb".into(),
            status: device_status,
        })
    }

    fn status(&mut self) -> Result<StatusResponse, DeviceError> {
        let mut state = self.control.state.lock().unwrap();
        if state.injected_disconnect == Some(InjectedDisconnect::Status) {
            state.injected_disconnect = None;
            state.connected = false;
            return Err(DeviceError::Transport(TransportError::Disconnected));
        }
        if !state.connected {
            return Err(DeviceError::Transport(TransportError::Disconnected));
        }
        state.operations.push(Operation::Status);
        let uptime_ms = state.connection_count * 1_000 + 100;
        let mut device_status = state
            .status_override
            .clone()
            .unwrap_or_else(|| status(uptime_ms));
        device_status.uptime_ms = uptime_ms;
        device_status.latest_interrupt_token = state.latest_interrupt_token;
        Ok(device_status)
    }

    fn provision(&mut self, _config: &NetworkConfig) -> Result<(), DeviceError> {
        let mut state = self.control.state.lock().unwrap();
        state.provision_attempts += 1;
        if state.injected_disconnect == Some(InjectedDisconnect::Provision) {
            state.injected_disconnect = None;
            state.connected = false;
            return Err(DeviceError::Transport(TransportError::Disconnected));
        }
        if !state.connected {
            return Err(DeviceError::Transport(TransportError::Disconnected));
        }
        state.operations.push(Operation::Provision);
        Ok(())
    }

    fn factory_reset(&mut self) -> Result<(), DeviceError> {
        self.with_connected(|state| state.operations.push(Operation::FactoryReset))
    }

    fn time_sync(&mut self, _sync: TimeSync) -> Result<(), DeviceError> {
        self.with_connected(|state| {
            state.operations.push(Operation::TimeSync);
            if state.reject_time_sync_as_wrong_tier {
                let mut device_status = state
                    .status_override
                    .clone()
                    .unwrap_or_else(|| status(state.connection_count * 1_000 + 100));
                device_status.tier = protocol::Tier::Networked;
                state.status_override = Some(device_status);
                return Err(DeviceError::Rejected(ErrorResponse {
                    code: ErrorCode::WrongTier,
                    diagnostic: "server owns this device".into(),
                }));
            }
            state.replay.time = true;
            Ok(())
        })?
    }

    fn apply_layout(
        &mut self,
        rotation: u16,
        _widgets: Vec<WidgetConfig>,
        _screens: Vec<ScreenConfig>,
    ) -> Result<(), DeviceError> {
        if let Some(error) = self.with_connected(|state| state.next_apply_layout_error.take())? {
            return Err(error);
        }
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
            state.operations.push(Operation::Push(widget_id.clone()));
            if state.refused_pushes.contains(&widget_id) {
                return Err(DeviceError::Rejected(ErrorResponse {
                    code: ErrorCode::InvalidPayload,
                    diagnostic: "invalid push data".into(),
                }));
            }
            state.replay.pushes.insert(widget_id, fields);
            Ok(())
        })?
    }

    fn activate_screen(&mut self, screen_id: String) -> Result<(), DeviceError> {
        self.with_connected(|state| {
            state.replay.active_screen = Some(screen_id.clone());
            state.operations.push(Operation::Activate(screen_id));
        })
    }

    fn push_scene(&mut self, push: PushScene) -> Result<(), DeviceError> {
        let gate = self.with_connected(|state| state.next_scene_gate.take())?;
        if let Some(gate) = gate {
            gate.enter_and_wait();
        }
        self.with_connected(|state| {
            state.operations.push(Operation::PushScene(push.clone()));
            if let Some(error) = state.scene_errors.pop_front() {
                return Err(error);
            }
            if state.refused_scenes.contains(&push.card_id) {
                return Err(DeviceError::Rejected(ErrorResponse {
                    code: ErrorCode::InvalidPayload,
                    diagnostic: "scene could not be rendered".into(),
                }));
            }
            Ok(())
        })?
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

    fn send_asset_begin(&mut self, _begin: AssetBegin) -> Result<Ack, DeviceError> {
        self.with_connected(|_state| Ack {
            acknowledged_type: protocol::TYPE_ASSET_BEGIN,
            revision: None,
            already_present: Some(false),
        })
    }

    fn send_asset_chunk(&mut self, _chunk: AssetChunk) -> Result<(), DeviceError> {
        self.with_connected(|_state| ())
    }

    fn send_asset_commit(&mut self, _commit: AssetCommit) -> Result<(), DeviceError> {
        self.with_connected(|_state| ())
    }

    fn send_asset_release(&mut self, _release: AssetRelease) -> Result<(), DeviceError> {
        self.with_connected(|_state| ())
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
        tier: protocol::Tier::Local,
        wifi_state: protocol::WifiState::Down,
        wifi_rssi: 0,
        ip: String::new(),
        ota_state: protocol::OtaState::Idle,
        last_network_error: None,
        last_ota_error: None,
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
            "alert": { "kind": "none" }
        }))
        .unwrap(),
    );
    config.playlists[0].entries.extend([
        PlaylistEntry {
            card_id: "json".into(),
            dwell_seconds: None,
        },
        PlaylistEntry {
            card_id: "rss".into(),
            dwell_seconds: None,
        },
    ]);
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
fn v2_status_and_current_capabilities_survive_into_the_device_snapshot() {
    let control = MockDeviceControl::default();
    let unknown_bit = 1_u64 << 63;
    let mut device_status = status(42);
    device_status.capabilities = protocol::CURRENT_CAPABILITIES | unknown_bit;
    device_status.tier = protocol::Tier::Networked;
    device_status.wifi_state = protocol::WifiState::Failed;
    device_status.wifi_rssi = -58;
    device_status.ip = "192.168.1.42".into();
    device_status.last_network_error = Some("dns resolution timed out".into());
    device_status.ota_state = protocol::OtaState::Downloading;
    control.set_status(device_status);

    let runtime = start_runtime(full_config(), &control, Duration::ZERO);
    let snapshot = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });

    assert_eq!(snapshot.device.tier, Some(DeviceTier::Networked));
    assert_eq!(snapshot.device.wifi_state, Some(DeviceWifiState::Failed));
    assert_eq!(snapshot.device.wifi_rssi, Some(-58));
    assert_eq!(snapshot.device.ip.as_deref(), Some("192.168.1.42"));
    assert_eq!(
        snapshot.device.last_network_error.as_deref(),
        Some("dns resolution timed out")
    );
    assert_eq!(snapshot.device.ota_state, Some(DeviceOtaState::Downloading));
    assert!(
        snapshot
            .device
            .capabilities
            .contains(&DeviceCapability::Networking)
    );
    assert!(
        snapshot
            .device
            .capabilities
            .contains(&DeviceCapability::SceneRender)
    );
    assert_eq!(snapshot.device.unknown_capability_bits, unknown_bit);
    let json = serde_json::to_value(&snapshot.device).unwrap();
    assert!(
        json["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|capability| capability.as_str() == Some("scene-render"))
    );
    assert_eq!(json["unknown_capability_bits"], "0x8000000000000000");
    assert_eq!(json["tier"], "networked");
    assert_eq!(json["wifi_state"], "failed");
    assert_eq!(json["wifi_rssi"], -58);
    assert_eq!(json["ip"], "192.168.1.42");
    assert_eq!(json["last_network_error"], "dns resolution timed out");
    assert_eq!(json["ota_state"], "downloading");
    runtime.shutdown().unwrap();
}

#[test]
fn wrong_tier_refusal_stops_retries_and_local_tier_recovers() {
    let control = MockDeviceControl::default();
    let mut device_status = status(42);
    device_status.tier = protocol::Tier::Networked;
    control.set_status(device_status);
    control.reject_time_sync_as_wrong_tier();

    let runtime = start_runtime(full_config(), &control, Duration::ZERO);
    let networked = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
            && snapshot.device.tier == Some(DeviceTier::Networked)
            && control.time_sync_attempts() >= 1
    });
    thread::sleep(Duration::from_millis(100));

    assert_eq!(
        control.time_sync_attempts(),
        1,
        "an explicit WrongTier refusal must suppress the host's full-sync retry loop"
    );
    assert_eq!(networked.runtime, RuntimeState::Running);

    control.allow_time_sync();
    let mut local_status = status(84);
    local_status.tier = protocol::Tier::Local;
    control.set_status(local_status);
    let recovered = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.tier == Some(DeviceTier::Local) && control.time_sync_attempts() >= 2
    });

    assert_eq!(control.time_sync_attempts(), 2);
    assert_eq!(recovered.runtime, RuntimeState::Running);
    assert!(
        control
            .operations()
            .iter()
            .any(|operation| matches!(operation, Operation::ApplyLayout(_)))
    );
    runtime.shutdown().unwrap();
}

#[test]
fn networked_status_without_wrong_tier_keeps_websocket_owner_synchronizing() {
    let control = MockDeviceControl::default();
    let mut device_status = status(42);
    device_status.tier = protocol::Tier::Networked;
    control.set_status(device_status);
    let mut runtime_options = options();
    runtime_options.time_sync_interval = Duration::from_millis(20);

    let runtime = RuntimeHandle::start(
        full_config(),
        Box::new(MockDevice::new(control.clone())),
        Box::new(FixedRefresher {
            delay: Duration::ZERO,
        }),
        runtime_options,
    )
    .unwrap();
    let networked = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
            && snapshot.device.tier == Some(DeviceTier::Networked)
    });
    wait_for(Duration::from_secs(1), || control.time_sync_attempts() >= 3);

    assert_eq!(networked.runtime, RuntimeState::Running);
    assert!(
        control
            .operations()
            .iter()
            .any(|operation| matches!(operation, Operation::ApplyLayout(_))),
        "a WebSocket owner must apply layout to a device that reports Networked"
    );
    runtime.shutdown().unwrap();
}

#[test]
fn provision_and_factory_reset_use_the_runtime_owned_device_without_reconnecting() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });
    let connections_before = control.connection_count();

    runtime
        .provision(NetworkConfig {
            ssid: "home-network".into(),
            psk: "wifi-passphrase".into(),
            server_url: "wss://deskmate.example/v1/device/link".into(),
            device_id: "dev-0042".into(),
            token: "device-token".into(),
            utc_offset_minutes: 240,
            tier: ProvisioningTier::Networked,
        })
        .unwrap();
    runtime.factory_reset().unwrap();

    assert_eq!(control.connection_count(), connections_before);
    let operations = control.operations();
    assert_eq!(
        operations
            .iter()
            .filter(|operation| **operation == Operation::Provision)
            .count(),
        1
    );
    assert_eq!(
        operations
            .iter()
            .filter(|operation| **operation == Operation::FactoryReset)
            .count(),
        1
    );
    runtime.shutdown().unwrap();
}

#[test]
fn push_scene_uses_the_runtime_owned_device_without_reconnecting() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });
    let connections_before = control.connection_count();
    let push = PushScene {
        card_id: "clock".into(),
        revision: 7,
        scene: Scene {
            revision: 7,
            background: 0,
            nodes: Vec::new(),
        },
    };

    runtime.push_scene(push.clone()).unwrap();

    assert_eq!(control.connection_count(), connections_before);
    assert_eq!(
        control
            .operations()
            .iter()
            .filter(|operation| **operation == Operation::PushScene(push.clone()))
            .count(),
        1
    );
    runtime.shutdown().unwrap();
}

#[test]
fn scene_capable_device_receives_the_active_card_as_a_scene() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);

    wait_for(Duration::from_secs(1), || {
        control.operations().iter().any(
            |operation| matches!(operation, Operation::PushScene(push) if push.card_id == "clock"),
        )
    });

    let operations = control.operations();
    let activation = operations
        .iter()
        .position(|operation| *operation == Operation::Activate("clock".into()))
        .expect("the device model is activated before its scene");
    let scene = operations
        .iter()
        .position(
            |operation| matches!(operation, Operation::PushScene(push) if push.card_id == "clock"),
        )
        .expect("scene-capable device receives PushScene");
    assert!(activation < scene, "the scene must remain the visible face");
    runtime.shutdown().unwrap();
}

#[test]
fn config_apply_replies_before_the_followup_scene_round_trip_finishes() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);
    wait_for(Duration::from_secs(1), || {
        control
            .operations()
            .iter()
            .any(|operation| matches!(operation, Operation::PushScene(_)))
    });

    let scene_gate = control.block_next_scene();
    let mut changed = full_config();
    let CardSettings::Clock { show_seconds, .. } = &mut changed.cards[0] else {
        panic!("fixture's first card stopped being a clock");
    };
    *show_seconds = false;

    let (reply_sender, reply_receiver) = std::sync::mpsc::sync_channel(1);
    thread::scope(|scope| {
        scope.spawn(|| {
            reply_sender.send(runtime.apply_config(changed)).unwrap();
        });

        scene_gate.wait_until_entered();
        let result_before_scene_reply = reply_receiver.recv_timeout(Duration::from_millis(250));
        scene_gate.open();
        assert_eq!(
            result_before_scene_reply
                .expect("config apply stayed blocked on the follow-up scene request/response"),
            Ok(())
        );
    });

    runtime.shutdown().unwrap();
}

#[test]
fn automatic_scene_delivery_does_not_gate_the_online_connection_state() {
    let control = MockDeviceControl::default();
    let scene_gate = control.block_next_scene();
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);

    let snapshot = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });
    assert_eq!(snapshot.device.connection, ConnectionState::Online);
    scene_gate.wait_until_entered();
    assert_eq!(
        runtime.snapshot().unwrap().device.connection,
        ConnectionState::Online,
        "a best-effort render request must not turn an owned device back into Connecting"
    );

    scene_gate.open();
    runtime.shutdown().unwrap();
}

#[test]
fn explicit_scene_command_follows_one_automatic_attempt_without_spawning_another() {
    let control = MockDeviceControl::default();
    let automatic_gate = control.block_next_scene();
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });
    automatic_gate.wait_until_entered();

    let explicit = PushScene {
        card_id: "clock".into(),
        revision: 77,
        scene: Scene {
            revision: 77,
            background: 0,
            nodes: Vec::new(),
        },
    };
    let (reply_sender, reply_receiver) = std::sync::mpsc::sync_channel(1);
    thread::scope(|scope| {
        scope.spawn(|| {
            reply_sender
                .send(runtime.push_scene(explicit.clone()))
                .unwrap();
        });
        automatic_gate.open();
        assert_eq!(
            reply_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("explicit PushScene remained stuck behind the automatic attempt"),
            Ok(())
        );
    });

    wait_for(Duration::from_secs(1), || {
        control
            .operations()
            .iter()
            .any(|operation| matches!(operation, Operation::PushScene(push) if push.revision == 77))
    });
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        control
            .operations()
            .iter()
            .filter(|operation| matches!(operation, Operation::PushScene(_)))
            .count(),
        2,
        "one automatic event plus one explicit command must produce exactly two scene attempts"
    );
    runtime.shutdown().unwrap();
}

#[test]
fn failed_full_sync_never_pushes_a_scene_before_activation_succeeds() {
    let control = MockDeviceControl::default();
    control.fail_next_apply_layout_with(DeviceError::Timeout);
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);

    wait_for(Duration::from_secs(1), || {
        control
            .operations()
            .iter()
            .any(|operation| matches!(operation, Operation::PushScene(_)))
    });
    let operations = control.operations();
    let activation = operations
        .iter()
        .position(|operation| *operation == Operation::Activate("clock".into()))
        .expect("the retried ownership sync never activated the card");
    let scene = operations
        .iter()
        .position(|operation| matches!(operation, Operation::PushScene(_)))
        .expect("the successful full-sync retry never produced its scene");
    assert!(
        activation < scene,
        "a scene reached the device before its config and activation"
    );
    runtime.shutdown().unwrap();
}

#[test]
fn legacy_device_receives_widget_config_and_never_receives_a_scene() {
    let control = MockDeviceControl::default();
    let mut legacy = status(42);
    legacy.capabilities &= !protocol::CAPABILITY_SCENE_RENDER;
    control.set_status(legacy);
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);

    wait_for(Duration::from_secs(1), || {
        let operations = control.operations();
        operations.contains(&Operation::ApplyLayout(270))
            && operations.contains(&Operation::Push("clock".into()))
            && operations.contains(&Operation::Activate("clock".into()))
    });
    // Soft negative over ten complete 10 ms runtime intervals: unlike the
    // positive event assertions, absence has no notification to wait on.
    thread::sleep(Duration::from_millis(100));
    assert!(
        control
            .operations()
            .iter()
            .all(|operation| !matches!(operation, Operation::PushScene(_))),
        "firmware without bit 8 must stay on the legacy widget path"
    );
    runtime.shutdown().unwrap();
}

#[test]
fn render_path_is_re_resolved_from_fresh_capabilities_on_every_reconnect() {
    let control = MockDeviceControl::default();
    let mut legacy = status(42);
    legacy.capabilities &= !protocol::CAPABILITY_SCENE_RENDER;
    control.set_status(legacy.clone());
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);

    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });
    assert!(
        control
            .operations()
            .iter()
            .all(|operation| !matches!(operation, Operation::PushScene(_)))
    );

    control.set_status(status(84));
    control.force_disconnect(false);
    wait_for(Duration::from_secs(1), || {
        control.connection_count() >= 2
            && control
                .operations()
                .iter()
                .any(|operation| matches!(operation, Operation::PushScene(_)))
    });

    let scene_count = control
        .operations()
        .iter()
        .filter(|operation| matches!(operation, Operation::PushScene(_)))
        .count();
    control.set_status(legacy);
    control.force_disconnect(false);
    wait_for(Duration::from_secs(1), || control.connection_count() >= 3);
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        control
            .operations()
            .iter()
            .filter(|operation| matches!(operation, Operation::PushScene(_)))
            .count(),
        scene_count,
        "a device returning on legacy firmware must switch back to widgets"
    );
    runtime.shutdown().unwrap();
}

#[test]
fn a_refused_scene_is_card_scoped_visible_and_not_periodically_retried() {
    let control = MockDeviceControl::default();
    control.refuse_scenes_for("clock");
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);

    let snapshot = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.card_errors.iter().any(|error| {
            error.kind == CardErrorKind::SceneRefused
                && error.card_id == "clock"
                && error.message.contains("scene")
        })
    });
    assert!(
        !matches!(snapshot.runtime, RuntimeState::Error { .. }),
        "one refused scene must not park the whole runtime"
    );
    assert_eq!(snapshot.device.connection, ConnectionState::Online);
    let attempts = control
        .operations()
        .iter()
        .filter(|operation| matches!(operation, Operation::PushScene(_)))
        .count();
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        control
            .operations()
            .iter()
            .filter(|operation| matches!(operation, Operation::PushScene(_)))
            .count(),
        attempts,
        "a terminal refusal is retried only after a new host event"
    );
    runtime.shutdown().unwrap();
}

#[test]
fn busy_scene_is_retried_without_becoming_a_card_fault() {
    let control = MockDeviceControl::default();
    control.fail_next_scene_with(DeviceError::Rejected(ErrorResponse {
        code: ErrorCode::Busy,
        diagnostic: "LVGL is flushing".into(),
    }));
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);

    wait_for(Duration::from_secs(1), || {
        control
            .operations()
            .iter()
            .filter(|operation| matches!(operation, Operation::PushScene(_)))
            .count()
            >= 2
    });
    assert!(runtime.snapshot().unwrap().card_errors.is_empty());
    runtime.shutdown().unwrap();
}

#[test]
fn scene_transport_failures_retry_without_masquerading_as_card_faults() {
    for error in [
        DeviceError::Timeout,
        DeviceError::Transport(TransportError::Io("temporary stall".into())),
        DeviceError::MalformedResponse("truncated ACK".into()),
        DeviceError::UnexpectedMessage,
    ] {
        let control = MockDeviceControl::default();
        control.fail_next_scene_with(error);
        let runtime = start_runtime(full_config(), &control, Duration::ZERO);

        wait_for(Duration::from_secs(1), || {
            control
                .operations()
                .iter()
                .filter(|operation| matches!(operation, Operation::PushScene(_)))
                .count()
                >= 2
        });
        let snapshot = runtime.snapshot().unwrap();
        assert!(snapshot.card_errors.is_empty());
        assert_eq!(snapshot.device.connection, ConnectionState::Online);
        runtime.shutdown().unwrap();
    }
}

#[test]
fn wrong_tier_scene_recovery_replays_the_model_and_rearms_the_scene() {
    let control = MockDeviceControl::default();
    control.fail_next_scene_with(DeviceError::Rejected(ErrorResponse {
        code: ErrorCode::WrongTier,
        diagnostic: "server owns this device".into(),
    }));
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);

    wait_for(Duration::from_secs(1), || {
        control
            .operations()
            .iter()
            .filter(|operation| matches!(operation, Operation::PushScene(_)))
            .count()
            >= 2
    });
    let operations = control.operations();
    let second_scene = operations
        .iter()
        .rposition(|operation| matches!(operation, Operation::PushScene(_)))
        .unwrap();
    let last_activation = operations
        .iter()
        .rposition(|operation| *operation == Operation::Activate("clock".into()))
        .expect("ownership recovery did not replay the active screen");
    assert!(last_activation < second_scene);
    assert!(runtime.snapshot().unwrap().card_errors.is_empty());
    runtime.shutdown().unwrap();
}

#[test]
fn scenes_push_on_host_events_and_never_on_clock_or_pomodoro_ticks() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);
    let scene_count = || {
        control
            .operations()
            .iter()
            .filter(|operation| matches!(operation, Operation::PushScene(_)))
            .count()
    };

    wait_for(Duration::from_secs(1), || scene_count() >= 1);
    let initial = scene_count();
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        scene_count(),
        initial,
        "the live clock must not schedule periodic scene pushes"
    );

    let mut changed = full_config();
    let CardSettings::Clock { show_seconds, .. } = &mut changed.cards[0] else {
        panic!("fixture's first card stopped being a clock");
    };
    *show_seconds = false;
    runtime.apply_config(changed).unwrap();
    wait_for(Duration::from_secs(1), || scene_count() > initial);
    let after_config = scene_count();

    runtime.activate_screen("pomodoro").unwrap();
    wait_for(Duration::from_secs(1), || {
        control.operations().iter().rev().any(
            |operation| matches!(operation, Operation::PushScene(push) if push.card_id == "pomodoro"),
        )
    });
    let after_playlist_advance = scene_count();
    assert!(after_playlist_advance > after_config);
    runtime
        .control_pomodoro("pomodoro", PomodoroAction::Start)
        .unwrap();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot
            .pomodoros
            .iter()
            .any(|timer| timer.widget_id == "pomodoro" && timer.state == PomodoroState::Running)
    });
    let after_timer_start = scene_count();
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        scene_count(),
        after_timer_start,
        "pomodoro scheduler ticks update bindings through PushData, not scene rebuilds"
    );

    runtime.activate_screen("calendar").unwrap();
    wait_for(Duration::from_secs(1), || {
        control.operations().iter().rev().any(
            |operation| matches!(operation, Operation::PushScene(push) if push.card_id == "calendar"),
        )
    });
    let before_provider = scene_count();
    runtime.refresh_provider("calendar").unwrap();
    wait_for(Duration::from_secs(1), || scene_count() > before_provider);
    assert!(control.operations().iter().rev().any(
        |operation| matches!(operation, Operation::PushScene(push) if push.card_id == "calendar"),
    ));
    runtime.shutdown().unwrap();
}

#[test]
fn stale_error_transition_rebuilds_the_active_scene_with_its_footer() {
    let control = MockDeviceControl::default();
    let runtime = RuntimeHandle::start(
        full_config(),
        Box::new(MockDevice::new(control.clone())),
        Box::<LastGoodRefresher>::default(),
        options(),
    )
    .unwrap();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot
            .providers
            .first()
            .is_some_and(|provider| matches!(provider.state, app_core::ProviderState::Fresh))
    });
    runtime.activate_screen("calendar").unwrap();
    wait_for(Duration::from_secs(1), || {
        control.operations().iter().any(
            |operation| matches!(operation, Operation::PushScene(push) if push.card_id == "calendar"),
        )
    });
    let before_stale = control
        .operations()
        .iter()
        .filter(|operation| matches!(operation, Operation::PushScene(_)))
        .count();

    runtime.refresh_provider("calendar").unwrap();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot
            .providers
            .first()
            .is_some_and(|provider| matches!(provider.state, app_core::ProviderState::Stale { .. }))
    });
    wait_for(Duration::from_secs(1), || {
        control
            .operations()
            .iter()
            .filter(|operation| matches!(operation, Operation::PushScene(_)))
            .count()
            > before_stale
    });
    let latest = control
        .operations()
        .into_iter()
        .rev()
        .find_map(|operation| match operation {
            Operation::PushScene(push) if push.card_id == "calendar" => Some(push),
            _ => None,
        })
        .expect("stale provider result rebuilt the calendar scene");
    assert!(latest.scene.nodes.iter().any(|node| {
        matches!(
            node,
            SceneNode::Text(text)
                if text.value == SceneValue::Literal("offline".into())
        )
    }));
    runtime.shutdown().unwrap();
}

#[test]
fn provisioning_while_disconnected_is_a_typed_error_and_never_reaches_the_device() {
    let control = MockDeviceControl::default();
    control.power_off();
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        matches!(
            snapshot.device.connection,
            ConnectionState::Disconnected { .. }
        )
    });

    let error = runtime
        .provision(NetworkConfig {
            ssid: "home-network".into(),
            psk: "wifi-passphrase".into(),
            server_url: "wss://deskmate.example/v1/device/link".into(),
            device_id: "dev-0042".into(),
            token: "device-token".into(),
            utc_offset_minutes: 240,
            tier: ProvisioningTier::Networked,
        })
        .unwrap_err();

    assert_eq!(error, RuntimeError::DeviceDisconnected);
    assert_eq!(control.provision_attempts(), 0);
    assert!(!control.operations().contains(&Operation::Provision));
    runtime.shutdown().unwrap();
}

#[test]
fn transport_failure_during_provisioning_marks_disconnected_and_reconnects() {
    let control = MockDeviceControl::default();
    let mut reconnecting_options = options();
    reconnecting_options.reconnect_interval = Duration::from_millis(100);
    // Exclude the ordinary status-poll disconnect path from this regression. Without
    // runtime_command_device_error marking the transport disconnected, no poll can
    // rescue the test before its one-second assertion deadline.
    reconnecting_options.status_interval = Duration::from_secs(5);
    let runtime = RuntimeHandle::start(
        full_config(),
        Box::new(MockDevice::new(control.clone())),
        Box::new(FixedRefresher {
            delay: Duration::ZERO,
        }),
        reconnecting_options,
    )
    .unwrap();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });
    let connections_before = control.connection_count();
    control.disconnect_during_next_provision();

    let error = runtime
        .provision(NetworkConfig {
            ssid: "home-network".into(),
            psk: "wifi-passphrase".into(),
            server_url: "wss://deskmate.example/v1/device/link".into(),
            device_id: "dev-0042".into(),
            token: "device-token".into(),
            utc_offset_minutes: 240,
            tier: ProvisioningTier::Networked,
        })
        .unwrap_err();

    assert_eq!(error, RuntimeError::DeviceDisconnected);
    let disconnected = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Standalone
            && control.connection_count() == connections_before
    });
    assert_eq!(disconnected.device.connection, ConnectionState::Standalone);

    let reconnected = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
            && control.connection_count() > connections_before
    });
    assert_eq!(reconnected.device.connection, ConnectionState::Online);
    runtime.shutdown().unwrap();
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
    // The counter must stay at zero for a dismissal the host *did* apply, or it
    // says nothing: one that always increments is not a signal.
    assert_eq!(
        runtime
            .snapshot()
            .unwrap()
            .diagnostics
            .interrupt_dismissals_ignored,
        0
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
fn bounded_hold_alert_completing_while_disconnected_survives_to_reconnect() {
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

    runtime
        .control_pomodoro("pomodoro", PomodoroAction::Start)
        .unwrap();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot
            .pomodoros
            .first()
            .is_some_and(|pomodoro| pomodoro.state == PomodoroState::Running)
    });
    control.power_off();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Standalone
    });
    wait_for_snapshot(&runtime, Duration::from_secs(2), |snapshot| {
        snapshot
            .pomodoros
            .first()
            .is_some_and(|pomodoro| pomodoro.state == PomodoroState::Completed)
    });

    // Stay unplugged beyond the bounded hold. The countdown must not start
    // until the interrupt has actually reached the device.
    thread::sleep(Duration::from_secs(6));
    assert!(
        !control.operations().contains(&Operation::Interrupt(1)),
        "an unpowered device cannot have received the interrupt"
    );

    control.power_on();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online && control.connection_count() >= 2
    });
    assert!(
        control.operations().contains(&Operation::Interrupt(1)),
        "the interrupt raised while unpowered must be delivered on reconnect"
    );

    let activations_after_delivery = activated_screen_ids(&control).len();
    wait_for(Duration::from_secs(7), || {
        activated_screen_ids(&control).len() > activations_after_delivery
    });
    runtime.shutdown().unwrap();
}

#[test]
fn until_dismissed_alert_raised_while_disconnected_reaches_device_on_reconnect() {
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
                hold: AlertHold::UntilDismissed,
            };
        }
    }
    let runtime = start_runtime(config, &control, Duration::ZERO);
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });

    runtime
        .control_pomodoro("pomodoro", PomodoroAction::Start)
        .unwrap();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot
            .pomodoros
            .first()
            .is_some_and(|pomodoro| pomodoro.state == PomodoroState::Running)
    });
    control.power_off();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Standalone
    });
    wait_for_snapshot(&runtime, Duration::from_secs(2), |snapshot| {
        snapshot
            .pomodoros
            .first()
            .is_some_and(|pomodoro| pomodoro.state == PomodoroState::Completed)
    });
    assert!(
        !control.operations().contains(&Operation::Interrupt(1)),
        "an unpowered device cannot have received the interrupt"
    );

    control.power_on();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online && control.connection_count() >= 2
    });
    assert!(
        control.operations().contains(&Operation::Interrupt(1)),
        "the until-dismissed interrupt must be delivered on reconnect"
    );
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

/// Final-review finding: a push the device refused aborted `push_dirty_widgets`
/// WITHOUT clearing the widget from `dirty_widgets`, so the runtime re-attempted the
/// identical payload every cycle, sat in `RuntimeState::Error` with raw protocol text
/// forever, and starved every widget queued behind it in the same cycle. A refusal is
/// terminal for that payload: drop it, record it against its card, keep going.
#[test]
fn a_refused_push_is_not_retried_and_does_not_starve_other_cards() {
    let control = MockDeviceControl::default();
    // `dirty_widgets` is ordered, so "clock" is attempted before "pomodoro": the
    // pomodoro pushes below prove the cycle continued past the refusal.
    control.refuse_pushes_for("clock");
    let runtime = start_runtime(full_config(), &control, Duration::from_millis(5));

    let snapshot = wait_for_snapshot(&runtime, Duration::from_secs(2), |snapshot| {
        !snapshot.card_errors.is_empty()
    });
    assert_eq!(snapshot.card_errors.len(), 1);
    assert_eq!(snapshot.card_errors[0].kind, CardErrorKind::DataRefused);
    assert_eq!(snapshot.card_errors[0].card_id, "clock");
    assert!(
        snapshot.card_errors[0].message.contains("refused"),
        "message must be actionable, got {:?}",
        snapshot.card_errors[0].message
    );
    assert!(
        !matches!(snapshot.runtime, RuntimeState::Error { .. }),
        "a card-scoped refusal must not park the whole runtime in Error"
    );

    // The widget ordered behind the refused one in the SAME cycle still reached the
    // device, and later cycles still push it.
    let pomodoro_pushes = |control: &MockDeviceControl| {
        control
            .operations()
            .iter()
            .filter(|operation| matches!(operation, Operation::Push(id) if id == "pomodoro"))
            .count()
    };
    assert!(
        pomodoro_pushes(&control) >= 1,
        "a refusal must not skip the widgets queued behind it"
    );
    runtime
        .control_pomodoro("pomodoro", PomodoroAction::Start)
        .unwrap();
    wait_for(Duration::from_secs(2), || pomodoro_pushes(&control) >= 2);

    let clock_pushes = control
        .operations()
        .iter()
        .filter(|operation| matches!(operation, Operation::Push(id) if id == "clock"))
        .count();
    assert_eq!(
        clock_pushes, 1,
        "the refused payload must be attempted once, not re-queued every cycle"
    );
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

fn clock_card(id: &str) -> CardSettings {
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

fn pomodoro_card(id: &str, alert: CardAlert) -> CardSettings {
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

#[test]
fn rotation_follows_active_playlist_entry_order_and_dwell() {
    let control = MockDeviceControl::default();
    let config = AppConfig {
        cards: vec![clock_card("a"), clock_card("b")],
        playlists: vec![Playlist {
            id: "p1".into(),
            name: "P1".into(),
            advance: CarouselAdvance::Timed {
                default_dwell_seconds: 10,
            },
            entries: vec![
                PlaylistEntry {
                    card_id: "b".into(),
                    dwell_seconds: Some(5),
                },
                PlaylistEntry {
                    card_id: "a".into(),
                    dwell_seconds: None,
                },
            ],
        }],
        active_playlist_id: "p1".into(),
        ..AppConfig::default()
    };
    let runtime = start_runtime(config, &control, Duration::ZERO);

    wait_for(Duration::from_secs(1), || {
        activated_screen_ids(&control).first().map(String::as_str) == Some("b")
    });
    wait_for(Duration::from_secs(7), || {
        activated_screen_ids(&control).get(1).map(String::as_str) == Some("a")
    });
    wait_for(Duration::from_secs(12), || {
        activated_screen_ids(&control).get(2).map(String::as_str) == Some("b")
    });

    assert_eq!(&activated_screen_ids(&control)[..3], ["b", "a", "b"]);
    runtime.shutdown().unwrap();
}

#[test]
fn switching_active_playlist_is_a_config_apply_that_replays() {
    let control = MockDeviceControl::default();
    let mut config = AppConfig {
        cards: vec![clock_card("shared"), clock_card("new-first")],
        playlists: vec![
            Playlist {
                id: "p1".into(),
                name: "P1".into(),
                advance: CarouselAdvance::Manual,
                entries: vec![PlaylistEntry {
                    card_id: "shared".into(),
                    dwell_seconds: None,
                }],
            },
            Playlist {
                id: "p2".into(),
                name: "P2".into(),
                advance: CarouselAdvance::Timed {
                    default_dwell_seconds: 10,
                },
                entries: vec![
                    PlaylistEntry {
                        card_id: "new-first".into(),
                        dwell_seconds: Some(5),
                    },
                    PlaylistEntry {
                        card_id: "shared".into(),
                        dwell_seconds: None,
                    },
                ],
            },
        ],
        active_playlist_id: "p1".into(),
        ..AppConfig::default()
    };
    let runtime = start_runtime(config.clone(), &control, Duration::ZERO);
    wait_for(Duration::from_secs(1), || {
        activated_screen_ids(&control) == ["shared"]
    });
    let apply_count_before = control
        .operations()
        .iter()
        .filter(|operation| matches!(operation, Operation::ApplyLayout(_)))
        .count();

    config.active_playlist_id = "p2".into();
    runtime.apply_config(config).unwrap();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.config.active_playlist_id == "p2"
            && snapshot.device.active_screen_id.as_deref() == Some("new-first")
    });
    wait_for(Duration::from_secs(1), || {
        control
            .operations()
            .iter()
            .filter(|operation| matches!(operation, Operation::ApplyLayout(_)))
            .count()
            == apply_count_before + 1
            && activated_screen_ids(&control).get(1).map(String::as_str) == Some("new-first")
    });

    thread::sleep(Duration::from_secs(4));
    assert_eq!(activated_screen_ids(&control), ["shared", "new-first"]);
    wait_for(Duration::from_secs(3), || {
        activated_screen_ids(&control).get(2).map(String::as_str) == Some("shared")
    });
    runtime.shutdown().unwrap();
}

#[test]
fn alert_outside_active_playlist_still_fires() {
    let control = MockDeviceControl::default();
    let config = AppConfig {
        cards: vec![
            clock_card("visible"),
            CardSettings::Pomodoro {
                id: "alert-only".into(),
                label: "Alert only".into(),
                duration_seconds: 1,
                template: DisplayTemplate::ProgressRing,
                tap_action: WidgetTapAction::None,
                refresh: RefreshPolicy::DeviceLocal,
                alert: CardAlert::OnTimerFinish {
                    hold: AlertHold::UntilDismissed,
                },
            },
        ],
        playlists: vec![Playlist {
            id: "p1".into(),
            name: "P1".into(),
            advance: CarouselAdvance::Manual,
            entries: vec![PlaylistEntry {
                card_id: "visible".into(),
                dwell_seconds: None,
            }],
        }],
        active_playlist_id: "p1".into(),
        ..AppConfig::default()
    };
    let runtime = start_runtime(config, &control, Duration::ZERO);
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
            && snapshot.device.active_screen_id.as_deref() == Some("visible")
    });

    runtime
        .control_pomodoro("alert-only", PomodoroAction::Start)
        .unwrap();
    wait_for(Duration::from_secs(2), || {
        control.operations().contains(&Operation::Interrupt(1))
    });
    control.push_event(DeviceEvent {
        sequence: 1,
        kind: EventKind::InterruptDismissed,
        widget_id: "alert-only".into(),
        screen_id: "alert-only".into(),
        action: EventAction::DismissInterrupt,
        interrupt_token: Some(1),
    });

    let after_dismissal = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.active_screen_id.as_deref() == Some("visible")
    });
    assert_eq!(
        after_dismissal.device.active_screen_id.as_deref(),
        Some("visible")
    );
    runtime.shutdown().unwrap();
}

#[test]
fn manual_advance_playlist_has_no_rotation_deadline() {
    let control = MockDeviceControl::default();
    let config = AppConfig {
        cards: vec![clock_card("first"), clock_card("second")],
        playlists: vec![Playlist {
            id: "p1".into(),
            name: "P1".into(),
            advance: CarouselAdvance::Manual,
            entries: vec![
                PlaylistEntry {
                    card_id: "first".into(),
                    dwell_seconds: None,
                },
                PlaylistEntry {
                    card_id: "second".into(),
                    dwell_seconds: None,
                },
            ],
        }],
        active_playlist_id: "p1".into(),
        ..AppConfig::default()
    };
    let runtime = start_runtime(config, &control, Duration::ZERO);
    wait_for(Duration::from_secs(1), || {
        activated_screen_ids(&control) == ["first"]
    });

    thread::sleep(Duration::from_secs(6));
    assert_eq!(activated_screen_ids(&control), ["first"]);
    runtime.shutdown().unwrap();
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
        playlists: vec![Playlist {
            id: "p1".into(),
            name: "P1".into(),
            advance: CarouselAdvance::Timed {
                default_dwell_seconds: 5,
            },
            entries: vec![
                PlaylistEntry {
                    card_id: "first".into(),
                    dwell_seconds: Some(5),
                },
                PlaylistEntry {
                    card_id: "second".into(),
                    dwell_seconds: Some(5),
                },
            ],
        }],
        active_playlist_id: "p1".into(),
        cards: vec![
            clock_card("first"),
            pomodoro_card(
                "alerting",
                CardAlert::OnTimerFinish {
                    hold: AlertHold::UntilDismissed,
                },
            ),
            clock_card("second"),
            clock_card("muted"),
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
/// otherwise-identical pomodoro card outside the playlist is included to prove
/// a non-alerting library-only card contributes no entry at all — a stale entry
/// would otherwise render in the settings preview as if it were live.
#[test]
fn card_data_carries_live_fields_and_excludes_library_only_cards() {
    let control = MockDeviceControl::default();
    let config = AppConfig {
        playlists: vec![Playlist {
            id: "p1".into(),
            name: "P1".into(),
            advance: CarouselAdvance::Manual,
            entries: vec![PlaylistEntry {
                card_id: "on".into(),
                dwell_seconds: None,
            }],
        }],
        active_playlist_id: "p1".into(),
        cards: vec![
            pomodoro_card("on", CardAlert::None),
            pomodoro_card("library-only", CardAlert::None),
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
        !snapshot
            .card_data
            .iter()
            .any(|card| card.card_id == "library-only"),
        "a non-alerting card outside the playlist must not contribute an entry to card_data"
    );

    runtime.shutdown().unwrap();
}

/// The interrupt token counter must follow the device's *interrupt* counter and
/// nothing else. `latest_revision` and `config_revision` are the data-push and
/// config counters; they climb with ordinary traffic and have no relationship to
/// interrupts. Coupling them made a delivered interrupt arrive with a token 25
/// higher than its predecessor during the 2026-08-20 hardware gate, which read as
/// lost interrupts and cost a session's worth of doubt on a path that has already
/// produced two real defects.
#[test]
fn interrupt_tokens_do_not_follow_the_unrelated_revision_counters() {
    let control = MockDeviceControl::default();
    let mut seeded = status(0);
    // A session's worth of ordinary field pushes and config writes.
    seeded.latest_revision = 84;
    seeded.config_revision = 77;
    control.set_status(seeded);
    control.set_latest_interrupt_token(60);
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
        control
            .operations()
            .iter()
            .any(|operation| matches!(operation, Operation::Interrupt(_)))
    });
    let tokens: Vec<u32> = control
        .operations()
        .iter()
        .filter_map(|operation| match operation {
            Operation::Interrupt(token) => Some(*token),
            _ => None,
        })
        .collect();
    assert_eq!(
        tokens,
        vec![61],
        "the token must be one past the device's interrupt counter (60), not one past \
         its data revision (84)"
    );
    runtime.shutdown().unwrap();
}

/// The other half of the same rule. `latest_interrupt_token` is additive in M3 and
/// decodes to 0 when a pre-M3 image omits key 21, and 0 is also what a current image
/// reports before it has accepted any interrupt. In both cases there is no interrupt
/// counter to follow, so the revisions stay as the monotonic floor — starting above a
/// counter the device has already issued is always accepted, starting below it is
/// rejected as stale forever.
#[test]
fn an_absent_interrupt_counter_still_takes_the_revision_floor() {
    let control = MockDeviceControl::default();
    let mut seeded = status(0);
    seeded.latest_revision = 84;
    seeded.config_revision = 77;
    control.set_status(seeded);
    control.set_latest_interrupt_token(0);
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
        control
            .operations()
            .iter()
            .any(|operation| matches!(operation, Operation::Interrupt(_)))
    });
    let tokens: Vec<u32> = control
        .operations()
        .iter()
        .filter_map(|operation| match operation {
            Operation::Interrupt(token) => Some(*token),
            _ => None,
        })
        .collect();
    assert_eq!(
        tokens,
        vec![85],
        "with no interrupt counter to follow, the highest revision (84) is the floor"
    );
    runtime.shutdown().unwrap();
}

/// A dismissal whose token the arbiter no longer tracks is deliberately not
/// applied — but it must not be *invisible*. The common cause is benign (a
/// bounded hold expired host-side and freed the slot before the user tapped the
/// overlay the device was still showing), yet during the 2026-08-15 hardware
/// session a tap that the host silently declined was indistinguishable from an
/// event that never arrived, and the difference is the whole diagnosis.
#[test]
fn a_dismissal_for_an_untracked_token_is_counted_rather_than_silently_dropped() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });
    // No interrupt was ever scheduled, so token 4242 is tracked by nobody.
    control.push_event(DeviceEvent {
        sequence: 1,
        kind: EventKind::InterruptDismissed,
        widget_id: "pomodoro".into(),
        screen_id: "pomodoro".into(),
        action: EventAction::DismissInterrupt,
        interrupt_token: Some(4242),
    });
    wait_for_snapshot(&runtime, Duration::from_secs(2), |snapshot| {
        snapshot.diagnostics.interrupt_dismissals_ignored == 1
    });
    assert_eq!(
        runtime
            .snapshot()
            .unwrap()
            .diagnostics
            .interrupt_dismissals_ignored,
        1
    );
    runtime.shutdown().unwrap();
}

/// A tap on an already-Completed pomodoro is a deliberate host no-op
/// (`engine::pomodoro::toggle` on `Completed` only refreshes), but the firmware
/// applies optimistic local feedback on every `START_PAUSE` tap regardless of
/// state -- `progress_ring_local_action` flips `progress_running` and repaints
/// the arc and status text in the running hue. Only an authoritative push of
/// `running: false` puts that back, because `progress_ring.c`'s patch path calls
/// `set_running_color` unconditionally.
///
/// So the host must still push after a no-op tap. The 2026-08-15 hardware
/// session recorded the red "done" sticking until the next tap and attributed it
/// to the host pushing nothing back; this pins what the host actually does.
#[test]
fn a_tap_on_a_completed_pomodoro_still_pushes_authoritative_state() {
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
    wait_for_snapshot(&runtime, Duration::from_secs(3), |snapshot| {
        snapshot
            .pomodoros
            .first()
            .is_some_and(|pomodoro| pomodoro.state == PomodoroState::Completed)
    });

    let pushes_before = control
        .operations()
        .iter()
        .filter(|operation| matches!(operation, Operation::Push(id) if id == "pomodoro"))
        .count();

    // The no-op tap.
    control.push_event(DeviceEvent {
        sequence: 2,
        kind: EventKind::Tap,
        widget_id: "pomodoro".into(),
        screen_id: "pomodoro".into(),
        action: EventAction::StartPause,
        interrupt_token: None,
    });
    wait_for(Duration::from_secs(2), || {
        control
            .operations()
            .iter()
            .filter(|operation| matches!(operation, Operation::Push(id) if id == "pomodoro"))
            .count()
            > pushes_before
    });

    assert_eq!(
        runtime
            .snapshot()
            .unwrap()
            .pomodoros
            .first()
            .map(|pomodoro| pomodoro.state),
        Some(PomodoroState::Completed),
        "the tap must remain a no-op on host state"
    );
    runtime.shutdown().unwrap();
}
