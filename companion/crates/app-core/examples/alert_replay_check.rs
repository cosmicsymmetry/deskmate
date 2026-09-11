//! Hardware re-verification harness for the unpowered-alert fix (2026-08-15).
//!
//! Flow (the harness sequences the user, so timing cannot be missed):
//! 1. Connects and shows the pomodoro card, then asks the user to UNPLUG.
//! 2. Only once the link drops does it start a short pomodoro host-side.
//! 3. The timer completes and more than the bounded hold elapses, all while
//!    the link is down — the exact pre-fix loss scenario.
//! 4. It then asks the user to REPLUG; the alert must take over the panel,
//!    delivered by the runtime's reconnect flush.
//!
//! Run from `companion/`:  `cargo run -p app-core --example alert_replay_check`

use std::time::{Duration, Instant};

use app_core::{
    AlertHold, AppConfig, CardAlert, CardSettings, ConnectionState, DisplayTemplate,
    PomodoroAction, PomodoroState, RefreshPolicy, RuntimeHandle, WidgetTapAction,
};

const POMODORO_SECONDS: u32 = 15;
const HOLD_SECONDS: u16 = 5;

#[derive(PartialEq)]
enum Phase {
    AwaitConnect,
    AwaitUnplug,
    TimerRunning,
    AwaitReplug,
    Verify,
}

#[allow(clippy::too_many_lines)] // linear phase script; splitting it would obscure the sequence
fn main() {
    let mut config = AppConfig::default();
    config.cards.push(CardSettings::Pomodoro {
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
    });
    config.validate().expect("harness config must validate");

    let handle = RuntimeHandle::start_serial(config, None).expect("start runtime");
    let subscription = handle.subscribe().expect("subscribe");

    println!(
        "== alert replay check: {POMODORO_SECONDS}s pomodoro, {HOLD_SECONDS}s bounded hold =="
    );
    println!("waiting for the device to connect...");

    let mut phase = Phase::AwaitConnect;
    let mut completed_at: Option<Instant> = None;
    let started_at = Instant::now();
    let mut last_line = String::new();
    let mut last_runtime = String::new();
    let mut seen_card_errors: Vec<String> = Vec::new();

    loop {
        let snapshot = match subscription.recv_timeout(Duration::from_millis(300)) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                // Timer/phase transitions that depend on wall time, not snapshots.
                if phase == Phase::TimerRunning
                    && let Some(done) = completed_at
                    && done.elapsed() >= Duration::from_secs(u64::from(HOLD_SECONDS) + 5)
                {
                    phase = Phase::AwaitReplug;
                    println!();
                    println!(
                        ">>> Timer completed and {}s (> the {HOLD_SECONDS}s hold) have passed, all while unplugged.",
                        done.elapsed().as_secs()
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
            Err(error) => {
                println!("runtime stopped: {error}");
                break;
            }
        };

        let online = matches!(snapshot.device.connection, ConnectionState::Online);
        let standalone = matches!(snapshot.device.connection, ConnectionState::Standalone);

        let runtime_line = format!("{:?}", snapshot.runtime);
        if runtime_line != last_runtime {
            println!(
                "[{:>5.1}s] runtime: {runtime_line}",
                started_at.elapsed().as_secs_f32()
            );
            last_runtime = runtime_line;
        }
        for error in &snapshot.card_errors {
            let line = format!("card error {}: {}", error.card_id, error.message);
            if !seen_card_errors.contains(&line) {
                println!("[{:>5.1}s] {line}", started_at.elapsed().as_secs_f32());
                seen_card_errors.push(line);
            }
        }

        match phase {
            Phase::AwaitConnect if online => {
                handle.activate_screen("focus").expect("activate focus");
                phase = Phase::AwaitUnplug;
                println!();
                println!(">>> Connected; the pomodoro card is on the panel (idle ring).");
                println!(
                    ">>> UNPLUG THE USB CABLE NOW. The timer starts the moment the link drops."
                );
                println!();
            }
            Phase::AwaitUnplug if standalone => {
                handle
                    .control_pomodoro("focus", PomodoroAction::Start)
                    .expect("start pomodoro");
                phase = Phase::TimerRunning;
                println!(
                    "[{:>5.1}s] link DOWN — {POMODORO_SECONDS}s timer STARTED host-side. Stay unplugged.",
                    started_at.elapsed().as_secs_f32()
                );
            }
            Phase::TimerRunning | Phase::AwaitReplug => {
                if online && phase == Phase::AwaitReplug {
                    phase = Phase::Verify;
                    println!(
                        "[{:>5.1}s] link ONLINE — reconnect flush ran. The takeover should be on the panel NOW.",
                        started_at.elapsed().as_secs_f32()
                    );
                    println!(
                        ">>> Report what the panel shows. Ctrl-C / kill the harness when done."
                    );
                } else if online && phase == Phase::TimerRunning {
                    println!(
                        "[{:>5.1}s] link came back BEFORE the hold window elapsed — scenario void, start over.",
                        started_at.elapsed().as_secs_f32()
                    );
                }
            }
            _ => {}
        }

        if let Some(pomodoro) = snapshot.pomodoros.iter().find(|p| p.widget_id == "focus") {
            let line = format!("{:?} {}s", pomodoro.state, pomodoro.remaining_seconds);
            if line != last_line {
                if pomodoro.state == PomodoroState::Completed && completed_at.is_none() {
                    completed_at = Some(Instant::now());
                    println!(
                        "[{:>5.1}s] pomodoro COMPLETED while {} — interrupt held host-side; waiting out the hold window...",
                        started_at.elapsed().as_secs_f32(),
                        if standalone {
                            "UNPLUGGED (correct)"
                        } else {
                            "CONNECTED (wrong — start over)"
                        }
                    );
                }
                last_line = line;
            }
        }
    }
}
