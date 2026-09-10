use app_core::config::ImageSource;
use app_core::{
    AlertHold, AppConfig, AppSnapshot, AssetKind, AssetSettings, AssetSource,
    CURRENT_SCHEMA_VERSION, CardAlert, CardDataSnapshot, CardError, CardErrorKind, CardField,
    CardFieldValue, CardSettings, CarouselAdvance, ConnectionState, DeviceCounters, DeviceSnapshot,
    DisplayTemplate, MAX_ASSET_BYTES, MAX_PLAYLIST_ENTRIES, MAX_PLAYLIST_NAME_LEN, MAX_PLAYLISTS,
    MAX_PLUGIN_ID_LEN, PersistenceState, Playlist, PlaylistEntry, PomodoroSnapshot, PomodoroState,
    ProviderSnapshot, ProviderState, RefreshPolicy, RuntimeDiagnostics, RuntimeState,
    ValidationCode, WidgetTapAction,
};
use protocol::{
    CAPABILITY_ASSET_TRANSFER, CAPABILITY_CONFIG_ROTATION, CAPABILITY_CORE_WIDGETS,
    CAPABILITY_EXTENDED_TEMPLATES, CAPABILITY_HOST_TAP_ACTIONS, Field, FieldValue, InterruptPolicy,
    Message, SizeClass, TapAction, TemplateKind, encode_message,
};

const DEFAULT_JSON: &str = include_str!("fixtures/default.json");
const FULL_JSON: &str = include_str!("fixtures/full.json");
const INVALID_JSON: &str = include_str!("fixtures/invalid.json");
const FUTURE_JSON: &str = include_str!("fixtures/future-v7.json");
const MALFORMED_JSON: &str = include_str!("fixtures/malformed.json");
const CARD_SURFACE_JSON: &str = include_str!("fixtures/card-surface.json");
const PLUGIN_CARD_JSON: &str = include_str!("fixtures/plugin-card.json");

// These frozen pre-v7 fixtures intentionally have no `image_sources` key. Parse
// them through the new default, then move only the version to the contract under
// test rather than rewriting fixtures outside this task's ownership.
fn current_config(json: &str) -> AppConfig {
    let mut config: AppConfig = serde_json::from_str(json).unwrap();
    config.schema_version = CURRENT_SCHEMA_VERSION;
    config
}

#[test]
fn default_fixture_is_the_canonical_default() {
    let from_fixture = current_config(DEFAULT_JSON);
    assert_eq!(from_fixture, AppConfig::default());
    from_fixture.validate().unwrap();

    let round_trip = serde_json::to_string(&from_fixture).unwrap();
    let decoded: AppConfig = serde_json::from_str(&round_trip).unwrap();
    assert_eq!(decoded, from_fixture);
}

#[test]
fn full_fixture_compiles_deterministically_to_m2_contract() {
    let config = current_config(FULL_JSON);
    let first = config.compile(42).unwrap();
    let second = config.compile(42).unwrap();
    assert_eq!(first, second);

    assert_eq!(first.layout.revision, 42);
    assert_eq!(first.layout.rotation, 270);
    assert_eq!(first.layout.widgets.len(), 3);
    assert_eq!(first.layout.screens[0].screen_id, "clock");
    assert_eq!(first.layout.screens[1].screen_id, "pomodoro");
    assert_eq!(first.layout.screens[2].screen_id, "plugin");

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

    let plugin = &first.layout.widgets[2];
    assert_eq!(plugin.template, TemplateKind::DigitalClock);
    assert_eq!(plugin.size_class, SizeClass::Full);
    assert_eq!(plugin.interrupt_policy, InterruptPolicy::Disabled);

    assert_eq!(first.initial_pushes.len(), 3);
    assert_eq!(first.initial_pushes[0].fields.len(), 4);
    assert_eq!(first.initial_pushes[1].fields.len(), 6);
    assert_eq!(first.initial_pushes[2].fields.len(), 3);

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
    let config = current_config(INVALID_JSON);
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
        // on-timer-finish hold.value: 4 is below MIN_ALERT_HOLD_SECONDS (5).
        ("cards[3].alert.hold.value", ValidationCode::OutOfRange),
        // on-timer-finish alerts are only valid on pomodoro cards; cards[0] is clock.
        ("cards[0].alert", ValidationCode::OutOfRange),
        // a playlist entry referencing a card id that does not exist.
        (
            "playlists[0].entries[3].card_id",
            ValidationCode::MissingReference,
        ),
        // "on-timer-mismatch" appears twice in playlist "p1"'s entries.
        (
            "playlists[0].entries[4].card_id",
            ValidationCode::DuplicateId,
        ),
        // Playlists "p1" and "p2" share the name "Main".
        ("playlists[1].name", ValidationCode::DuplicateId),
        ("cards[4].source_id", ValidationCode::MissingReference),
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
    let future = FUTURE_JSON.replacen("\"schema_version\": 8", "\"schema_version\": 9", 1);
    let config: AppConfig = serde_json::from_str(&future).unwrap();
    let error = config.validate().unwrap_err();
    assert!(error.issues.iter().any(|issue| {
        issue.path == "schema_version" && issue.code == ValidationCode::UnsupportedVersion
    }));
}

#[test]
fn card_surface_is_closed_bounded_and_capability_gated() {
    let config = current_config(CARD_SURFACE_JSON);
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
    assert_eq!(first.issues.len(), 1);
    assert_eq!(first.issues[0].code, ValidationCode::RequiresCapability);
}

#[test]
fn asset_budget_total_is_enforced() {
    let config = AppConfig {
        assets: (0..5)
            .map(|index| AssetSettings {
                id: format!("asset-{index}"),
                source: AssetSource::File(format!("/tmp/{index}.ttf")),
                kind: AssetKind::Font,
                maximum_bytes: MAX_ASSET_BYTES,
            })
            .collect(),
        ..AppConfig::default()
    };

    let issues = config.validate().expect_err("over budget");
    assert!(issues.issues.iter().any(|issue| issue.path == "assets"));
}

#[test]
fn font_asset_no_longer_carries_a_pixel_size() {
    // The four-size ceiling is gone: size is chosen per text node at render time
    // (tiny_ttf rasterizes on demand), so pinning one in the config would be
    // meaningless. The unknown-field rejection on the tagged `kind` object is what
    // this test actually exercises: `AssetKind` is a hand-written `Deserialize`
    // (`strict_tagged_enum::AssetKindInner`) precisely because serde's
    // `deny_unknown_fields` is a silent no-op on internally tagged enums.
    let json = r#"{
        "id": "inter",
        "source": { "kind": "file", "value": "/f.ttf" },
        "kind": { "kind": "font", "pixel_size": 48 },
        "maximum_bytes": 262144
    }"#;
    assert!(serde_json::from_str::<AssetSettings>(json).is_err());
}

#[test]
fn a_font_asset_compiles_and_requires_the_asset_transfer_capability() {
    let mut config = AppConfig::default();
    config.assets.push(AssetSettings {
        id: "inter".into(),
        source: AssetSource::File("/tmp/inter.ttf".into()),
        kind: AssetKind::Font,
        maximum_bytes: MAX_ASSET_BYTES,
    });
    config.validate().unwrap();
    assert_ne!(
        config.required_device_capabilities() & CAPABILITY_ASSET_TRANSFER,
        0
    );

    let compiled = config.compile(1).expect("assets must compile now");
    assert_eq!(compiled.assets.len(), 1);
    assert_eq!(compiled.assets[0].id, "inter");
}

#[test]
fn malformed_and_unknown_json_are_rejected_by_serde() {
    assert!(serde_json::from_str::<AppConfig>(MALFORMED_JSON).is_err());

    let with_unknown = DEFAULT_JSON.replace(
        "\"schema_version\": 8,",
        "\"schema_version\": 8, \"unexpected\": true,",
    );
    // A schema bump moves this anchor, and a `replace` that matches nothing
    // returns the input unchanged -- which would leave the assertion below
    // parsing a perfectly valid config and reporting a vacuous pass. Fail here
    // instead, where the message says what actually went wrong.
    assert_ne!(
        with_unknown, DEFAULT_JSON,
        "the unknown-field injection matched nothing; update the schema_version anchor"
    );
    assert!(serde_json::from_str::<AppConfig>(&with_unknown).is_err());

    let nested_unknown = CARD_SURFACE_JSON.replacen(
        r#""template": { "kind": "analog-clock" }"#,
        r#""template": { "kind": "analog-clock", "unexpected": true }"#,
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
            tier: None,
            wifi_state: None,
            wifi_rssi: None,
            ip: None,
            last_network_error: None,
            ota_state: None,
            active_screen_id: Some("clock".into()),
            counters: DeviceCounters::default(),
        },
        providers: vec![ProviderSnapshot {
            widget_id: "air-quality".into(),
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
            card_id: "air-quality".into(),
            fields: vec![CardField {
                key: "row0_title".into(),
                value: CardFieldValue::Text {
                    value: "Design review".into(),
                },
            }],
        }],
        card_errors: vec![CardError {
            kind: CardErrorKind::DataRefused,
            card_id: "air-quality".into(),
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
    assert_eq!(json["card_errors"][0]["card_id"], "air-quality");
    assert_eq!(json["card_errors"][0]["kind"], "data-refused");
    assert_eq!(json["persistence"]["kind"], "clean");
    assert_eq!(
        serde_json::from_value::<AppSnapshot>(json).unwrap(),
        snapshot
    );
}

#[test]
fn card_behaviour_types_round_trip_as_closed_tagged_json() {
    let alert = CardAlert::OnTimerFinish {
        hold: AlertHold::Seconds { value: 60 },
    };
    let json = serde_json::to_value(alert).unwrap();
    assert_eq!(json["kind"], "on-timer-finish");
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

fn picture_card(id: &str, source_id: &str) -> CardSettings {
    CardSettings::Picture {
        id: id.to_owned(),
        title: "Limits".to_owned(),
        source_id: source_id.to_owned(),
        tap_action: WidgetTapAction::None,
        refresh: RefreshPolicy::Manual,
        alert: CardAlert::None,
    }
}

#[test]
fn a_picture_card_naming_a_known_source_validates() {
    let mut config = AppConfig {
        image_sources: vec![ImageSource {
            id: "limits".into(),
            name: "Claude limits".into(),
        }],
        ..AppConfig::default()
    };
    config.cards.push(picture_card("shot", "limits"));
    config.playlists[0].entries.push(PlaylistEntry {
        card_id: "shot".into(),
        dwell_seconds: None,
    });

    assert!(config.validate().is_ok(), "{:?}", config.validate());
    let encoded = serde_json::to_string(&config).expect("serialize picture card");
    let decoded: AppConfig = serde_json::from_str(&encoded).expect("deserialize picture card");
    assert_eq!(decoded, config);
}

#[test]
fn a_picture_card_naming_an_unknown_source_is_a_typed_missing_reference() {
    let mut config = AppConfig::default();
    config.cards.push(picture_card("shot", "nope"));
    config.playlists[0].entries.push(PlaylistEntry {
        card_id: "shot".into(),
        dwell_seconds: None,
    });

    let issues = config
        .validate()
        .expect_err("unknown source must not validate")
        .issues;
    let issue = issues
        .iter()
        .find(|issue| issue.path.ends_with("source_id"))
        .expect("an issue naming source_id");
    // Typed, never a silently blank card.
    assert_eq!(issue.code, ValidationCode::MissingReference);
    assert!(issue.message.contains("nope"));
}

#[test]
fn a_picture_card_compiles_to_the_digital_clock_wire_template() {
    // The device learns nothing new: same byte every plugin card sends.
    let mut config = AppConfig {
        image_sources: vec![ImageSource {
            id: "limits".into(),
            name: "L".into(),
        }],
        ..AppConfig::default()
    };
    config.cards = vec![picture_card("shot", "limits")];
    config.playlists[0].entries = vec![PlaylistEntry {
        card_id: "shot".into(),
        dwell_seconds: None,
    }];

    let compiled = config.compile(1).expect("compiles");
    assert_eq!(
        compiled.layout.widgets[0].template,
        protocol::TemplateKind::DigitalClock
    );
}

#[test]
fn more_than_the_maximum_image_sources_is_rejected() {
    assert_eq!(app_core::config::MAX_IMAGE_SOURCES, 8);
    let mut config = AppConfig::default();
    config.image_sources = (0..=app_core::config::MAX_IMAGE_SOURCES)
        .map(|index| ImageSource {
            id: format!("source-{index}"),
            name: format!("Source {index}"),
        })
        .collect();

    let issues = config.validate().expect_err("over capacity").issues;
    assert!(
        issues
            .iter()
            .any(|issue| issue.code == ValidationCode::TooMany)
    );
}

#[test]
fn two_image_sources_may_not_share_an_id() {
    let config = AppConfig {
        image_sources: vec![
            ImageSource {
                id: "same".into(),
                name: "One".into(),
            },
            ImageSource {
                id: "same".into(),
                name: "Two".into(),
            },
        ],
        ..AppConfig::default()
    };

    let issues = config.validate().expect_err("duplicate id").issues;
    assert!(
        issues
            .iter()
            .any(|issue| issue.code == ValidationCode::DuplicateId)
    );
}

// -- Task 6: the plugin card kind -------------------------------------------------

fn plugin_card(tap_action: WidgetTapAction, refresh: RefreshPolicy) -> CardSettings {
    CardSettings::Plugin {
        id: "aqi".into(),
        title: "Air quality".into(),
        plugin_id: "aqi".into(),
        tap_action,
        refresh,
        alert: CardAlert::None,
    }
}

#[test]
fn plugin_card_fixture_deserializes_validates_and_round_trips() {
    let config = current_config(PLUGIN_CARD_JSON);
    config.validate().unwrap();
    assert_eq!(
        config.cards[0],
        CardSettings::Plugin {
            id: "aqi".into(),
            title: "Air quality".into(),
            plugin_id: "aqi".into(),
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::Interval { minutes: 15 },
            alert: CardAlert::None,
        }
    );

    let round_trip = serde_json::to_string(&config).unwrap();
    let decoded: AppConfig = serde_json::from_str(&round_trip).unwrap();
    assert_eq!(decoded, config);
}

#[test]
fn plugin_card_rejects_a_template_field_as_unknown() {
    // A plugin card deliberately carries no `template`: it renders from its
    // manifest-compiled scene, not one of the six built-in `DisplayTemplate`s.
    // `CardSettingsInner`'s `plugin` arm does not list `template` among its allowed
    // fields, so supplying one must be an unknown-field rejection like any other name
    // that arm does not list — not silently ignored.
    let json = r#"{
        "kind": "plugin",
        "id": "aqi",
        "title": "Air quality",
        "plugin_id": "aqi",
        "template": { "kind": "digital-clock" },
        "tap_action": { "kind": "none" },
        "refresh": { "kind": "interval", "minutes": 15 },
        "alert": { "kind": "none" }
    }"#;
    let error = serde_json::from_str::<CardSettings>(json).unwrap_err();
    assert!(
        error.to_string().contains("unknown field: template"),
        "expected an unknown-field rejection naming `template`, got {error}"
    );
}

#[test]
fn plugin_card_refuses_device_local_refresh_and_pomodoro_only_tap_actions() {
    // `device-local` belongs only to the two device-local card kinds, so a plugin
    // card is refused by `validate_composition`'s refresh-policy match.
    let config = single_card_config(plugin_card(
        WidgetTapAction::None,
        RefreshPolicy::DeviceLocal,
    ));
    let error = config
        .validate()
        .expect_err("device-local refresh must not validate on a plugin card");
    assert!(
        error.issues.iter().any(|issue| {
            issue.path == "cards[0].refresh" && issue.code == ValidationCode::InvalidComposition
        }),
        "expected cards[0].refresh InvalidComposition: {:?}",
        error.issues
    );

    // `start-pause`/`reset` require a pomodoro provider; a plugin card is never one.
    // This is the provider-gated tap-action check, which still runs for `Plugin` even
    // though the template-gated one above it in `validate_composition` is skipped
    // entirely (a plugin card passes `template: None`).
    for tap_action in [WidgetTapAction::StartPause, WidgetTapAction::Reset] {
        let config = single_card_config(plugin_card(
            tap_action.clone(),
            RefreshPolicy::Interval { minutes: 15 },
        ));
        let error = config.validate().expect_err(&format!(
            "{tap_action:?} must not validate on a plugin card"
        ));
        assert!(
            error.issues.iter().any(|issue| {
                issue.path == "cards[0].tap_action"
                    && issue.code == ValidationCode::InvalidComposition
            }),
            "{tap_action:?} must report cards[0].tap_action InvalidComposition: {:?}",
            error.issues
        );
    }

    // The plugin-legal combination validates cleanly.
    let config = single_card_config(plugin_card(
        WidgetTapAction::None,
        RefreshPolicy::Interval { minutes: 15 },
    ));
    config
        .validate()
        .expect("a plugin card with interval refresh and no tap action must validate");
}

#[test]
fn plugin_id_bounds_enforced() {
    let mut empty = single_card_config(plugin_card(
        WidgetTapAction::None,
        RefreshPolicy::Interval { minutes: 15 },
    ));
    if let CardSettings::Plugin { plugin_id, .. } = &mut empty.cards[0] {
        *plugin_id = String::new();
    }
    let error = empty
        .validate()
        .expect_err("empty plugin_id must not validate");
    assert!(error.issues.iter().any(|issue| {
        issue.path == "cards[0].plugin_id" && issue.code == ValidationCode::Empty
    }));

    let mut at_bound = single_card_config(plugin_card(
        WidgetTapAction::None,
        RefreshPolicy::Interval { minutes: 15 },
    ));
    if let CardSettings::Plugin { plugin_id, .. } = &mut at_bound.cards[0] {
        *plugin_id = "x".repeat(MAX_PLUGIN_ID_LEN);
    }
    at_bound
        .validate()
        .expect("plugin_id at exactly MAX_PLUGIN_ID_LEN must validate");

    let mut one_past = single_card_config(plugin_card(
        WidgetTapAction::None,
        RefreshPolicy::Interval { minutes: 15 },
    ));
    if let CardSettings::Plugin { plugin_id, .. } = &mut one_past.cards[0] {
        *plugin_id = "x".repeat(MAX_PLUGIN_ID_LEN + 1);
    }
    let error = one_past
        .validate()
        .expect_err("plugin_id one byte past MAX_PLUGIN_ID_LEN must not validate");
    assert!(error.issues.iter().any(|issue| {
        issue.path == "cards[0].plugin_id" && issue.code == ValidationCode::TooLong
    }));
}

#[test]
fn plugin_card_lowers_to_the_wire_and_pushes_the_three_field_shape() {
    let config = single_card_config(plugin_card(
        WidgetTapAction::None,
        RefreshPolicy::Interval { minutes: 15 },
    ));
    let compiled = config.compile(1).unwrap();

    let widget = &compiled.layout.widgets[0];
    // `TemplateKind::DigitalClock` is the documented inert placeholder
    // (`wire_config`'s comment): firmware no longer switches on this field for any
    // card since stage 3a, so it carries no rendering meaning for a plugin card.
    assert_eq!(widget.template, TemplateKind::DigitalClock);
    assert_eq!(widget.size_class, SizeClass::Full);
    assert_eq!(widget.tap_action, TapAction::None);
    assert_eq!(widget.interrupt_policy, InterruptPolicy::Disabled);
    assert_eq!(compiled.layout.screens[0].widget_id, "aqi");

    assert_eq!(
        compiled.initial_pushes[0].fields,
        vec![
            Field {
                key: "title".into(),
                value: FieldValue::Text("Air quality".into()),
            },
            Field {
                key: "stale".into(),
                value: FieldValue::Boolean(true),
            },
            Field {
                key: "error".into(),
                value: FieldValue::Text("Waiting for provider refresh".into()),
            },
        ]
    );
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
        CardSettings::Plugin {
            id: "aqi".into(),
            title: "Air quality".into(),
            plugin_id: "aqi".into(),
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::Interval { minutes: 15 },
            alert: CardAlert::None,
        },
        CardSettings::Picture {
            id: "picture".into(),
            title: "Studio".into(),
            source_id: "studio".into(),
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::Manual,
            alert: CardAlert::None,
        },
    ];

    for card in cards {
        let is_picture = matches!(card, CardSettings::Picture { .. });
        let mut config = single_card_config(card);
        if is_picture {
            config.image_sources.push(ImageSource {
                id: "studio".into(),
                name: "Studio".into(),
            });
        }
        config.validate().unwrap_or_else(|error| {
            panic!("{} card failed validate(): {error:?}", config.cards[0].id())
        });
        config.compile(1).unwrap_or_else(|error| {
            panic!("{} card failed compile(): {error:?}", config.cards[0].id())
        });
    }
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
fn default_config_is_v8_with_one_playlist() {
    let config = AppConfig::default();
    // A literal, not `CURRENT_SCHEMA_VERSION`: this test exists to catch a bump that
    // forgot to update `AppConfig::default()`, and comparing the constant to itself
    // could never fail that way. The boundary tests in `tests/store.rs` (`found: 9,
    // supported: 8`) keep the same literal discipline for the same reason.
    assert_eq!(config.schema_version, 8);
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
