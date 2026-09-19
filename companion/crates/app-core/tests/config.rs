use app_core::config::ImageSource;
use app_core::{
    AlertHold, AppConfig, AppSnapshot, AssetKind, AssetSettings, AssetSource, CardAlert,
    CardDataSnapshot, CardError, CardErrorKind, CardField, CardFieldValue, CardSettings,
    CarouselAdvance, ConnectionState, DeviceCounters, DeviceSnapshot, DisplayTemplate,
    MAX_ASSET_BYTES, PersistenceState, PomodoroSnapshot, PomodoroState, RefreshPolicy,
    RuntimeDiagnostics, RuntimeState, ValidationCode, WidgetTapAction,
};
use protocol::{CAPABILITY_ASSET_TRANSFER, Message, TapAction, encode_message};
use serde::de::DeserializeOwned;

const DEFAULT_JSON: &str = include_str!("fixtures/default.json");
const FULL_JSON: &str = include_str!("fixtures/full.json");
const INVALID_JSON: &str = include_str!("fixtures/invalid.json");
const FUTURE_JSON: &str = include_str!("fixtures/future-version.json");
const MALFORMED_JSON: &str = include_str!("fixtures/malformed.json");
const CARD_SURFACE_JSON: &str = include_str!("fixtures/card-surface.json");

fn assert_json_rejected<T: DeserializeOwned>(case: &str, json: &str) {
    assert!(
        serde_json::from_str::<T>(json).is_err(),
        "case {case} unexpectedly deserialized"
    );
}

fn assert_json_accepted<T: DeserializeOwned>(case: &str, json: &str) {
    assert!(
        serde_json::from_str::<T>(json).is_ok(),
        "case {case} must deserialize without extra fields"
    );
}

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
fn v10_omissions_keep_the_frozen_defaults_and_serialize_explicitly() {
    let mut value: serde_json::Value = serde_json::from_str(DEFAULT_JSON).unwrap();
    assert!(
        value.get("image_sources").is_none(),
        "the canonical v10 fixture must exercise the accepted omission"
    );
    value["preferences"]
        .as_object_mut()
        .unwrap()
        .remove("orientation");
    value["cards"][0]
        .as_object_mut()
        .unwrap()
        .remove("dwell_seconds");

    let config: AppConfig = serde_json::from_value(value).unwrap();
    assert!(config.image_sources.is_empty());
    assert_eq!(config.preferences.orientation.rotation_degrees(), 90);
    assert_eq!(config.cards[0].dwell_seconds(), None);

    let normalized = serde_json::to_value(config).unwrap();
    assert_eq!(normalized["image_sources"], serde_json::json!([]));
    assert_eq!(normalized["preferences"]["orientation"], "landscape");
    assert_eq!(
        normalized["cards"][0].get("dwell_seconds"),
        Some(&serde_json::Value::Null)
    );
}

#[test]
fn full_fixture_compiles_deterministically_to_the_current_wire_contract() {
    let config: AppConfig = serde_json::from_str(FULL_JSON).unwrap();
    let first = config.compile(42).unwrap();
    let second = config.compile(42).unwrap();
    assert_eq!(first, second);

    assert_eq!(first.layout.revision, 42);
    assert_eq!(first.layout.rotation, 270);
    // The whole wire model of a card in protocol v2: its id and what a tap
    // means. Face layout and alert policy remain host-owned.
    assert_eq!(first.layout.cards.len(), 3);
    assert_eq!(first.layout.cards[0].card_id, "clock");
    assert_eq!(first.layout.cards[0].tap_action, TapAction::None);
    assert_eq!(first.layout.cards[1].card_id, "pomodoro");
    assert_eq!(first.layout.cards[1].tap_action, TapAction::StartPause);
    assert_eq!(first.layout.cards[2].card_id, "picture");
    assert_eq!(first.layout.cards[2].tap_action, TapAction::None);

    // Only a card that has a timer contributes one; a clock and a picture do
    // not, where protocol v1 sent all three a field bag.
    assert_eq!(first.initial_timers.len(), 1);
    assert_eq!(first.initial_timers[0].card_id, "pomodoro");
    assert!(!first.initial_timers[0].running);
    assert_eq!(
        first.initial_timers[0].remaining_ms,
        first.initial_timers[0].total_ms
    );

    encode_message(1, &Message::ApplyConfig(first.layout)).unwrap();
    for (request_id, push) in first.initial_timers.into_iter().enumerate() {
        encode_message(
            u32::try_from(request_id + 2).unwrap(),
            &Message::PushTimer(push),
        )
        .unwrap();
    }
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
        // A card's own dwell_seconds: 4 is below MIN_DWELL_SECONDS (5).
        // Playlist-shaped rules are unrepresentable in schema v10 because
        // dwell belongs directly to the card in the single ordered loop.
        ("cards[1].dwell_seconds", ValidationCode::OutOfRange),
        // on-timer-finish hold.value: 4 is below MIN_ALERT_HOLD_SECONDS (5).
        ("cards[3].alert.hold.value", ValidationCode::OutOfRange),
        // on-timer-finish alerts are only valid on pomodoro cards; cards[0] is clock.
        ("cards[0].alert", ValidationCode::OutOfRange),
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
fn future_schema_establishes_a_clean_refusal_boundary() {
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

    // Asset transfer is the only thing a configuration can still require of a
    // device. Protocol v2 retired the bits for core widgets, config rotation
    // and extended templates, which all described template rendering.
    assert_eq!(
        config.required_device_capabilities(),
        CAPABILITY_ASSET_TRANSFER
    );

    let first = config.compile(9).unwrap();
    let second = config.compile(9).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.layout.cards.len(), 3);
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
    assert_json_rejected::<AssetSettings>("font-pixel-size", json);
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
        "\"schema_version\": 10,",
        "\"schema_version\": 10, \"unexpected\": true,",
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
    assert_json_rejected::<CardSettings>("clock-refresh-bogus", json);
}

#[test]
fn display_template_rejects_unknown_fields_on_every_unit_variant() {
    // `DisplayTemplate` is internally tagged, and a plain `#[serde(deny_unknown_fields)]`
    // derive is a no-op on such an enum's unit variants -- they slipped unknown fields
    // through entirely. Probe every current variant explicitly, plus valid
    // round-trips.
    for kind in ["digital-clock", "analog-clock", "progress-ring"] {
        let with_bogus = format!(r#"{{"kind":"{kind}","bogus":1}}"#);
        assert_json_rejected::<DisplayTemplate>(kind, &with_bogus);
        let valid = format!(r#"{{"kind":"{kind}"}}"#);
        assert_json_accepted::<DisplayTemplate>(kind, &valid);
    }
}

#[test]
fn widget_tap_action_rejects_unknown_fields_on_every_unit_variant() {
    for kind in ["none", "start-pause", "reset", "dismiss"] {
        let with_bogus = format!(r#"{{"kind":"{kind}","bogus":1}}"#);
        assert_json_rejected::<WidgetTapAction>(kind, &with_bogus);
        let valid = format!(r#"{{"kind":"{kind}"}}"#);
        assert_json_accepted::<WidgetTapAction>(kind, &valid);
    }

    assert_json_rejected::<WidgetTapAction>(
        "open-url",
        r#"{"kind":"open-url","url":"https://example.test","bogus":1}"#,
    );
    assert_json_rejected::<WidgetTapAction>(
        "open-application",
        r#"{"kind":"open-application","application_id":"com.example.app","bogus":1}"#,
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
        assert_json_rejected::<RefreshPolicy>(kind, &with_bogus);
        let valid = format!(r#"{{"kind":"{kind}"}}"#);
        assert_json_accepted::<RefreshPolicy>(kind, &valid);
    }

    assert_json_rejected::<RefreshPolicy>(
        "interval",
        r#"{"kind":"interval","minutes":15,"bogus":1}"#,
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
            active_card_id: Some("clock".into()),
            counters: DeviceCounters::default(),
        },
        pomodoros: vec![PomodoroSnapshot {
            card_id: "pomodoro".into(),
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

    let advance = CarouselAdvance::Timed {
        default_dwell_seconds: 20,
    };
    assert_eq!(advance.default_dwell_seconds(), Some(20));
    assert_eq!(CarouselAdvance::Manual.default_dwell_seconds(), None);
}

#[test]
fn card_alert_rejects_unknown_nested_fields_in_hold() {
    // Unknown field nested in hold for OnTimerFinish
    assert_json_rejected::<CardAlert>(
        "on-timer-finish-nested-hold",
        r#"{"kind":"on-timer-finish","hold":{"kind":"seconds","value":60,"extra":1}}"#,
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
    for (name, json) in [
        ("until-dismissed", r#"{"kind":"until-dismissed","extra":1}"#),
        ("seconds", r#"{"kind":"seconds","value":60,"extra":1}"#),
    ] {
        assert_json_rejected::<AlertHold>(name, json);
    }

    // Valid round-trip
    let valid = serde_json::from_str::<AlertHold>(r#"{"kind":"seconds","value":60}"#).unwrap();
    assert_eq!(valid, AlertHold::Seconds { value: 60 });
}

#[test]
fn carousel_advance_rejects_unknown_fields() {
    for (name, json) in [
        ("manual", r#"{"kind":"manual","extra":1}"#),
        (
            "timed",
            r#"{"kind":"timed","default_dwell_seconds":20,"extra":1}"#,
        ),
    ] {
        assert_json_rejected::<CarouselAdvance>(name, json);
    }

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
        dwell_seconds: None,
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
        dwell_seconds: None,
    }
}

/// A configuration holding exactly one card. Since schema v10 `cards` IS the
/// loop, so there is nothing to enrol it in.
fn single_card_config(card: CardSettings) -> AppConfig {
    AppConfig {
        cards: vec![card],
        advance: CarouselAdvance::Manual,
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
        advance: CarouselAdvance::Manual,
        ..AppConfig::default()
    };

    let compiled = config.compile(7).unwrap();

    // `cards` is the loop, and since protocol v2 it is also the whole wire
    // model -- there is no second array restating it.
    let card_ids: Vec<&str> = compiled
        .layout
        .cards
        .iter()
        .map(|card| card.card_id.as_str())
        .collect();
    assert_eq!(card_ids, ["clock", "focus", "muted"]);

    // Only the pomodoro carries device-side state: a timer. The two clocks
    // contribute nothing, where protocol v1 sent all three a field bag.
    let timer_ids: Vec<&str> = compiled
        .initial_timers
        .iter()
        .map(|timer| timer.card_id.as_str())
        .collect();
    assert_eq!(timer_ids, ["focus"]);
    assert_eq!(compiled.initial_timers[0].total_ms, 1_500_000);
}

/// A card kind may use a template only when it can populate that template's
/// bindings; otherwise the card would render permanent placeholders.
#[test]
fn compositions_the_card_cannot_populate_are_rejected() {
    let rejected = [
        // A clock sends no timer, so a progress ring would show an empty arc
        // and a "Pomodoro" label forever.
        CardSettings::Clock {
            id: "clock".into(),
            title: "Desk".into(),
            show_seconds: true,
            template: DisplayTemplate::ProgressRing,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
            dwell_seconds: None,
        },
        // A pomodoro on a clock face would draw the time and never its timer.
        // `tap_action` is None here so the failure can only be the template --
        // the timer-action rule is pinned separately below.
        CardSettings::Pomodoro {
            id: "focus".into(),
            label: "Focus".into(),
            duration_seconds: 1_500,
            template: DisplayTemplate::DigitalClock,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
            dwell_seconds: None,
        },
        // And the analog face is the same mistake with the other clock template.
        CardSettings::Pomodoro {
            id: "focus-analog".into(),
            label: "Focus".into(),
            duration_seconds: 1_500,
            template: DisplayTemplate::AnalogClock,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
            dwell_seconds: None,
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

/// Timer tap actions require the progress-ring composition. This assertion
/// stays specific to the `tap_action` path even though the fixture is also an
/// invalid card/template combination.
#[test]
fn timer_tap_actions_require_the_progress_ring_template() {
    for tap_action in [WidgetTapAction::StartPause, WidgetTapAction::Reset] {
        let config = single_card_config(CardSettings::Pomodoro {
            id: "focus".into(),
            label: "Focus".into(),
            duration_seconds: 1_500,
            template: DisplayTemplate::DigitalClock,
            tap_action: tap_action.clone(),
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
            dwell_seconds: None,
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
        dwell_seconds: None,
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

    assert!(config.validate().is_ok(), "{:?}", config.validate());
    let encoded = serde_json::to_string(&config).expect("serialize picture card");
    let decoded: AppConfig = serde_json::from_str(&encoded).expect("deserialize picture card");
    assert_eq!(decoded, config);
}

#[test]
fn a_picture_card_naming_an_unknown_source_is_a_typed_missing_reference() {
    let mut config = AppConfig::default();
    config.cards.push(picture_card("shot", "nope"));

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
fn a_picture_card_compiles_to_an_ordinary_wire_card_with_no_tap_action() {
    // The device learns nothing new: picture content arrives through the durable asset path.
    let mut config = AppConfig {
        image_sources: vec![ImageSource {
            id: "limits".into(),
            name: "L".into(),
        }],
        ..AppConfig::default()
    };
    config.cards = vec![picture_card("shot", "limits")];

    let compiled = config.compile(1).expect("compiles");
    assert_eq!(compiled.layout.cards[0].card_id, "shot");
    assert_eq!(compiled.layout.cards[0].tap_action, TapAction::None);
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

#[test]
fn compilation_is_deterministic_for_identical_input() {
    let config = AppConfig::default();
    assert_eq!(config.compile(4).unwrap(), config.compile(4).unwrap());
}

#[test]
fn timed_advance_no_longer_requires_an_unimplemented_capability() {
    let config = AppConfig {
        advance: CarouselAdvance::Timed {
            default_dwell_seconds: 20,
        },
        ..AppConfig::default()
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
        CardSettings::Picture {
            id: "picture".into(),
            title: "Studio".into(),
            source_id: "studio".into(),
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::Manual,
            alert: CardAlert::None,
            dwell_seconds: None,
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

// -- Schema v10: the card list is the loop ---------------------------------

#[test]
fn default_config_is_v10_with_one_card() {
    let config = AppConfig::default();
    // A literal, not `CURRENT_SCHEMA_VERSION`: this test exists to catch a bump that
    // forgot to update `AppConfig::default()`, and comparing the constant to itself
    // could never fail that way. The boundary tests in `tests/store.rs` keep the
    // same literal discipline for the same reason.
    assert_eq!(config.schema_version, 10);
    assert_eq!(config.cards.len(), 1);
    assert_eq!(config.advance, CarouselAdvance::Manual);
    assert!(config.validate().is_ok());
}

#[test]
fn timed_advance_default_dwell_bounds_are_enforced() {
    let config = AppConfig {
        advance: CarouselAdvance::Timed {
            default_dwell_seconds: 1,
        },
        ..AppConfig::default()
    };
    let error = config.validate().unwrap_err();
    assert!(error.issues.iter().any(|issue| {
        issue.path == "advance.default_dwell_seconds" && issue.code == ValidationCode::OutOfRange
    }));
}

#[test]
fn a_cards_own_dwell_bounds_are_enforced() {
    let config = AppConfig {
        cards: vec![set_card_dwell(clock_card("clock-a"), Some(1))],
        ..AppConfig::default()
    };
    let error = config.validate().unwrap_err();
    assert!(error.issues.iter().any(|issue| {
        issue.path == "cards[0].dwell_seconds" && issue.code == ValidationCode::OutOfRange
    }));
}

fn set_card_dwell(mut card: CardSettings, dwell: Option<u16>) -> CardSettings {
    if let CardSettings::Clock { dwell_seconds, .. } = &mut card {
        *dwell_seconds = dwell;
    }
    card
}
