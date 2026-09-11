use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;

#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use app_core::{
    AlertHold, AppConfig, CURRENT_SCHEMA_VERSION, CardAlert, CardSettings, CarouselAdvance,
    ConfigOrigin, ConfigStore, DisplayOrientation, DisplayTemplate, LoadOutcome,
    MAX_CONFIG_FILE_BYTES, RefreshPolicy, SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE, StoreError,
    WidgetTapAction,
};

fn test_directory(name: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("deskmate-app-core-{name}-"))
        .tempdir()
        .unwrap()
}

#[test]
fn missing_file_loads_defaults_without_writing() {
    let directory = test_directory("first-run");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);

    let outcome = store.load();
    assert_eq!(outcome.origin(), ConfigOrigin::Defaults);
    assert_eq!(outcome.config(), &AppConfig::default());
    assert_eq!(outcome.recovery(), None);
    assert!(!path.exists());
}

#[test]
fn invalid_persisted_config_keeps_last_good_and_reports_typed_error() {
    let directory = test_directory("validation-recovery");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);
    let mut last_good = AppConfig::default();
    last_good.preferences.autostart = true;
    store.save(&last_good).unwrap();

    let mut invalid = serde_json::to_value(AppConfig::default()).unwrap();
    invalid["active_playlist_id"] = serde_json::json!("ghost");
    let invalid_bytes = serde_json::to_vec_pretty(&invalid).unwrap();
    fs::write(&path, &invalid_bytes).unwrap();

    let LoadOutcome::ValidationFailed {
        config,
        origin,
        issues,
    } = store.load()
    else {
        panic!("validation failure must remain typed");
    };
    assert_eq!(config, last_good);
    assert_eq!(origin, ConfigOrigin::LastGood);
    assert!(
        issues
            .iter()
            .any(|issue| issue.path == "active_playlist_id")
    );
    assert_eq!(
        StoreError::Validation { issues }.to_string(),
        SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE
    );
    assert_eq!(store.last_good().unwrap(), last_good);
    assert_eq!(fs::read(&path).unwrap(), invalid_bytes);
}

#[test]
fn invalid_persisted_config_without_last_good_labels_fallback_as_defaults() {
    let directory = test_directory("validation-defaults");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);
    let mut invalid = serde_json::to_value(AppConfig::default()).unwrap();
    invalid["active_playlist_id"] = serde_json::json!("ghost");
    fs::write(&path, serde_json::to_vec_pretty(&invalid).unwrap()).unwrap();

    let LoadOutcome::ValidationFailed {
        config,
        origin,
        issues,
    } = store.load()
    else {
        panic!("validation failure must remain typed");
    };
    assert_eq!(config, AppConfig::default());
    assert_eq!(origin, ConfigOrigin::Defaults);
    assert!(!issues.is_empty());
}

#[test]
fn save_round_trips_and_migration_is_explicit() {
    let directory = test_directory("round-trip");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);
    let mut config = AppConfig::default();
    config.preferences.timezone = "Asia/Tbilisi".into();

    let receipt = store.save(&config).unwrap();
    assert_eq!(receipt.generation, 1);
    assert_eq!(receipt.warning, None);
    let loaded = store.load();
    assert_eq!(loaded.origin(), ConfigOrigin::Current);
    assert_eq!(loaded.config(), &config);

    fs::write(&path, include_bytes!("fixtures/legacy-v0.json")).unwrap();
    let migrated = store.load();
    assert_eq!(migrated.origin(), ConfigOrigin::MigratedV0);
    assert_eq!(migrated.config().schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(migrated.config().preferences.timezone, "Europe/Paris");
    assert!(!migrated.config().preferences.autostart);

    fs::write(&path, include_bytes!("fixtures/released-m3-v1.json")).unwrap();
    let migrated = store.load();
    assert_eq!(migrated.origin(), ConfigOrigin::MigratedV1);
    assert_eq!(migrated.config().schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(migrated.config().preferences.timezone, "Asia/Tbilisi");
    assert_eq!(migrated.config().cards.len(), 2);
    assert_eq!(
        migrated.config().preferences.orientation,
        DisplayOrientation::LandscapeFlipped
    );
    // The retired calendar is dropped; the surviving screen order remains pomodoro,
    // then clock.
    assert!(matches!(
        &migrated.config().cards[0],
        CardSettings::Pomodoro {
            id,
            label,
            duration_seconds: 1_500,
            template: DisplayTemplate::ProgressRing,
            tap_action: WidgetTapAction::StartPause,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::OnTimerFinish { hold: AlertHold::UntilDismissed },
            ..
        } if id == "pomodoro" && label == "Focus"
    ));
    assert!(matches!(
        &migrated.config().cards[1],
        CardSettings::Clock {
            id,
            title,
            show_seconds: true,
            template: DisplayTemplate::DigitalClock,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
            ..
        } if id == "clock" && title == "Desk"
    ));
    assert!(migrated.config().assets.is_empty());
    assert_eq!(
        migrated.config().playlists[0].advance,
        app_core::CarouselAdvance::Manual
    );
    migrated.config().compile(7).unwrap();
}

#[cfg(unix)]
#[test]
fn saved_config_is_user_readable_and_writable_only() {
    let directory = test_directory("private-save");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);

    store.save(&AppConfig::default()).unwrap();

    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn loading_repairs_a_permissive_existing_config_mode() {
    let directory = test_directory("private-load");
    let path = directory.path().join("config.json");
    let bytes = serde_json::to_vec_pretty(&AppConfig::default()).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&path)
        .unwrap()
        .write_all(&bytes)
        .unwrap();
    // Apply the exact mode explicitly so this assertion is independent of the test
    // process's umask.
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

    let loaded = ConfigStore::new(&path).load();

    assert_eq!(loaded.origin(), ConfigOrigin::Current);
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn v3_migrates_to_one_playlist_preserving_rotation_order_and_dwell() {
    let directory = test_directory("v3-migration");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);
    fs::write(&path, include_bytes!("fixtures/v3-roundtrip.json")).unwrap();

    let outcome = store.load();
    assert_eq!(outcome.origin(), ConfigOrigin::MigratedV3);
    assert!(outcome.recovery().is_none());

    let config = outcome.config();
    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(config.active_playlist_id, "my-playlist");
    assert_eq!(config.playlists.len(), 1);
    let playlist = &config.playlists[0];
    assert_eq!(playlist.id, "my-playlist");
    assert_eq!(playlist.name, "My playlist");
    assert_eq!(
        playlist.advance,
        CarouselAdvance::Timed {
            default_dwell_seconds: 30
        }
    );
    assert_eq!(playlist.entries.len(), 1);
    assert_eq!(playlist.entries[0].card_id, "a");
    assert_eq!(playlist.entries[0].dwell_seconds, Some(20));

    let ids: Vec<&str> = config.cards.iter().map(CardSettings::id).collect();
    assert_eq!(ids, ["a", "b"]);
    assert!(matches!(
        &config.cards[0],
        CardSettings::Clock {
            title,
            show_seconds: true,
            template: DisplayTemplate::AnalogClock,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
            ..
        } if title == "Desk"
    ));
    assert!(matches!(
        &config.cards[1],
        CardSettings::Pomodoro {
            label,
            duration_seconds: 1_200,
            template: DisplayTemplate::ProgressRing,
            tap_action: WidgetTapAction::Reset,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::OnTimerFinish {
                hold: AlertHold::UntilDismissed
            },
            ..
        } if label == "Deep work"
    ));
    assert_eq!(config.preferences.timezone, "Asia/Tbilisi");
    assert!(config.preferences.autostart);
    assert!(config.assets.is_empty());
    assert_eq!(config.updater.channel, app_core::UpdateChannel::Beta);
    assert!(config.validate().is_ok());
}

#[test]
fn v3_all_alert_only_migrates_to_valid_config() {
    let directory = test_directory("v3-alert-only");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);
    let alert_only = br#"{
      "schema_version": 3,
      "preferences": {
        "timezone": "UTC",
        "autostart": false,
        "paused": false,
        "orientation": "landscape"
      },
      "cards": [
        {
          "kind": "pomodoro",
          "id": "focus",
          "label": "Focus",
          "duration_seconds": 1500,
          "template": { "kind": "progress-ring" },
          "tap_action": { "kind": "start-pause" },
          "refresh": { "kind": "device-local" },
          "presence": { "kind": "alert-only" },
          "alert": { "kind": "on-timer-finish", "hold": { "kind": "until-dismissed" } }
        },
        {
          "kind": "calendar",
          "id": "agenda",
          "title": "Agenda",
          "source": { "kind": "url", "value": "https://example.test/agenda.ics" },
          "template": { "kind": "row-list" },
          "tap_action": { "kind": "none" },
          "refresh": { "kind": "interval", "minutes": 15 },
          "presence": { "kind": "alert-only" },
          "alert": { "kind": "before-event", "lead_minutes": 5, "hold": { "kind": "seconds", "value": 60 } }
        }
      ],
      "assets": [],
      "carousel": { "advance": { "kind": "manual" } },
      "updater": { "channel": "stable", "checks": "notify" }
    }"#;
    fs::write(&path, alert_only).unwrap();

    let outcome = store.load();
    assert_eq!(outcome.origin(), ConfigOrigin::MigratedV3);
    assert!(outcome.recovery().is_none());
    assert_eq!(outcome.config().cards.len(), 1);
    assert_eq!(outcome.config().playlists[0].entries.len(), 1);
    assert_eq!(outcome.config().playlists[0].entries[0].card_id, "focus");
    assert_eq!(
        outcome.config().cards[0].alert(),
        CardAlert::OnTimerFinish {
            hold: AlertHold::UntilDismissed
        }
    );
    assert!(outcome.config().validate().is_ok());
}

#[test]
fn v4_config_migrates_to_v9_with_surviving_cards_unchanged() {
    // config.rs's compile step has always rejected a non-empty `assets` array, so no
    // saved v4 config has ever contained one: migration to the current schema is a
    // version bump with no data transformation (v4 -> v7 directly, not chained
    // through intermediate schemas).
    let directory = test_directory("v4-migration");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);
    fs::write(&path, include_bytes!("fixtures/v4-roundtrip.json")).unwrap();

    let outcome = store.load();
    assert_eq!(outcome.origin(), ConfigOrigin::MigratedV4);
    assert!(outcome.recovery().is_none());

    let config = outcome.config();
    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    assert!(config.image_sources.is_empty());
    assert!(config.assets.is_empty());
    assert_eq!(config.preferences.timezone, "Asia/Tbilisi");
    assert_eq!(config.active_playlist_id, "workday");
    assert_eq!(config.playlists[0].entries.len(), 2);
    assert_eq!(config.cards.len(), 2);
    assert!(config.validate().is_ok());
}

#[test]
fn v5_config_migrates_to_v9_dropping_retired_cards() {
    let directory = test_directory("v5-migration");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);
    fs::write(&path, include_bytes!("fixtures/v5-roundtrip.json")).unwrap();

    let outcome = store.load();
    assert_eq!(outcome.origin(), ConfigOrigin::MigratedV5);
    assert!(outcome.recovery().is_none());

    let migrated = outcome.config();
    assert_eq!(migrated.schema_version, CURRENT_SCHEMA_VERSION);

    assert_eq!(
        migrated
            .cards
            .iter()
            .map(CardSettings::id)
            .collect::<Vec<_>>(),
        ["analog", "focus"]
    );
    assert!(migrated.image_sources.is_empty());
    assert_eq!(migrated.playlists[0].entries.len(), 2);
    assert!(migrated.validate().is_ok());
}

#[test]
fn a_v6_document_migrates_to_v9_with_no_image_sources_and_loses_nothing() {
    // A real v6 document: no `image_sources` key at all. It must parse, not fail.
    let v6 = serde_json::json!({
        "schema_version": 6,
        "preferences": { "timezone": "UTC", "autostart": false, "paused": false },
        "cards": [{
            "kind": "clock",
            "id": "clock",
            "title": "Desk",
            "show_seconds": true,
            "template": { "kind": "digital-clock" },
            "tap_action": { "kind": "none" },
            "refresh": { "kind": "device-local" },
            "alert": { "kind": "none" }
        }],
        "assets": [],
        "playlists": [{
            "id": "my-playlist",
            "name": "My playlist",
            "advance": { "kind": "manual" },
            "entries": [{ "card_id": "clock", "dwell_seconds": null }]
        }],
        "active_playlist_id": "my-playlist",
        "updater": { "channel": "stable", "checks": "notify" }
    });

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("config.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&v6).expect("encode")).expect("write");

    let store = app_core::ConfigStore::new(path);
    let loaded = store.load();
    let config = loaded.config();

    assert_eq!(loaded.origin(), ConfigOrigin::MigratedV6);
    assert_eq!(config.schema_version, app_core::CURRENT_SCHEMA_VERSION);
    assert_eq!(config.schema_version, 9);
    assert!(config.image_sources.is_empty(), "v6 knew no sources");
    // Lossless: everything else survived untouched.
    assert_eq!(config.cards.len(), 1);
    assert_eq!(config.active_playlist_id, "my-playlist");
    assert_eq!(config.playlists[0].entries[0].card_id, "clock");
}

#[test]
fn v7_document_migrates_to_v9_dropping_retired_cards_and_entries() {
    let directory = test_directory("v7-migration");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);
    fs::write(&path, include_bytes!("fixtures/v7-roundtrip.json")).unwrap();

    let outcome = store.load();
    assert_eq!(outcome.origin(), ConfigOrigin::MigratedV7);
    assert!(outcome.recovery().is_none());
    let config = outcome.config();
    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(
        config
            .cards
            .iter()
            .map(CardSettings::id)
            .collect::<Vec<_>>(),
        ["clock", "pomodoro"]
    );
    assert_eq!(
        config.playlists[0]
            .entries
            .iter()
            .map(|entry| entry.card_id.as_str())
            .collect::<Vec<_>>(),
        ["clock", "pomodoro"]
    );
    config.validate().unwrap();
}

#[test]
fn v8_document_migrates_to_v9_dropping_plugin_cards_and_entries() {
    let directory = test_directory("v8-migration");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);
    fs::write(&path, include_bytes!("fixtures/plugin-card.json")).unwrap();

    let outcome = store.load();
    assert_eq!(outcome.origin(), ConfigOrigin::MigratedV8);
    assert!(outcome.recovery().is_none());
    let config = outcome.config();
    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(
        config
            .cards
            .iter()
            .map(CardSettings::id)
            .collect::<Vec<_>>(),
        ["clock"]
    );
    assert_eq!(
        config.playlists[0]
            .entries
            .iter()
            .map(|entry| entry.card_id.as_str())
            .collect::<Vec<_>>(),
        ["clock"]
    );
    config.validate().unwrap();
}

#[test]
fn v8_roundtrip_fixture_preserves_every_surviving_card() {
    let directory = test_directory("v8-roundtrip");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);
    fs::write(&path, include_bytes!("fixtures/v8-roundtrip.json")).unwrap();

    let outcome = store.load();
    assert_eq!(outcome.origin(), ConfigOrigin::MigratedV8);
    assert_eq!(
        outcome
            .config()
            .cards
            .iter()
            .map(CardSettings::id)
            .collect::<Vec<_>>(),
        ["clock", "pomodoro"]
    );
}

#[test]
fn an_all_retired_v7_document_gets_the_default_clock_and_loses_empty_inactive_playlists() {
    let directory = test_directory("v7-all-retired");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);
    let document = br#"{
      "schema_version": 7,
      "preferences": { "timezone": "UTC", "autostart": false, "paused": false, "orientation": "landscape" },
      "cards": [
        { "kind": "calendar", "id": "agenda", "title": "Agenda", "source": { "kind": "url", "value": "https://example.test/a.ics" }, "template": { "kind": "row-list" }, "tap_action": { "kind": "none" }, "refresh": { "kind": "interval", "minutes": 15 }, "alert": { "kind": "none" } },
        { "kind": "weather", "id": "outside", "title": "Outside", "location": "Tbilisi", "units": "metric", "template": { "kind": "icon-badge-text", "icon_asset_id": null }, "tap_action": { "kind": "none" }, "refresh": { "kind": "interval", "minutes": 30 }, "alert": { "kind": "none" } },
        { "kind": "json-feed", "id": "metric", "title": "Metric", "url": "https://example.test/data.json", "mappings": [], "template": { "kind": "big-number-label" }, "tap_action": { "kind": "none" }, "refresh": { "kind": "manual" }, "alert": { "kind": "none" } },
        { "kind": "rss", "id": "news", "title": "News", "url": "https://example.test/feed.xml", "max_items": 3, "template": { "kind": "row-list" }, "tap_action": { "kind": "none" }, "refresh": { "kind": "interval", "minutes": 30 }, "alert": { "kind": "none" } }
      ],
      "assets": [],
      "playlists": [
        { "id": "active", "name": "Active", "advance": { "kind": "manual" }, "entries": [{ "card_id": "agenda", "dwell_seconds": null }, { "card_id": "outside", "dwell_seconds": null }] },
        { "id": "inactive", "name": "Inactive", "advance": { "kind": "manual" }, "entries": [{ "card_id": "metric", "dwell_seconds": null }, { "card_id": "news", "dwell_seconds": null }] }
      ],
      "active_playlist_id": "active",
      "updater": { "channel": "stable", "checks": "notify" }
    }"#;
    fs::write(&path, document).unwrap();

    let outcome = store.load();
    assert_eq!(outcome.origin(), ConfigOrigin::MigratedV7);
    let config = outcome.config();
    let defaults = AppConfig::default();
    assert_eq!(config.cards, defaults.cards);
    assert_eq!(config.playlists.len(), 1);
    assert_eq!(config.playlists[0].id, "active");
    assert_eq!(config.playlists[0].entries, defaults.playlists[0].entries);
    config.validate().unwrap();
}

#[test]
fn v0_v1_v2_migrate_directly_to_v9() {
    let directory = test_directory("legacy-direct-to-v9");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);

    for (fixture, origin, expected_ids) in [
        (
            include_bytes!("fixtures/legacy-v0.json").as_slice(),
            ConfigOrigin::MigratedV0,
            &["clock"][..],
        ),
        (
            include_bytes!("fixtures/released-m3-v1.json").as_slice(),
            ConfigOrigin::MigratedV1,
            &["pomodoro", "clock"][..],
        ),
        (
            include_bytes!("fixtures/v2-legacy.json").as_slice(),
            ConfigOrigin::MigratedV2,
            &["clock", "focus"][..],
        ),
    ] {
        fs::write(&path, fixture).unwrap();
        let outcome = store.load();
        assert_eq!(outcome.origin(), origin);
        assert!(outcome.recovery().is_none());
        assert_eq!(outcome.config().schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(outcome.config().playlists.len(), 1);
        let entry_ids: Vec<&str> = outcome.config().playlists[0]
            .entries
            .iter()
            .map(|entry| entry.card_id.as_str())
            .collect();
        assert_eq!(entry_ids, expected_ids);
        assert!(outcome.config().validate().is_ok());
    }
}

#[test]
fn malformed_truncated_and_oversized_files_preserve_bytes_and_last_good() {
    let directory = test_directory("recovery");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);
    let mut last_good = AppConfig::default();
    last_good.preferences.autostart = true;
    store.save(&last_good).unwrap();

    let malformed = include_bytes!("fixtures/malformed.json");
    fs::write(&path, malformed).unwrap();
    let recovered = store.load();
    assert_eq!(recovered.origin(), ConfigOrigin::LastGood);
    assert_eq!(recovered.config(), &last_good);
    assert!(matches!(
        recovered.recovery(),
        Some(StoreError::InvalidJson { .. })
    ));
    assert_eq!(fs::read(&path).unwrap(), malformed);

    let invalid_utf8 = [0xff, 0xfe, 0xfd];
    fs::write(&path, invalid_utf8).unwrap();
    let recovered = store.load();
    assert_eq!(recovered.config(), &last_good);
    assert_eq!(recovered.recovery(), Some(StoreError::InvalidUtf8));
    assert_eq!(fs::read(&path).unwrap(), invalid_utf8);

    let oversized = vec![b'x'; MAX_CONFIG_FILE_BYTES + 1];
    fs::write(&path, &oversized).unwrap();
    let recovered = store.load();
    assert_eq!(recovered.config(), &last_good);
    assert!(matches!(
        recovered.recovery(),
        Some(StoreError::TooLarge { .. })
    ));
    assert_eq!(fs::metadata(&path).unwrap().len(), oversized.len() as u64);
}

#[test]
fn failed_replace_preserves_last_good() {
    let directory = test_directory("replace-failure");
    let target = directory.path().join("target-is-a-directory");
    fs::create_dir(&target).unwrap();
    let store = ConfigStore::new(&target);
    let before = store.last_good().unwrap();

    let error = store.save(&AppConfig::default()).unwrap_err();
    assert!(matches!(error, StoreError::Io { .. }));
    assert_eq!(store.last_good().unwrap(), before);
    assert!(target.is_dir());
}

#[test]
fn concurrent_saves_are_serialized_and_disk_matches_latest_generation() {
    let directory = test_directory("concurrent");
    let path = directory.path().join("config.json");
    let store = Arc::new(ConfigStore::new(&path));
    let barrier = Arc::new(Barrier::new(5));
    let mut threads = Vec::new();

    for index in 0..4_u32 {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        threads.push(thread::spawn(move || {
            let mut config = AppConfig::default();
            config.preferences.timezone = match index {
                0 => "UTC",
                1 => "Europe/Paris",
                2 => "Asia/Tbilisi",
                _ => "America/New_York",
            }
            .into();
            barrier.wait();
            let receipt = store.save(&config).unwrap();
            (receipt.generation, config)
        }));
    }
    barrier.wait();

    let latest = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .max_by_key(|(generation, _)| *generation)
        .unwrap();
    assert_eq!(latest.0, 4);
    let disk: AppConfig = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(disk, latest.1);
    assert_eq!(store.last_good().unwrap(), latest.1);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn v2_documents_migrate_to_cards_in_screen_order() {
    let directory = test_directory("v2-migration");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);
    fs::write(&path, include_bytes!("fixtures/v2-legacy.json")).unwrap();

    let outcome = store.load();
    assert_eq!(outcome.origin(), ConfigOrigin::MigratedV2);
    assert!(outcome.recovery().is_none());

    let config = outcome.config();
    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);

    // Order follows screens[], not widgets[] (the fixture deliberately lists the
    // pomodoro widget before the clock widget, but the clock screen comes first).
    let ids: Vec<&str> = config.cards.iter().map(CardSettings::id).collect();
    assert_eq!(ids, ["clock", "focus"]);

    // Every migrated card is in the playlist, inheriting its default dwell.
    assert_eq!(
        config.compiled_card_ids(),
        config
            .cards
            .iter()
            .map(CardSettings::id)
            .collect::<Vec<_>>()
    );

    // interrupt_policy: enabled becomes the kind-appropriate alert.
    assert_eq!(
        config.cards[1].alert(),
        CardAlert::OnTimerFinish {
            hold: AlertHold::UntilDismissed
        }
    );
    assert_eq!(config.cards[0].alert(), CardAlert::None);

    // auto_advance_seconds becomes timed advance.
    assert_eq!(
        config.playlists[0].advance,
        CarouselAdvance::Timed {
            default_dwell_seconds: 30
        }
    );

    // Preferences survive untouched.
    assert_eq!(config.preferences.timezone, "Asia/Tbilisi");
    assert_eq!(
        config.preferences.orientation,
        DisplayOrientation::LandscapeFlipped
    );
}

#[test]
fn migrated_v2_documents_always_satisfy_the_rotation_rule() {
    let directory = test_directory("v2-rotation-rule");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);
    fs::write(&path, include_bytes!("fixtures/v2-legacy.json")).unwrap();

    let loaded = store.load();
    let config = loaded.config();
    assert!(config.validate().is_ok());
}

#[test]
fn future_v10_is_a_recoverable_error_preserving_bytes() {
    let directory = test_directory("future-version");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);

    let mut last_good = AppConfig::default();
    last_good.preferences.autostart = true;
    store.save(&last_good).unwrap();

    let future = include_str!("fixtures/future-v7.json");
    fs::write(&path, future).unwrap();

    let outcome = store.load();
    assert_eq!(outcome.origin(), ConfigOrigin::LastGood);
    assert_eq!(outcome.config(), &last_good);
    assert!(matches!(
        outcome.recovery(),
        Some(StoreError::UnsupportedVersion {
            found: 10,
            supported: 9
        })
    ));
    // The unreadable source bytes are never rewritten.
    assert_eq!(fs::read_to_string(&path).unwrap(), future);
}

#[test]
fn v2_dashboard_layout_document_fails_to_deserialize_as_a_recoverable_error() {
    // A dashboard layout could never appear in a persisted v2 file: v2's `compile()`
    // rejected `ScreenLayout::Dashboard` with `requires-capability`, and
    // `ConfigStore::save` validates/compiles before writing to disk. `LegacyLayoutV2`
    // therefore has only a `Single` variant, so a document like this one — which could
    // only have been produced by bypassing the app entirely — fails to deserialize as
    // an unknown `kind` variant, surfacing as a recoverable `StoreError::InvalidJson`.
    // This fresh store has no last-good document, so the fallback is explicitly
    // labeled as defaults rather than inventing a dashboard-to-card mapping.
    let directory = test_directory("v2-dashboard");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);

    let dashboard_v2 = br#"{
      "schema_version": 2,
      "preferences": {
        "timezone": "UTC",
        "autostart": false,
        "paused": false,
        "orientation": "landscape"
      },
      "widgets": [
        {
          "kind": "clock",
          "id": "clock",
          "size": "full",
          "title": "Desk",
          "show_seconds": true,
          "template": { "kind": "digital-clock" },
          "tap_action": { "kind": "none" },
          "refresh": { "kind": "device-local" },
          "interrupt_policy": "disabled"
        }
      ],
      "screens": [
        {
          "id": "dash",
          "layout": {
            "kind": "dashboard",
            "columns": 2,
            "rows": 2,
            "tiles": [
              { "widget_id": "clock", "column": 0, "row": 0, "column_span": 1, "row_span": 1 }
            ]
          }
        }
      ]
    }"#;
    fs::write(&path, dashboard_v2).unwrap();

    let outcome = store.load();
    assert_eq!(outcome.origin(), ConfigOrigin::Defaults);
    assert_eq!(outcome.config(), &AppConfig::default());
    assert!(matches!(
        outcome.recovery(),
        Some(StoreError::InvalidJson { .. })
    ));
    // The unreadable source bytes are never rewritten.
    assert_eq!(fs::read(&path).unwrap(), dashboard_v2);
}

#[test]
fn v2_orphan_widgets_become_alert_only_or_off_instead_of_being_dropped() {
    // A widget referenced by no screen was already impossible under v2's own
    // validation, but the on-disk format does not enforce that; migration must not
    // silently discard configuration the user actually wrote to disk. The
    // pomodoro-kind orphan's historical `interrupt_policy: enabled` means it becomes
    // alert-only (its alert can still fire); the clock-kind orphan's `disabled`
    // becomes off.
    let directory = test_directory("v2-orphans");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);

    let with_orphans = br#"{
      "schema_version": 2,
      "preferences": {
        "timezone": "UTC",
        "autostart": false,
        "paused": false,
        "orientation": "landscape"
      },
      "widgets": [
        {
          "kind": "clock",
          "id": "clock",
          "size": "full",
          "title": "Desk",
          "show_seconds": true,
          "template": { "kind": "digital-clock" },
          "tap_action": { "kind": "none" },
          "refresh": { "kind": "device-local" },
          "interrupt_policy": "disabled"
        },
        {
          "kind": "clock",
          "id": "zeta-orphan",
          "size": "full",
          "title": "Unreferenced",
          "show_seconds": false,
          "template": { "kind": "digital-clock" },
          "tap_action": { "kind": "none" },
          "refresh": { "kind": "device-local" },
          "interrupt_policy": "disabled"
        },
        {
          "kind": "pomodoro",
          "id": "alpha-orphan",
          "size": "standard",
          "label": "Focus",
          "duration_seconds": 1500,
          "template": { "kind": "progress-ring" },
          "tap_action": { "kind": "start-pause" },
          "refresh": { "kind": "device-local" },
          "interrupt_policy": "enabled"
        }
      ],
      "screens": [
        { "id": "clock-screen", "layout": { "kind": "single", "widget_id": "clock" } }
      ]
    }"#;
    fs::write(&path, with_orphans).unwrap();

    let outcome = store.load();
    assert_eq!(outcome.origin(), ConfigOrigin::MigratedV2);
    assert!(outcome.recovery().is_none());

    let config = outcome.config();
    // Card order: screens[] first ("clock"), then orphans sorted by ID
    // ("alpha-orphan" before "zeta-orphan").
    let ids: Vec<&str> = config.cards.iter().map(CardSettings::id).collect();
    assert_eq!(ids, ["clock", "alpha-orphan", "zeta-orphan"]);

    assert_eq!(config.playlists[0].entries[0].card_id, "clock");
    assert_eq!(config.compiled_card_ids(), ["clock", "alpha-orphan"]);
    assert_eq!(
        config.cards[1].alert(),
        CardAlert::OnTimerFinish {
            hold: AlertHold::UntilDismissed
        }
    );
    assert_eq!(config.cards[2].alert(), CardAlert::None);

    config.validate().unwrap();
}

#[test]
fn v1_orphan_widgets_become_alert_only_or_off_instead_of_being_dropped() {
    // Same rule as the v2 path, exercised against v1: a widget referenced by no
    // screen must not be silently dropped. Pomodoro widgets are historically
    // `interrupt_policy: enabled` (per the design spec's migration rule), so the
    // orphaned pomodoro becomes alert-only; the orphaned clock becomes off.
    let directory = test_directory("v1-orphans");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);

    let with_orphans = br#"{
      "schema_version": 1,
      "preferences": {
        "timezone": "UTC",
        "autostart": false,
        "paused": false,
        "orientation": "landscape"
      },
      "widgets": [
        { "kind": "clock", "id": "clock", "size": "full", "title": "Desk", "show_seconds": true },
        { "kind": "clock", "id": "zeta-orphan", "size": "full", "title": "Unreferenced", "show_seconds": false },
        { "kind": "pomodoro", "id": "alpha-orphan", "size": "standard", "label": "Focus", "duration_seconds": 1500 }
      ],
      "screens": [
        { "id": "clock-screen", "widget_id": "clock" }
      ]
    }"#;
    fs::write(&path, with_orphans).unwrap();

    let outcome = store.load();
    assert_eq!(outcome.origin(), ConfigOrigin::MigratedV1);
    assert!(outcome.recovery().is_none());

    let config = outcome.config();
    // Card order: screens[] first ("clock"), then orphans sorted by ID
    // ("alpha-orphan" before "zeta-orphan").
    let ids: Vec<&str> = config.cards.iter().map(CardSettings::id).collect();
    assert_eq!(ids, ["clock", "alpha-orphan", "zeta-orphan"]);

    assert_eq!(config.playlists[0].entries[0].card_id, "clock");
    assert_eq!(config.compiled_card_ids(), ["clock", "alpha-orphan"]);
    assert_eq!(
        config.cards[1].alert(),
        CardAlert::OnTimerFinish {
            hold: AlertHold::UntilDismissed
        }
    );
    assert_eq!(config.cards[2].alert(), CardAlert::None);

    config.validate().unwrap();
}
