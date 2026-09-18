//! The TypeScript contract fixture.
//!
//! Serializes one value of every DTO that crosses the wire to the browser and
//! asserts the checked `src/lib/types.contract.ts` matches byte for byte. The
//! frontend compiles that fixture against its own declarations, so a Rust field
//! rename that TypeScript has not been told about fails here, and a TypeScript
//! declaration that no longer describes the Rust fails there.
//!
//! Keeping the fixture generator beside the Rust DTOs makes their serialized
//! shapes the source of truth and catches drift that either compiler alone
//! cannot see.
//!
//! To regenerate after an intentional DTO change:
//! `cargo test -p server --lib -- --ignored print_typescript_contract_fixture --nocapture`

use std::fs;
use std::path::Path;

use app_core::{
    AlertHold, AppConfig, AppPreferences, AppSnapshot, AssetKind, AssetSource,
    CURRENT_SCHEMA_VERSION, CardAlert, CardDataSnapshot, CardError, CardErrorKind, CardField,
    CardFieldValue, CardSettings, CarouselAdvance, ConnectionState, DeviceCapability,
    DeviceCounters, DeviceSnapshot, DisplayOrientation, DisplayTemplate, IconGlyphMapping,
    PersistenceState, PomodoroAction, PomodoroSnapshot, PomodoroState, RefreshPolicy,
    RuntimeDiagnostics, RuntimeState, SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE, SaveReceipt,
    StoreWarning, UpdateChannel, UpdateCheckPolicy, UpdaterSettings, ValidationCode,
    ValidationIssue, WidgetTapAction,
};
use serde::Serialize;

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
}

fn contract_card_kind(card: &CardSettings) -> &'static str {
    match card {
        CardSettings::Clock { .. } => "clock",
        CardSettings::Pomodoro { .. } => "pomodoro",
        CardSettings::Picture { .. } => "picture",
    }
}

#[allow(clippy::too_many_lines)]
fn contract_fixtures() -> ContractFixtures {
    let issue = ValidationIssue {
        path: "active_playlist_id".into(),
        code: ValidationCode::MissingReference,
        message: "active playlist does not exist".into(),
    };
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
                card_id: "limits-picture".into(),
                message: "the display refused this card's data (InvalidPayload): invalid push data"
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

#[test]
fn contract_fixture_covers_every_card_settings_variant() {
    let kinds = contract_fixtures()
        .card_settings
        .iter()
        .map(contract_card_kind)
        .collect::<Vec<_>>();
    assert_eq!(kinds, ["clock", "pomodoro", "picture"]);
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
