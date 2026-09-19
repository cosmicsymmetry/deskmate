use super::*;

#[test]
fn preview_card_scene_builds_the_same_face_the_device_would_receive() {
    // The preview must not be a second renderer: it builds the scene the device
    // would be pushed. A picture card is refused rather than drawn blank,
    // because its frames live on the server and this process has no
    // ImageSourceHost.
    let mut config = AppConfig::default();
    let clock_id = config.cards[0].id().to_owned();

    let scene = app_core::preview_card_scene(&config, &clock_id, &[])
        .expect("a clock card previews locally");
    assert!(
        !scene.nodes.is_empty(),
        "a clock preview must draw something"
    );

    let missing = app_core::preview_card_scene(&config, "no-such-card", &[]);
    assert!(
        missing.is_err(),
        "an unknown card id must be a typed refusal, not an empty frame"
    );

    config.cards.push(CardSettings::Picture {
        id: "picture".into(),
        title: "Usage".into(),
        source_id: "usage".into(),
        tap_action: WidgetTapAction::None,
        refresh: RefreshPolicy::DeviceLocal,
        alert: CardAlert::None,
        dwell_seconds: None,
    });
    let picture = app_core::preview_card_scene(&config, "picture", &[]);
    assert!(
        picture.is_err(),
        "a picture card has no locally renderable face and must be refused"
    );
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

    let runtime = start_runtime(full_config(), &control);
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

    let runtime = start_runtime(full_config(), &control);
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
fn push_scene_uses_the_runtime_owned_device_without_reconnecting() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control);
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
    let runtime = start_runtime(full_config(), &control);

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
    let runtime = start_runtime(full_config(), &control);
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
    let runtime = start_runtime(full_config(), &control);

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
    let runtime = start_runtime(full_config(), &control);
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
    let runtime = start_runtime(full_config(), &control);

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

/// A device that does not advertise scene rendering still receives the card
/// model -- config and activation -- because that is what tells it which card
/// is live and what a tap on it means. What it does not receive is a face.
///
/// Protocol v1 called this the "legacy widget path": the device drew a C
/// template from the pushed field bag instead. There is no such path in v2,
/// so the card says so as a typed refusal rather than going quietly blank.
#[test]
fn a_device_without_scene_rendering_gets_the_card_model_but_no_face() {
    let control = MockDeviceControl::default();
    let mut legacy = status(42);
    legacy.capabilities &= !protocol::CAPABILITY_SCENE_RENDER;
    control.set_status(legacy);
    let runtime = start_runtime(full_config(), &control);

    wait_for(Duration::from_secs(1), || {
        let operations = control.operations();
        operations.contains(&Operation::ApplyLayout(270))
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
        "firmware without bit 8 must not be sent a scene"
    );

    let snapshot = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        !snapshot.card_errors.is_empty()
    });
    assert_eq!(snapshot.card_errors[0].kind, CardErrorKind::SceneRefused);
    assert_eq!(snapshot.card_errors[0].card_id, "clock");
    runtime.shutdown().unwrap();
}

#[test]
fn render_path_is_re_resolved_from_fresh_capabilities_on_every_reconnect() {
    let control = MockDeviceControl::default();
    let mut legacy = status(42);
    legacy.capabilities &= !protocol::CAPABILITY_SCENE_RENDER;
    control.set_status(legacy.clone());
    let runtime = start_runtime(full_config(), &control);

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
        "scene delivery must stop when the device no longer advertises scene rendering"
    );
    runtime.shutdown().unwrap();
}

#[test]
fn a_refused_scene_is_card_scoped_visible_and_not_periodically_retried() {
    let control = MockDeviceControl::default();
    control.refuse_scenes_for("clock");
    let runtime = start_runtime(full_config(), &control);

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
    let runtime = start_runtime(full_config(), &control);

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
        let runtime = start_runtime(full_config(), &control);

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
fn no_device_scene_failure_enters_reconnect_without_becoming_a_card_fault() {
    let control = MockDeviceControl::default();
    control.fail_next_scene_with(DeviceError::NoDevice);
    let runtime = start_runtime(full_config(), &control);

    wait_for(Duration::from_secs(1), || control.connection_count() >= 2);
    let snapshot = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });
    assert!(snapshot.card_errors.is_empty());
    assert!(
        control
            .operations()
            .iter()
            .filter(|operation| matches!(operation, Operation::PushScene(_)))
            .count()
            >= 2,
        "the reconnect did not retry the scene event"
    );
    runtime.shutdown().unwrap();
}

#[test]
fn terminal_scene_session_faults_are_visible_and_never_silently_retried() {
    for error in [
        DeviceError::VersionMismatch(2),
        DeviceError::InvalidRequest,
        DeviceError::RevisionExhausted,
    ] {
        let expected = error.to_string();
        let control = MockDeviceControl::default();
        control.fail_next_scene_with(error);
        let runtime = start_runtime(full_config(), &control);

        let snapshot = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
            matches!(
                &snapshot.runtime,
                RuntimeState::Error { message } if message.contains(&expected)
            )
        });
        assert!(snapshot.card_errors.is_empty());
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
            "terminal scene session fault was silently retried"
        );
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
    let runtime = start_runtime(full_config(), &control);

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
fn scenes_push_on_config_and_navigation_events_but_not_clock_or_pomodoro_ticks() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control);
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

    control.push_event(DeviceEvent {
        sequence: 1,
        kind: EventKind::Navigation,
        card_id: "pomodoro".into(),
        action: EventAction::NavigateNext,
        interrupt_token: None,
    });
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.active_card_id.as_deref() == Some("pomodoro")
    });
    wait_for(Duration::from_secs(1), || {
        control.operations().iter().rev().any(
            |operation| matches!(operation, Operation::PushScene(push) if push.card_id == "pomodoro"),
        )
    });
    let after_activation = scene_count();
    assert!(after_activation > after_config);
    runtime
        .control_pomodoro("pomodoro", PomodoroAction::Start)
        .unwrap();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot
            .pomodoros
            .iter()
            .any(|timer| timer.card_id == "pomodoro" && timer.state == PomodoroState::Running)
    });
    let after_timer_start = scene_count();
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        scene_count(),
        after_timer_start,
        "pomodoro scheduler ticks update bindings through PushTimer, not scene rebuilds"
    );

    runtime.shutdown().unwrap();
}
