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

#[test]
fn a_pre_v4_document_is_refused_as_an_unsupported_version() {
    // No supported companion version writes v0-v3 documents. Refusing one is
    // a typed, legible outcome -- not a validation failure, which would be
    // presented to the owner as broken settings.
    let directory = test_directory("pre-v4-refused");
    let path = directory.path().join("config.json");
    fs::write(
        &path,
        br#"{"schema_version":3,"preferences":{},"cards":[]}"#,
    )
    .expect("write v3 document");

    let store = ConfigStore::new(&path);
    let outcome = store.load();

    let recovery = outcome.recovery().expect("a v3 document must be refused");
    assert!(
        matches!(
            recovery,
            StoreError::UnsupportedVersion {
                found: 3,
                supported: CURRENT_SCHEMA_VERSION
            }
        ),
        "a v3 document must be an UnsupportedVersion refusal, got {recovery:?}"
    );
}

#[test]
fn v9_folds_the_active_playlist_into_the_card_order_and_loses_no_card() {
    // The whole point of v10: one list. The active playlist's order becomes the
    // card order and its dwells ride along; cards it never named are appended
    // rather than dropped, which is what a v4-v9 "library outside the loop"
    // becomes. Playlist names and the inactive playlist itself are the only
    // things lost, on purpose.
    let directory = test_directory("v9-two-playlists");
    let path = directory.path().join("config.json");
    fs::write(&path, include_bytes!("fixtures/v9-two-playlists.json")).unwrap();

    let outcome = ConfigStore::new(&path).load();
    assert_eq!(outcome.origin(), ConfigOrigin::MigratedV9);
    let config = outcome.config();
    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);

    // Entry order first, then everything the active playlist did not name, in
    // the order the card library had them.
    let ids: Vec<&str> = config.cards.iter().map(CardSettings::id).collect();
    assert_eq!(ids, ["desk", "focus", "library-only", "evening-only"]);

    // Dwell rode across from the entry, and an unnamed card gets none.
    assert_eq!(config.cards[0].dwell_seconds(), Some(20));
    assert_eq!(config.cards[1].dwell_seconds(), None);
    assert_eq!(config.cards[2].dwell_seconds(), None);
    assert_eq!(config.cards[3].dwell_seconds(), None);

    // The active playlist's advance became the document's; the inactive
    // playlist's `manual` did not win.
    assert_eq!(
        config.advance,
        CarouselAdvance::Timed {
            default_dwell_seconds: 30
        }
    );

    config.compile(7).unwrap();
}

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
    invalid["cards"][0]["dwell_seconds"] = serde_json::json!(1);
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
            .any(|issue| issue.path == "cards[0].dwell_seconds")
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
    invalid["cards"][0]["dwell_seconds"] = serde_json::json!(1);
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

    // A migration is never silent: the outcome names the version it came from,
    // which is what lets a caller tell "this is what you saved" from "this is
    // what we made of what you saved".
    fs::write(&path, include_bytes!("fixtures/v4-roundtrip.json")).unwrap();
    let migrated = store.load();
    assert_eq!(migrated.origin(), ConfigOrigin::MigratedV4);
    assert_eq!(migrated.config().schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(migrated.config().preferences.timezone, "Asia/Tbilisi");
    assert!(migrated.config().preferences.autostart);
    assert_eq!(
        migrated.config().preferences.orientation,
        DisplayOrientation::LandscapeFlipped
    );
    assert_eq!(migrated.config().cards.len(), 2);
    assert!(matches!(
        &migrated.config().cards[0],
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
    assert!(matches!(
        &migrated.config().cards[1],
        CardSettings::Pomodoro {
            id,
            label,
            duration_seconds: 1_500,
            template: DisplayTemplate::ProgressRing,
            tap_action: WidgetTapAction::StartPause,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::OnTimerFinish { hold: AlertHold::UntilDismissed },
            ..
        } if id == "focus" && label == "Focus"
    ));
    assert!(migrated.config().assets.is_empty());
    assert_eq!(
        migrated.config().advance,
        CarouselAdvance::Timed {
            default_dwell_seconds: 30
        }
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
    assert_eq!(config.schema_version, 10);
    assert!(config.image_sources.is_empty(), "v6 knew no sources");
    // Lossless: everything else survived untouched.
    assert_eq!(config.cards.len(), 1);
    assert_eq!(config.cards[0].id(), "clock");
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
fn an_all_retired_v7_document_gets_the_default_clock() {
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
    // Every card was a retired kind, so the loop would have been empty. It
    // gets exactly the fallback card `AppConfig::default()` ships, never an
    // empty panel.
    assert_eq!(config.cards, defaults.cards);
    config.validate().unwrap();
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
fn a_future_schema_is_a_recoverable_error_preserving_bytes() {
    let directory = test_directory("future-version");
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);

    let mut last_good = AppConfig::default();
    last_good.preferences.autostart = true;
    store.save(&last_good).unwrap();

    let future = include_str!("fixtures/future-version.json");
    fs::write(&path, future).unwrap();

    let outcome = store.load();
    assert_eq!(outcome.origin(), ConfigOrigin::LastGood);
    assert_eq!(outcome.config(), &last_good);
    assert!(matches!(
        outcome.recovery(),
        Some(StoreError::UnsupportedVersion {
            found: 11,
            supported: CURRENT_SCHEMA_VERSION
        })
    ));
    // The unreadable source bytes are never rewritten.
    assert_eq!(fs::read_to_string(&path).unwrap(), future);
}
