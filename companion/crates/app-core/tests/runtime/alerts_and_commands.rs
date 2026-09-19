use super::*;

#[test]
fn cold_boot_and_two_power_resets_replay_the_complete_owned_state() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control);
    wait_until_online(&runtime);

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
        .position(|operation| matches!(operation, Operation::PushTimer { .. }))
        .unwrap();
    let pushed_card_count = operations
        .iter()
        .filter_map(|operation| match operation {
            Operation::PushTimer { card_id, .. } => Some(card_id),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .len();
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
    assert_eq!(
        replayed
            .iter()
            .filter(|operation| matches!(operation, Operation::ReplayPush(_)))
            .count(),
        pushed_card_count * 2
    );
    runtime.shutdown().unwrap();
}

#[test]
fn local_navigation_becomes_the_authoritative_screen_for_reset_replay() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control);
    wait_until_online(&runtime);

    control.navigate_next(1, "pomodoro");
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.active_card_id.as_deref() == Some("pomodoro")
    });

    control.force_disconnect(true);
    wait_for(Duration::from_secs(1), || control.connection_count() >= 2);
    wait_for(Duration::from_secs(1), || {
        control
            .operations()
            .iter()
            .rev()
            .any(|operation| *operation == Operation::ReplayActivate("pomodoro".into()))
    });
    let last_replay_activation = control
        .operations()
        .into_iter()
        .rev()
        .find(|operation| matches!(operation, Operation::ReplayActivate(_)))
        .unwrap();
    assert_eq!(
        last_replay_activation,
        Operation::ReplayActivate("pomodoro".into())
    );
    runtime.shutdown().unwrap();
}

#[test]
fn pomodoro_events_complete_once_and_dismissed_interrupts_do_not_replay() {
    let control = MockDeviceControl::default();
    let config = short_pomodoro_config(CardAlert::OnTimerFinish {
        hold: AlertHold::UntilDismissed,
    });
    let runtime = start_runtime(config, &control);
    wait_until_online(&runtime);
    control.tap_pomodoro(1);
    wait_for_pomodoro_state(&runtime, Duration::from_secs(1), PomodoroState::Running);
    wait_for_pomodoro_state(&runtime, Duration::from_secs(2), PomodoroState::Completed);
    wait_for(Duration::from_secs(1), || {
        control.operations().contains(&Operation::Interrupt(1))
    });
    control.push_event(DeviceEvent {
        sequence: 2,
        kind: EventKind::InterruptDismissed,
        card_id: "pomodoro".into(),
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

// A wall-clock proof that a bounded `AlertHold::Seconds` deadline runs through
// the full `run_runtime` loop: the interrupt fires once, then — with no touch
// dismissal from the device — the hold expires, the host dismisses it from its
// own arbiter, and re-sends `ActivateCard` for the saved carousel screen.
// `full_config` uses `Manual` advance, so the only source of a second activation
// here is the hold expiring, not rotation.
//
// This does **not** prove the physical panel leaves the interrupt overlay.
// Per `docs/protocol/v2.md`'s `ActivateCard` rule, activation during an interrupt
// changes the saved underlying card; the overlay remains until a tap dismisses
// it. `MockDevice` has no interrupt-takeover model: it records the activation.
// This exercises `alert_hold_due` → `InterruptArbiter::dismiss` →
// `active_card_dirty` → `send_screen` through a real threaded runtime loop.
// See `arm_alert_hold` in `src/runtime/timers.rs` for the host-only bookkeeping
// and the resulting host/device divergence until a tap or full resync.
#[test]
fn a_bounded_alert_hold_re_sends_the_saved_carousel_screen_activation() {
    let control = MockDeviceControl::default();
    let config = short_pomodoro_config(CardAlert::OnTimerFinish {
        hold: AlertHold::Seconds { value: 5 },
    });
    let runtime = start_runtime(config, &control);
    wait_until_online(&runtime);
    let activations_before_completion = activated_card_ids(&control).len();
    control.tap_pomodoro(1);
    wait_for_pomodoro_state(&runtime, Duration::from_secs(1), PomodoroState::Running);
    wait_for_pomodoro_state(&runtime, Duration::from_secs(2), PomodoroState::Completed);
    wait_for(Duration::from_secs(1), || {
        control.operations().contains(&Operation::Interrupt(1))
    });
    // No spontaneous activations from the manual-advance carousel between
    // completion and the interrupt firing.
    assert_eq!(
        activated_card_ids(&control).len(),
        activations_before_completion
    );

    // No dismissal event from the device: the 5s hold must expire on its own
    // and re-send `ActivateCard`, which is the only other source of a new
    // activation under `CarouselAdvance::Manual`.
    wait_for(Duration::from_secs(7), || {
        activated_card_ids(&control).len() > activations_before_completion
    });
    runtime.shutdown().unwrap();
}

#[test]
fn bounded_hold_alert_completing_while_disconnected_survives_to_reconnect() {
    let control = MockDeviceControl::default();
    let config = short_pomodoro_config(CardAlert::OnTimerFinish {
        hold: AlertHold::Seconds { value: 5 },
    });
    let runtime = start_runtime(config, &control);
    wait_until_online(&runtime);

    runtime
        .control_pomodoro("pomodoro", PomodoroAction::Start)
        .unwrap();
    wait_for_pomodoro_state(&runtime, Duration::from_secs(1), PomodoroState::Running);
    control.power_off();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Standalone
    });
    wait_for_pomodoro_state(&runtime, Duration::from_secs(2), PomodoroState::Completed);

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

    let activations_after_delivery = activated_card_ids(&control).len();
    wait_for(Duration::from_secs(7), || {
        activated_card_ids(&control).len() > activations_after_delivery
    });
    runtime.shutdown().unwrap();
}

#[test]
fn until_dismissed_alert_raised_while_disconnected_reaches_device_on_reconnect() {
    let control = MockDeviceControl::default();
    let config = short_pomodoro_config(CardAlert::OnTimerFinish {
        hold: AlertHold::UntilDismissed,
    });
    let runtime = start_runtime(config, &control);
    wait_until_online(&runtime);

    runtime
        .control_pomodoro("pomodoro", PomodoroAction::Start)
        .unwrap();
    wait_for_pomodoro_state(&runtime, Duration::from_secs(1), PomodoroState::Running);
    control.power_off();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Standalone
    });
    wait_for_pomodoro_state(&runtime, Duration::from_secs(2), PomodoroState::Completed);
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
    let config = short_pomodoro_config(CardAlert::None);
    let runtime = start_runtime(config, &control);
    wait_until_online(&runtime);
    control.tap_pomodoro(1);
    wait_for_pomodoro_state(&runtime, Duration::from_secs(1), PomodoroState::Running);
    wait_for_pomodoro_state(&runtime, Duration::from_secs(2), PomodoroState::Completed);
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
fn runtime_restart_seeds_interrupt_tokens_from_the_still_powered_device() {
    assert_next_interrupt_token(0, 0, 40, 41);
}

#[test]
fn subscribers_are_bounded_and_coalesce_pressure_to_the_latest_snapshot() {
    let control = MockDeviceControl::default();
    let mut runtime_options = options();
    runtime_options.maximum_subscribers = 1;
    let runtime = start_runtime_with_options(full_config(), &control, runtime_options);
    let subscription = runtime.subscribe().unwrap();
    assert!(matches!(runtime.subscribe(), Err(RuntimeError::QueueFull)));
    apply_paused_preference(&runtime, true).unwrap();
    apply_paused_preference(&runtime, false).unwrap();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.active_card_id.as_deref() == Some("clock")
            && snapshot.runtime == RuntimeState::Running
    });
    let latest = subscription
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert_eq!(latest.device.active_card_id.as_deref(), Some("clock"));
    assert_eq!(latest.runtime, RuntimeState::Running);
    runtime.shutdown().unwrap();
}

#[test]
fn lagging_subscriber_does_not_cause_a_diagnostic_only_second_snapshot() {
    let control = MockDeviceControl::default();
    let mut runtime_options = options();
    runtime_options.pomodoro_interval = Duration::from_hours(1);
    runtime_options.status_interval = Duration::from_hours(1);
    let runtime = start_runtime_with_options(AppConfig::default(), &control, runtime_options);
    wait_until_online(&runtime);
    let subscription = runtime.subscribe().unwrap();
    subscription
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .expect("subscription starts with the latest snapshot");

    apply_paused_preference(&runtime, true).unwrap();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.runtime == RuntimeState::Paused
    });
    apply_paused_preference(&runtime, false).unwrap();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.runtime == RuntimeState::Running
    });

    let latest = subscription
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .expect("the unread slot contains the latest substantive state");
    assert_eq!(latest.runtime, RuntimeState::Running);
    assert!(
        subscription
            .recv_timeout(Duration::from_millis(50))
            .unwrap()
            .is_none(),
        "delivery diagnostics must not publish themselves"
    );
    runtime.shutdown().unwrap();
}

#[test]
fn paused_config_defers_timer_pushes_until_a_resumed_config_is_applied() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control);
    let pushes_before = control.count_operations(|operation| {
        matches!(operation, Operation::PushTimer { card_id, .. } if card_id == "pomodoro")
    });

    apply_paused_preference(&runtime, true).unwrap();
    runtime
        .control_pomodoro("pomodoro", PomodoroAction::Start)
        .unwrap();
    thread::sleep(Duration::from_millis(40));
    assert_eq!(
        control.count_operations(|operation| {
            matches!(operation, Operation::PushTimer { card_id, .. } if card_id == "pomodoro")
        }),
        pushes_before
    );
    assert_eq!(runtime.snapshot().unwrap().runtime, RuntimeState::Paused);

    apply_paused_preference(&runtime, false).unwrap();
    wait_for(Duration::from_secs(1), || {
        control.count_operations(|operation| {
            matches!(operation, Operation::PushTimer { card_id, .. } if card_id == "pomodoro")
        })
            > pushes_before
    });
    runtime.shutdown().unwrap();
}

#[test]
fn paused_config_defers_picture_assets_and_scene_until_a_resumed_config_is_applied() {
    let control = MockDeviceControl::default();
    let host = FakeImageSourceHostControl::default();
    let initial_digest = [0x61; protocol::ASSET_DIGEST_LEN];
    let updated_digest = [0x62; protocol::ASSET_DIGEST_LEN];
    host.stage_picture_frame("camera", initial_digest, PICTURE_BLOB, false);
    let runtime =
        start_picture_runtime(picture_config(true), &control, Some(Box::new(host.host())));
    wait_for(Duration::from_secs(1), || {
        latest_picture_push(&control.operations()).is_some_and(|push| {
            push.scene.nodes.iter().any(
                |node| matches!(node, SceneNode::Image(image) if image.digest == initial_digest),
            )
        })
    });

    apply_paused_preference(&runtime, true).unwrap();
    let before_update = control.operations().len();
    host.stage_picture_frame("camera", updated_digest, PICTURE_BLOB, false);
    runtime
        .image_source_updated("camera", updated_digest)
        .unwrap();
    thread::sleep(Duration::from_millis(30));
    let paused_operations = control.operations();
    assert!(
        paused_operations[before_update..].iter().all(|operation| {
            !matches!(
                operation,
                Operation::AssetBegin(_)
                    | Operation::AssetChunk(_, _)
                    | Operation::AssetCommit(_)
                    | Operation::AssetRelease(_)
                    | Operation::PushScene(_)
            )
        }),
        "paused picture update touched the device: {:?}",
        &paused_operations[before_update..]
    );

    apply_paused_preference(&runtime, false).unwrap();
    wait_for(Duration::from_secs(1), || {
        latest_picture_push(&control.operations()).is_some_and(|push| {
            push.scene.nodes.iter().any(
                |node| matches!(node, SceneNode::Image(image) if image.digest == updated_digest),
            )
        })
    });
    let resumed_operations = control.operations();
    let resumed = &resumed_operations[before_update..];
    let commit = resumed
        .iter()
        .position(|operation| *operation == Operation::AssetCommit(updated_digest))
        .expect("resume installs the latest picture bytes");
    let release = resumed
        .iter()
        .position(|operation| {
            matches!(operation, Operation::AssetRelease(digests) if digests == &vec![updated_digest])
        })
        .expect("resume sends the complete latest keep-set");
    let scene = resumed
        .iter()
        .position(|operation| {
            matches!(operation, Operation::PushScene(push) if push.scene.nodes.iter().any(
                |node| matches!(node, SceneNode::Image(image) if image.digest == updated_digest)
            ))
        })
        .expect("resume pushes the latest picture scene");
    assert!(
        commit < release && release < scene,
        "resume transcript: {resumed:?}"
    );
    runtime.shutdown().unwrap();
}

/// A refusal is terminal for that payload: drop it, record it against its
/// card, and continue so other dirty cards are not starved.
///
/// Both cards here are pomodoros because protocol v2 pushes one thing per card
/// -- the timer its `timer.*` bindings resolve against -- and only a pomodoro
/// has one. A clock card reaches the device as a scene and nothing else, so it
/// has no push to refuse.
#[test]
fn a_refused_push_is_not_retried_and_does_not_starve_other_cards() {
    let control = MockDeviceControl::default();
    let mut config = full_config();
    config.cards = vec![
        CardSettings::Pomodoro {
            id: "first".into(),
            label: "First".into(),
            duration_seconds: 1_500,
            template: DisplayTemplate::ProgressRing,
            tap_action: WidgetTapAction::StartPause,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
            dwell_seconds: None,
        },
        CardSettings::Pomodoro {
            id: "second".into(),
            label: "Second".into(),
            duration_seconds: 1_500,
            template: DisplayTemplate::ProgressRing,
            tap_action: WidgetTapAction::StartPause,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
            dwell_seconds: None,
        },
    ];
    config.image_sources = Vec::new();
    // `dirty_cards` is ordered, so "first" is attempted before "second": the
    // pushes below prove the cycle continued past the refusal.
    control.refuse_pushes_for("first");
    let runtime = start_runtime(config, &control);

    let snapshot = wait_for_snapshot(&runtime, Duration::from_secs(2), |snapshot| {
        !snapshot.card_errors.is_empty()
    });
    assert_eq!(snapshot.card_errors.len(), 1);
    assert_eq!(snapshot.card_errors[0].kind, CardErrorKind::DataRefused);
    assert_eq!(snapshot.card_errors[0].card_id, "first");
    assert!(
        snapshot.card_errors[0].message.contains("refused"),
        "message must be actionable, got {:?}",
        snapshot.card_errors[0].message
    );
    assert!(
        !matches!(snapshot.runtime, RuntimeState::Error { .. }),
        "a card-scoped refusal must not park the whole runtime in Error"
    );

    // The card ordered behind the refused one in the SAME cycle still reached the
    // device, and later cycles still push it.
    let second_pushes = |control: &MockDeviceControl| {
        control.count_operations(|operation| {
            matches!(operation, Operation::PushTimer { card_id, .. } if card_id == "second")
        })
    };
    assert!(
        second_pushes(&control) >= 1,
        "a refusal must not skip the cards queued behind it"
    );
    runtime
        .control_pomodoro("second", PomodoroAction::Start)
        .unwrap();
    wait_for(Duration::from_secs(2), || second_pushes(&control) >= 2);

    let first_pushes = control.count_operations(
        |operation| matches!(operation, Operation::PushTimer { card_id, .. } if card_id == "first"),
    );
    assert_eq!(
        first_pushes, 1,
        "the refused payload must be attempted once, not re-queued every cycle"
    );
    runtime.shutdown().unwrap();
}

#[test]
fn command_queue_rejects_pressure_without_growing() {
    let control = MockDeviceControl::default();
    let mut runtime_options = options();
    runtime_options.command_capacity = 1;
    let runtime = Arc::new(start_runtime_with_options(
        full_config(),
        &control,
        runtime_options,
    ));
    wait_until_online(&runtime);
    let gate = control.block_next_push();
    let first_runtime = Arc::clone(&runtime);
    let first =
        thread::spawn(move || first_runtime.control_pomodoro("pomodoro", PomodoroAction::Start));
    gate.wait_until_entered();
    let second_runtime = Arc::clone(&runtime);
    let second = thread::spawn(move || apply_paused_preference(&second_runtime, true));
    thread::sleep(Duration::from_millis(20));
    assert_eq!(
        runtime.control_pomodoro("pomodoro", PomodoroAction::Reset),
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
fn shutdown_returns_queue_full_without_waiting_for_queue_space() {
    let control = MockDeviceControl::default();
    let mut runtime_options = options();
    runtime_options.command_capacity = 1;
    runtime_options.command_timeout = Duration::from_millis(250);
    let runtime = Arc::new(start_runtime_with_options(
        full_config(),
        &control,
        runtime_options,
    ));
    wait_until_online(&runtime);
    let gate = control.block_next_push();
    let first_runtime = Arc::clone(&runtime);
    let first =
        thread::spawn(move || first_runtime.control_pomodoro("pomodoro", PomodoroAction::Start));
    gate.wait_until_entered();
    let queued_runtime = Arc::clone(&runtime);
    let queued = thread::spawn(move || apply_paused_preference(&queued_runtime, true));
    thread::sleep(Duration::from_millis(20));

    let shutdown_runtime = Arc::clone(&runtime);
    let (result_sender, result_receiver) = std::sync::mpsc::sync_channel(1);
    let shutdown = thread::spawn(move || {
        let _ = result_sender.send(shutdown_runtime.shutdown());
    });
    let while_closed = result_receiver.recv_timeout(Duration::from_millis(100));
    gate.open();
    let _ = first.join().unwrap();
    let _ = queued.join().unwrap();
    let eventual = while_closed
        .clone()
        .or_else(|_| result_receiver.recv_timeout(Duration::from_secs(1)));
    shutdown.join().unwrap();

    assert_eq!(
        while_closed,
        Ok(Err(RuntimeError::QueueFull)),
        "shutdown must reject a full queue while the worker remains gated; eventual={eventual:?}"
    );
    runtime.shutdown().unwrap();
}

#[test]
fn accepted_shutdown_times_out_while_device_io_is_blocked_and_retry_reaps() {
    let control = MockDeviceControl::default();
    let mut runtime_options = options();
    runtime_options.command_timeout = Duration::from_millis(50);
    let runtime = Arc::new(start_runtime_with_options(
        full_config(),
        &control,
        runtime_options,
    ));
    wait_until_online(&runtime);
    let gate = control.block_next_push();
    let command_runtime = Arc::clone(&runtime);
    let command =
        thread::spawn(move || command_runtime.control_pomodoro("pomodoro", PomodoroAction::Start));
    gate.wait_until_entered();

    let shutdown_runtime = Arc::clone(&runtime);
    let (result_sender, result_receiver) = std::sync::mpsc::sync_channel(1);
    let shutdown = thread::spawn(move || {
        let _ = result_sender.send(shutdown_runtime.shutdown());
    });
    let while_closed = result_receiver.recv_timeout(Duration::from_millis(150));
    gate.open();
    let _ = command.join().unwrap();
    let eventual = while_closed
        .clone()
        .or_else(|_| result_receiver.recv_timeout(Duration::from_secs(1)));
    shutdown.join().unwrap();

    assert_eq!(
        while_closed,
        Ok(Err(RuntimeError::ResponseTimeout)),
        "accepted shutdown must return on its budget while I/O is blocked; eventual={eventual:?}"
    );
    assert_eq!(runtime.shutdown(), Ok(()));
}

#[test]
fn shutdown_waits_for_actual_device_cleanup_completion() {
    let control = MockDeviceControl::default();
    let drop_gate = Arc::new(PushGate::default());
    let mut runtime_options = options();
    runtime_options.command_timeout = Duration::from_millis(50);
    let runtime = Arc::new(
        RuntimeHandle::start(
            full_config(),
            Box::new(MockDevice::with_drop_gate(control, Arc::clone(&drop_gate))),
            runtime_options,
        )
        .unwrap(),
    );
    wait_until_online(&runtime);

    let shutdown_runtime = Arc::clone(&runtime);
    let (result_sender, result_receiver) = std::sync::mpsc::sync_channel(1);
    let shutdown = thread::spawn(move || {
        let _ = result_sender.send(shutdown_runtime.shutdown());
    });
    drop_gate.wait_until_entered();
    let during_cleanup = result_receiver.recv_timeout(Duration::from_millis(100));
    drop_gate.open();
    let eventual = during_cleanup
        .clone()
        .or_else(|_| result_receiver.recv_timeout(Duration::from_secs(1)));
    shutdown.join().unwrap();

    assert_eq!(
        during_cleanup,
        Ok(Err(RuntimeError::ResponseTimeout)),
        "command receipt is not worker completion; eventual={eventual:?}"
    );
    assert_eq!(runtime.shutdown(), Ok(()));
}

#[test]
fn concurrent_and_repeated_shutdown_are_idempotent() {
    let control = MockDeviceControl::default();
    let drop_gate = Arc::new(PushGate::default());
    let runtime = Arc::new(
        RuntimeHandle::start(
            full_config(),
            Box::new(MockDevice::with_drop_gate(control, Arc::clone(&drop_gate))),
            options(),
        )
        .unwrap(),
    );
    wait_until_online(&runtime);
    let barrier = Arc::new(std::sync::Barrier::new(4));
    let callers: Vec<_> = (0..4)
        .map(|_| {
            let runtime = Arc::clone(&runtime);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                runtime.shutdown()
            })
        })
        .collect();
    drop_gate.wait_until_entered();
    thread::sleep(Duration::from_millis(20));
    drop_gate.open();
    let results: Vec<_> = callers
        .into_iter()
        .map(|caller| caller.join().unwrap())
        .collect();

    assert_eq!(results, vec![Ok(()), Ok(()), Ok(()), Ok(())]);
    assert_eq!(runtime.shutdown(), Ok(()));
}

#[test]
fn worker_panic_is_reported_then_reaped_shutdown_is_idempotent() {
    let control = MockDeviceControl::default();
    let runtime = RuntimeHandle::start(
        full_config(),
        Box::new(MockDevice::panics_on_drop(control)),
        options(),
    )
    .unwrap();
    wait_until_online(&runtime);

    assert_eq!(runtime.shutdown(), Err(RuntimeError::WorkerStopped));
    assert_eq!(runtime.shutdown(), Ok(()));
}

#[test]
fn dropping_runtime_does_not_wait_for_a_blocked_worker() {
    let control = MockDeviceControl::default();
    let scene_gate = control.block_next_scene();
    let mut runtime_options = options();
    runtime_options.command_timeout = Duration::from_millis(50);
    let runtime = start_runtime_with_options(full_config(), &control, runtime_options);
    scene_gate.wait_until_entered();
    let (dropped_sender, dropped_receiver) = std::sync::mpsc::sync_channel(1);
    let dropper = thread::spawn(move || {
        drop(runtime);
        let _ = dropped_sender.send(());
    });
    let while_closed = dropped_receiver.recv_timeout(Duration::from_millis(100));
    scene_gate.open();
    let eventual = while_closed.or_else(|_| dropped_receiver.recv_timeout(Duration::from_secs(1)));
    dropper.join().unwrap();

    assert_eq!(
        while_closed,
        Ok(()),
        "dropping the handle must detach from blocked worker cleanup; eventual={eventual:?}"
    );
}

#[test]
fn invalid_commands_do_not_mutate_runtime_state() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(AppConfig::default(), &control);
    wait_until_online(&runtime);
    let before = runtime.snapshot().unwrap().config;
    let apply_count =
        control.count_operations(|operation| matches!(operation, Operation::ApplyLayout(_)));
    assert!(matches!(
        runtime.control_pomodoro("missing", PomodoroAction::Reset),
        Err(RuntimeError::UnknownCard { .. })
    ));
    let mut invalid = AppConfig::default();
    invalid.cards.push(invalid.cards[0].clone());
    assert!(matches!(
        runtime.apply_config(invalid),
        Err(RuntimeError::InvalidConfig { issues })
            if issues.iter().any(|issue| issue.code == app_core::ValidationCode::DuplicateId)
    ));
    assert_eq!(runtime.snapshot().unwrap().config, before);
    assert_eq!(
        control.count_operations(|operation| { matches!(operation, Operation::ApplyLayout(_)) }),
        apply_count
    );
    runtime.shutdown().unwrap();
}

#[test]
fn config_and_preference_edits_preserve_live_timer_and_screen() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control);
    wait_until_online(&runtime);
    runtime
        .control_pomodoro("pomodoro", PomodoroAction::Start)
        .unwrap();
    control.navigate_next(1, "pomodoro");
    let before = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot
            .pomodoros
            .first()
            .is_some_and(|timer| timer.state == PomodoroState::Running)
            && snapshot.device.active_card_id.as_deref() == Some("pomodoro")
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
    edited.preferences.autostart = false;
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
    assert_eq!(after.device.active_card_id.as_deref(), Some("pomodoro"));
    assert!(control.operations().contains(&Operation::ApplyLayout(90)));
    runtime.shutdown().unwrap();
}

#[test]
fn runtime_restart_intentionally_resets_transient_timer_state_to_idle() {
    let config = full_config();
    let first_control = MockDeviceControl::default();
    let first = start_runtime(config.clone(), &first_control);
    first
        .control_pomodoro("pomodoro", PomodoroAction::Start)
        .unwrap();
    wait_for_pomodoro_state(&first, Duration::from_secs(1), PomodoroState::Running);
    first.shutdown().unwrap();

    let second_control = MockDeviceControl::default();
    let second = start_runtime(config, &second_control);
    let restarted = wait_until_online(&second);
    let timer = restarted.pomodoros.first().unwrap();
    assert_eq!(timer.state, PomodoroState::Idle);
    assert_eq!(timer.remaining_seconds, timer.duration_seconds);
    second.shutdown().unwrap();
}

#[test]
fn applying_a_config_outlasts_the_ordinary_command_budget() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(AppConfig::default(), &control);
    wait_for_snapshot(&runtime, Duration::from_secs(5), |snapshot| {
        matches!(snapshot.device.connection, ConnectionState::Online)
    });

    // Longer than options()'s one-second command_timeout, so the default budget
    // would report a timeout here.
    control.set_call_delay(Duration::from_millis(1_500));

    let mut config = AppConfig::default();
    config.preferences.autostart = true;
    let result = runtime.apply_config(config);

    control.set_call_delay(Duration::from_millis(0));
    assert!(
        result.is_ok(),
        "a slow but successful synchronize must not be reported as a timeout: {result:?}"
    );
}
