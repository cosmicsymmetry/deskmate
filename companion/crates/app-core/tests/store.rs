use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use app_core::{AppConfig, ConfigOrigin, ConfigStore, MAX_CONFIG_FILE_BYTES, StoreError};

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
    assert_eq!(migrated.config.schema_version, 1);
    assert_eq!(migrated.config.preferences.timezone, "Europe/Paris");
    assert!(!migrated.config.preferences.autostart);
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
