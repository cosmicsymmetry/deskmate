//! The TypeScript contract fixture.
//!
//! Serializes representative values for the DTOs that cross the wire to the
//! browser and asserts the checked `src/lib/types.contract.ts` matches byte for
//! byte. The frontend compiles those cases against its own declarations, so a
//! represented Rust field rename that TypeScript has not been told about fails
//! here, and a TypeScript declaration that no longer describes a represented
//! Rust value fails there.
//!
//! Keeping the fixture generator beside the Rust DTOs makes the represented
//! serialized shapes the source of truth for this compatibility seam.
//!
//! To regenerate after an intentional DTO change:
//! `cargo test -p server --lib -- --ignored print_typescript_contract_fixture --nocapture`

use std::fs;
use std::path::Path;

use app_core::{
    AlertHold, AppConfig, AppPreferences, AppSnapshot, AssetKind, AssetSource,
    CURRENT_SCHEMA_VERSION, CardAlert, CardDataSnapshot, CardError, CardErrorKind, CardField,
    CardFieldValue, CardSettings, CarouselAdvance, ConnectionState, DeviceCapability,
    DeviceCounters, DeviceOtaState, DeviceSnapshot, DeviceTier, DeviceWifiState,
    DisplayOrientation, DisplayTemplate, IconGlyphMapping, PersistenceState, PomodoroAction,
    PomodoroSnapshot, PomodoroState, RefreshPolicy, RuntimeDiagnostics, RuntimeState,
    SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE, SaveReceipt, StoreWarning, UpdateChannel,
    UpdateCheckPolicy, UpdaterSettings, ValidationCode, ValidationIssue, WidgetTapAction,
};
use serde::Serialize;

use crate::data_cards::{FaceDescriptor, FaceFieldDescriptor};

use super::{AppApiError, CompanionSnapshot, ConfigApplyResult, DraftValidation, PreviewFrame};

#[derive(Serialize)]
struct ContractFixtures {
    snapshot: CompanionSnapshot,
    configs: Vec<AppConfig>,
    card_settings: Vec<CardSettings>,
    card_alerts: Vec<CardAlert>,
    alert_holds: Vec<AlertHold>,
    carousel_advances: Vec<CarouselAdvance>,
    display_templates: Vec<DisplayTemplate>,
    tap_actions: Vec<WidgetTapAction>,
    refresh_policies: Vec<RefreshPolicy>,
    asset_sources: Vec<AssetSource>,
    asset_kinds: Vec<AssetKind>,
    update_channels: Vec<UpdateChannel>,
    update_check_policies: Vec<UpdateCheckPolicy>,
    display_orientations: Vec<DisplayOrientation>,
    device_capabilities: Vec<DeviceCapability>,
    runtime_states: Vec<RuntimeState>,
    connection_states: Vec<ConnectionState>,
    pomodoro_states: Vec<PomodoroState>,
    card_data: Vec<CardDataSnapshot>,
    persistence_states: Vec<PersistenceState>,
    validation_codes: Vec<ValidationCode>,
    pomodoro_actions: Vec<PomodoroAction>,
    errors: Vec<AppApiError>,
    draft_validation: DraftValidation,
    config_apply_result: ConfigApplyResult,
    preview_frame: PreviewFrame,
    device_rows: Vec<super::DeviceRow>,
    mint_source_responses: Vec<serde_json::Value>,
    image_source_descriptors: Vec<serde_json::Value>,
    face_descriptors: Vec<FaceDescriptor>,
    card_error_kinds: Vec<CardErrorKind>,
    card_field_values: Vec<CardFieldValue>,
    store_warnings: Vec<StoreWarning>,
    device_tiers: Vec<DeviceTier>,
    device_wifi_states: Vec<DeviceWifiState>,
    device_ota_states: Vec<DeviceOtaState>,
    face_field_descriptors: Vec<FaceFieldDescriptor>,
}

#[allow(clippy::too_many_lines)]
fn contract_fixtures() -> ContractFixtures {
    let cards = vec![
        CardSettings::Clock {
            id: "clock".into(),
            title: "Desk".into(),
            show_seconds: true,
            template: DisplayTemplate::DigitalClock,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
            dwell_seconds: None,
        },
        CardSettings::Pomodoro {
            id: "pomodoro".into(),
            label: "Focus".into(),
            duration_seconds: 1_500,
            template: DisplayTemplate::ProgressRing,
            tap_action: WidgetTapAction::StartPause,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::OnTimerFinish {
                hold: AlertHold::UntilDismissed,
            },
            dwell_seconds: None,
        },
        CardSettings::Picture {
            id: "limits-picture".into(),
            title: "Limits".into(),
            source_id: "limits".into(),
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::Manual,
            alert: CardAlert::None,
            dwell_seconds: None,
        },
    ];
    let all_card_settings = cards.clone();
    let config = AppConfig {
        schema_version: CURRENT_SCHEMA_VERSION,
        preferences: AppPreferences {
            timezone: "Asia/Tbilisi".into(),
            autostart: true,
            paused: false,
            orientation: DisplayOrientation::LandscapeFlipped,
        },
        cards: cards.clone(),
        image_sources: vec![app_core::config::ImageSource {
            id: "limits".into(),
            name: "Claude limits".into(),
        }],
        assets: Vec::new(),
        advance: CarouselAdvance::Timed {
            default_dwell_seconds: 30,
        },
        updater: UpdaterSettings::default(),
    };
    let mut invalid_config = config.clone();
    let Some(CardSettings::Pomodoro {
        duration_seconds, ..
    }) = invalid_config.cards.get_mut(1)
    else {
        panic!("contract fixture card 1 is not a pomodoro");
    };
    *duration_seconds = 0;
    let issues = invalid_config
        .compile(1)
        .expect_err("zero-duration pomodoro must be invalid")
        .issues;
    let [issue] = issues.as_slice() else {
        panic!("zero-duration pomodoro produced {} issues", issues.len());
    };
    let issue = issue.clone();
    let card_data = vec![CardDataSnapshot {
        card_id: "limits-picture".into(),
        fields: vec![
            CardField {
                key: "summary".into(),
                value: CardFieldValue::Text {
                    value: "42 · Good".into(),
                },
            },
            CardField {
                key: "stale".into(),
                value: CardFieldValue::Boolean { value: false },
            },
        ],
    }];
    let snapshot = CompanionSnapshot {
        has_saved_config: true,
        host_protocol_version: protocol::PROTOCOL_VERSION,
        app: AppSnapshot {
            config: config.clone(),
            runtime: RuntimeState::Error {
                message: "example runtime error".into(),
            },
            device: DeviceSnapshot {
                connection: ConnectionState::Disconnected {
                    reason: Some("the display is not linked to the server".into()),
                },
                port_name: None,
                firmware_version: Some("1.0.0".into()),
                protocol_version: Some(2),
                max_protocol_version: Some(2),
                capabilities: vec![DeviceCapability::SceneRender],
                unknown_capability_bits: 0,
                uptime_ms: Some(42),
                free_heap: Some(123_456),
                rotation: Some(90),
                tier: None,
                wifi_state: None,
                wifi_rssi: None,
                ip: None,
                last_network_error: None,
                ota_state: None,
                active_card_id: Some("clock".into()),
                counters: DeviceCounters {
                    host_reconnects: 1,
                    valid_frames: 2,
                    malformed_frames: 3,
                    crc_errors: 4,
                    overflow_frames: 5,
                    dropped_responses: 6,
                    rx_dropped_bytes: 7,
                    dropped_events: 8,
                    event_queue_high_water: 9,
                    dropped_ui_commands: 10,
                    ui_queue_high_water: 11,
                    host_dropped_events: 12,
                    detected_event_gaps: 13,
                },
            },
            pomodoros: vec![PomodoroSnapshot {
                card_id: "pomodoro".into(),
                state: PomodoroState::Running,
                duration_seconds: 1_500,
                remaining_seconds: 900,
            }],
            card_data: card_data.clone(),
            card_errors: vec![CardError {
                kind: CardErrorKind::DataRefused,
                card_id: "pomodoro".into(),
                message:
                    "the display refused this card's timer (InvalidPayload): invalid push data"
                        .into(),
            }],
            persistence: PersistenceState::ValidationFailed {
                message: SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE.into(),
                issues: vec![issue.clone()],
            },
            diagnostics: RuntimeDiagnostics {
                commands_processed: 1,
                command_queue_full: 2,
                subscriber_snapshots_overwritten: 6,
                interrupt_dismissals_ignored: 7,
            },
        },
    };
    let face_descriptors = crate::data_cards::creatable_faces(&crate::ServerState::in_memory());
    let face_fields = face_descriptors
        .iter()
        .flat_map(|descriptor| descriptor.fields.iter())
        .collect::<Vec<_>>();
    let face_field_descriptors = [
        face_fields
            .iter()
            .find(|field| matches!(field, FaceFieldDescriptor::Text { .. })),
        face_fields
            .iter()
            .find(|field| matches!(field, FaceFieldDescriptor::Url { .. })),
        face_fields
            .iter()
            .find(|field| matches!(field, FaceFieldDescriptor::Enum { .. })),
    ]
    .into_iter()
    .map(|field| (*field.expect("creatable faces cover every field descriptor variant")).clone())
    .collect();

    ContractFixtures {
        snapshot,
        configs: vec![config],
        card_settings: all_card_settings,
        card_alerts: vec![
            CardAlert::None,
            CardAlert::OnTimerFinish {
                hold: AlertHold::UntilDismissed,
            },
        ],
        alert_holds: vec![AlertHold::UntilDismissed, AlertHold::Seconds { value: 60 }],
        carousel_advances: vec![
            CarouselAdvance::Manual,
            CarouselAdvance::Timed {
                default_dwell_seconds: 20,
            },
        ],
        display_templates: vec![
            DisplayTemplate::DigitalClock,
            DisplayTemplate::AnalogClock,
            DisplayTemplate::ProgressRing,
        ],
        tap_actions: vec![
            WidgetTapAction::None,
            WidgetTapAction::StartPause,
            WidgetTapAction::Reset,
            WidgetTapAction::Dismiss,
            WidgetTapAction::OpenUrl {
                url: "https://example.test/action".into(),
            },
            WidgetTapAction::OpenApplication {
                application_id: "com.example.app".into(),
            },
        ],
        refresh_policies: vec![
            RefreshPolicy::DeviceLocal,
            RefreshPolicy::Manual,
            RefreshPolicy::Interval { minutes: 15 },
        ],
        asset_sources: vec![AssetSource::File("/tmp/status-icons.ttf".into())],
        asset_kinds: vec![
            AssetKind::Font,
            AssetKind::IconFont {
                glyphs: vec![IconGlyphMapping {
                    name: "cloud-rain".into(),
                    codepoint: 0xf729,
                }],
            },
            AssetKind::Image,
        ],
        update_channels: vec![
            UpdateChannel::Stable,
            UpdateChannel::Beta,
            UpdateChannel::Manual,
        ],
        update_check_policies: vec![UpdateCheckPolicy::Disabled, UpdateCheckPolicy::Notify],
        display_orientations: vec![
            DisplayOrientation::Landscape,
            DisplayOrientation::LandscapeFlipped,
        ],
        device_capabilities: DeviceCapability::from_bits(DeviceCapability::known_bits()),
        runtime_states: vec![
            RuntimeState::Starting,
            RuntimeState::Running,
            RuntimeState::Paused,
            RuntimeState::Error {
                message: "error".into(),
            },
        ],
        connection_states: vec![
            ConnectionState::Disconnected { reason: None },
            ConnectionState::Connecting,
            ConnectionState::Online,
            ConnectionState::Standalone,
        ],
        pomodoro_states: vec![
            PomodoroState::Idle,
            PomodoroState::Running,
            PomodoroState::Paused,
            PomodoroState::Completed,
        ],
        card_data,
        persistence_states: vec![
            PersistenceState::Clean,
            PersistenceState::Saving,
            PersistenceState::RecoverableError {
                message: "error".into(),
            },
            PersistenceState::ValidationFailed {
                message: SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE.into(),
                issues: vec![issue.clone()],
            },
        ],
        validation_codes: vec![
            ValidationCode::UnsupportedVersion,
            ValidationCode::Empty,
            ValidationCode::TooLong,
            ValidationCode::TooMany,
            ValidationCode::DuplicateId,
            ValidationCode::MissingReference,
            ValidationCode::OutOfRange,
            ValidationCode::InvalidTimezone,
            ValidationCode::InvalidSource,
            ValidationCode::InvalidComposition,
            ValidationCode::TooLarge,
            ValidationCode::RequiresCapability,
        ],
        pomodoro_actions: vec![
            PomodoroAction::Start,
            PomodoroAction::Pause,
            PomodoroAction::Toggle,
            PomodoroAction::Reset,
        ],
        // Every variant the server can answer with, and no others. The union on
        // the TypeScript side is exactly this long for the same reason.
        errors: vec![
            AppApiError::InvalidPayload {
                message: "invalid".into(),
            },
            AppApiError::PayloadTooLarge {
                message: "large".into(),
                maximum_bytes: app_core::MAX_CONFIG_FILE_BYTES,
            },
            AppApiError::Validation {
                message: "validation".into(),
                issues: vec![issue.clone()],
            },
            AppApiError::Persistence {
                message: "persistence".into(),
            },
            AppApiError::RuntimeUnavailable {
                message: "unavailable".into(),
            },
            AppApiError::NotFound {
                message: "missing".into(),
            },
            AppApiError::Device {
                message: "device".into(),
            },
            AppApiError::Internal {
                message: "internal".into(),
            },
        ],
        draft_validation: DraftValidation {
            valid: false,
            issues: vec![issue],
        },
        config_apply_result: ConfigApplyResult {
            save: SaveReceipt {
                generation: 7,
                warning: Some(StoreWarning::Io {
                    operation: "sync config directory".into(),
                    message: "example warning".into(),
                }),
            },
        },
        preview_frame: PreviewFrame {
            png_base64: Some("iVBORw0KGgo=".into()),
            sample: true,
            state: None,
        },
        device_rows: vec![
            super::DeviceRow {
                id: "unconfigured-display".into(),
                connected: false,
                has_saved_config: false,
                configured_at: None,
                state: "pending",
            },
            super::DeviceRow {
                id: "configured-display".into(),
                connected: true,
                has_saved_config: true,
                configured_at: Some(1_700_000_000),
                state: "active",
            },
        ],
        mint_source_responses: crate::images::contract_mint_source_responses(),
        image_source_descriptors: crate::images::contract_image_source_descriptors(),
        face_descriptors,
        card_error_kinds: vec![CardErrorKind::DataRefused, CardErrorKind::SceneRefused],
        card_field_values: vec![
            CardFieldValue::Text {
                value: "text".into(),
            },
            CardFieldValue::Integer { value: 42 },
            CardFieldValue::Boolean { value: true },
        ],
        store_warnings: vec![StoreWarning::Io {
            operation: "write config".into(),
            message: "example warning".into(),
        }],
        device_tiers: vec![DeviceTier::Local, DeviceTier::Networked],
        device_wifi_states: vec![
            DeviceWifiState::Down,
            DeviceWifiState::Connecting,
            DeviceWifiState::Connected,
            DeviceWifiState::Failed,
        ],
        device_ota_states: vec![
            DeviceOtaState::Idle,
            DeviceOtaState::Checking,
            DeviceOtaState::Downloading,
            DeviceOtaState::PendingVerify,
            DeviceOtaState::Failed,
        ],
        face_field_descriptors,
    }
}

fn typescript_contract_source() -> String {
    let json = serde_json::to_string_pretty(&contract_fixtures()).unwrap();
    format!(
        "// Generated by the Rust contract test; edit the DTOs, not this fixture.\n\
         import type {{ ApiContractFixtures }} from \"./types\";\n\n\
         export const apiContractFixtures = {json} as const satisfies ApiContractFixtures;\n"
    )
}

/// Where the generated fixture is checked in. Relative to this crate, because
/// the frontend and the server now live in one workspace and the fixture is the
/// seam between them.
fn contract_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/deskmate/src/lib/types.contract.ts")
}

fn string_contract_tag(value: &serde_json::Value) -> Option<&str> {
    value.as_str()
}

fn kind_contract_tag(value: &serde_json::Value) -> Option<&str> {
    value.get("kind")?.as_str()
}

fn category_contract_tag(value: &serde_json::Value) -> Option<&str> {
    value.get("category")?.as_str()
}

fn type_contract_tag(value: &serde_json::Value) -> Option<&str> {
    value.get("type")?.as_str()
}

macro_rules! assert_enum_contract {
    ($values:expr, $enum_type:ty, $selector:expr, { $($pattern:pat => $tag:literal),+ $(,)? }) => {{
        let variant_tag = |value: &$enum_type| match value {
            $($pattern => $tag),+
        };
        let expected_tags = [$($tag),+];
        let actual_tags = ($values)
            .iter()
            .map(|value| {
                let matched_tag = variant_tag(value);
                let serialized = serde_json::to_value(value).expect("contract value serializes");
                let serialized_tag = ($selector)(&serialized).expect("serialized contract tag");
                assert_eq!(serialized_tag, matched_tag);
                matched_tag
            })
            .collect::<Vec<_>>();
        let expected_set = expected_tags.iter().copied().collect::<std::collections::BTreeSet<_>>();
        let actual_set = actual_tags.iter().copied().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual_tags.len(), expected_tags.len());
        assert_eq!(actual_set, expected_set);
    }};
}

#[test]
fn contract_fixture_covers_card_and_loop_enums() {
    let fixtures = contract_fixtures();
    assert_enum_contract!(&fixtures.card_settings, CardSettings, kind_contract_tag, {
        CardSettings::Clock { .. } => "clock",
        CardSettings::Pomodoro { .. } => "pomodoro",
        CardSettings::Picture { .. } => "picture",
    });
    assert_enum_contract!(&fixtures.card_alerts, CardAlert, kind_contract_tag, {
        CardAlert::None => "none",
        CardAlert::OnTimerFinish { .. } => "on-timer-finish",
    });
    assert_enum_contract!(&fixtures.alert_holds, AlertHold, kind_contract_tag, {
        AlertHold::UntilDismissed => "until-dismissed",
        AlertHold::Seconds { .. } => "seconds",
    });
    assert_enum_contract!(&fixtures.carousel_advances, CarouselAdvance, kind_contract_tag, {
        CarouselAdvance::Manual => "manual",
        CarouselAdvance::Timed { .. } => "timed",
    });
}

#[test]
fn contract_fixture_covers_rendering_enums() {
    let fixtures = contract_fixtures();
    assert_enum_contract!(&fixtures.display_templates, DisplayTemplate, kind_contract_tag, {
        DisplayTemplate::DigitalClock => "digital-clock",
        DisplayTemplate::AnalogClock => "analog-clock",
        DisplayTemplate::ProgressRing => "progress-ring",
    });
    assert_enum_contract!(&fixtures.tap_actions, WidgetTapAction, kind_contract_tag, {
        WidgetTapAction::None => "none",
        WidgetTapAction::StartPause => "start-pause",
        WidgetTapAction::Reset => "reset",
        WidgetTapAction::Dismiss => "dismiss",
        WidgetTapAction::OpenUrl { .. } => "open-url",
        WidgetTapAction::OpenApplication { .. } => "open-application",
    });
    assert_enum_contract!(&fixtures.refresh_policies, RefreshPolicy, kind_contract_tag, {
        RefreshPolicy::DeviceLocal => "device-local",
        RefreshPolicy::Manual => "manual",
        RefreshPolicy::Interval { .. } => "interval",
    });
    assert_enum_contract!(&fixtures.asset_sources, AssetSource, kind_contract_tag, {
        AssetSource::File(_) => "file",
    });
    assert_enum_contract!(&fixtures.asset_kinds, AssetKind, kind_contract_tag, {
        AssetKind::Font => "font",
        AssetKind::IconFont { .. } => "icon-font",
        AssetKind::Image => "image",
    });
}

#[test]
fn contract_fixture_covers_scalar_config_enums() {
    let fixtures = contract_fixtures();
    assert_enum_contract!(&fixtures.update_channels, UpdateChannel, string_contract_tag, {
        UpdateChannel::Stable => "stable",
        UpdateChannel::Beta => "beta",
        UpdateChannel::Manual => "manual",
    });
    assert_enum_contract!(&fixtures.update_check_policies, UpdateCheckPolicy, string_contract_tag, {
        UpdateCheckPolicy::Disabled => "disabled",
        UpdateCheckPolicy::Notify => "notify",
    });
    assert_enum_contract!(&fixtures.display_orientations, DisplayOrientation, string_contract_tag, {
        DisplayOrientation::Landscape => "landscape",
        DisplayOrientation::LandscapeFlipped => "landscape-flipped",
    });
    assert_enum_contract!(&fixtures.device_capabilities, DeviceCapability, string_contract_tag, {
        DeviceCapability::AssetTransfer => "asset-transfer",
        DeviceCapability::FirmwareUpdate => "firmware-update",
        DeviceCapability::Networking => "networking",
        DeviceCapability::SceneRender => "scene-render",
        DeviceCapability::VolatileAssets => "volatile-assets",
        DeviceCapability::DurableAssetEncoding => "durable-asset-encoding",
    });
    assert_enum_contract!(&fixtures.validation_codes, ValidationCode, string_contract_tag, {
        ValidationCode::UnsupportedVersion => "unsupported-version",
        ValidationCode::Empty => "empty",
        ValidationCode::TooLong => "too-long",
        ValidationCode::TooMany => "too-many",
        ValidationCode::DuplicateId => "duplicate-id",
        ValidationCode::MissingReference => "missing-reference",
        ValidationCode::OutOfRange => "out-of-range",
        ValidationCode::InvalidTimezone => "invalid-timezone",
        ValidationCode::InvalidSource => "invalid-source",
        ValidationCode::InvalidComposition => "invalid-composition",
        ValidationCode::TooLarge => "too-large",
        ValidationCode::RequiresCapability => "requires-capability",
    });
}

#[test]
fn contract_fixture_covers_snapshot_enums() {
    let fixtures = contract_fixtures();
    assert_enum_contract!(&fixtures.runtime_states, RuntimeState, kind_contract_tag, {
        RuntimeState::Starting => "starting",
        RuntimeState::Running => "running",
        RuntimeState::Paused => "paused",
        RuntimeState::Error { .. } => "error",
    });
    assert_enum_contract!(&fixtures.connection_states, ConnectionState, kind_contract_tag, {
        ConnectionState::Disconnected { .. } => "disconnected",
        ConnectionState::Connecting => "connecting",
        ConnectionState::Online => "online",
        ConnectionState::Standalone => "standalone",
    });
    assert_enum_contract!(&fixtures.pomodoro_states, PomodoroState, string_contract_tag, {
        PomodoroState::Idle => "idle",
        PomodoroState::Running => "running",
        PomodoroState::Paused => "paused",
        PomodoroState::Completed => "completed",
    });
    assert_enum_contract!(&fixtures.persistence_states, PersistenceState, kind_contract_tag, {
        PersistenceState::Clean => "clean",
        PersistenceState::Saving => "saving",
        PersistenceState::RecoverableError { .. } => "recoverable-error",
        PersistenceState::ValidationFailed { .. } => "validation-failed",
    });
    assert_enum_contract!(&fixtures.card_error_kinds, CardErrorKind, string_contract_tag, {
        CardErrorKind::DataRefused => "data-refused",
        CardErrorKind::SceneRefused => "scene-refused",
    });
    assert_enum_contract!(&fixtures.card_field_values, CardFieldValue, kind_contract_tag, {
        CardFieldValue::Text { .. } => "text",
        CardFieldValue::Integer { .. } => "integer",
        CardFieldValue::Boolean { .. } => "boolean",
    });
}

#[test]
fn contract_fixture_covers_api_and_device_enums() {
    let fixtures = contract_fixtures();
    assert_enum_contract!(&fixtures.pomodoro_actions, PomodoroAction, string_contract_tag, {
        PomodoroAction::Start => "start",
        PomodoroAction::Pause => "pause",
        PomodoroAction::Toggle => "toggle",
        PomodoroAction::Reset => "reset",
    });
    assert_enum_contract!(&fixtures.errors, AppApiError, category_contract_tag, {
        AppApiError::InvalidPayload { .. } => "invalid-payload",
        AppApiError::PayloadTooLarge { .. } => "payload-too-large",
        AppApiError::Validation { .. } => "validation",
        AppApiError::Persistence { .. } => "persistence",
        AppApiError::RuntimeUnavailable { .. } => "runtime-unavailable",
        AppApiError::NotFound { .. } => "not-found",
        AppApiError::Device { .. } => "device",
        AppApiError::Internal { .. } => "internal",
    });
    assert_enum_contract!(&fixtures.store_warnings, StoreWarning, kind_contract_tag, {
        StoreWarning::Io { .. } => "io",
    });
    assert_enum_contract!(&fixtures.device_tiers, DeviceTier, string_contract_tag, {
        DeviceTier::Local => "local",
        DeviceTier::Networked => "networked",
    });
    assert_enum_contract!(&fixtures.device_wifi_states, DeviceWifiState, string_contract_tag, {
        DeviceWifiState::Down => "down",
        DeviceWifiState::Connecting => "connecting",
        DeviceWifiState::Connected => "connected",
        DeviceWifiState::Failed => "failed",
    });
    assert_enum_contract!(&fixtures.device_ota_states, DeviceOtaState, string_contract_tag, {
        DeviceOtaState::Idle => "idle",
        DeviceOtaState::Checking => "checking",
        DeviceOtaState::Downloading => "downloading",
        DeviceOtaState::PendingVerify => "pending-verify",
        DeviceOtaState::Failed => "failed",
    });
    assert_enum_contract!(&fixtures.face_field_descriptors, FaceFieldDescriptor, type_contract_tag, {
        FaceFieldDescriptor::Text { .. } => "text",
        FaceFieldDescriptor::Url { .. } => "url",
        FaceFieldDescriptor::Enum { .. } => "enum",
    });
}

#[test]
fn contract_validation_issue_comes_from_the_config_compiler() {
    let fixtures = contract_fixtures();
    let config = &fixtures.configs[0];
    assert!(config.compile(1).is_ok());

    let mut invalid = config.clone();
    let Some(CardSettings::Pomodoro {
        duration_seconds, ..
    }) = invalid.cards.get_mut(1)
    else {
        panic!("contract fixture card 1 is not a pomodoro");
    };
    *duration_seconds = 0;

    let issues = invalid.compile(1).unwrap_err().issues;
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].path, "cards[1].duration_seconds");
    assert_eq!(issues[0].code, ValidationCode::OutOfRange);

    let fixture_issue_slices: [&[ValidationIssue]; 4] = [
        match &fixtures.snapshot.app.persistence {
            PersistenceState::ValidationFailed { issues, .. } => issues.as_slice(),
            _ => panic!("snapshot persistence is not validation-failed"),
        },
        fixtures
            .persistence_states
            .iter()
            .find_map(|state| match state {
                PersistenceState::ValidationFailed { issues, .. } => Some(issues.as_slice()),
                _ => None,
            })
            .expect("fixture persistence states include validation-failed"),
        fixtures
            .errors
            .iter()
            .find_map(|error| match error {
                AppApiError::Validation { issues, .. } => Some(issues.as_slice()),
                _ => None,
            })
            .expect("fixture errors include validation"),
        fixtures.draft_validation.issues.as_slice(),
    ];
    for fixture_issues in fixture_issue_slices {
        assert_eq!(fixture_issues, issues.as_slice());
    }
}

#[test]
fn contract_fixture_covers_nullable_response_fields() {
    let fixtures = contract_fixtures();
    assert!(
        fixtures
            .device_rows
            .iter()
            .any(|row| row.configured_at.is_none())
    );
    assert!(
        fixtures
            .device_rows
            .iter()
            .any(|row| row.configured_at.is_some())
    );
    assert!(
        fixtures
            .image_source_descriptors
            .iter()
            .any(|descriptor| descriptor["face"].is_null())
    );
    assert!(
        fixtures
            .image_source_descriptors
            .iter()
            .any(|descriptor| descriptor["face"].is_object())
    );
    assert!(fixtures.face_field_descriptors.iter().any(|field| {
        matches!(field, FaceFieldDescriptor::Enum { options, .. } if !options.is_empty())
    }));
}

#[test]
fn typescript_contract_fixture_stays_in_sync() {
    let path = contract_path();
    let checked = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "cannot read checked TypeScript contract {}: {error}",
            path.display()
        )
    });
    assert_eq!(checked, typescript_contract_source());
}

#[test]
#[ignore = "prints the checked TypeScript fixture for intentional regeneration"]
fn print_typescript_contract_fixture() {
    print!("{}", typescript_contract_source());
}
