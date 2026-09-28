//! One-time migration from the original instance-wide flat configuration layout.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::credential::decode_digest;
use crate::identity::{AccountId, DeviceState, IdentityError, IdentityStore, normalize_email};

const IDENTITY_FILE: &str = "identity.db";
const MIGRATING_IDENTITY_FILE: &str = "identity.db.migrating";
const REGISTRY_FILE: &str = "device-identities.json";
const ACCOUNT_FILES: [&str; 3] = ["image-sources.json", "data-cards.json", "face-state.json"];
const INSTANCE_FILES: [&str; 3] = [REGISTRY_FILE, "producer-credentials.json", "secrets.enc"];

#[derive(Debug, PartialEq, Eq)]
pub enum MigrationOutcome {
    NotNeeded,
    Migrated {
        account: AccountId,
        devices: usize,
        legacy_dir: PathBuf,
    },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MigrationError {
    #[error("DESKMATE_OWNER_EMAIL must be set to migrate the existing flat configuration layout")]
    OwnerEmailRequired,
    #[error("DESKMATE_OWNER_EMAIL must be a valid email address")]
    InvalidOwnerEmail,
    #[error("configuration migration failed: {0}")]
    Io(String),
    #[error("configuration migration failed: {0}")]
    Identity(#[from] IdentityError),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FlatRegistry {
    schema_version: u32,
    next_sequence: u64,
    devices: Vec<FlatDevice>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FlatDevice {
    device_id: String,
    token_sha256: String,
}

struct StagedMigration {
    account: AccountId,
    devices: usize,
    flat_device_files: Vec<PathBuf>,
}

/// Migrates a pre-account configuration directory exactly once.
///
/// The old per-account files remain untouched until the replacement identity
/// database has been committed under its final name. A process failure before
/// that rename therefore leaves the original layout authoritative and safe to
/// retry on the next start.
pub fn migrate_if_needed(
    config_dir: &Path,
    owner_email: Option<&str>,
    now: DateTime<Utc>,
) -> Result<MigrationOutcome, MigrationError> {
    if config_dir.join(IDENTITY_FILE).exists() || !flat_layout_present(config_dir)? {
        return Ok(MigrationOutcome::NotNeeded);
    }

    let owner_email = owner_email.ok_or(MigrationError::OwnerEmailRequired)?;
    let owner_email = normalize_email(owner_email).ok_or(MigrationError::InvalidOwnerEmail)?;
    let legacy_dir = config_dir.join(format!("legacy-{}", now.format("%Y%m%dT%H%M%SZ")));
    if legacy_dir.exists() {
        return Err(MigrationError::Io(format!(
            "legacy directory already exists: {}",
            legacy_dir.display()
        )));
    }
    let staged = stage(config_dir, &owner_email, now)?;

    checkpoint_migrating_database(config_dir)?;
    fs::rename(
        config_dir.join(MIGRATING_IDENTITY_FILE),
        config_dir.join(IDENTITY_FILE),
    )
    .map_err(|error| io_error("install identity database", config_dir, &error))?;
    sync_directory(config_dir)?;

    fs::create_dir(&legacy_dir)
        .map_err(|error| io_error("create legacy archive", &legacy_dir, &error))?;
    for file_name in ACCOUNT_FILES {
        move_if_present(&config_dir.join(file_name), &legacy_dir.join(file_name))?;
    }
    move_if_present(
        &config_dir.join("image-frames"),
        &legacy_dir.join("image-frames"),
    )?;
    for source in &staged.flat_device_files {
        let file_name = source.file_name().ok_or_else(|| {
            MigrationError::Io(format!(
                "device config has no filename: {}",
                source.display()
            ))
        })?;
        move_if_present(source, &legacy_dir.join(file_name))?;
    }
    sync_directory(config_dir)?;

    tracing::warn!(
        target: "server",
        account_id = %staged.account,
        legacy_dir = %legacy_dir.display(),
        devices = staged.devices,
        "migrated the flat configuration layout into the owner's account"
    );
    Ok(MigrationOutcome::Migrated {
        account: staged.account,
        devices: staged.devices,
        legacy_dir,
    })
}

/// Stops after the retry-safe staging phase, simulating a process failure just
/// before the identity database's final rename.
#[doc(hidden)]
pub fn fail_before_rename_for_tests(config_dir: &Path, owner_email: &str) {
    let owner_email = normalize_email(owner_email).expect("test owner email is valid");
    stage(config_dir, &owner_email, Utc::now()).expect("migration staging succeeds");
}

fn stage(
    config_dir: &Path,
    owner_email: &str,
    now: DateTime<Utc>,
) -> Result<StagedMigration, MigrationError> {
    let flat_device_files = flat_device_configs(config_dir)?;
    let registry_devices = registry_device_ids(config_dir)?;

    fs::create_dir_all(config_dir)
        .map_err(|error| io_error("create configuration directory", config_dir, &error))?;
    remove_migrating_database(config_dir)?;

    // With no final identity database, this directory can only be output from
    // an interrupted migration. Rebuild it from the still-authoritative flat
    // originals so retries cannot accumulate orphan account folders.
    let accounts_root = config_dir.join("accounts");
    remove_directory_if_present(&accounts_root)?;

    let database_path = config_dir.join(MIGRATING_IDENTITY_FILE);
    let identity = IdentityStore::open(&database_path)?;
    let owner = identity.create_account(owner_email, false, true, now)?;
    for device_id in &registry_devices {
        identity.assign_device(device_id, &owner.id, DeviceState::Active, now)?;
    }
    drop(identity);

    let owner_root = accounts_root.join(&owner.id.0);
    fs::create_dir_all(owner_root.join("devices"))
        .map_err(|error| io_error("create migrated account directory", &owner_root, &error))?;
    for file_name in ACCOUNT_FILES {
        copy_file_if_present(&config_dir.join(file_name), &owner_root.join(file_name))?;
    }
    copy_directory_if_present(
        &config_dir.join("image-frames"),
        &owner_root.join("image-frames"),
    )?;
    for source in &flat_device_files {
        let file_name = source.file_name().ok_or_else(|| {
            MigrationError::Io(format!(
                "device config has no filename: {}",
                source.display()
            ))
        })?;
        copy_file(source, &owner_root.join("devices").join(file_name))?;
    }

    Ok(StagedMigration {
        account: owner.id,
        devices: registry_devices.len(),
        flat_device_files,
    })
}

fn flat_layout_present(config_dir: &Path) -> Result<bool, MigrationError> {
    for file_name in INSTANCE_FILES.into_iter().chain(ACCOUNT_FILES) {
        if config_dir
            .join(file_name)
            .try_exists()
            .map_err(|error| io_error("inspect flat configuration", config_dir, &error))?
        {
            return Ok(true);
        }
    }
    Ok(!flat_device_configs(config_dir)?.is_empty())
}

fn flat_device_configs(config_dir: &Path) -> Result<Vec<PathBuf>, MigrationError> {
    let entries = match fs::read_dir(config_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(io_error(
                "scan flat device configurations",
                config_dir,
                &error,
            ));
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry
            .map_err(|error| io_error("read flat device configuration", config_dir, &error))?;
        if !entry
            .file_type()
            .map_err(|error| io_error("inspect flat device configuration", &entry.path(), &error))?
            .is_file()
        {
            continue;
        }
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let path = Path::new(file_name);
        if path.extension() == Some(std::ffi::OsStr::new("json"))
            && path
                .file_stem()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|stem| stem.starts_with("dev-"))
        {
            paths.push(entry.path());
        }
    }
    paths.sort_unstable();
    Ok(paths)
}

fn registry_device_ids(config_dir: &Path) -> Result<Vec<String>, MigrationError> {
    let path = config_dir.join(REGISTRY_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_error("read device identity registry", &path, &error)),
    };
    let registry: FlatRegistry = serde_json::from_slice(&bytes)
        .map_err(|error| MigrationError::Io(format!("parse {}: {error}", path.display())))?;
    if registry.schema_version != 1 {
        return Err(MigrationError::Io(format!(
            "{} has unsupported schema version {}",
            path.display(),
            registry.schema_version
        )));
    }

    let mut ids = HashSet::with_capacity(registry.devices.len());
    let mut digests = HashSet::with_capacity(registry.devices.len());
    let mut greatest_sequence = 0;
    for device in registry.devices {
        let sequence = canonical_device_sequence(&device.device_id).ok_or_else(|| {
            MigrationError::Io(format!(
                "{} contains invalid device id {:?}",
                path.display(),
                device.device_id
            ))
        })?;
        let digest = decode_digest(&device.token_sha256).map_err(|_| {
            MigrationError::Io(format!(
                "{} contains an invalid token digest for {}",
                path.display(),
                device.device_id
            ))
        })?;
        if !ids.insert(device.device_id.clone()) || !digests.insert(digest) {
            return Err(MigrationError::Io(format!(
                "{} contains a duplicate device id or token digest",
                path.display()
            )));
        }
        greatest_sequence = greatest_sequence.max(sequence);
    }
    if registry.next_sequence < greatest_sequence {
        return Err(MigrationError::Io(format!(
            "{} has next_sequence below an existing device id",
            path.display()
        )));
    }
    let mut ids = ids.into_iter().collect::<Vec<_>>();
    ids.sort_unstable();
    Ok(ids)
}

fn canonical_device_sequence(device_id: &str) -> Option<u64> {
    let digits = device_id.strip_prefix("dev-")?;
    if digits.len() < 4 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let sequence = digits.parse::<u64>().ok()?;
    (sequence != 0 && format!("dev-{sequence:04}") == device_id).then_some(sequence)
}

fn checkpoint_migrating_database(config_dir: &Path) -> Result<(), MigrationError> {
    let path = config_dir.join(MIGRATING_IDENTITY_FILE);
    let connection = rusqlite::Connection::open(&path).map_err(|error| {
        MigrationError::Identity(IdentityError::Sqlite(format!(
            "open staged identity database: {error}"
        )))
    })?;
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
        .map_err(|error| {
            MigrationError::Identity(IdentityError::Sqlite(format!(
                "checkpoint staged identity database: {error}"
            )))
        })?;
    Ok(())
}

fn remove_migrating_database(config_dir: &Path) -> Result<(), MigrationError> {
    for file_name in [
        MIGRATING_IDENTITY_FILE.to_string(),
        format!("{MIGRATING_IDENTITY_FILE}-wal"),
        format!("{MIGRATING_IDENTITY_FILE}-shm"),
    ] {
        remove_file_if_present(&config_dir.join(file_name))?;
    }
    Ok(())
}

fn remove_file_if_present(path: &Path) -> Result<(), MigrationError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("remove stale migration file", path, &error)),
    }
}

fn remove_directory_if_present(path: &Path) -> Result<(), MigrationError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("remove stale migration directory", path, &error)),
    }
}

fn copy_file_if_present(source: &Path, destination: &Path) -> Result<(), MigrationError> {
    match source.try_exists() {
        Ok(true) => copy_file(source, destination),
        Ok(false) => Ok(()),
        Err(error) => Err(io_error("inspect migration source", source, &error)),
    }
}

fn copy_file(source: &Path, destination: &Path) -> Result<(), MigrationError> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| io_error("create migration destination", parent, &error))?;
    }
    fs::copy(source, destination)
        .map(|_| ())
        .map_err(|error| io_error("copy migration source", source, &error))
}

fn copy_directory_if_present(source: &Path, destination: &Path) -> Result<(), MigrationError> {
    let entries = match fs::read_dir(source) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error("read migration source directory", source, &error)),
    };
    fs::create_dir_all(destination)
        .map_err(|error| io_error("create migration destination", destination, &error))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| io_error("read migration source directory", source, &error))?;
        let target = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| io_error("inspect migration source", &entry.path(), &error))?
            .is_dir()
        {
            copy_directory_if_present(&entry.path(), &target)?;
        } else {
            copy_file(&entry.path(), &target)?;
        }
    }
    Ok(())
}

fn move_if_present(source: &Path, destination: &Path) -> Result<(), MigrationError> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("archive migrated source", source, &error)),
    }
}

fn sync_directory(path: &Path) -> Result<(), MigrationError> {
    app_core::secure_file::sync_parent(path).map_err(|error| {
        let (operation, source) = error.into_strings("migration directory");
        MigrationError::Io(format!("{operation} {}: {source}", path.display()))
    })
}

fn io_error(operation: &str, path: &Path, error: &io::Error) -> MigrationError {
    MigrationError::Io(format!("{operation} {}: {error}", path.display()))
}
