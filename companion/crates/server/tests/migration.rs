use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{TimeZone as _, Utc};
use server::ServerState;
use server::firmware::FirmwareCatalog;
use server::identity::DeviceState;
use server::migrate::{self, MigrationError, MigrationOutcome, migrate_if_needed};

use crate::support::IN_MEMORY_ADMIN_TOKEN;

mod support;

fn copy_fixture(name: &str) -> tempfile::TempDir {
    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).expect("create fixture directory");
        for entry in std::fs::read_dir(from).expect("read fixture directory") {
            let entry = entry.expect("fixture entry");
            let destination = to.join(entry.file_name());
            if entry.file_type().expect("fixture file type").is_dir() {
                copy_tree(&entry.path(), &destination);
            } else {
                std::fs::copy(entry.path(), destination).expect("copy fixture file");
            }
        }
    }

    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let destination = tempfile::tempdir().expect("fixture tempdir");
    copy_tree(&source, destination.path());
    destination
}

fn snapshot_tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, snapshot: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(directory).expect("read snapshot directory") {
            let entry = entry.expect("snapshot entry");
            if entry.file_type().expect("snapshot file type").is_dir() {
                visit(root, &entry.path(), snapshot);
            } else {
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .expect("entry beneath snapshot root")
                    .to_path_buf();
                snapshot.insert(
                    relative,
                    std::fs::read(entry.path()).expect("snapshot file"),
                );
            }
        }
    }

    let mut snapshot = BTreeMap::new();
    visit(root, root, &mut snapshot);
    snapshot
}

fn migration_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 23, 12, 34, 56)
        .single()
        .expect("fixed migration time")
}

#[test]
fn migrates_the_flat_layout_into_the_owner_account() {
    let dir = copy_fixture("flat-layout");
    let outcome = migrate_if_needed(dir.path(), Some(" Owner@Example.COM "), migration_time())
        .expect("migration succeeds");
    let MigrationOutcome::Migrated {
        account,
        devices,
        legacy_dir,
    } = outcome
    else {
        panic!("flat layout was not migrated")
    };
    assert_eq!(devices, 2);
    assert_eq!(
        legacy_dir.file_name().and_then(|name| name.to_str()),
        Some("legacy-20260923T123456Z")
    );

    let space = dir.path().join("accounts").join(&account.0);
    for (from, to) in [
        ("image-sources.json", "image-sources.json"),
        (
            "image-frames/image-0123456789abcdef01234567.bin",
            "image-frames/image-0123456789abcdef01234567.bin",
        ),
        ("data-cards.json", "data-cards.json"),
        ("face-state.json", "face-state.json"),
        ("dev-0001.json", "devices/dev-0001.json"),
        ("dev-0005.json", "devices/dev-0005.json"),
    ] {
        assert_eq!(
            std::fs::read(legacy_dir.join(from)).expect("archived source"),
            std::fs::read(space.join(to)).expect("migrated copy"),
            "{from}"
        );
        assert!(
            !dir.path().join(from).exists(),
            "the flat original must move to the legacy directory: {from}"
        );
    }
    assert!(
        dir.path().join("device-identities.json").exists(),
        "registry stays at the root"
    );
    assert!(
        dir.path().join("producer-credentials.json").exists(),
        "integrations stay at the root"
    );
    for stale in [
        "identity.db.migrating",
        "identity.db.migrating-wal",
        "identity.db.migrating-shm",
    ] {
        assert!(!dir.path().join(stale).exists(), "stale {stale}");
    }

    let state = ServerState::new(
        IN_MEMORY_ADMIN_TOKEN.into(),
        FirmwareCatalog::in_memory(),
        dir.path().to_path_buf(),
    );
    let accounts = state.identity().accounts().expect("read migrated account");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].id, account);
    assert_eq!(accounts[0].email, "owner@example.com");
    assert!(!accounts[0].email_verified);
    assert!(accounts[0].is_instance_owner);
    for device_id in ["dev-0001", "dev-0005"] {
        let owner = state
            .identity()
            .device_owner(device_id)
            .expect("read migrated owner")
            .expect("device has an owner");
        assert_eq!(owner.account_id, account);
        assert_eq!(owner.state, DeviceState::Active);
        assert_eq!(state.account_space(&owner.account_id).account_id, account);
    }
}

#[test]
fn refuses_without_an_owner_email_and_touches_nothing() {
    let dir = copy_fixture("flat-layout");
    let before = snapshot_tree(dir.path());
    assert!(matches!(
        migrate_if_needed(dir.path(), None, migration_time()),
        Err(MigrationError::OwnerEmailRequired)
    ));
    assert_eq!(snapshot_tree(dir.path()), before);
}

#[test]
fn refuses_an_invalid_owner_email_and_touches_nothing() {
    let dir = copy_fixture("flat-layout");
    let before = snapshot_tree(dir.path());
    assert!(matches!(
        migrate_if_needed(dir.path(), Some("not an email"), migration_time()),
        Err(MigrationError::InvalidOwnerEmail)
    ));
    assert_eq!(snapshot_tree(dir.path()), before);
}

#[test]
fn refuses_a_malformed_registry_and_touches_nothing() {
    type RegistryMutation = fn(&mut serde_json::Value);
    let cases: [(&str, RegistryMutation); 6] = [
        ("schema", |registry| registry["schema_version"] = 2.into()),
        ("sequence", |registry| registry["next_sequence"] = 0.into()),
        ("device id", |registry| {
            registry["devices"][0]["device_id"] = "device-one".into();
        }),
        ("digest", |registry| {
            registry["devices"][0]["token_sha256"] = "NOT-A-DIGEST".into();
        }),
        ("duplicate id", |registry| {
            registry["devices"][1]["device_id"] = "dev-0001".into();
        }),
        ("duplicate digest", |registry| {
            registry["devices"][1]["token_sha256"] = registry["devices"][0]["token_sha256"].clone();
        }),
    ];

    for (case, mutate) in cases {
        let dir = copy_fixture("flat-layout");
        let registry_path = dir.path().join("device-identities.json");
        let mut registry: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&registry_path).unwrap()).unwrap();
        mutate(&mut registry);
        std::fs::write(
            &registry_path,
            serde_json::to_vec_pretty(&registry).unwrap(),
        )
        .unwrap();
        let before = snapshot_tree(dir.path());

        assert!(
            matches!(
                migrate_if_needed(dir.path(), Some("o@example.com"), migration_time()),
                Err(MigrationError::Io(_))
            ),
            "malformed {case}"
        );
        assert_eq!(snapshot_tree(dir.path()), before, "malformed {case}");
    }
}

#[test]
fn refuses_to_overwrite_a_legacy_archive_and_touches_nothing() {
    let dir = copy_fixture("flat-layout");
    let legacy = dir.path().join("legacy-20260923T123456Z");
    std::fs::create_dir(&legacy).unwrap();
    std::fs::write(legacy.join("sentinel"), b"keep me").unwrap();
    let before = snapshot_tree(dir.path());

    assert!(matches!(
        migrate_if_needed(dir.path(), Some("o@example.com"), migration_time()),
        Err(MigrationError::Io(_))
    ));
    assert_eq!(snapshot_tree(dir.path()), before);
}

#[test]
fn runs_once() {
    let dir = copy_fixture("flat-layout");
    migrate_if_needed(dir.path(), Some("o@example.com"), migration_time()).unwrap();
    assert!(matches!(
        migrate_if_needed(dir.path(), Some("o@example.com"), migration_time()).unwrap(),
        MigrationOutcome::NotNeeded
    ));
}

#[test]
fn a_crash_before_the_final_rename_migrates_again_from_untouched_originals() {
    let dir = copy_fixture("flat-layout");
    let before = snapshot_tree(dir.path());
    migrate::fail_before_rename_for_tests(dir.path(), "o@example.com");
    assert!(!dir.path().join("identity.db").exists());
    for (path, bytes) in &before {
        assert_eq!(
            &std::fs::read(dir.path().join(path)).unwrap(),
            bytes,
            "{}",
            path.display()
        );
    }
    assert!(matches!(
        migrate_if_needed(dir.path(), Some("o@example.com"), migration_time()).unwrap(),
        MigrationOutcome::Migrated { .. }
    ));
    assert_eq!(
        std::fs::read_dir(dir.path().join("accounts"))
            .expect("accounts directory")
            .count(),
        1,
        "retry removes the partial account produced before the simulated crash"
    );
}

#[test]
fn a_fresh_empty_directory_needs_no_migration() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        migrate_if_needed(dir.path(), None, migration_time()).unwrap(),
        MigrationOutcome::NotNeeded
    ));
}
