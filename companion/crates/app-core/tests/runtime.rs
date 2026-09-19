use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use app_core::runtime::ImageSourceFrame;
use app_core::{
    AlertHold, AppConfig, CardAlert, CardErrorKind, CardField, CardFieldValue, CardSettings,
    CarouselAdvance, ConnectionState, DesiredAsset, DeviceCapability, DeviceConnection,
    DeviceOtaState, DeviceRenderProfile, DeviceTier, DeviceWifiState, DisplayOrientation,
    DisplayTemplate, PomodoroAction, PomodoroState, RefreshPolicy, RuntimeDevice, RuntimeError,
    RuntimeHandle, RuntimeOptions, RuntimeState, WidgetTapAction, analyze_scene,
    validate_native_scene,
};
use device::{DeviceError, ReceivedEvent, SessionDiagnostics, TransportError};
use protocol::{
    Ack, AssetBegin, AssetChunk, AssetCommit, AssetRelease, CardConfig, DeviceEvent, ErrorCode,
    ErrorResponse, EventAction, EventKind, PushScene, PushTimer, Scene, SceneNode, SceneValue,
    StatusResponse, TimeSync, TriggerInterrupt,
};

#[path = "runtime/alerts_and_commands.rs"]
mod alerts_and_commands;
#[path = "runtime/loop_and_events.rs"]
mod loop_and_events;
#[path = "runtime/picture_sources.rs"]
mod picture_sources;
#[path = "runtime/scene_delivery.rs"]
mod scene_delivery;

const FULL_JSON: &str = include_str!("fixtures/full.json");

#[derive(Debug, Clone, PartialEq, Eq)]
enum Operation {
    TimeSync,
    ApplyLayout(u16),
    PushTimer {
        card_id: String,
        total_ms: u32,
        remaining_ms: u32,
        running: bool,
    },
    PushScene(PushScene),
    AssetBegin([u8; protocol::ASSET_DIGEST_LEN]),
    AssetChunk([u8; protocol::ASSET_DIGEST_LEN], u32),
    AssetCommit([u8; protocol::ASSET_DIGEST_LEN]),
    AssetRelease(Vec<[u8; protocol::ASSET_DIGEST_LEN]>),
    Activate(String),
    Interrupt(u32),
    ReplayLayout,
    ReplayPush(String),
    ReplayActivate(String),
    ReplayInterrupt(u32),
}

#[derive(Default)]
struct ReplayCache {
    layout: bool,
    pushes: BTreeMap<String, PushTimer>,
    active_card: Option<String>,
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
}

#[derive(Default)]
struct MockState {
    connected: bool,
    power: MockPower,
    connection_count: u64,
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
    /// Makes every device call take this long, so a command whose budget is
    /// too small to contain a real synchronize can be observed timing out.
    call_delay: Duration,
    scene_errors: VecDeque<DeviceError>,
    /// Cards whose timer pushes receive an injected `InvalidPayload` refusal.
    refused_pushes: BTreeSet<String>,
    /// Cards whose scene the device understands but cannot render exactly.
    refused_scenes: BTreeSet<String>,
    /// Durable assets whose transfer fails after `AssetBegin`, proving an
    /// incomplete pass sends neither a scene nor its closing keep-set.
    refused_asset_chunks: BTreeSet<[u8; protocol::ASSET_DIGEST_LEN]>,
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
        self.enter_and_wait_for(Duration::from_secs(1));
    }

    fn enter_and_wait_for(&self, timeout: Duration) {
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        self.changed.notify_all();
        let (state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |(_, opened)| !*opened)
            .unwrap();
        let opened = state.1;
        drop(state);
        assert!(opened, "push gate was not opened within {timeout:?}");
    }

    fn open(&self) {
        let mut state = self.state.lock().unwrap();
        state.1 = true;
        self.changed.notify_all();
    }
}

#[test]
fn push_gate_waiter_panics_when_the_gate_is_not_opened() {
    let gate = Arc::new(PushGate::default());
    let waiter_gate = Arc::clone(&gate);
    let (result_sender, result_receiver) = std::sync::mpsc::sync_channel(1);
    let waiter = thread::spawn(move || {
        let panicked = std::panic::catch_unwind(|| {
            waiter_gate.enter_and_wait_for(Duration::from_millis(20));
        })
        .is_err();
        let _ = result_sender.send(panicked);
    });

    let while_closed = result_receiver.recv_timeout(Duration::from_millis(100));
    gate.open();
    let eventual = while_closed.or_else(|_| result_receiver.recv_timeout(Duration::from_secs(1)));
    waiter.join().unwrap();
    assert_eq!(
        while_closed,
        Ok(true),
        "a gate fixture timeout must panic instead of hanging; eventual={eventual:?}"
    );
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

    /// Makes every device call take `delay`, so a synchronize costs real time.
    fn set_call_delay(&self, delay: Duration) {
        self.state.lock().unwrap().call_delay = delay;
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

    fn refuse_pushes_for(&self, card_id: &str) {
        self.state
            .lock()
            .unwrap()
            .refused_pushes
            .insert(card_id.to_owned());
    }

    fn refuse_scenes_for(&self, card_id: &str) {
        self.state
            .lock()
            .unwrap()
            .refused_scenes
            .insert(card_id.to_owned());
    }

    fn refuse_asset_chunks_for(&self, digest: [u8; protocol::ASSET_DIGEST_LEN]) {
        self.state
            .lock()
            .unwrap()
            .refused_asset_chunks
            .insert(digest);
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
    drop_gate: Option<Arc<PushGate>>,
    panic_on_drop: bool,
}

impl MockDevice {
    fn new(control: MockDeviceControl) -> Self {
        Self {
            control,
            drop_gate: None,
            panic_on_drop: false,
        }
    }

    fn with_drop_gate(control: MockDeviceControl, drop_gate: Arc<PushGate>) -> Self {
        Self {
            control,
            drop_gate: Some(drop_gate),
            panic_on_drop: false,
        }
    }

    fn panics_on_drop(control: MockDeviceControl) -> Self {
        Self {
            control,
            drop_gate: None,
            panic_on_drop: true,
        }
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

impl Drop for MockDevice {
    fn drop(&mut self) {
        if let Some(gate) = &self.drop_gate {
            gate.enter_and_wait();
        }
        assert!(
            !self.panic_on_drop,
            "fixture worker panic during device cleanup"
        );
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
        if reconnect {
            if reset && state.replay.layout {
                state.operations.push(Operation::ReplayLayout);
                let card_ids: Vec<String> = state.replay.pushes.keys().cloned().collect();
                for card_id in card_ids {
                    state.operations.push(Operation::ReplayPush(card_id));
                }
            }
            if let Some(card_id) = state.replay.active_card.clone() {
                state.operations.push(Operation::ReplayActivate(card_id));
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
        let uptime_ms = state.connection_count * 1_000 + 100;
        let mut device_status = state
            .status_override
            .clone()
            .unwrap_or_else(|| status(uptime_ms));
        device_status.uptime_ms = uptime_ms;
        device_status.latest_interrupt_token = state.latest_interrupt_token;
        Ok(device_status)
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
            Ok(())
        })?
    }

    fn apply_layout(&mut self, rotation: u16, _cards: Vec<CardConfig>) -> Result<(), DeviceError> {
        // Simulates a real synchronize's cost: an asset reconcile can take
        // far longer than a bare message exchange.
        std::thread::sleep(self.control.state.lock().unwrap().call_delay);
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

    fn push_timer(
        &mut self,
        card_id: String,
        total_ms: u32,
        remaining_ms: u32,
        running: bool,
    ) -> Result<(), DeviceError> {
        let gate = self.with_connected(|state| state.next_push_gate.take())?;
        if let Some(gate) = gate {
            gate.enter_and_wait();
        }
        self.with_connected(|state| {
            state.operations.push(Operation::PushTimer {
                card_id: card_id.clone(),
                total_ms,
                remaining_ms,
                running,
            });
            if state.refused_pushes.contains(&card_id) {
                return Err(DeviceError::Rejected(ErrorResponse {
                    code: ErrorCode::InvalidPayload,
                    diagnostic: "invalid timer".into(),
                }));
            }
            state.replay.pushes.insert(
                card_id.clone(),
                PushTimer {
                    card_id,
                    revision: 0,
                    total_ms,
                    remaining_ms,
                    running,
                },
            );
            Ok(())
        })?
    }

    fn activate_card(&mut self, card_id: String) -> Result<(), DeviceError> {
        self.with_connected(|state| {
            state.replay.active_card = Some(card_id.clone());
            state.operations.push(Operation::Activate(card_id));
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

    fn send_asset_begin(&mut self, begin: AssetBegin) -> Result<Ack, DeviceError> {
        self.with_connected(|state| {
            state.operations.push(Operation::AssetBegin(begin.digest));
            Ack {
                acknowledged_type: protocol::TYPE_ASSET_BEGIN,
                revision: None,
                already_present: Some(false),
            }
        })
    }

    fn send_asset_chunk(&mut self, chunk: AssetChunk) -> Result<(), DeviceError> {
        self.with_connected(|state| {
            state
                .operations
                .push(Operation::AssetChunk(chunk.digest, chunk.offset));
            if state.refused_asset_chunks.contains(&chunk.digest) {
                return Err(DeviceError::Rejected(ErrorResponse {
                    code: ErrorCode::InvalidPayload,
                    diagnostic: "fixture asset chunk failure".into(),
                }));
            }
            Ok(())
        })?
    }

    fn send_asset_commit(&mut self, commit: AssetCommit) -> Result<(), DeviceError> {
        self.with_connected(|state| {
            state.operations.push(Operation::AssetCommit(commit.digest));
        })
    }

    fn send_asset_release(&mut self, release: AssetRelease) -> Result<(), DeviceError> {
        self.with_connected(|state| {
            state
                .operations
                .push(Operation::AssetRelease(release.digests));
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
            state.replay.active_card = Some(event.event.card_id.clone());
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

#[derive(Default)]
struct FakeImageSourceHostState {
    desired_assets: Vec<DesiredAsset>,
    picture_frames: BTreeMap<String, ImageSourceFrame>,
    pending_picture_frames: BTreeMap<String, ImageSourceFrame>,
}

#[derive(Clone, Default)]
struct FakeImageSourceHostControl {
    state: Arc<Mutex<FakeImageSourceHostState>>,
}

impl FakeImageSourceHostControl {
    fn host(&self) -> FakeImageSourceHost {
        FakeImageSourceHost {
            control: self.clone(),
        }
    }

    fn set_desired_assets(&self, desired_assets: Vec<DesiredAsset>) {
        self.state.lock().unwrap().desired_assets = desired_assets;
    }

    /// Stages a source update exactly as the server does: the desired asset is
    /// visible to the reconciliation pass that adopts it, and only then does
    /// scene construction observe the new digest. This keeps the ordering
    /// assertions deterministic even while the runtime worker is ticking.
    fn stage_picture_frame(
        &self,
        source_id: &str,
        digest: [u8; protocol::ASSET_DIGEST_LEN],
        bytes: &[u8],
        stale: bool,
    ) {
        self.state.lock().unwrap().pending_picture_frames.insert(
            source_id.to_owned(),
            ImageSourceFrame {
                digest,
                bytes: Arc::from(bytes),
                stale,
            },
        );
    }

    fn set_picture_stale(&self, source_id: &str, stale: bool) {
        self.state
            .lock()
            .unwrap()
            .picture_frames
            .get_mut(source_id)
            .expect("the fixture source has a frame")
            .stale = stale;
    }
}

struct FakeImageSourceHost {
    control: FakeImageSourceHostControl,
}

impl app_core::ImageSourceHost for FakeImageSourceHost {
    fn desired_assets(&mut self) -> Vec<DesiredAsset> {
        let mut state = self.control.state.lock().unwrap();
        let pending = std::mem::take(&mut state.pending_picture_frames);
        state.picture_frames.extend(pending);
        let mut desired = state.desired_assets.clone();
        for frame in state.picture_frames.values() {
            if !desired.iter().any(|asset| asset.digest == frame.digest) {
                desired.push(DesiredAsset {
                    digest: frame.digest,
                    kind: protocol::AssetKind::Image,
                    bytes: Arc::clone(&frame.bytes),
                });
            }
        }
        desired
    }

    fn image_source_frame(&mut self, source_id: &str) -> Option<ImageSourceFrame> {
        self.control
            .state
            .lock()
            .unwrap()
            .picture_frames
            .get(source_id)
            .cloned()
    }
}

fn status(uptime_ms: u64) -> StatusResponse {
    StatusResponse {
        firmware_version: "mock-firmware".into(),
        uptime_ms,
        display_width: 368,
        display_height: 448,
        ..protocol::test_support::sample_status_response()
    }
}

#[test]
fn status_fixture_preserves_its_portrait_dimensions() {
    let fixture = status(0);
    assert_eq!((fixture.display_width, fixture.display_height), (368, 448));
}

fn options() -> RuntimeOptions {
    RuntimeOptions {
        command_capacity: 4,
        maximum_subscribers: 2,
        command_timeout: Duration::from_secs(1),
        loop_maximum_wait: Duration::from_millis(2),
        reconnect_interval: Duration::from_millis(10),
        pomodoro_interval: Duration::from_millis(10),
        status_interval: Duration::from_millis(10),
        time_sync_interval: Duration::from_hours(1),
    }
}

fn full_config() -> AppConfig {
    let mut config: AppConfig = serde_json::from_str(FULL_JSON).unwrap();
    // Runtime tests exercise runtime behavior, not migration, so install this
    // broad fixture at the current version.
    config.schema_version = app_core::CURRENT_SCHEMA_VERSION;
    config
}

fn short_pomodoro_config(alert: CardAlert) -> AppConfig {
    let mut config = full_config();
    let card = config
        .cards
        .iter_mut()
        .find(|card| card.id() == "pomodoro")
        .expect("full_config fixture must contain the pomodoro card");
    let CardSettings::Pomodoro {
        duration_seconds,
        alert: card_alert,
        ..
    } = card
    else {
        panic!("full_config fixture's pomodoro card must remain a pomodoro");
    };
    *duration_seconds = 1;
    *card_alert = alert;
    config
}

fn assert_next_interrupt_token(
    latest_revision: u32,
    config_revision: u32,
    latest_interrupt_token: u32,
    expected: u32,
) {
    let control = MockDeviceControl::default();
    let mut seeded = status(0);
    seeded.latest_revision = latest_revision;
    seeded.config_revision = config_revision;
    control.set_status(seeded);
    control.set_latest_interrupt_token(latest_interrupt_token);
    let config = short_pomodoro_config(CardAlert::OnTimerFinish {
        hold: AlertHold::UntilDismissed,
    });
    let runtime = start_runtime(config, &control);
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });
    control.push_event(DeviceEvent {
        sequence: 1,
        kind: EventKind::Tap,
        card_id: "pomodoro".into(),
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
    assert_eq!(tokens, vec![expected]);
    assert_eq!(runtime.snapshot().unwrap().runtime, RuntimeState::Running);
    runtime.shutdown().unwrap();
}

fn picture_config(picture_is_active: bool) -> AppConfig {
    let mut config = AppConfig::default();
    let picture = CardSettings::Picture {
        id: "picture-card".into(),
        title: "Picture card".into(),
        source_id: "camera".into(),
        tap_action: WidgetTapAction::None,
        refresh: RefreshPolicy::Manual,
        alert: CardAlert::None,
        dwell_seconds: None,
    };
    config.image_sources = vec![app_core::config::ImageSource {
        id: "camera".into(),
        name: "Camera".into(),
    }];
    if picture_is_active {
        // The active card is simply the first in the loop now.
        config.cards = vec![picture];
    } else {
        config.cards.push(picture);
    }
    config
}

fn start_picture_runtime(
    config: AppConfig,
    control: &MockDeviceControl,
    host: Option<Box<dyn app_core::ImageSourceHost>>,
) -> RuntimeHandle {
    RuntimeHandle::start_with_image_source_host(
        config,
        Box::new(MockDevice::new(control.clone())),
        options(),
        host,
    )
    .unwrap()
}

fn start_runtime(config: AppConfig, control: &MockDeviceControl) -> RuntimeHandle {
    RuntimeHandle::start(
        config,
        Box::new(MockDevice::new(control.clone())),
        options(),
    )
    .unwrap()
}

fn apply_paused_preference(runtime: &RuntimeHandle, paused: bool) -> Result<(), RuntimeError> {
    let mut config = runtime.snapshot()?.config;
    config.preferences.paused = paused;
    runtime.apply_config(config)
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

fn clock_card(id: &str) -> CardSettings {
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

/// A copy of `card` that stays on the panel for `seconds` instead of taking
/// the document's default dwell.
fn with_dwell(card: CardSettings, seconds: u16) -> CardSettings {
    match card {
        CardSettings::Clock {
            id,
            title,
            show_seconds,
            template,
            tap_action,
            refresh,
            alert,
            ..
        } => CardSettings::Clock {
            id,
            title,
            show_seconds,
            template,
            tap_action,
            refresh,
            alert,
            dwell_seconds: Some(seconds),
        },
        other => other,
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
        dwell_seconds: None,
    }
}

/// The screen IDs sent to the device via `ActivateCard`, in the order they
/// were sent. The very first entry is always the initial full-sync
/// activation of the boot-time active screen, issued on connect regardless of
/// carousel mode; rotation-triggered activations (if any) follow it.
fn activated_card_ids(control: &MockDeviceControl) -> Vec<String> {
    control
        .operations()
        .into_iter()
        .filter_map(|operation| match operation {
            Operation::Activate(card_id) => Some(card_id),
            _ => None,
        })
        .collect()
}

const PICTURE_BLOB: &[u8] = &[0x19, 0x12, 0, 0, 0xc0, 1, 0x70, 1, 0x80, 3, 0, 0, 1, 2];

fn latest_picture_push(operations: &[Operation]) -> Option<PushScene> {
    operations
        .iter()
        .rev()
        .find_map(|operation| match operation {
            Operation::PushScene(push) if push.card_id == "picture-card" => Some(push.clone()),
            _ => None,
        })
}

fn run_image_source_update_case(
    picture_is_active: bool,
    existing_digest: [u8; protocol::ASSET_DIGEST_LEN],
    picture_digest: [u8; protocol::ASSET_DIGEST_LEN],
) -> Vec<Operation> {
    let control = MockDeviceControl::default();
    let host = FakeImageSourceHostControl::default();
    host.set_desired_assets(vec![DesiredAsset {
        digest: existing_digest,
        kind: protocol::AssetKind::Image,
        bytes: Arc::from(&b"resident picture"[..]),
    }]);
    let runtime = start_picture_runtime(
        picture_config(picture_is_active),
        &control,
        Some(Box::new(host.host())),
    );
    wait_for(Duration::from_secs(1), || {
        let initial_face_received = if picture_is_active {
            latest_picture_push(&control.operations()).is_some()
        } else {
            control.operations().iter().any(
                |operation| matches!(operation, Operation::PushScene(push) if push.card_id == "clock"),
            )
        };
        initial_face_received && control.operations().iter().any(
            |operation| matches!(operation, Operation::AssetRelease(digests) if digests == &vec![existing_digest]),
        )
    });
    let before = control.operations().len();

    host.stage_picture_frame("camera", picture_digest, PICTURE_BLOB, false);
    runtime
        .image_source_updated("camera", picture_digest)
        .unwrap();
    if picture_is_active {
        wait_for(Duration::from_secs(1), || {
            control.operations()[before..].iter().any(|operation| {
                matches!(operation, Operation::PushScene(push) if push.scene.nodes.iter().any(
                    |node| matches!(node, SceneNode::Image(image) if image.digest == picture_digest)
                ))
            })
        });
    } else {
        wait_for(Duration::from_secs(1), || {
            control.operations()[before..].iter().any(|operation| {
                matches!(operation, Operation::AssetRelease(digests) if digests == &vec![existing_digest, picture_digest])
            })
        });
        thread::sleep(Duration::from_millis(30));
    }

    let operations = control.operations();
    let relevant = operations[before..]
        .iter()
        .filter(|operation| {
            matches!(
                operation,
                Operation::AssetBegin(_)
                    | Operation::AssetChunk(_, _)
                    | Operation::AssetCommit(_)
                    | Operation::AssetRelease(_)
                    | Operation::PushScene(_)
            )
        })
        .cloned()
        .collect();
    runtime.shutdown().unwrap();
    relevant
}

fn assert_image_update_asset_prefix(
    operations: &[Operation],
    existing_digest: [u8; protocol::ASSET_DIGEST_LEN],
    picture_digest: [u8; protocol::ASSET_DIGEST_LEN],
) {
    assert_eq!(
        &operations[..7],
        &[
            Operation::AssetBegin(existing_digest),
            Operation::AssetChunk(existing_digest, 0),
            Operation::AssetCommit(existing_digest),
            Operation::AssetBegin(picture_digest),
            Operation::AssetChunk(picture_digest, 0),
            Operation::AssetCommit(picture_digest),
            Operation::AssetRelease(vec![existing_digest, picture_digest]),
        ]
    );
}
