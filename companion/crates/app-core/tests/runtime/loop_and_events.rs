use super::*;

#[test]
fn a_tap_on_a_picture_card_reaches_the_sink_with_its_source() {
    let control = MockDeviceControl::default();
    let sink = Arc::new(RecordingTapSink::default());
    let runtime = start_runtime_with_tap_sink(
        config_with_picture("usage", "claude-limits"),
        &control,
        sink.clone(),
    );
    wait_until_online(&runtime);

    control.push_event(tap_event(1, "usage"));
    wait_for(Duration::from_secs(1), || {
        !sink.0.lock().unwrap().is_empty()
    });

    assert_eq!(
        sink.taken(),
        vec![("usage".to_owned(), "claude-limits".to_owned())]
    );
    runtime.shutdown().unwrap();
}

#[test]
fn a_tap_on_a_pomodoro_still_toggles_its_timer_and_never_reaches_the_sink() {
    let control = MockDeviceControl::default();
    let sink = Arc::new(RecordingTapSink::default());
    let runtime =
        start_runtime_with_tap_sink(config_with_pomodoro("focus"), &control, sink.clone());
    wait_until_online(&runtime);

    control.push_event(tap_event(1, "focus"));
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot
            .pomodoros
            .iter()
            .any(|timer| timer.card_id == "focus" && timer.state == PomodoroState::Running)
    });

    assert!(sink.taken().is_empty());
    runtime.shutdown().unwrap();
}

#[test]
fn a_tap_naming_a_card_that_does_not_exist_is_counted_and_dropped() {
    let control = MockDeviceControl::default();
    let sink = Arc::new(RecordingTapSink::default());
    let runtime = start_runtime_with_tap_sink(
        config_with_picture("usage", "claude-limits"),
        &control,
        sink.clone(),
    );
    wait_until_online(&runtime);

    control.push_event(tap_event(1, "ghost"));
    wait_for(Duration::from_secs(1), || runtime.taps_dropped() == 1);

    assert!(sink.taken().is_empty());
    assert_eq!(runtime.taps_dropped(), 1);
    // The accessor is in-process only, so it cannot answer the question a real
    // board raises: was this tap declined by the host, or did its event never
    // arrive? That needs the counter in the snapshot the admin API and the
    // companion read. On 2026-09-24 it was asked of `dev-0005` and could not be
    // answered, because the snapshot carries every other counter but this one.
    assert_eq!(
        runtime.snapshot().unwrap().diagnostics.taps_dropped,
        1,
        "a dropped tap must be visible in the snapshot, not only through the accessor"
    );
    runtime.shutdown().unwrap();
}

#[test]
fn a_runtime_with_no_sink_drops_a_picture_tap_without_panicking() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(config_with_picture("usage", "claude-limits"), &control);
    wait_until_online(&runtime);

    control.push_event(tap_event(1, "usage"));
    wait_for(Duration::from_secs(1), || runtime.taps_dropped() == 1);

    assert!(runtime.snapshot().is_ok());
    assert_eq!(runtime.taps_dropped(), 1);
    runtime.shutdown().unwrap();
}

#[test]
fn rotation_follows_card_order_and_each_cards_own_dwell() {
    let control = MockDeviceControl::default();
    // "b" first with an explicit 5s dwell, then "a" taking the 10s default:
    // since schema v10 the card order IS the loop order.
    let config = AppConfig {
        cards: vec![with_dwell(clock_card("b"), 5), clock_card("a")],
        advance: CarouselAdvance::Timed {
            default_dwell_seconds: 10,
        },
        ..AppConfig::default()
    };
    let runtime = start_runtime(config, &control);

    wait_for(Duration::from_secs(1), || {
        activated_card_ids(&control).first().map(String::as_str) == Some("b")
    });
    wait_for(Duration::from_secs(7), || {
        activated_card_ids(&control).get(1).map(String::as_str) == Some("a")
    });
    wait_for(Duration::from_secs(12), || {
        activated_card_ids(&control).get(2).map(String::as_str) == Some("b")
    });

    assert_eq!(&activated_card_ids(&control)[..3], ["b", "a", "b"]);
    runtime.shutdown().unwrap();
}

#[test]
fn reordering_the_loop_replays_and_keeps_the_card_on_the_panel() {
    // Reordering the loop must keep the panel on the card it is showing --
    // an unrelated edit must not jump to a different card.
    let control = MockDeviceControl::default();
    let mut config = AppConfig {
        cards: vec![clock_card("shared"), clock_card("new-first")],
        advance: CarouselAdvance::Manual,
        ..AppConfig::default()
    };
    let runtime = start_runtime(config.clone(), &control);
    wait_for(Duration::from_secs(1), || {
        activated_card_ids(&control) == ["shared"]
    });
    let apply_count_before =
        control.count_operations(|operation| matches!(operation, Operation::ApplyLayout(_)));

    config.cards.reverse();
    runtime.apply_config(config.clone()).unwrap();
    wait_for(Duration::from_secs(1), || {
        control.count_operations(|operation| matches!(operation, Operation::ApplyLayout(_)))
            == apply_count_before + 1
    });
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.active_card_id.as_deref() == Some("shared")
    });

    // But a card that leaves the loop cannot stay on the panel: the first card
    // of the new loop takes over.
    config.cards.retain(|card| card.id() != "shared");
    runtime.apply_config(config).unwrap();
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.active_card_id.as_deref() == Some("new-first")
    });
    runtime.shutdown().unwrap();
}

#[test]
fn alert_from_a_non_visible_loop_card_still_fires() {
    let control = MockDeviceControl::default();
    let config = AppConfig {
        cards: vec![
            clock_card("visible"),
            CardSettings::Pomodoro {
                id: "off-screen".into(),
                label: "Off-screen".into(),
                duration_seconds: 1,
                template: DisplayTemplate::ProgressRing,
                tap_action: WidgetTapAction::None,
                refresh: RefreshPolicy::DeviceLocal,
                alert: CardAlert::OnTimerFinish {
                    hold: AlertHold::UntilDismissed,
                },
                dwell_seconds: None,
            },
        ],
        advance: CarouselAdvance::Manual,
        ..AppConfig::default()
    };
    let runtime = start_runtime(config, &control);
    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
            && snapshot.device.active_card_id.as_deref() == Some("visible")
    });

    runtime
        .control_pomodoro("off-screen", PomodoroAction::Start)
        .unwrap();
    wait_for(Duration::from_secs(2), || {
        control.operations().contains(&Operation::Interrupt(1))
    });
    control.push_event(DeviceEvent {
        sequence: 1,
        kind: EventKind::InterruptDismissed,
        card_id: "off-screen".into(),
        action: EventAction::DismissInterrupt,
        interrupt_token: Some(1),
    });

    let after_dismissal = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.active_card_id.as_deref() == Some("visible")
    });
    assert_eq!(
        after_dismissal.device.active_card_id.as_deref(),
        Some("visible")
    );
    runtime.shutdown().unwrap();
}

#[test]
fn manual_advance_has_no_rotation_deadline() {
    let control = MockDeviceControl::default();
    let config = AppConfig {
        cards: vec![clock_card("first"), clock_card("second")],
        advance: CarouselAdvance::Manual,
        ..AppConfig::default()
    };
    let runtime = start_runtime(config, &control);
    wait_for(Duration::from_secs(1), || {
        activated_card_ids(&control) == ["first"]
    });

    thread::sleep(Duration::from_secs(6));
    assert_eq!(activated_card_ids(&control), ["first"]);
    runtime.shutdown().unwrap();
}

// Rotation's pure logic (per-card dwell resolution, manual-mode disarming,
// ordering/wrap-around, and swipe index resolution) is covered by instant unit
// tests in `src/runtime/mod.rs`, using synthetic `Instant`s. This one-dwell test
// proves that `scheduler.rotation_due` in the real `run_runtime` loop reaches
// the mock device via `advance_rotation` → `active_card_dirty` → `send_screen`.
// One dwell period (the validated minimum, 5s) is enough to prove that wiring;
// the unit tests pin the ordering rules.
#[test]
fn timed_advance_wiring_reaches_the_device_after_one_dwell() {
    let control = MockDeviceControl::default();
    let config = AppConfig {
        advance: CarouselAdvance::Timed {
            default_dwell_seconds: 5,
        },
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
    let runtime = start_runtime(config, &control);
    wait_until_online(&runtime);
    // The initial full sync activates the first card ("first"),
    // independent of rotation.
    wait_for(Duration::from_secs(1), || {
        activated_card_ids(&control) == ["first"]
    });

    // One dwell later, rotation has walked to the next card in the loop and the
    // activation reached the mock device.
    wait_for(Duration::from_secs(8), || {
        activated_card_ids(&control) == ["first", "alerting"]
    });
    runtime.shutdown().unwrap();
}

/// Proves the `WorkerState::latest_fields` -> `AppSnapshot.card_data` wiring
/// (`src/runtime/mod.rs`'s `snapshot` method) by driving a real `RuntimeHandle` and
/// reading `card_data` back off `RuntimeHandle::snapshot()`. A pomodoro card
/// pushes its fields into
/// `latest_fields` synchronously at config-install time (no device
/// connection round trip needed), which makes the live card's
/// exact field values available on the very first snapshot.
///
/// A second otherwise-identical pomodoro card is included because since schema
/// v10 both are in the loop -- a card that contributes no entry at all is no
/// longer representable, so what this now pins is that BOTH get their live
/// fields rather than only the active one.
#[test]
fn card_data_carries_live_fields_for_every_card_in_the_loop() {
    let control = MockDeviceControl::default();
    let config = AppConfig {
        advance: CarouselAdvance::Manual,
        cards: vec![
            pomodoro_card("on", CardAlert::None),
            pomodoro_card("second", CardAlert::None),
        ],
        ..AppConfig::default()
    };
    let runtime = start_runtime(config, &control);

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
        snapshot
            .card_data
            .iter()
            .any(|card| card.card_id == "second"),
        "every card is in the loop since v10, so every card contributes card_data"
    );

    runtime.shutdown().unwrap();
}

/// The interrupt token counter must follow the device's *interrupt* counter and
/// nothing else. `latest_revision` and `config_revision` are the data-push and
/// config counters; they climb with ordinary traffic and have no relationship to
/// interrupts. Coupling them would create token gaps that look like lost
/// interrupts whenever ordinary data revisions advance.
#[test]
fn interrupt_tokens_do_not_follow_the_unrelated_revision_counters() {
    assert_next_interrupt_token(84, 77, 60, 61);
}

/// The other half of the same rule. `latest_interrupt_token` decodes to zero
/// when the field is absent, and zero is also what a device reports before it
/// has accepted any interrupt. In both cases there is no interrupt counter to
/// follow, so revisions stay as the monotonic floor.
#[test]
fn an_absent_interrupt_counter_still_takes_the_revision_floor() {
    assert_next_interrupt_token(84, 77, 0, 85);
}

/// A dismissal whose token the arbiter no longer tracks is deliberately not
/// applied — but it must not be *invisible*. The common cause is benign (a
/// bounded hold expired host-side and freed the slot before the user tapped the
/// overlay the device was still showing). Counting the event distinguishes a
/// deliberately ignored dismissal from an event that never arrived.
#[test]
fn a_dismissal_for_an_untracked_token_is_counted_rather_than_silently_dropped() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control);
    wait_until_online(&runtime);
    // No interrupt was ever scheduled, so token 4242 is tracked by nobody.
    control.push_event(DeviceEvent {
        sequence: 1,
        kind: EventKind::InterruptDismissed,
        card_id: "pomodoro".into(),
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
/// (`Pomodoro::toggle` on `Completed` only refreshes), but the firmware
/// applies optimistic local feedback on every `START_PAUSE` tap regardless of
/// state, repainting the arc and status as running. Only an authoritative push
/// of `running: false` restores the completed state, so the host must still
/// push after a no-op tap.
#[test]
fn a_tap_on_a_completed_pomodoro_still_pushes_authoritative_state() {
    let control = MockDeviceControl::default();
    let config = short_pomodoro_config(CardAlert::None);
    let runtime = start_runtime(config, &control);
    wait_until_online(&runtime);
    control.tap_pomodoro(1);
    wait_for_pomodoro_state(&runtime, Duration::from_secs(3), PomodoroState::Completed);

    let before = control.operations().len();

    // The no-op tap.
    control.tap_pomodoro(2);
    wait_for(Duration::from_secs(2), || {
        control.operations()[before..].iter().any(|operation| {
            matches!(operation, Operation::PushTimer {
                card_id,
                total_ms: 1_000,
                remaining_ms: 0,
                running: false,
            } if card_id == "pomodoro")
        })
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
