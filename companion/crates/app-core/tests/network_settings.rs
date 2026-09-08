use std::fs;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use app_core::{
    AppConfig, ConfigStore, DeviceTier, NetworkSettingsLoadOutcome, NetworkSettingsStore,
    NetworkSettingsUpdate,
};

fn test_directory(name: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("deskmate-network-settings-{name}-"))
        .tempdir()
        .unwrap()
}

#[test]
fn admin_credential_and_tier_round_trip_with_the_secret_redacted_from_readback() {
    let directory = test_directory("round-trip");
    let config_path = directory.path().join("config.json");
    ConfigStore::new(&config_path)
        .save(&AppConfig::default())
        .unwrap();
    let config_before = fs::read(&config_path).unwrap();

    let settings_path = directory.path().join("network-settings.json");
    NetworkSettingsStore::new(&settings_path)
        .save(NetworkSettingsUpdate::new(
            "https://deskmate.example",
            "dev-0042",
            Some(DeviceTier::Networked),
            Some("admin-secret".into()),
        ))
        .unwrap();

    let outcome = NetworkSettingsStore::new(&settings_path).load();
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
        app_core::CURRENT_SCHEMA_VERSION
    );
}

#[test]
fn blank_write_only_input_preserves_admin_token_and_tier_across_store_recreation() {
    let directory = test_directory("preserve-secrets");
    let settings_path = directory.path().join("network-settings.json");
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
    let directory = test_directory("legacy-device-token");
    let settings_path = directory.path().join("network-settings.json");
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
    let directory = test_directory("recovery");
    let settings_path = directory.path().join("network-settings.json");
    let store = NetworkSettingsStore::new(&settings_path);

    let missing = store.load();
    assert_eq!(missing.settings().server_url, "");
    assert_eq!(missing.settings().device_id, "");
    assert_eq!(missing.settings().tier, None);
    assert_eq!(missing.recovery(), None);
    assert!(!settings_path.exists());

    fs::write(&settings_path, b"{\"format_version\":1,\"admin_token\":").unwrap();
    let NetworkSettingsLoadOutcome::Recovered { settings, error: _ } =
        NetworkSettingsStore::new(&settings_path).load()
    else {
        panic!("corrupt settings must recover instead of failing startup");
    };
    assert_eq!(settings.server_url, "");
    assert_eq!(settings.device_id, "");
}

#[cfg(unix)]
#[test]
fn network_settings_file_is_private() {
    let directory = test_directory("mode");
    let settings_path = directory.path().join("network-settings.json");
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
