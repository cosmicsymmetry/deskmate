//! Hardware harness for alert delivery across a USB link interruption.
//!
//! Flow (the harness sequences the user, so timing cannot be missed):
//! 1. Connects and shows the pomodoro card, then asks the user to UNPLUG.
//! 2. Only once the link drops does it start a short pomodoro host-side.
//! 3. The timer completes and more than the bounded hold elapses, all while
//!    the link is down.
//! 4. It then asks the user to REPLUG; the alert must take over the panel,
//!    delivered by the runtime's reconnect flush.
//!
//! Run from `companion/`:  `cargo run -p app-core --example alert_replay_check`

use std::time::{Duration, Instant};

use app_core::{
    AlertHold, AppConfig, CardAlert, CardSettings, ConnectionState, DeviceConnection,
    DisplayTemplate, PomodoroAction, PomodoroState, RefreshPolicy, RuntimeDevice, RuntimeHandle,
    RuntimeOptions, RuntimeSubscription, WidgetTapAction,
};
use device::{ConnectedSession, DeviceError, ReceivedEvent, SessionDiagnostics, connect_session};
use protocol::{
    Ack, ActivateCard, AssetBegin, AssetChunk, AssetCommit, AssetRelease, CardConfig, PushScene,
    StatusResponse, TimeSync, TriggerInterrupt,
};

const POMODORO_SECONDS: u32 = 15;
const HOLD_SECONDS: u16 = 5;

/// Serial adapter kept with this hardware-only harness so the reusable runtime
/// remains transport-neutral.
struct HarnessSerialDevice {
    connected: Option<ConnectedSession>,
}

impl HarnessSerialDevice {
    fn connected(&self) -> Result<&ConnectedSession, DeviceError> {
        self.connected.as_ref().ok_or(DeviceError::NoDevice)
    }
}

impl RuntimeDevice for HarnessSerialDevice {
    fn connect(&mut self) -> Result<DeviceConnection, DeviceError> {
        if let Some(connected) = self.connected.as_mut() {
            match connected.reconnect(None) {
                Ok(()) => {
                    return Ok(DeviceConnection {
                        port_name: connected.port_name.clone(),
                        status: connected.initial_status.clone(),
                    });
                }
                Err(error) => {
                    // A stalled worker cannot accept the replacement transport.
                    // Discard that session so the next attempt starts a new one.
                    if !connected.session.is_stalled() {
                        return Err(error);
                    }
                    self.connected = None;
                }
            }
        }
        let connected = connect_session(None)?;
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
        self.connected()?.session.time_sync(sync)
    }

    fn apply_layout(&mut self, rotation: u16, cards: Vec<CardConfig>) -> Result<(), DeviceError> {
        self.connected()?.session.apply_next_config(rotation, cards)
    }

    fn push_timer(
        &mut self,
        card_id: String,
        total_ms: u32,
        remaining_ms: u32,
        running: bool,
    ) -> Result<(), DeviceError> {
        self.connected()?
            .session
            .push_next_timer(card_id, total_ms, remaining_ms, running)
    }

    fn push_scene(&mut self, push: PushScene) -> Result<(), DeviceError> {
        self.connected()?.session.push_scene(push)
    }

    fn activate_card(&mut self, card_id: String) -> Result<(), DeviceError> {
        self.connected()?
            .session
            .activate_card(ActivateCard { card_id })
    }

    fn trigger_interrupt(&mut self, interrupt: TriggerInterrupt) -> Result<(), DeviceError> {
        self.connected()?.session.trigger_interrupt(interrupt)
    }

    fn send_asset_begin(&mut self, begin: AssetBegin) -> Result<Ack, DeviceError> {
        self.connected()?.session.asset_begin(begin)
    }

    fn send_asset_chunk(&mut self, chunk: AssetChunk) -> Result<(), DeviceError> {
        self.connected()?.session.asset_chunk(chunk)
    }

    fn send_asset_commit(&mut self, commit: AssetCommit) -> Result<(), DeviceError> {
        self.connected()?.session.asset_commit(commit)
    }

    fn send_asset_release(&mut self, release: AssetRelease) -> Result<(), DeviceError> {
        self.connected()?.session.asset_release(release)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    AwaitConnect,
    AwaitUnplug,
    TimerRunning,
    AwaitReplug,
    Verify,
}

struct ScriptSnapshot {
    online: bool,
    standalone: bool,
    completed: bool,
    timer_line: Option<String>,
    runtime_line: String,
    card_errors: Vec<String>,
}

enum ScriptObservation {
    Snapshot(ScriptSnapshot),
    Timeout,
    SubscriptionFailed(String),
    #[cfg(test)]
    End,
}

struct TimedObservation {
    at: Instant,
    elapsed: Duration,
    observation: ScriptObservation,
}

trait ScriptRuntime {
    fn receive(&mut self, timeout: Duration) -> TimedObservation;
    fn start_pomodoro(&mut self) -> Result<(), String>;
}

struct LiveRuntime {
    handle: RuntimeHandle,
    subscription: RuntimeSubscription,
    started_at: Instant,
}

impl ScriptRuntime for LiveRuntime {
    fn receive(&mut self, timeout: Duration) -> TimedObservation {
        let observation = match self.subscription.recv_timeout(timeout) {
            Ok(Some(snapshot)) => {
                let focus = snapshot
                    .pomodoros
                    .iter()
                    .find(|pomodoro| pomodoro.card_id == "focus");
                ScriptObservation::Snapshot(ScriptSnapshot {
                    online: matches!(snapshot.device.connection, ConnectionState::Online),
                    standalone: matches!(snapshot.device.connection, ConnectionState::Standalone),
                    completed: focus
                        .is_some_and(|pomodoro| pomodoro.state == PomodoroState::Completed),
                    timer_line: focus.map(|pomodoro| {
                        format!("{:?} {}s", pomodoro.state, pomodoro.remaining_seconds)
                    }),
                    runtime_line: format!("{:?}", snapshot.runtime),
                    card_errors: snapshot
                        .card_errors
                        .iter()
                        .map(|error| format!("card error {}: {}", error.card_id, error.message))
                        .collect(),
                })
            }
            Ok(None) => ScriptObservation::Timeout,
            Err(error) => ScriptObservation::SubscriptionFailed(error.to_string()),
        };
        TimedObservation {
            at: Instant::now(),
            elapsed: self.started_at.elapsed(),
            observation,
        }
    }

    fn start_pomodoro(&mut self) -> Result<(), String> {
        self.handle
            .control_pomodoro("focus", PomodoroAction::Start)
            .map_err(|error| format!("start pomodoro: {error}"))
    }
}

fn validate_timer_observation(
    phase: Phase,
    online: bool,
    standalone: bool,
    newly_completed: bool,
) -> Result<(), String> {
    if phase == Phase::TimerRunning && online {
        return Err("link came back before the hold window elapsed; scenario is invalid".into());
    }
    if phase == Phase::TimerRunning && newly_completed && !standalone {
        return Err("pomodoro completed without a Standalone snapshot; scenario is invalid".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    struct FakeRuntime {
        observations: VecDeque<TimedObservation>,
        last_at: Instant,
        starts: usize,
    }

    impl FakeRuntime {
        fn new(observations: Vec<TimedObservation>) -> Self {
            let last_at = observations
                .last()
                .map_or_else(Instant::now, |observation| observation.at);
            Self {
                observations: observations.into(),
                last_at,
                starts: 0,
            }
        }
    }

    impl ScriptRuntime for FakeRuntime {
        fn receive(&mut self, _timeout: Duration) -> TimedObservation {
            self.observations.pop_front().unwrap_or(TimedObservation {
                at: self.last_at,
                elapsed: Duration::ZERO,
                observation: ScriptObservation::End,
            })
        }

        fn start_pomodoro(&mut self) -> Result<(), String> {
            self.starts += 1;
            Ok(())
        }
    }

    fn observation(
        base: Instant,
        after: Duration,
        observation: ScriptObservation,
    ) -> TimedObservation {
        TimedObservation {
            at: base + after,
            elapsed: after,
            observation,
        }
    }

    fn snapshot(
        base: Instant,
        after_seconds: u64,
        online: bool,
        standalone: bool,
        completed: bool,
    ) -> TimedObservation {
        observation(
            base,
            Duration::from_secs(after_seconds),
            ScriptObservation::Snapshot(ScriptSnapshot {
                online,
                standalone,
                completed,
                timer_line: None,
                runtime_line: String::new(),
                card_errors: Vec::new(),
            }),
        )
    }

    fn timer_running_prefix(base: Instant) -> Vec<TimedObservation> {
        vec![
            snapshot(base, 0, true, false, false),
            snapshot(base, 1, false, true, false),
        ]
    }

    #[test]
    fn script_rejects_early_reconnect_and_non_standalone_completion() {
        let base = Instant::now();
        let mut early_reconnect = timer_running_prefix(base);
        early_reconnect.push(snapshot(base, 2, true, false, false));
        let error = run(&mut FakeRuntime::new(early_reconnect)).unwrap_err();
        assert!(error.contains("link came back"), "{error}");

        let mut invalid_completion = timer_running_prefix(base);
        invalid_completion.push(snapshot(base, 2, false, false, true));
        let error = run(&mut FakeRuntime::new(invalid_completion)).unwrap_err();
        assert!(error.contains("without a Standalone snapshot"), "{error}");
    }

    #[test]
    fn script_propagates_subscription_failure() {
        let base = Instant::now();
        let mut runtime = FakeRuntime::new(vec![observation(
            base,
            Duration::ZERO,
            ScriptObservation::SubscriptionFailed("fixture channel closed".into()),
        )]);
        assert_eq!(
            run(&mut runtime),
            Err("runtime stopped: fixture channel closed".into())
        );
    }

    #[test]
    fn production_entrypoint_uses_the_tested_script_driver() {
        let source = include_str!("alert_replay_check.rs");
        assert!(source.contains(
            "fn main() -> Result<(), String> {\n    let mut runtime = start_live_runtime()?;\n    run(&mut runtime).map(|_| ())\n}"
        ));
    }

    fn disconnected_completion(base: Instant) -> Vec<TimedObservation> {
        let mut observations = timer_running_prefix(base);
        observations.push(snapshot(base, 2, false, true, true));
        observations
    }

    #[test]
    fn verify_requires_disconnected_completion_elapsed_hold_and_later_reconnect() {
        let base = Instant::now();
        let required_hold = Duration::from_secs(u64::from(HOLD_SECONDS) + 5);

        let mut before_hold = disconnected_completion(base);
        before_hold.push(observation(
            base,
            Duration::from_secs(2) + required_hold - Duration::from_secs(1),
            ScriptObservation::Timeout,
        ));
        let mut runtime = FakeRuntime::new(before_hold);
        assert_eq!(run(&mut runtime).unwrap(), Phase::TimerRunning);
        assert_eq!(runtime.starts, 1);

        let mut without_reconnect = disconnected_completion(base);
        without_reconnect.push(observation(
            base,
            Duration::from_secs(2) + required_hold,
            ScriptObservation::Timeout,
        ));
        let mut runtime = FakeRuntime::new(without_reconnect);
        assert_eq!(run(&mut runtime).unwrap(), Phase::AwaitReplug);

        let mut valid = disconnected_completion(base);
        valid.push(observation(
            base,
            Duration::from_secs(2) + required_hold,
            ScriptObservation::Timeout,
        ));
        valid.push(snapshot(
            base,
            3 + required_hold.as_secs(),
            true,
            false,
            true,
        ));
        let mut runtime = FakeRuntime::new(valid);
        assert_eq!(run(&mut runtime).unwrap(), Phase::Verify);
        assert_eq!(runtime.starts, 1);
    }
}

fn main() -> Result<(), String> {
    let mut runtime = start_live_runtime()?;
    run(&mut runtime).map(|_| ())
}

fn start_live_runtime() -> Result<LiveRuntime, String> {
    let mut config = AppConfig::default();
    config.cards.insert(
        0,
        CardSettings::Pomodoro {
            id: "focus".into(),
            label: "Replay check".into(),
            duration_seconds: POMODORO_SECONDS,
            template: DisplayTemplate::ProgressRing,
            tap_action: WidgetTapAction::StartPause,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::OnTimerFinish {
                hold: AlertHold::Seconds {
                    value: HOLD_SECONDS,
                },
            },
            dwell_seconds: None,
        },
    );
    config
        .validate()
        .map_err(|error| format!("harness config must validate: {error}"))?;

    let handle = RuntimeHandle::start(
        config,
        Box::new(HarnessSerialDevice { connected: None }),
        RuntimeOptions::default(),
    )
    .map_err(|error| format!("start runtime: {error}"))?;
    let subscription = handle
        .subscribe()
        .map_err(|error| format!("subscribe to runtime: {error}"))?;
    Ok(LiveRuntime {
        handle,
        subscription,
        started_at: Instant::now(),
    })
}

#[allow(clippy::too_many_lines)] // linear phase script; splitting it would obscure the sequence
fn run(runtime: &mut impl ScriptRuntime) -> Result<Phase, String> {
    println!(
        "== alert replay check: {POMODORO_SECONDS}s pomodoro, {HOLD_SECONDS}s bounded hold =="
    );
    println!("waiting for the device to connect...");

    let mut phase = Phase::AwaitConnect;
    let mut completed_at: Option<Instant> = None;
    let mut last_line = String::new();
    let mut last_runtime = String::new();
    let mut seen_card_errors: Vec<String> = Vec::new();

    loop {
        let received = runtime.receive(Duration::from_millis(300));
        let snapshot = match received.observation {
            ScriptObservation::Snapshot(snapshot) => snapshot,
            ScriptObservation::Timeout => {
                // Timer/phase transitions that depend on wall time, not snapshots.
                if phase == Phase::TimerRunning
                    && let Some(done) = completed_at
                    && received.at.saturating_duration_since(done)
                        >= Duration::from_secs(u64::from(HOLD_SECONDS) + 5)
                {
                    phase = Phase::AwaitReplug;
                    println!();
                    println!(
                        ">>> Timer completed and {}s (> the {HOLD_SECONDS}s hold) have passed, all while unplugged.",
                        received.at.saturating_duration_since(done).as_secs()
                    );
                    println!(">>> REPLUG THE USB CABLE NOW.");
                    println!(
                        ">>> PASS = the alert takeover appears on the panel within a few seconds."
                    );
                    println!(
                        ">>> Then tap once to dismiss (the card underneath looks the same — that's expected)."
                    );
                    println!();
                }
                continue;
            }
            ScriptObservation::SubscriptionFailed(error) => {
                return Err(format!("runtime stopped: {error}"));
            }
            #[cfg(test)]
            ScriptObservation::End => return Ok(phase),
        };

        let newly_completed = snapshot.completed && completed_at.is_none();

        if snapshot.runtime_line != last_runtime {
            println!(
                "[{:>5.1}s] runtime: {}",
                received.elapsed.as_secs_f32(),
                snapshot.runtime_line
            );
            last_runtime.clone_from(&snapshot.runtime_line);
        }
        for line in &snapshot.card_errors {
            if !seen_card_errors.contains(line) {
                println!("[{:>5.1}s] {line}", received.elapsed.as_secs_f32());
                seen_card_errors.push(line.clone());
            }
        }

        validate_timer_observation(phase, snapshot.online, snapshot.standalone, newly_completed)?;

        match phase {
            Phase::AwaitConnect if snapshot.online => {
                phase = Phase::AwaitUnplug;
                println!();
                println!(">>> Connected; the pomodoro card is on the panel (idle ring).");
                println!(
                    ">>> UNPLUG THE USB CABLE NOW. The timer starts the moment the link drops."
                );
                println!();
            }
            Phase::AwaitUnplug if snapshot.standalone => {
                runtime.start_pomodoro()?;
                phase = Phase::TimerRunning;
                println!(
                    "[{:>5.1}s] link DOWN — {POMODORO_SECONDS}s timer STARTED host-side. Stay unplugged.",
                    received.elapsed.as_secs_f32()
                );
            }
            Phase::AwaitReplug if snapshot.online => {
                phase = Phase::Verify;
                println!(
                    "[{:>5.1}s] link ONLINE — reconnect flush ran. The takeover should be on the panel NOW.",
                    received.elapsed.as_secs_f32()
                );
                println!(">>> Report what the panel shows. Ctrl-C / kill the harness when done.");
            }
            _ => {}
        }

        if let Some(line) = snapshot.timer_line {
            if line != last_line {
                if newly_completed {
                    completed_at = Some(received.at);
                    println!(
                        "[{:>5.1}s] pomodoro COMPLETED while {} — interrupt held host-side; waiting out the hold window...",
                        received.elapsed.as_secs_f32(),
                        if snapshot.standalone {
                            "UNPLUGGED (correct)"
                        } else {
                            "CONNECTED (wrong — start over)"
                        }
                    );
                }
                last_line = line;
            }
        } else if newly_completed {
            completed_at = Some(received.at);
        }
    }
}
