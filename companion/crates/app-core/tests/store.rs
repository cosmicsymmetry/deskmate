use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use app_core::{
    AlertHold, AppConfig, CalendarSource, CardAlert, CardSettings, CarouselAdvance, ConfigOrigin,
    ConfigStore, DisplayOrientation, DisplayTemplate, MAX_CONFIG_FILE_BYTES, RefreshPolicy,
    StoreError, WidgetTapAction,
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "deskmate-app-core-{name}-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn config_path(&self) -> PathBuf {
        self.0.join("config.json")
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn missing_file_loads_defaults_without_writing() {
    let directory = TestDirectory::new("first-run");
    let path = directory.config_path();
    let store = ConfigStore::new(&path);

    let outcome = store.load();
    assert_eq!(outcome.origin, ConfigOrigin::Defaults);
    assert_eq!(outcome.config, AppConfig::default());
    assert_eq!(outcome.recovery, None);
    assert!(!path.exists());
}

#[test]
fn save_round_trips_and_migration_is_explicit() {
    let directory = TestDirectory::new("round-trip");
    let path = directory.config_path();
    let store = ConfigStore::new(&path);
    let mut config = AppConfig::default();
    config.preferences.timezone = "Asia/Tbilisi".into();

    let receipt = store.save(&config).unwrap();
    assert_eq!(receipt.generation, 1);
    assert_eq!(receipt.warning, None);
    let loaded = store.load();
    assert_eq!(loaded.origin, ConfigOrigin::Current);
    assert_eq!(loaded.config, config);

    fs::write(&path, include_bytes!("fixtures/legacy-v0.json")).unwrap();
    let migrated = store.load();
    assert_eq!(migrated.origin, ConfigOrigin::MigratedV0);
    assert_eq!(migrated.config.schema_version, 4);
    assert_eq!(migrated.config.preferences.timezone, "Europe/Paris");
    assert!(!migrated.config.preferences.autostart);

    fs::write(&path, include_bytes!("fixtures/released-m3-v1.json")).unwrap();
    let migrated = store.load();
    assert_eq!(migrated.origin, ConfigOrigin::MigratedV1);
    assert_eq!(migrated.config.schema_version, 4);
    assert_eq!(migrated.config.preferences.timezone, "Asia/Tbilisi");
    assert_eq!(migrated.config.cards.len(), 3);
    assert_eq!(
        migrated.config.preferences.orientation,
        DisplayOrientation::LandscapeFlipped
    );
    // The legacy fixture's `screens` list orders pomodoro, clock, calendar; the card
    // model's rotation order is now the card order, so migration preserves that order.
    assert!(matches!(
        &migrated.config.cards[0],
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
        &migrated.config.cards[1],
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
    // The legacy migration's calendar default is `interrupt_policy: disabled`, so this
    // must migrate to `alert: none`, not `before-event` — a calendar card only gets
    // `before-event` when it is derived from a legacy `interrupt_policy: enabled`.
    assert!(matches!(
        &migrated.config.cards[2],
        CardSettings::Calendar {
            id,
            source: CalendarSource::Url(source),
            refresh: RefreshPolicy::Interval { minutes: 15 },
            alert: CardAlert::None,
            ..
        } if id == "calendar" && source == "https://example.com/calendar.ics"
    ));
    assert!(migrated.config.assets.is_empty());
    assert_eq!(
        migrated.config.playlists[0].advance,
        app_core::CarouselAdvance::Manual
    );
    migrated.config.compile(7).unwrap();
}

#[test]
fn malformed_truncated_and_oversized_files_preserve_bytes_and_last_good() {
    let directory = TestDirectory::new("recovery");
    let path = directory.config_path();
    let store = ConfigStore::new(&path);
    let mut last_good = AppConfig::default();
    last_good.preferences.autostart = true;
    store.save(&last_good).unwrap();

    let malformed = include_bytes!("fixtures/malformed.json");
    fs::write(&path, malformed).unwrap();
    let recovered = store.load();
    assert_eq!(recovered.origin, ConfigOrigin::LastGood);
    assert_eq!(recovered.config, last_good);
    assert!(matches!(
        recovered.recovery,
        Some(StoreError::InvalidJson { .. })
    ));
    assert_eq!(fs::read(&path).unwrap(), malformed);

    let invalid_utf8 = [0xff, 0xfe, 0xfd];
    fs::write(&path, invalid_utf8).unwrap();
    let recovered = store.load();
    assert_eq!(recovered.config, last_good);
    assert_eq!(recovered.recovery, Some(StoreError::InvalidUtf8));
    assert_eq!(fs::read(&path).unwrap(), invalid_utf8);

    let oversized = vec![b'x'; MAX_CONFIG_FILE_BYTES + 1];
    fs::write(&path, &oversized).unwrap();
    let recovered = store.load();
    assert_eq!(recovered.config, last_good);
    assert!(matches!(
        recovered.recovery,
        Some(StoreError::TooLarge { .. })
    ));
    assert_eq!(fs::metadata(&path).unwrap().len(), oversized.len() as u64);

    let future = include_bytes!("fixtures/future-v5.json");
    fs::write(&path, future).unwrap();
    let recovered = store.load();
    assert_eq!(recovered.origin, ConfigOrigin::LastGood);
    assert_eq!(recovered.config, last_good);
    assert_eq!(
        recovered.recovery,
        Some(StoreError::UnsupportedVersion {
            found: 5,
            supported: 4,
        })
    );
    assert_eq!(fs::read(&path).unwrap(), future);
}

#[test]
fn failed_replace_preserves_last_good() {
    let directory = TestDirectory::new("replace-failure");
    let target = directory.0.join("target-is-a-directory");
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
    let directory = TestDirectory::new("concurrent");
    let path = directory.config_path();
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
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
}

#[test]
fn calendar_persistence_contains_source_metadata_but_no_fetched_payload() {
    let directory = TestDirectory::new("calendar-metadata");
    let path = directory.config_path();
    let store = ConfigStore::new(&path);
    let mut config: AppConfig = serde_json::from_str(include_str!("fixtures/full.json")).unwrap();
    let calendar = config
        .cards
        .iter_mut()
        .find(|card| matches!(card, CardSettings::Calendar { .. }))
        .unwrap();
    if let CardSettings::Calendar { source, .. } = calendar {
        *source = CalendarSource::File("/tmp/work.ics".into());
    }

    store.save(&config).unwrap();
    let json = fs::read_to_string(path).unwrap();
    assert!(json.contains("/tmp/work.ics"));
    assert!(!json.contains("remaining_seconds"));
    assert!(!json.contains("last_success"));
    assert!(!json.contains("Design review"));
}

#[test]
fn v2_documents_migrate_to_cards_in_screen_order() {
    let directory = TestDirectory::new("v2-migration");
    let path = directory.config_path();
    let store = ConfigStore::new(&path);
    fs::write(&path, include_bytes!("fixtures/v2-legacy.json")).unwrap();

    let outcome = store.load();
    assert_eq!(outcome.origin, ConfigOrigin::MigratedV2);
    assert!(outcome.recovery.is_none());

    let config = outcome.config;
    assert_eq!(config.schema_version, 4);

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
    let directory = TestDirectory::new("v2-rotation-rule");
    let path = directory.config_path();
    let store = ConfigStore::new(&path);
    fs::write(&path, include_bytes!("fixtures/v2-legacy.json")).unwrap();

    let config = store.load().config;
    assert!(config.validate().is_ok());
}

#[test]
fn unknown_future_versions_still_fail_safely() {
    let directory = TestDirectory::new("future-version");
    let path = directory.config_path();
    let store = ConfigStore::new(&path);

    let future = include_bytes!("fixtures/future-v5.json");
    fs::write(&path, future).unwrap();

    let outcome = store.load();
    assert_eq!(outcome.origin, ConfigOrigin::LastGood);
    assert!(matches!(
        outcome.recovery,
        Some(StoreError::UnsupportedVersion {
            found: 5,
            supported: 4
        })
    ));
    // The unreadable source bytes are never rewritten.
    assert_eq!(fs::read(&path).unwrap(), future);
}

#[test]
fn v2_dashboard_layout_document_fails_to_deserialize_as_a_recoverable_error() {
    // A dashboard layout could never appear in a persisted v2 file: v2's `compile()`
    // rejected `ScreenLayout::Dashboard` with `requires-capability`, and
    // `ConfigStore::save` validates/compiles before writing to disk. `LegacyLayoutV2`
    // therefore has only a `Single` variant, so a document like this one — which could
    // only have been produced by bypassing the app entirely — fails to deserialize as
    // an unknown `kind` variant, surfacing as a recoverable `StoreError::InvalidJson`
    // with the in-memory last-good config left authoritative, rather than a panic or a
    // lossy invented dashboard-to-card mapping.
    let directory = TestDirectory::new("v2-dashboard");
    let path = directory.config_path();
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
    assert_eq!(outcome.origin, ConfigOrigin::LastGood);
    assert_eq!(outcome.config, AppConfig::default());
    assert!(matches!(
        outcome.recovery,
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
    let directory = TestDirectory::new("v2-orphans");
    let path = directory.config_path();
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
    assert_eq!(outcome.origin, ConfigOrigin::MigratedV2);
    assert!(outcome.recovery.is_none());

    let config = outcome.config;
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
    let directory = TestDirectory::new("v1-orphans");
    let path = directory.config_path();
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
    assert_eq!(outcome.origin, ConfigOrigin::MigratedV1);
    assert!(outcome.recovery.is_none());

    let config = outcome.config;
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
