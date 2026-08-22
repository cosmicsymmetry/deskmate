use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use app_core::{
    AppConfig, ConfigStore, DeviceTier, NetworkSettingsLoadOutcome, NetworkSettingsOrigin,
    NetworkSettingsStore, NetworkSettingsUpdate,
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "deskmate-network-settings-{name}-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn admin_credential_and_tier_round_trip_with_the_secret_redacted_from_readback() {
    let directory = TestDirectory::new("round-trip");
    let config_path = directory.path("config.json");
    ConfigStore::new(&config_path)
        .save(&AppConfig::default())
        .unwrap();
    let config_before = fs::read(&config_path).unwrap();

    let settings_path = directory.path("network-settings.json");
    NetworkSettingsStore::new(&settings_path)
        .save(NetworkSettingsUpdate::new(
            "https://deskmate.example",
            "dev-0042",
            Some(DeviceTier::Networked),
            Some("admin-secret".into()),
        ))
        .unwrap();

    let outcome = NetworkSettingsStore::new(&settings_path).load();
    assert_eq!(outcome.origin(), NetworkSettingsOrigin::Current);
    assert_eq!(outcome.settings().server_url, "https://deskmate.example");
    assert_eq!(outcome.settings().device_id, "dev-0042");
    assert_eq!(outcome.settings().tier, Some(DeviceTier::Networked));
    assert_eq!(outcome.recovery(), None);

    let public_json = serde_json::to_string(outcome.settings()).unwrap();
    let public_debug = format!("{outcome:?}");
    for forbidden in ["admin-secret", "admin_token", "device_token"] {
        assert!(!public_json.contains(forbidden));
        assert!(!public_debug.contains(forbidden));
    }

    let persisted = fs::read_to_string(&settings_path).unwrap();
    assert!(persisted.contains("admin-secret"));
    assert!(!persisted.contains("device_token"));
    assert_eq!(fs::read(&config_path).unwrap(), config_before);
    assert_eq!(
        serde_json::from_slice::<AppConfig>(&config_before)
            .unwrap()
            .schema_version,
        5
    );
}

#[test]
fn blank_write_only_input_preserves_admin_token_and_tier_across_store_recreation() {
    let directory = TestDirectory::new("preserve-secrets");
    let settings_path = directory.path("network-settings.json");
    NetworkSettingsStore::new(&settings_path)
        .save(NetworkSettingsUpdate::new(
            "https://old.example",
            "dev-0042",
            Some(DeviceTier::Networked),
            Some("admin-secret".into()),
        ))
        .unwrap();

    NetworkSettingsStore::new(&settings_path)
        .save(NetworkSettingsUpdate::new(
            "https://new.example",
            "dev-0042",
            None,
            None,
        ))
        .unwrap();

    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(&settings_path).unwrap()).unwrap();
    assert_eq!(persisted["server_url"], "https://new.example");
    assert_eq!(persisted["tier"], "networked");
    assert_eq!(persisted["admin_token"], "admin-secret");
    assert!(persisted.get("device_token").is_none());

    let reopened = NetworkSettingsStore::new(&settings_path);
    let used = reopened
        .with_admin_token(|token| token == "admin-secret")
        .unwrap();
    assert_eq!(used, Some(true));
}

#[test]
fn legacy_device_token_is_accepted_then_removed_on_the_next_save() {
    let directory = TestDirectory::new("legacy-device-token");
    let settings_path = directory.path("network-settings.json");
    fs::write(
        &settings_path,
        br#"{
  "format_version": 1,
  "server_url": "https://desk.example",
  "device_id": "desk-1",
  "admin_token": "admin-secret",
  "device_token": "unused-device-secret"
}"#,
    )
    .unwrap();

    let store = NetworkSettingsStore::new(&settings_path);
    assert!(store.discard_device_token().unwrap());
    assert!(!store.discard_device_token().unwrap());

    let persisted = fs::read_to_string(settings_path).unwrap();
    assert!(!persisted.contains("unused-device-secret"));
    assert!(!persisted.contains("device_token"));
}

#[test]
fn absent_and_corrupt_network_settings_degrade_to_redacted_defaults() {
    let directory = TestDirectory::new("recovery");
    let settings_path = directory.path("network-settings.json");
    let store = NetworkSettingsStore::new(&settings_path);

    let missing = store.load();
    assert_eq!(missing.origin(), NetworkSettingsOrigin::Defaults);
    assert_eq!(missing.settings().server_url, "");
    assert_eq!(missing.settings().device_id, "");
    assert_eq!(missing.settings().tier, None);
    assert_eq!(missing.recovery(), None);
    assert!(!settings_path.exists());

    fs::write(&settings_path, b"{\"format_version\":1,\"admin_token\":").unwrap();
    let NetworkSettingsLoadOutcome::Recovered {
        settings,
        origin,
        error: _,
    } = NetworkSettingsStore::new(&settings_path).load()
    else {
        panic!("corrupt settings must recover instead of failing startup");
    };
    assert_eq!(origin, NetworkSettingsOrigin::Defaults);
    assert_eq!(settings.server_url, "");
    assert_eq!(settings.device_id, "");
}

#[cfg(unix)]
#[test]
fn network_settings_file_is_private() {
    let directory = TestDirectory::new("mode");
    let settings_path = directory.path("network-settings.json");
    NetworkSettingsStore::new(&settings_path)
        .save(NetworkSettingsUpdate::new(
            "https://deskmate.example",
            "dev-0042",
            Some(DeviceTier::Networked),
            Some("admin-secret".into()),
        ))
        .unwrap();

    let mode = fs::metadata(settings_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}
