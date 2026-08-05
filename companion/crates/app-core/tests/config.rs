use app_core::{
    AlertHold, AppConfig, AppSnapshot, AssetSettings, CardAlert, CardPresence, CardSettings,
    CarouselAdvance, CarouselSettings, ConnectionState, DeviceCounters, DeviceSnapshot,
    DisplayTemplate, FirmwareArtifactMetadata, MAX_ASSET_BYTES, MAX_PROVIDER_URL_LEN,
    MAX_UPDATE_ARTIFACT_BYTES, PersistenceState, PomodoroSnapshot, PomodoroState, ProviderSnapshot,
    ProviderState, RefreshPolicy, RuntimeDiagnostics, RuntimeState, ValidationCode,
    WidgetTapAction,
};
use protocol::{
    CAPABILITY_ASSET_TRANSFER, CAPABILITY_CONFIG_ROTATION, CAPABILITY_CORE_WIDGETS,
    CAPABILITY_EXTENDED_TEMPLATES, CAPABILITY_HOST_TAP_ACTIONS, InterruptPolicy, Message,
    SizeClass, TapAction, TemplateKind, encode_message,
};

const DEFAULT_JSON: &str = include_str!("fixtures/default.json");
const FULL_JSON: &str = include_str!("fixtures/full.json");
const INVALID_JSON: &str = include_str!("fixtures/invalid.json");
const FUTURE_JSON: &str = include_str!("fixtures/future-v4.json");
const MALFORMED_JSON: &str = include_str!("fixtures/malformed.json");
const CARD_SURFACE_JSON: &str = include_str!("fixtures/card-surface.json");

#[test]
fn default_fixture_is_the_canonical_default() {
    let from_fixture: AppConfig = serde_json::from_str(DEFAULT_JSON).unwrap();
    assert_eq!(from_fixture, AppConfig::default());
    from_fixture.validate().unwrap();

    let round_trip = serde_json::to_string(&from_fixture).unwrap();
    let decoded: AppConfig = serde_json::from_str(&round_trip).unwrap();
    assert_eq!(decoded, from_fixture);
}

#[test]
fn full_fixture_compiles_deterministically_to_m2_contract() {
    let config: AppConfig = serde_json::from_str(FULL_JSON).unwrap();
    let first = config.compile(42).unwrap();
    let second = config.compile(42).unwrap();
    assert_eq!(first, second);

    assert_eq!(first.layout.revision, 42);
    assert_eq!(first.layout.rotation, 270);
    assert_eq!(first.layout.widgets.len(), 3);
    assert_eq!(first.layout.screens[0].screen_id, "clock");
    assert_eq!(first.layout.screens[1].screen_id, "pomodoro");
    assert_eq!(first.layout.screens[2].screen_id, "calendar");

    let clock = &first.layout.widgets[0];
    assert_eq!(clock.template, TemplateKind::DigitalClock);
    assert_eq!(clock.size_class, SizeClass::Full);
    assert_eq!(clock.tap_action, TapAction::None);
    assert_eq!(clock.interrupt_policy, InterruptPolicy::Disabled);

    let pomodoro = &first.layout.widgets[1];
    assert_eq!(pomodoro.template, TemplateKind::ProgressRing);
    assert_eq!(pomodoro.size_class, SizeClass::Full);
    assert_eq!(pomodoro.tap_action, TapAction::StartPause);
    assert_eq!(pomodoro.interrupt_policy, InterruptPolicy::Enabled);

    let calendar = &first.layout.widgets[2];
    assert_eq!(calendar.template, TemplateKind::RowList);
    assert_eq!(calendar.size_class, SizeClass::Full);
    assert_eq!(calendar.interrupt_policy, InterruptPolicy::Disabled);

    assert_eq!(first.initial_pushes.len(), 3);
    assert_eq!(first.initial_pushes[0].fields.len(), 4);
    assert_eq!(first.initial_pushes[1].fields.len(), 6);
    assert_eq!(first.initial_pushes[2].fields.len(), 13);

    encode_message(1, &Message::ApplyConfig(first.layout)).unwrap();
    for (request_id, push) in first.initial_pushes.into_iter().enumerate() {
        encode_message(
            u32::try_from(request_id + 2).unwrap(),
            &Message::PushData(push),
        )
        .unwrap();
    }
}

#[test]
fn invalid_fixture_reports_all_domain_boundaries_before_compile() {
    let config: AppConfig = serde_json::from_str(INVALID_JSON).unwrap();
    let error = config.compile(1).unwrap_err();
    let codes = error
        .issues
        .iter()
        .map(|issue| issue.code)
        .collect::<Vec<_>>();

    for expected in [
        ValidationCode::InvalidTimezone,
        ValidationCode::DuplicateId,
        ValidationCode::OutOfRange,
        ValidationCode::InvalidSource,
        ValidationCode::MissingReference,
    ] {
        assert!(codes.contains(&expected), "missing {expected:?}: {error:?}");
    }
}

#[test]
fn future_schema_establishes_a_clean_migration_boundary() {
    let config: AppConfig = serde_json::from_str(FUTURE_JSON).unwrap();
    let error = config.validate().unwrap_err();
    assert!(error.issues.iter().any(|issue| {
        issue.path == "schema_version" && issue.code == ValidationCode::UnsupportedVersion
    }));
}

#[test]
fn card_surface_is_closed_bounded_and_capability_gated() {
    let config: AppConfig = serde_json::from_str(CARD_SURFACE_JSON).unwrap();
    config.validate().unwrap();
    assert_eq!(
        serde_json::from_str::<AppConfig>(&serde_json::to_string(&config).unwrap()).unwrap(),
        config
    );

    let expected_capabilities = CAPABILITY_CORE_WIDGETS
        | CAPABILITY_CONFIG_ROTATION
        | CAPABILITY_EXTENDED_TEMPLATES
        | CAPABILITY_HOST_TAP_ACTIONS
        | CAPABILITY_ASSET_TRANSFER;
    assert_eq!(config.required_device_capabilities(), expected_capabilities);

    let first = config.compile(9).unwrap_err();
    let second = config.compile(9).unwrap_err();
    assert_eq!(first, second);
    assert!(
        first
            .issues
            .iter()
            .all(|issue| issue.code == ValidationCode::RequiresCapability)
    );
}

#[test]
fn card_surface_rejects_oversized_sources_and_asset_budgets() {
    let mut config: AppConfig = serde_json::from_str(CARD_SURFACE_JSON).unwrap();
    if let CardSettings::JsonFeed { url, .. } = &mut config.cards[4] {
        *url = format!("https://example.test/{}", "x".repeat(MAX_PROVIDER_URL_LEN));
    }
    let asset = config.assets[0].clone();
    config.assets = (0..5)
        .map(|index| AssetSettings {
            id: format!("asset-{index}"),
            maximum_bytes: MAX_ASSET_BYTES,
            ..asset.clone()
        })
        .collect();

    let error = config.validate().unwrap_err();
    for expected in [
        ValidationCode::TooLong,
        ValidationCode::TooLarge,
        ValidationCode::MissingReference,
    ] {
        assert!(
            error.issues.iter().any(|issue| issue.code == expected),
            "missing {expected:?}: {error:?}"
        );
    }
}

#[test]
fn network_provider_urls_cannot_embed_credentials_and_weather_has_a_refresh_floor() {
    let mut config: AppConfig = serde_json::from_str(CARD_SURFACE_JSON).unwrap();
    for card in &mut config.cards {
        match card {
            CardSettings::Calendar { source, .. } => {
                *source = app_core::CalendarSource::Url(
                    "https://user:calendar-secret@example.test/feed.ics".into(),
                );
            }
            CardSettings::Weather { refresh, .. } => {
                *refresh = app_core::RefreshPolicy::Interval { minutes: 9 };
            }
            CardSettings::JsonFeed { url, mappings, .. } => {
                *url = "https://user:secret@example.test/feed.json".into();
                mappings[0].field = "title".into();
            }
            _ => {}
        }
    }
    let error = config.validate().unwrap_err();
    assert_eq!(
        error
            .issues
            .iter()
            .filter(|issue| issue.code == ValidationCode::InvalidSource)
            .count(),
        2
    );
    assert!(error.issues.iter().any(|issue| {
        issue.path.ends_with(".mappings[0].field")
            && issue.code == ValidationCode::InvalidComposition
    }));
    assert!(error.issues.iter().any(|issue| {
        issue.path.ends_with(".refresh.minutes") && issue.code == ValidationCode::OutOfRange
    }));
    assert!(!format!("{error:?}").contains("secret"));
    assert!(!format!("{error:?}").contains("calendar-secret"));
}

#[test]
fn firmware_artifact_metadata_is_bounded_before_update_work_exists() {
    let valid = FirmwareArtifactMetadata {
        version: "1.0.0".into(),
        model: "waveshare-1.8".into(),
        byte_length: MAX_UPDATE_ARTIFACT_BYTES,
        sha256_hex: "a".repeat(64),
        signing_key_id: "deskmate-release-1".into(),
        signature_base64: format!("{}==", "A".repeat(86)),
    };
    valid.validate().unwrap();

    let invalid = FirmwareArtifactMetadata {
        byte_length: MAX_UPDATE_ARTIFACT_BYTES + 1,
        sha256_hex: "not-a-digest".into(),
        signature_base64: "invalid".into(),
        ..valid
    };
    let error = invalid.validate().unwrap_err();
    assert!(
        error
            .issues
            .iter()
            .any(|issue| issue.code == ValidationCode::OutOfRange)
    );
    assert!(
        error
            .issues
            .iter()
            .any(|issue| issue.code == ValidationCode::InvalidSource)
    );
}

#[test]
fn malformed_and_unknown_json_are_rejected_by_serde() {
    assert!(serde_json::from_str::<AppConfig>(MALFORMED_JSON).is_err());

    let with_unknown = DEFAULT_JSON.replace(
        "\"schema_version\": 3,",
        "\"schema_version\": 3, \"unexpected\": true,",
    );
    assert!(serde_json::from_str::<AppConfig>(&with_unknown).is_err());

    let nested_unknown = CARD_SURFACE_JSON.replacen(
        r#""template": { "kind": "icon-badge-text", "icon_asset_id": "icons" }"#,
        r#""template": { "kind": "icon-badge-text", "icon_asset_id": "icons", "unexpected": true }"#,
        1,
    );
    assert!(serde_json::from_str::<AppConfig>(&nested_unknown).is_err());

    let unknown_variant = DEFAULT_JSON.replace("digital-clock", "future-template");
    assert!(serde_json::from_str::<AppConfig>(&unknown_variant).is_err());
}

#[test]
fn card_settings_rejects_an_unknown_field_nested_inside_presence() {
    // CardSettings is itself an internally tagged enum ("kind" = clock/pomodoro/...).
    // Serde silently ignores `deny_unknown_fields` on internally tagged enums, so this
    // proves the validating Deserialize impl catches unknown fields nested inside a
    // card's own `presence` object, not just at the card's top level.
    let json = r#"{
        "kind": "clock",
        "id": "clock",
        "title": "Desk",
        "show_seconds": true,
        "template": { "kind": "digital-clock" },
        "tap_action": { "kind": "none" },
        "refresh": { "kind": "device-local" },
        "presence": { "kind": "off", "bogus": 1 },
        "alert": { "kind": "none" }
    }"#;
    assert!(serde_json::from_str::<CardSettings>(json).is_err());
}

#[test]
fn zero_revision_is_rejected_without_producing_wire_state() {
    let config = AppConfig::default();
    let error = config.compile(0).unwrap_err();
    assert_eq!(error.issues[0].path, "revision");
    assert_eq!(error.issues[0].code, ValidationCode::OutOfRange);
}

#[test]
fn runtime_snapshot_uses_tagged_states_for_frontend_contract() {
    let snapshot = AppSnapshot {
        config: AppConfig::default(),
        runtime: RuntimeState::Paused,
        device: DeviceSnapshot {
            connection: ConnectionState::Disconnected {
                reason: Some("USB unavailable".into()),
            },
            port_name: None,
            firmware_version: None,
            protocol_version: None,
            max_protocol_version: None,
            capabilities: Vec::new(),
            unknown_capability_bits: 1_u64 << 63,
            uptime_ms: None,
            free_heap: None,
            rotation: None,
            active_screen_id: Some("clock".into()),
            counters: DeviceCounters::default(),
        },
        providers: vec![ProviderSnapshot {
            widget_id: "calendar".into(),
            state: ProviderState::Stale {
                message: "offline".into(),
            },
            last_success_unix_ms: Some(1_787_000_000_000),
            age_seconds: Some(60),
        }],
        pomodoros: vec![PomodoroSnapshot {
            widget_id: "pomodoro".into(),
            state: PomodoroState::Paused,
            duration_seconds: 1_500,
            remaining_seconds: 900,
        }],
        persistence: PersistenceState::Clean,
        diagnostics: RuntimeDiagnostics::default(),
    };

    let json = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(json["runtime"]["kind"], "paused");
    assert_eq!(json["device"]["connection"]["kind"], "disconnected");
    assert_eq!(
        json["device"]["unknown_capability_bits"],
        "0x8000000000000000"
    );
    assert_eq!(json["providers"][0]["state"]["kind"], "stale");
    assert_eq!(json["pomodoros"][0]["state"], "paused");
    assert_eq!(json["persistence"]["kind"], "clean");
    assert_eq!(
        serde_json::from_value::<AppSnapshot>(json).unwrap(),
        snapshot
    );
}

#[test]
fn card_presence_resolves_dwell_against_the_carousel_default() {
    assert_eq!(
        CardPresence::InRotation {
            dwell_seconds: Some(45)
        }
        .dwell_seconds(20),
        Some(45)
    );
    assert_eq!(
        CardPresence::InRotation {
            dwell_seconds: None
        }
        .dwell_seconds(20),
        Some(20)
    );
    assert_eq!(CardPresence::AlertOnly.dwell_seconds(20), None);
    assert_eq!(CardPresence::Off.dwell_seconds(20), None);

    assert!(
        CardPresence::InRotation {
            dwell_seconds: None
        }
        .is_in_rotation()
    );
    assert!(!CardPresence::AlertOnly.is_in_rotation());
    assert!(CardPresence::Off.is_off());
    assert!(!CardPresence::AlertOnly.is_off());
}

#[test]
fn card_behaviour_types_round_trip_as_closed_tagged_json() {
    let presence = CardPresence::InRotation {
        dwell_seconds: Some(30),
    };
    let json = serde_json::to_value(presence).unwrap();
    assert_eq!(json["kind"], "in-rotation");
    assert_eq!(json["dwell_seconds"], 30);
    assert_eq!(
        serde_json::from_value::<CardPresence>(json).unwrap(),
        presence
    );

    let alert = CardAlert::BeforeEvent {
        lead_minutes: 5,
        hold: AlertHold::Seconds { value: 60 },
    };
    let json = serde_json::to_value(alert).unwrap();
    assert_eq!(json["kind"], "before-event");
    assert_eq!(json["hold"]["kind"], "seconds");
    assert_eq!(serde_json::from_value::<CardAlert>(json).unwrap(), alert);

    assert!(CardAlert::None.is_none());
    assert!(!alert.is_none());

    let advance = CarouselAdvance::Timed {
        default_dwell_seconds: 20,
    };
    assert_eq!(advance.default_dwell_seconds(), Some(20));
    assert_eq!(CarouselAdvance::Manual.default_dwell_seconds(), None);

    // Unknown fields are rejected at every level.
    assert!(
        serde_json::from_str::<CardPresence>(r#"{"kind":"alert-only","dwell_seconds":10}"#)
            .is_err()
    );
}

#[test]
fn card_alert_rejects_unknown_nested_fields_in_hold() {
    // Unknown field nested in hold for OnTimerFinish
    assert!(
        serde_json::from_str::<CardAlert>(
            r#"{"kind":"on-timer-finish","hold":{"kind":"seconds","value":60,"extra":1}}"#
        )
        .is_err(),
        "should reject unknown field in nested hold for OnTimerFinish"
    );

    // Unknown field nested in hold for BeforeEvent
    assert!(
        serde_json::from_str::<CardAlert>(
            r#"{"kind":"before-event","lead_minutes":5,"hold":{"kind":"seconds","value":60,"extra":1}}"#
        )
        .is_err(),
        "should reject unknown field in nested hold for BeforeEvent"
    );

    // Valid nested hold still works for OnTimerFinish
    let valid_timer = serde_json::from_str::<CardAlert>(
        r#"{"kind":"on-timer-finish","hold":{"kind":"until-dismissed"}}"#,
    )
    .expect("valid OnTimerFinish should deserialize");
    assert!(matches!(
        valid_timer,
        CardAlert::OnTimerFinish {
            hold: AlertHold::UntilDismissed
        }
    ));

    // Valid nested hold still works for BeforeEvent
    let valid_before = serde_json::from_str::<CardAlert>(
        r#"{"kind":"before-event","lead_minutes":10,"hold":{"kind":"seconds","value":30}}"#,
    )
    .expect("valid BeforeEvent should deserialize");
    assert!(matches!(
        valid_before,
        CardAlert::BeforeEvent {
            lead_minutes: 10,
            ..
        }
    ));
}

#[test]
fn alert_hold_rejects_unknown_fields_when_deserialized_directly() {
    // Unknown field on UntilDismissed variant
    assert!(
        serde_json::from_str::<AlertHold>(r#"{"kind":"until-dismissed","extra":1}"#).is_err(),
        "should reject unknown field on until-dismissed"
    );

    // Unknown field on Seconds variant
    assert!(
        serde_json::from_str::<AlertHold>(r#"{"kind":"seconds","value":60,"extra":1}"#).is_err(),
        "should reject unknown field on seconds"
    );

    // Valid round-trip
    let valid = serde_json::from_str::<AlertHold>(r#"{"kind":"seconds","value":60}"#).unwrap();
    assert_eq!(valid, AlertHold::Seconds { value: 60 });
}

#[test]
fn carousel_advance_rejects_unknown_fields() {
    // Unknown field on Manual variant
    assert!(
        serde_json::from_str::<CarouselAdvance>(r#"{"kind":"manual","extra":1}"#).is_err(),
        "should reject unknown field on manual"
    );

    // Unknown field on Timed variant
    assert!(
        serde_json::from_str::<CarouselAdvance>(
            r#"{"kind":"timed","default_dwell_seconds":20,"extra":1}"#
        )
        .is_err(),
        "should reject unknown field on timed"
    );

    // Valid round-trip
    let valid =
        serde_json::from_str::<CarouselAdvance>(r#"{"kind":"timed","default_dwell_seconds":20}"#)
            .unwrap();
    assert_eq!(
        valid,
        CarouselAdvance::Timed {
            default_dwell_seconds: 20
        }
    );
}

#[test]
fn card_alert_hold_method_extracts_hold() {
    assert_eq!(CardAlert::None.hold(), None);

    let timer_hold = CardAlert::OnTimerFinish {
        hold: AlertHold::UntilDismissed,
    }
    .hold();
    assert_eq!(timer_hold, Some(AlertHold::UntilDismissed));

    let before_hold = CardAlert::BeforeEvent {
        lead_minutes: 5,
        hold: AlertHold::Seconds { value: 60 },
    }
    .hold();
    assert_eq!(before_hold, Some(AlertHold::Seconds { value: 60 }));
}

fn clock_card(id: &str, presence: CardPresence) -> CardSettings {
    CardSettings::Clock {
        id: id.into(),
        title: "Desk".into(),
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
        label: "Focus".into(),
        duration_seconds: 1_500,
        template: DisplayTemplate::ProgressRing,
        tap_action: WidgetTapAction::StartPause,
        refresh: RefreshPolicy::DeviceLocal,
        presence,
        alert,
    }
}

#[test]
fn compilation_lowers_cards_to_the_frozen_wire_shape() {
    let config = AppConfig {
        schema_version: 3,
        cards: vec![
            clock_card(
                "clock",
                CardPresence::InRotation {
                    dwell_seconds: Some(10),
                },
            ),
            pomodoro_card(
                "focus",
                CardPresence::AlertOnly,
                CardAlert::OnTimerFinish {
                    hold: AlertHold::UntilDismissed,
                },
            ),
            clock_card("muted", CardPresence::Off),
        ],
        ..AppConfig::default()
    };

    let compiled = config.compile(7).unwrap();

    // Off cards vanish entirely; alert-only cards are screenless widgets.
    let widget_ids: Vec<&str> = compiled
        .layout
        .widgets
        .iter()
        .map(|widget| widget.widget_id.as_str())
        .collect();
    assert_eq!(widget_ids, ["clock", "focus"]);

    // Screen ID is the card ID, in card order, for in-rotation cards only.
    let screens: Vec<(&str, &str)> = compiled
        .layout
        .screens
        .iter()
        .map(|screen| (screen.screen_id.as_str(), screen.widget_id.as_str()))
        .collect();
    assert_eq!(screens, [("clock", "clock")]);

    // Size class is pinned to Full for every emitted widget, forever.
    assert!(
        compiled
            .layout
            .widgets
            .iter()
            .all(|widget| widget.size_class == protocol::SizeClass::Full)
    );

    // Off cards receive no initial push.
    let pushed: Vec<&str> = compiled
        .initial_pushes
        .iter()
        .map(|push| push.widget_id.as_str())
        .collect();
    assert_eq!(pushed, ["clock", "focus"]);
}

#[test]
fn compilation_is_deterministic_for_identical_input() {
    let config = AppConfig::default();
    assert_eq!(config.compile(4).unwrap(), config.compile(4).unwrap());
}

#[test]
fn timed_advance_no_longer_requires_an_unimplemented_capability() {
    let config = AppConfig {
        carousel: CarouselSettings {
            advance: CarouselAdvance::Timed {
                default_dwell_seconds: 20,
            },
        },
        ..AppConfig::default()
    };
    assert!(config.compile(1).is_ok());
}
