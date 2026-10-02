use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;

#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use app_core::{
    AppConfig, AssetKind, AssetSettings, AssetSource, CURRENT_SCHEMA_VERSION, ConfigOrigin,
    ConfigStore, IconGlyphMapping, LoadOutcome, MAX_ASSET_SOURCE_LEN, MAX_CONFIG_FILE_BYTES,
    MAX_ICON_GLYPH_NAME_LEN, MAX_ICON_GLYPHS, SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE,
    StoreError,
};

#[test]
fn unsupported_schema_matrix_preserves_bytes_and_uses_only_genuine_fallbacks() {
    for version in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12] {
        for bytes in [
            format!(r#"{{"schema_version":{version}}}"#).into_bytes(),
            format!(
                r#"{{"schema_version":{version},"preferences":{{}},"cards":[],"playlists":[],"active_playlist_id":"old"}}"#
            )
            .into_bytes(),
        ] {
            assert_unsupported_version_uses_only_genuine_fallbacks(version, &bytes);
        }
    }
}

fn assert_unsupported_version_uses_only_genuine_fallbacks(version: u32, bytes: &[u8]) {
    let directory = test_directory(&format!("unsupported-v{version}"));
    let path = directory.path().join("config.json");
    fs::write(&path, bytes).unwrap();

    let store = ConfigStore::new(&path);
    let fresh = store.load();
    assert_eq!(fresh.origin(), ConfigOrigin::Defaults, "schema v{version}");
    assert_eq!(fresh.config(), &AppConfig::default(), "schema v{version}");
    assert_eq!(
        fresh.recovery(),
        Some(StoreError::UnsupportedVersion {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        }),
        "schema v{version}"
    );
    assert_eq!(fs::read(&path).unwrap(), bytes, "schema v{version}");

    let repeated = store.load();
    assert_eq!(
        repeated.origin(),
        ConfigOrigin::Defaults,
        "schema v{version}"
    );
    assert_eq!(
        repeated.config(),
        &AppConfig::default(),
        "schema v{version}"
    );
    assert_eq!(
        repeated.recovery(),
        Some(StoreError::UnsupportedVersion {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        }),
        "schema v{version}"
    );
    assert_eq!(fs::read(&path).unwrap(), bytes, "schema v{version}");

    let loaded_path = directory.path().join("loaded.json");
    let loaded_store = ConfigStore::new(&loaded_path);
    let mut loaded_last_good = AppConfig::default();
    loaded_last_good.preferences.timezone = "Europe/Paris".into();
    fs::write(
        &loaded_path,
        serde_json::to_vec_pretty(&loaded_last_good).unwrap(),
    )
    .unwrap();
    let loaded = loaded_store.load();
    assert_eq!(loaded.origin(), ConfigOrigin::Current, "schema v{version}");
    assert_eq!(loaded.config(), &loaded_last_good, "schema v{version}");
    fs::write(&loaded_path, bytes).unwrap();
    let recovered_after_load = loaded_store.load();
    assert_eq!(
        recovered_after_load.origin(),
        ConfigOrigin::LastGood,
        "schema v{version}"
    );
    assert_eq!(
        recovered_after_load.config(),
        &loaded_last_good,
        "schema v{version}"
    );
    assert_eq!(
        recovered_after_load.recovery(),
        Some(StoreError::UnsupportedVersion {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        }),
        "schema v{version}"
    );
    assert_eq!(fs::read(&loaded_path).unwrap(), bytes, "schema v{version}");

    let saved_path = directory.path().join("saved.json");
    let saved_store = ConfigStore::new(&saved_path);
    let mut saved_last_good = AppConfig::default();
    saved_last_good.preferences.timezone = "Asia/Tbilisi".into();
    saved_store.save(&saved_last_good).unwrap();
    fs::write(&saved_path, bytes).unwrap();

    let recovered = saved_store.load();
    assert_eq!(
        recovered.origin(),
        ConfigOrigin::LastGood,
        "schema v{version}"
    );
    assert_eq!(recovered.config(), &saved_last_good, "schema v{version}");
    assert_eq!(
        recovered.recovery(),
        Some(StoreError::UnsupportedVersion {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        }),
        "schema v{version}"
    );
    assert_eq!(fs::read(&saved_path).unwrap(), bytes, "schema v{version}");
}

#[test]
fn v11_without_image_sources_loads_as_current_without_rewriting() {
    let (_directory, path, store) = test_store("v10-omitted-image-sources");
    let bytes = include_bytes!("fixtures/default.json");
    fs::write(&path, bytes).unwrap();

    let loaded = store.load();

    assert_eq!(loaded.origin(), ConfigOrigin::Current);
    assert!(loaded.config().image_sources.is_empty());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn an_empty_v10_loop_is_validation_failed_not_repaired() {
    let (_directory, path, store) = test_store("empty-v10-loop");
    let mut value: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/v10.json")).unwrap();
    value["cards"] = serde_json::json!([]);
    let bytes = serde_json::to_vec_pretty(&value).unwrap();
    fs::write(&path, &bytes).unwrap();

    let outcome = store.load();

    let LoadOutcome::ValidationFailed {
        config,
        origin,
        issues,
    } = outcome
    else {
        panic!("an empty migrated loop must remain a validation failure");
    };
    assert_eq!(origin, ConfigOrigin::Defaults);
    assert_eq!(config, AppConfig::default());
    assert!(issues.iter().any(|issue| issue.path == "cards"));
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

fn test_directory(name: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("deskmate-app-core-{name}-"))
        .tempdir()
        .unwrap()
}

fn test_store(name: &str) -> (tempfile::TempDir, std::path::PathBuf, ConfigStore) {
    let directory = test_directory(name);
    let path = directory.path().join("config.json");
    let store = ConfigStore::new(&path);
    (directory, path, store)
}

#[test]
fn missing_file_loads_defaults_without_writing() {
    let (_directory, path, store) = test_store("first-run");

    let outcome = store.load();
    assert_eq!(outcome.origin(), ConfigOrigin::Defaults);
    assert_eq!(outcome.config(), &AppConfig::default());
    assert_eq!(outcome.recovery(), None);
    assert!(!path.exists());
}

#[test]
fn invalid_persisted_config_keeps_last_good_and_reports_typed_error() {
    let (_directory, path, store) = test_store("validation-recovery");
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
    let (_directory, path, store) = test_store("validation-defaults");
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
fn save_round_trips() {
    let (_directory, _path, store) = test_store("round-trip");
    let mut config = AppConfig::default();
    config.preferences.timezone = "Asia/Tbilisi".into();

    let receipt = store.save(&config).unwrap();
    assert_eq!(receipt.generation, 1);
    assert_eq!(receipt.warning, None);
    let loaded = store.load();
    assert_eq!(loaded.origin(), ConfigOrigin::Current);
    assert_eq!(loaded.config(), &config);
}

#[cfg(unix)]
#[test]
fn saved_config_is_user_readable_and_writable_only() {
    let (_directory, path, store) = test_store("private-save");

    store.save(&AppConfig::default()).unwrap();

    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn loading_repairs_a_permissive_existing_config_mode() {
    let (_directory, path, store) = test_store("private-load");
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

    let loaded = store.load();

    assert_eq!(loaded.origin(), ConfigOrigin::Current);
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn malformed_truncated_and_oversized_files_preserve_bytes_and_last_good() {
    let (_directory, path, store) = test_store("recovery");
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
fn a_save_at_the_size_limit_is_refused_and_one_byte_under_reloads() {
    let (_directory, path, store) = test_store("save-size-limit");
    let last_good = AppConfig::default();
    store.save(&last_good).unwrap();
    let saved_bytes = fs::read(&path).unwrap();

    let mut config = AppConfig::default();
    for index in 0..3 {
        config.assets.push(AssetSettings {
            id: format!("icons-{index}"),
            source: AssetSource::File("icons.ttf".into()),
            kind: AssetKind::IconFont {
                glyphs: (0..MAX_ICON_GLYPHS)
                    .map(|glyph| IconGlyphMapping {
                        name: format!("{glyph:0MAX_ICON_GLYPH_NAME_LEN$}"),
                        codepoint: 65,
                    })
                    .collect(),
            },
            maximum_bytes: 1,
        });
    }
    config.assets.push(AssetSettings {
        id: "font".into(),
        source: AssetSource::File(String::new()),
        kind: AssetKind::Font,
        maximum_bytes: 1,
    });

    let source_len = loop {
        let size = serde_json::to_vec_pretty(&config).unwrap().len();
        if let Some(delta) = MAX_CONFIG_FILE_BYTES.checked_sub(size)
            && (1..=MAX_ASSET_SOURCE_LEN).contains(&delta)
        {
            break delta;
        }
        config
            .assets
            .iter_mut()
            .find_map(|asset| match &mut asset.kind {
                AssetKind::IconFont { glyphs } if glyphs.len() > 1 => Some(glyphs),
                _ => None,
            })
            .expect("enough glyphs to trim to the file limit")
            .pop();
    };
    config.assets.last_mut().unwrap().source = AssetSource::File("x".repeat(source_len));
    assert!(config.validate().is_ok());
    assert_eq!(
        serde_json::to_vec_pretty(&config).unwrap().len(),
        MAX_CONFIG_FILE_BYTES
    );

    assert_eq!(
        store.save(&config),
        Err(StoreError::TooLarge {
            maximum: MAX_CONFIG_FILE_BYTES
        })
    );
    assert_eq!(fs::read(&path).unwrap(), saved_bytes);
    assert_eq!(store.last_good().unwrap(), last_good);

    let AssetSource::File(source) = &mut config.assets.last_mut().unwrap().source;
    source.pop();
    store.save(&config).unwrap();
    let mut expected_bytes = serde_json::to_vec_pretty(&config).unwrap();
    expected_bytes.push(b'\n');
    let written_bytes = fs::read(&path).unwrap();
    assert_eq!(written_bytes.len(), MAX_CONFIG_FILE_BYTES);
    assert_eq!(written_bytes, expected_bytes);
    assert_eq!(
        ConfigStore::new(&path).load(),
        LoadOutcome::Loaded {
            config,
            origin: ConfigOrigin::Current
        }
    );
}

#[test]
fn failed_replace_preserves_last_good() {
    let (_directory, path, store) = test_store("replace-failure");
    let mut seeded = AppConfig::default();
    seeded.preferences.timezone = "Europe/Paris".into();
    store.save(&seeded).unwrap();
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();

    let mut attempted = AppConfig::default();
    attempted.preferences.timezone = "Asia/Tbilisi".into();
    let error = store.save(&attempted).unwrap_err();
    assert!(matches!(error, StoreError::Io { .. }));
    assert_eq!(store.last_good().unwrap(), seeded);
    assert!(path.is_dir());
}

#[test]
fn concurrent_saves_are_serialized_and_disk_matches_latest_generation() {
    let (directory, path, store) = test_store("concurrent");
    let store = Arc::new(store);
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
    let (_directory, path, store) = test_store("future-version");

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
            found: 12,
            supported: CURRENT_SCHEMA_VERSION
        })
    ));
    // The unreadable source bytes are never rewritten.
    assert_eq!(fs::read_to_string(&path).unwrap(), future);
}

#[test]
fn v10_migration_preserves_boot_brightness_and_original_bytes() {
    let (_directory, path, store) = test_store("brightness-v10");
    let bytes = include_bytes!("fixtures/v10.json");
    fs::write(&path, bytes).unwrap();
    let loaded = store.load();
    assert_eq!(loaded.origin(), ConfigOrigin::Migrated);
    assert_eq!(loaded.config().schema_version, 11);
    assert_eq!(loaded.config().preferences.brightness, 78);
    assert_eq!(loaded.config().preferences.brightness_level(), 200);
    assert_eq!(
        loaded.config().compile(1).unwrap().layout.brightness,
        Some(200)
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    store.save(loaded.config()).unwrap();
    assert_eq!(store.load().origin(), ConfigOrigin::Current);
    assert_eq!(store.load().config(), loaded.config());
}

#[test]
fn v10_migration_keeps_closed_preferences_and_validation() {
    for (field, value) in [
        ("brightness", serde_json::json!(78)),
        ("extra", serde_json::json!(true)),
    ] {
        let (_directory, path, store) = test_store("brightness-closed-v10");
        let mut document: serde_json::Value =
            serde_json::from_slice(include_bytes!("fixtures/v10.json")).unwrap();
        document["preferences"][field] = value;
        let bytes = serde_json::to_vec(&document).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(matches!(
            store.load().recovery(),
            Some(StoreError::InvalidJson { .. })
        ));
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn v10_migration_preserves_all_authored_fields() {
    for fixture in [
        include_str!("fixtures/full.json"),
        include_str!("fixtures/card-surface.json"),
    ] {
        let expected: AppConfig = serde_json::from_str(fixture).unwrap();
        let mut legacy = serde_json::to_value(&expected).unwrap();
        legacy["schema_version"] = serde_json::json!(10);
        legacy["preferences"]
            .as_object_mut()
            .unwrap()
            .remove("brightness");
        let bytes = serde_json::to_vec(&legacy).unwrap();
        let (_directory, path, store) = test_store("brightness-v10-preserve");
        fs::write(&path, &bytes).unwrap();
        let loaded = store.load();
        assert_eq!(loaded.origin(), ConfigOrigin::Migrated);
        assert_eq!(loaded.config(), &expected);
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}
