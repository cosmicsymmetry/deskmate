use app_core::{
    AlertHold, AppConfig, AppSnapshot, AssetSettings, CalendarSource, CardAlert, CardDataSnapshot,
    CardError, CardField, CardFieldValue, CardSettings, CarouselAdvance, ConnectionState,
    DeviceCounters, DeviceSnapshot, DisplayTemplate, FirmwareArtifactMetadata, JsonFieldMapping,
    MAX_ASSET_BYTES, MAX_PLAYLIST_ENTRIES, MAX_PLAYLIST_NAME_LEN, MAX_PLAYLISTS,
    MAX_PROVIDER_URL_LEN, MAX_UPDATE_ARTIFACT_BYTES, PersistenceState, Playlist, PlaylistEntry,
    PomodoroSnapshot, PomodoroState, ProviderSnapshot, ProviderState, RefreshPolicy,
    RuntimeDiagnostics, RuntimeState, ValidationCode, WeatherUnits, WidgetTapAction,
};
use protocol::{
    CAPABILITY_ASSET_TRANSFER, CAPABILITY_CONFIG_ROTATION, CAPABILITY_CORE_WIDGETS,
    CAPABILITY_EXTENDED_TEMPLATES, CAPABILITY_HOST_TAP_ACTIONS, InterruptPolicy, Message,
    SizeClass, TapAction, TemplateKind, encode_message,
};

const DEFAULT_JSON: &str = include_str!("fixtures/default.json");
const FULL_JSON: &str = include_str!("fixtures/full.json");
const INVALID_JSON: &str = include_str!("fixtures/invalid.json");
const FUTURE_JSON: &str = include_str!("fixtures/future-v5.json");
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
fn v4_playlist_compiles_byte_identically_to_the_v3_full_fixture() {
    // `full.json` is the schema-v4 form of the frozen schema-v3 full fixture:
    // the active playlist contains its three former InRotation cards in the
    // same order, and the pomodoro alert keeps the same interrupt policy.
    let config: AppConfig = serde_json::from_str(FULL_JSON).unwrap();
    let compiled = config.compile(42).unwrap();

    let actual = encode_message(1, &Message::ApplyConfig(compiled.layout)).unwrap();
    let v3_apply_config_wire = [
        3, 1, 9, 1, 2, 1, 1, 1, 2, 123, 2, 164, 6, 24, 42, 1, 131, 165, 12, 101, 99, 108, 111, 99,
        107, 1, 1, 2, 1, 3, 2, 4, 2, 165, 19, 104, 112, 111, 109, 111, 100, 111, 114, 111, 1, 2, 2,
        1, 3, 1, 4, 1, 165, 15, 104, 99, 97, 108, 101, 110, 100, 97, 114, 1, 3, 2, 1, 3, 2, 4, 4,
        2, 131, 162, 15, 101, 99, 108, 111, 99, 107, 1, 101, 99, 108, 111, 99, 107, 162, 21, 104,
        112, 111, 109, 111, 100, 111, 114, 111, 1, 104, 112, 111, 109, 111, 100, 111, 114, 111,
        162, 28, 104, 99, 97, 108, 101, 110, 100, 97, 114, 1, 104, 99, 97, 108, 101, 110, 100, 97,
        114, 3, 25, 1, 14, 200, 59, 108, 173, 0,
    ];

    assert_eq!(actual, v3_apply_config_wire);
}

#[test]
fn invalid_fixture_reports_all_domain_boundaries_before_compile() {
    let config: AppConfig = serde_json::from_str(INVALID_JSON).unwrap();
    let error = config.compile(1).unwrap_err();

    // Match on (path, code) pairs, not code membership: several of these codes
    // (OutOfRange especially) could otherwise be "proven" by an unrelated issue
    // elsewhere in the fixture, which would silently stop covering the rule this test
    // means to exercise.
    for (expected_path, expected_code) in [
        ("preferences.timezone", ValidationCode::InvalidTimezone),
        // Two cards share id "duplicate"; the second occurrence is flagged.
        ("cards[2].id", ValidationCode::DuplicateId),
        // playlist entry dwell_seconds: 4 is below MIN_DWELL_SECONDS (5).
        (
            "playlists[0].entries[1].dwell_seconds",
            ValidationCode::OutOfRange,
        ),
        // before-event lead_minutes: 61 is above MAX_ALERT_LEAD_MINUTES (60).
        ("cards[3].alert.lead_minutes", ValidationCode::OutOfRange),
        // on-timer-finish hold.value: 4 is below MIN_ALERT_HOLD_SECONDS (5).
        ("cards[4].alert.hold.value", ValidationCode::OutOfRange),
        // on-timer-finish alerts are only valid on pomodoro cards; cards[0] is clock.
        ("cards[0].alert", ValidationCode::OutOfRange),
        // a playlist entry referencing a card id that does not exist.
        (
            "playlists[0].entries[4].card_id",
            ValidationCode::MissingReference,
        ),
        // "on-timer-mismatch" appears twice in playlist "p1"'s entries.
        (
            "playlists[0].entries[5].card_id",
            ValidationCode::DuplicateId,
        ),
        // Playlists "p1" and "p2" share the name "Main".
        ("playlists[1].name", ValidationCode::DuplicateId),
        ("cards[3].source", ValidationCode::InvalidSource),
        (
            "cards[5].template.icon_asset_id",
            ValidationCode::MissingReference,
        ),
    ] {
        assert!(
            error
                .issues
                .iter()
                .any(|issue| issue.path == expected_path && issue.code == expected_code),
            "missing ({expected_path:?}, {expected_code:?}) in {error:?}"
        );
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
        "\"schema_version\": 4,",
        "\"schema_version\": 4, \"unexpected\": true,",
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
fn card_settings_rejects_an_unknown_field_nested_inside_refresh() {
    // CardSettings is itself an internally tagged enum ("kind" = clock/pomodoro/...).
    // Serde silently ignores `deny_unknown_fields` on internally tagged enums, so this
    // proves the validating Deserialize impl catches unknown fields nested inside a
    // card's own `refresh` object, not just at the card's top level.
    let json = r#"{
        "kind": "clock",
        "id": "clock",
        "title": "Desk",
        "show_seconds": true,
        "template": { "kind": "digital-clock" },
        "tap_action": { "kind": "none" },
        "refresh": { "kind": "device-local", "bogus": 1 },
        "alert": { "kind": "none" }
    }"#;
    assert!(serde_json::from_str::<CardSettings>(json).is_err());
}

#[test]
fn display_template_rejects_unknown_fields_on_every_unit_variant() {
    // `DisplayTemplate` is internally tagged. A prior version of this test only probed
    // `icon-badge-text` (a struct variant), which happened to reject unknown fields even
    // under the plain `#[serde(deny_unknown_fields)]` derive and gave false confidence:
    // the unit variants (DigitalClock, AnalogClock, ProgressRing, RowList,
    // BigNumberLabel) slipped unknown fields through entirely under that derive. Probe
    // every unit variant explicitly, plus the struct variant, plus valid round-trips.
    for kind in [
        "digital-clock",
        "analog-clock",
        "progress-ring",
        "row-list",
        "big-number-label",
    ] {
        let with_bogus = format!(r#"{{"kind":"{kind}","bogus":1}}"#);
        assert!(
            serde_json::from_str::<DisplayTemplate>(&with_bogus).is_err(),
            "unit variant {kind:?} must reject an unknown field"
        );
        let valid = format!(r#"{{"kind":"{kind}"}}"#);
        assert!(
            serde_json::from_str::<DisplayTemplate>(&valid).is_ok(),
            "unit variant {kind:?} must still deserialize without extra fields"
        );
    }

    assert!(
        serde_json::from_str::<DisplayTemplate>(
            r#"{"kind":"icon-badge-text","icon_asset_id":"icons","bogus":1}"#
        )
        .is_err()
    );
    let valid_struct = serde_json::from_str::<DisplayTemplate>(
        r#"{"kind":"icon-badge-text","icon_asset_id":"icons"}"#,
    )
    .unwrap();
    assert_eq!(
        valid_struct,
        DisplayTemplate::IconBadgeText {
            icon_asset_id: Some("icons".into())
        }
    );
}

#[test]
fn widget_tap_action_rejects_unknown_fields_on_every_unit_variant() {
    for kind in ["none", "start-pause", "reset", "dismiss"] {
        let with_bogus = format!(r#"{{"kind":"{kind}","bogus":1}}"#);
        assert!(
            serde_json::from_str::<WidgetTapAction>(&with_bogus).is_err(),
            "unit variant {kind:?} must reject an unknown field"
        );
        let valid = format!(r#"{{"kind":"{kind}"}}"#);
        assert!(
            serde_json::from_str::<WidgetTapAction>(&valid).is_ok(),
            "unit variant {kind:?} must still deserialize without extra fields"
        );
    }

    assert!(
        serde_json::from_str::<WidgetTapAction>(
            r#"{"kind":"open-url","url":"https://example.test","bogus":1}"#
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<WidgetTapAction>(
            r#"{"kind":"open-application","application_id":"com.example.app","bogus":1}"#
        )
        .is_err()
    );
    let valid_url = serde_json::from_str::<WidgetTapAction>(
        r#"{"kind":"open-url","url":"https://example.test"}"#,
    )
    .unwrap();
    assert_eq!(
        valid_url,
        WidgetTapAction::OpenUrl {
            url: "https://example.test".into()
        }
    );
}

#[test]
fn refresh_policy_rejects_unknown_fields_on_every_unit_variant() {
    for kind in ["device-local", "manual"] {
        let with_bogus = format!(r#"{{"kind":"{kind}","bogus":1}}"#);
        assert!(
            serde_json::from_str::<RefreshPolicy>(&with_bogus).is_err(),
            "unit variant {kind:?} must reject an unknown field"
        );
        let valid = format!(r#"{{"kind":"{kind}"}}"#);
        assert!(
            serde_json::from_str::<RefreshPolicy>(&valid).is_ok(),
            "unit variant {kind:?} must still deserialize without extra fields"
        );
    }

    assert!(
        serde_json::from_str::<RefreshPolicy>(r#"{"kind":"interval","minutes":15,"bogus":1}"#)
            .is_err()
    );
    let valid_interval =
        serde_json::from_str::<RefreshPolicy>(r#"{"kind":"interval","minutes":15}"#).unwrap();
    assert_eq!(valid_interval, RefreshPolicy::Interval { minutes: 15 });
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
        card_data: vec![CardDataSnapshot {
            card_id: "calendar".into(),
            fields: vec![CardField {
                key: "row0_title".into(),
                value: CardFieldValue::Text {
                    value: "Design review".into(),
                },
            }],
        }],
        card_errors: vec![CardError {
            card_id: "json-feed".into(),
            message: "the display refused this card's data".into(),
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
    assert_eq!(json["card_data"][0]["fields"][0]["value"]["kind"], "text");
    assert_eq!(json["card_errors"][0]["card_id"], "json-feed");
    assert_eq!(json["persistence"]["kind"], "clean");
    assert_eq!(
        serde_json::from_value::<AppSnapshot>(json).unwrap(),
        snapshot
    );
}

#[test]
fn card_behaviour_types_round_trip_as_closed_tagged_json() {
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

fn clock_card(id: &str) -> CardSettings {
    CardSettings::Clock {
        id: id.into(),
        title: "Desk".into(),
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
        label: "Focus".into(),
        duration_seconds: 1_500,
        template: DisplayTemplate::ProgressRing,
        tap_action: WidgetTapAction::StartPause,
        refresh: RefreshPolicy::DeviceLocal,
        alert,
    }
}

/// A single card, wrapped in a playlist that contains exactly that card (so
/// `active_playlist_id` always resolves and the active playlist is never
/// empty) — the v4 stand-in for the pre-Task-1 "just make this one card
/// in-rotation" pattern most single-card tests need.
fn single_card_config(card: CardSettings) -> AppConfig {
    let id = card.id().to_owned();
    AppConfig {
        cards: vec![card],
        playlists: vec![Playlist {
            id: "p1".into(),
            name: "P1".into(),
            advance: CarouselAdvance::Manual,
            entries: vec![PlaylistEntry {
                card_id: id,
                dwell_seconds: None,
            }],
        }],
        active_playlist_id: "p1".into(),
        ..AppConfig::default()
    }
}

#[test]
fn compilation_lowers_cards_to_the_frozen_wire_shape() {
    let config = AppConfig {
        cards: vec![
            clock_card("clock"),
            pomodoro_card(
                "focus",
                CardAlert::OnTimerFinish {
                    hold: AlertHold::UntilDismissed,
                },
            ),
            clock_card("muted"),
        ],
        playlists: vec![Playlist {
            id: "p1".into(),
            name: "P1".into(),
            advance: CarouselAdvance::Manual,
            entries: vec![PlaylistEntry {
                card_id: "clock".into(),
                dwell_seconds: Some(10),
            }],
        }],
        active_playlist_id: "p1".into(),
        ..AppConfig::default()
    };

    let compiled = config.compile(7).unwrap();

    // A card outside the playlist with no alert vanishes entirely; a card
    // outside the playlist with an alert is a screenless widget.
    let widget_ids: Vec<&str> = compiled
        .layout
        .widgets
        .iter()
        .map(|widget| widget.widget_id.as_str())
        .collect();
    assert_eq!(widget_ids, ["clock", "focus"]);

    // Screen ID is the card ID, for playlist entries only.
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

    // A card compiled out entirely receives no initial push.
    let pushed: Vec<&str> = compiled
        .initial_pushes
        .iter()
        .map(|push| push.widget_id.as_str())
        .collect();
    assert_eq!(pushed, ["clock", "focus"]);
}

/// Final-review finding: `wire_config()` lowering every template removed the only
/// backstop that had been keeping host-valid-but-device-invalid compositions from
/// being saved. A card kind may use a template only when the provider populates the
/// fields that template declares — otherwise the card renders its placeholders
/// forever and every field it does send inflates the device's `unknown_field_count`
/// on every refresh, degrading the drift diagnostic Task 2 deliberately preserved.
#[test]
fn compositions_the_provider_cannot_populate_are_rejected() {
    let rejected = [
        // Clock sends no `value`: big-number-label would show "--" forever.
        CardSettings::Clock {
            id: "clock".into(),
            title: "Desk".into(),
            show_seconds: true,
            template: DisplayTemplate::BigNumberLabel,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
        },
        // Calendar sends `title` plus ten `rowN_*` fields icon-badge-text declares none of.
        CardSettings::Calendar {
            id: "agenda".into(),
            title: "Up next".into(),
            source: CalendarSource::Url("https://example.test/calendar.ics".into()),
            template: DisplayTemplate::IconBadgeText {
                icon_asset_id: None,
            },
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::Interval { minutes: 15 },
            alert: CardAlert::None,
        },
        CardSettings::Rss {
            id: "news".into(),
            title: "Headlines".into(),
            url: "https://example.test/feed.xml".into(),
            max_items: 3,
            template: DisplayTemplate::IconBadgeText {
                icon_asset_id: None,
            },
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::Interval { minutes: 30 },
            alert: CardAlert::None,
        },
        // Pomodoro sends label/duration_seconds/remaining_seconds/running, of which
        // big-number-label declares only `label`: `value` stays "--" forever and the
        // other three count as unknown on every tick, not merely on every refresh.
        // `tap_action` is None here so the failure can only be the template — the
        // timer-action rule is pinned separately below.
        CardSettings::Pomodoro {
            id: "focus".into(),
            label: "Focus".into(),
            duration_seconds: 1_500,
            template: DisplayTemplate::BigNumberLabel,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
        },
    ];

    for card in rejected {
        let id = card.id().to_owned();
        let config = single_card_config(card);
        let error = config
            .validate()
            .expect_err(&format!("{id} must not validate"));
        assert!(
            error.issues.iter().any(|issue| {
                issue.path == "cards[0].template"
                    && issue.code == ValidationCode::InvalidComposition
            }),
            "{id} must report an incompatible template: {:?}",
            error.issues
        );
    }
}

/// `validate()` runs on config LOAD and nothing migrates a saved card to a new
/// template, so every weather card saved before the extended templates shipped is
/// still on `row-list`. Narrowing weather's allowed set would make those saved
/// configurations fail to load, stranding the user's whole configuration.
#[test]
fn already_saved_weather_cards_on_row_list_still_load() {
    let config = single_card_config(CardSettings::Weather {
        id: "weather".into(),
        title: "Weather".into(),
        location: "Tbilisi".into(),
        units: WeatherUnits::Metric,
        template: DisplayTemplate::RowList,
        tap_action: WidgetTapAction::None,
        refresh: RefreshPolicy::Interval { minutes: 30 },
        alert: CardAlert::None,
    });
    config.validate().expect("saved weather cards must load");
    config.compile(1).expect("and must still compile");
}

/// Final-review finding: `widget_model.c` refuses any widget whose template is not
/// `PROGRESS_RING` while carrying a non-`NONE` tap action, and `validate_config` is
/// all-or-nothing — so one such card made the device reject the ENTIRE `ApplyConfig`
/// and nothing on the display updated at all.
///
/// The fixture below is now rejected on its template as well, since pomodoro no longer
/// allows `big-number-label` at all. This still pins the tap-action rule specifically:
/// the assertion demands an issue on the `tap_action` path, which the template rule
/// does not raise. Keeping it independent matters because the guard mirrors a firmware
/// rule about templates in general, not about which templates pomodoro may use.
#[test]
fn timer_tap_actions_require_the_progress_ring_template() {
    for tap_action in [WidgetTapAction::StartPause, WidgetTapAction::Reset] {
        let config = single_card_config(CardSettings::Pomodoro {
            id: "focus".into(),
            label: "Focus".into(),
            duration_seconds: 1_500,
            template: DisplayTemplate::BigNumberLabel,
            tap_action: tap_action.clone(),
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
        });
        let error = config.validate().expect_err(&format!(
            "{tap_action:?} off progress-ring must not validate"
        ));
        assert!(
            error.issues.iter().any(|issue| {
                issue.path == "cards[0].tap_action"
                    && issue.code == ValidationCode::InvalidComposition
            }),
            "{tap_action:?} must be reported on the tap_action path: {:?}",
            error.issues
        );
    }

    // The same action on `progress-ring` stays valid.
    let config = single_card_config(pomodoro_card("focus", CardAlert::None));
    config.validate().expect("progress-ring timers stay valid");
}

#[test]
fn compilation_is_deterministic_for_identical_input() {
    let config = AppConfig::default();
    assert_eq!(config.compile(4).unwrap(), config.compile(4).unwrap());
}

#[test]
fn timed_advance_no_longer_requires_an_unimplemented_capability() {
    let mut config = AppConfig::default();
    config.playlists[0].advance = CarouselAdvance::Timed {
        default_dwell_seconds: 20,
    };
    assert!(config.compile(1).is_ok());
}

/// Regression test for the final-review finding that a freshly-added weather or
/// json-feed card could be added, edited, and still fail to save: `configDraft.ts`'s
/// `addCard` defaulted both to `DisplayTemplate::BigNumberLabel`, which `validate()`
/// accepts but `wire_config()` cannot lower (`RequiresCapability`), and the IPC
/// `validate_config_draft` command ran only `validate()`, never `compile()` — so the
/// settings UI reported the draft valid right up until Save rejected it with an error
/// attached to no field. `wire_config()` now lowers every `DisplayTemplate` (this is
/// the M4 task-3/8 fix), so `addCard` defaults weather and json-feed to the templates
/// their field composition was designed for — `icon-badge-text` and
/// `big-number-label` — instead of the `row-list` placeholder this test used to pin.
/// Each of these six cards mirrors exactly what `addCard` produces for that kind today
/// (see `configDraft.ts`), with the field(s) addCard deliberately leaves empty
/// (calendar source / weather location / json-feed url and its empty `mappings` list /
/// rss url) filled in — this test is about whether the REST of a freshly-added card's
/// defaults are wire-compilable, not about the separate, already-correctly-surfaced
/// "required field left empty" validation error.
#[test]
fn every_freshly_added_card_kind_validates_and_compiles() {
    let cards = [
        clock_card("clock"),
        pomodoro_card(
            "pomodoro",
            CardAlert::OnTimerFinish {
                hold: AlertHold::UntilDismissed,
            },
        ),
        CardSettings::Calendar {
            id: "calendar".into(),
            title: "Up next".into(),
            source: CalendarSource::Url("https://example.test/calendar.ics".into()),
            template: DisplayTemplate::RowList,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::Interval { minutes: 15 },
            alert: CardAlert::None,
        },
        CardSettings::Weather {
            id: "weather".into(),
            title: "Weather".into(),
            location: "Tbilisi".into(),
            units: WeatherUnits::Metric,
            template: DisplayTemplate::IconBadgeText {
                icon_asset_id: None,
            },
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::Interval { minutes: 30 },
            alert: CardAlert::None,
        },
        CardSettings::JsonFeed {
            id: "json-feed".into(),
            title: "Feed".into(),
            url: "https://example.test/feed.json".into(),
            // `addCard` itself defaults `mappings` to empty (the user adds one via
            // "Add field mapping"), and `mappings` requires at least one entry, the
            // same "field left empty" shape as calendar source / weather location /
            // rss url. One mapping here represents the user having done their part.
            mappings: vec![JsonFieldMapping {
                field: "value".into(),
                path: "$.value".into(),
            }],
            template: DisplayTemplate::BigNumberLabel,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::Interval { minutes: 15 },
            alert: CardAlert::None,
        },
        CardSettings::Rss {
            id: "rss".into(),
            title: "Headlines".into(),
            url: "https://example.test/feed.xml".into(),
            max_items: 3,
            template: DisplayTemplate::RowList,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::Interval { minutes: 30 },
            alert: CardAlert::None,
        },
    ];

    for card in cards {
        let config = single_card_config(card);
        config.validate().unwrap_or_else(|error| {
            panic!("{} card failed validate(): {error:?}", config.cards[0].id())
        });
        config.compile(1).unwrap_or_else(|error| {
            panic!("{} card failed compile(): {error:?}", config.cards[0].id())
        });
    }
}

/// Task 8: `wire_config()` used to return `None` for `AnalogClock`, `BigNumberLabel`,
/// and `IconBadgeText`, so any card configured with one of them validated cleanly but
/// could never compile (`RequiresCapability`). Exercises all six `DisplayTemplate`
/// variants, each on a card kind `validate_composition` actually allows it on, and
/// asserts `compile()` now succeeds for every one.
#[test]
fn every_display_template_lowers_to_the_wire() {
    let cards = [
        clock_card("digital-clock"),
        CardSettings::Clock {
            id: "analog-clock".into(),
            title: "Desk".into(),
            show_seconds: true,
            template: DisplayTemplate::AnalogClock,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
        },
        pomodoro_card(
            "progress-ring",
            CardAlert::OnTimerFinish {
                hold: AlertHold::UntilDismissed,
            },
        ),
        CardSettings::Calendar {
            id: "row-list".into(),
            title: "Up next".into(),
            source: CalendarSource::Url("https://example.test/calendar.ics".into()),
            template: DisplayTemplate::RowList,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::Interval { minutes: 15 },
            alert: CardAlert::None,
        },
        CardSettings::Weather {
            id: "big-number-label".into(),
            title: "Weather".into(),
            location: "Tbilisi".into(),
            units: WeatherUnits::Metric,
            template: DisplayTemplate::BigNumberLabel,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::Interval { minutes: 30 },
            alert: CardAlert::None,
        },
        CardSettings::Weather {
            id: "icon-badge-text".into(),
            title: "Weather".into(),
            location: "Tbilisi".into(),
            units: WeatherUnits::Metric,
            template: DisplayTemplate::IconBadgeText {
                icon_asset_id: None,
            },
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::Interval { minutes: 30 },
            alert: CardAlert::None,
        },
    ];

    for card in cards {
        let template = card.template().clone();
        let config = single_card_config(card);
        config.compile(1).unwrap_or_else(|error| {
            panic!("{template:?} must lower to the wire, but compile() failed: {error:?}")
        });
    }
}

/// Task 8: `required_device_capabilities()` already flags `CAPABILITY_EXTENDED_TEMPLATES`
/// for the three extended templates and only those (see `config.rs`); this pins that
/// contract from the outside so a future change to the template set can't silently
/// widen or narrow which templates demand the capability.
#[test]
fn extended_templates_require_the_extended_capability() {
    let extended = single_card_config(CardSettings::Weather {
        id: "big-number-label".into(),
        title: "Weather".into(),
        location: "Tbilisi".into(),
        units: WeatherUnits::Metric,
        template: DisplayTemplate::BigNumberLabel,
        tap_action: WidgetTapAction::None,
        refresh: RefreshPolicy::Interval { minutes: 30 },
        alert: CardAlert::None,
    });
    assert_eq!(
        extended.required_device_capabilities() & CAPABILITY_EXTENDED_TEMPLATES,
        CAPABILITY_EXTENDED_TEMPLATES,
        "big-number-label must require the extended-templates capability"
    );

    let core = single_card_config(clock_card("digital-clock"));
    assert_eq!(
        core.required_device_capabilities() & CAPABILITY_EXTENDED_TEMPLATES,
        0,
        "core templates must not demand the extended-templates capability"
    );
}

#[test]
fn card_data_serializes_as_tagged_values_for_the_preview() {
    let data = CardDataSnapshot {
        card_id: "upnext".into(),
        fields: vec![
            CardField {
                key: "row0_title".into(),
                value: CardFieldValue::Text {
                    value: "Design review".into(),
                },
            },
            CardField {
                key: "next_start_unix_ms".into(),
                value: CardFieldValue::Integer {
                    value: 1_787_000_000_000,
                },
            },
            CardField {
                key: "stale".into(),
                value: CardFieldValue::Boolean { value: false },
            },
        ],
    };

    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(json["card_id"], "upnext");
    assert_eq!(json["fields"][0]["value"]["kind"], "text");
    assert_eq!(json["fields"][1]["value"]["kind"], "integer");
    assert_eq!(json["fields"][2]["value"]["kind"], "boolean");
    assert_eq!(
        serde_json::from_value::<CardDataSnapshot>(json).unwrap(),
        data
    );
}

// -- Schema v4: playlists replace presence and the global carousel ---------

/// Two clock cards ("clock-a"/"clock-b"), the given playlists, and the given
/// active playlist id — the shared shape most of the playlist tests below
/// need.
fn v4_config_with(playlists: Vec<Playlist>, active: &str) -> AppConfig {
    AppConfig {
        cards: vec![clock_card("clock-a"), clock_card("clock-b")],
        playlists,
        active_playlist_id: active.into(),
        ..AppConfig::default()
    }
}

fn manual_playlist(id: &str, name: &str, entries: Vec<PlaylistEntry>) -> Playlist {
    Playlist {
        id: id.into(),
        name: name.into(),
        advance: CarouselAdvance::Manual,
        entries,
    }
}

fn entry(card_id: &str) -> PlaylistEntry {
    PlaylistEntry {
        card_id: card_id.into(),
        dwell_seconds: None,
    }
}

#[test]
fn default_config_is_v4_with_one_playlist() {
    let config = AppConfig::default();
    assert_eq!(config.schema_version, 4);
    assert_eq!(config.playlists.len(), 1);
    assert_eq!(config.active_playlist_id, config.playlists[0].id);
    assert_eq!(config.playlists[0].entries.len(), 1);
    assert!(config.validate().is_ok());
}

#[test]
fn active_playlist_id_must_resolve() {
    let config = v4_config_with(
        vec![manual_playlist("p1", "P1", vec![entry("clock-a")])],
        "nope",
    );
    let error = config.validate().unwrap_err();
    assert!(error.issues.iter().any(|issue| {
        issue.path == "active_playlist_id" && issue.code == ValidationCode::MissingReference
    }));
}

#[test]
fn playlist_entry_must_reference_existing_card() {
    let config = v4_config_with(
        vec![manual_playlist("p1", "P1", vec![entry("ghost")])],
        "p1",
    );
    let error = config.validate().unwrap_err();
    assert!(error.issues.iter().any(|issue| {
        issue.path == "playlists[0].entries[0].card_id"
            && issue.code == ValidationCode::MissingReference
    }));
}

#[test]
fn card_twice_in_one_playlist_rejected() {
    let config = v4_config_with(
        vec![manual_playlist(
            "p1",
            "P1",
            vec![entry("clock-a"), entry("clock-a")],
        )],
        "p1",
    );
    let error = config.validate().unwrap_err();
    assert!(error.issues.iter().any(|issue| {
        issue.path == "playlists[0].entries[1].card_id" && issue.code == ValidationCode::DuplicateId
    }));
}

#[test]
fn same_card_in_two_playlists_is_allowed() {
    let config = v4_config_with(
        vec![
            manual_playlist("p1", "P1", vec![entry("clock-a")]),
            manual_playlist("p2", "P2", vec![entry("clock-a")]),
        ],
        "p1",
    );
    config
        .validate()
        .expect("the same card in two playlists must validate");
}

#[test]
fn playlist_name_bounds_enforced() {
    let empty_name = v4_config_with(
        vec![manual_playlist("p1", "", vec![entry("clock-a")])],
        "p1",
    );
    let error = empty_name.validate().unwrap_err();
    assert!(
        error
            .issues
            .iter()
            .any(|issue| issue.path == "playlists[0].name" && issue.code == ValidationCode::Empty)
    );

    let too_long_name = v4_config_with(
        vec![manual_playlist(
            "p1",
            &"x".repeat(MAX_PLAYLIST_NAME_LEN + 1),
            vec![entry("clock-a")],
        )],
        "p1",
    );
    let error = too_long_name.validate().unwrap_err();
    assert!(error.issues.iter().any(|issue| {
        issue.path == "playlists[0].name" && issue.code == ValidationCode::TooLong
    }));
}

#[test]
fn duplicate_playlist_names_rejected() {
    let config = v4_config_with(
        vec![
            manual_playlist("p1", "Work", vec![entry("clock-a")]),
            manual_playlist("p2", "Work", vec![entry("clock-b")]),
        ],
        "p1",
    );
    let error = config.validate().unwrap_err();
    assert!(error.issues.iter().any(|issue| {
        issue.path == "playlists[1].name" && issue.code == ValidationCode::DuplicateId
    }));
}

#[test]
fn playlist_names_are_trimmed_for_uniqueness() {
    let config = v4_config_with(
        vec![
            manual_playlist("p1", "Work", vec![entry("clock-a")]),
            manual_playlist("p2", "Work ", vec![entry("clock-b")]),
        ],
        "p1",
    );
    let error = config.validate().unwrap_err();
    assert!(error.issues.iter().any(|issue| {
        issue.path == "playlists[1].name" && issue.code == ValidationCode::DuplicateId
    }));
}

#[test]
fn empty_active_playlist_rejected_when_cards_exist() {
    let config = v4_config_with(vec![manual_playlist("p1", "P1", vec![])], "p1");
    let error = config.validate().unwrap_err();
    assert!(error.issues.iter().any(|issue| {
        issue.path == "playlists[0].entries" && issue.code == ValidationCode::OutOfRange
    }));
}

#[test]
fn per_playlist_timed_advance_bounds() {
    let config = v4_config_with(
        vec![Playlist {
            id: "p1".into(),
            name: "P1".into(),
            advance: CarouselAdvance::Timed {
                default_dwell_seconds: 1,
            },
            entries: vec![entry("clock-a")],
        }],
        "p1",
    );
    let error = config.validate().unwrap_err();
    assert!(error.issues.iter().any(|issue| {
        issue.path == "playlists[0].advance.default_dwell_seconds"
            && issue.code == ValidationCode::OutOfRange
    }));
}

#[test]
fn unknown_field_in_playlist_rejected() {
    let json = r#"{"id":"p1","name":"Work","advance":{"kind":"manual"},"entries":[],"extra":1}"#;
    assert!(serde_json::from_str::<Playlist>(json).is_err());
}

#[test]
fn unknown_field_in_playlist_entry_rejected() {
    let json = r#"{"card_id":"x","extra":1}"#;
    assert!(serde_json::from_str::<PlaylistEntry>(json).is_err());
}

#[test]
fn compiled_card_ids_are_entries_then_alert_outsiders() {
    let config = AppConfig {
        cards: vec![
            clock_card("b"),
            clock_card("a"),
            pomodoro_card(
                "c",
                CardAlert::OnTimerFinish {
                    hold: AlertHold::UntilDismissed,
                },
            ),
            clock_card("d"),
        ],
        playlists: vec![manual_playlist("p1", "P1", vec![entry("b"), entry("a")])],
        active_playlist_id: "p1".into(),
        ..AppConfig::default()
    };
    assert_eq!(config.compiled_card_ids(), vec!["b", "a", "c"]);
}

#[test]
fn too_many_playlists_rejected() {
    let playlists = (0..=MAX_PLAYLISTS)
        .map(|index| {
            manual_playlist(
                &format!("p{index}"),
                &format!("P{index}"),
                vec![entry("clock-a")],
            )
        })
        .collect();
    let config = v4_config_with(playlists, "p0");
    let error = config.validate().unwrap_err();
    assert!(
        error
            .issues
            .iter()
            .any(|issue| issue.path == "playlists" && issue.code == ValidationCode::TooMany)
    );
}

#[test]
fn too_many_entries_in_one_playlist_rejected() {
    let entries = vec![entry("clock-a"); MAX_PLAYLIST_ENTRIES + 1];
    let config = v4_config_with(vec![manual_playlist("p1", "P1", entries)], "p1");
    let error = config.validate().unwrap_err();
    assert!(error.issues.iter().any(|issue| {
        issue.path == "playlists[0].entries" && issue.code == ValidationCode::TooMany
    }));
}
