//! Device identity registry: mints one-time bearer tokens, persists only their
//! SHA-256 digests, and authenticates presented tokens in constant time.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use app_core::secure_file::{self, BoundedReadError, FileIoError, FileOperation};
use serde::{Deserialize, Serialize};

use crate::credential::{DigestDecodeError, decode_digest as decode_credential_digest};
use crate::credential::{random_token, token_digest};

pub(crate) const DEVICE_IDENTITY_STORE_FILE: &str = "device-identities.json";

const REGISTRY_SCHEMA_VERSION: u32 = 1;
const MAX_REGISTRY_FILE_BYTES: usize = 64 * 1_024;

/// A device's opaque identifier, e.g. `dev-0001`.
pub type DeviceId = String;

/// A freshly minted device identity: the id the server assigns internally and
/// the bearer token the device presents on every request. The token is
/// returned exactly once, at mint time; the server never logs it again, and
/// `Debug` never prints it either (see the hand-written impl below), so a
/// stray `tracing::info!(?identity)` can't leak a live bearer secret.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub device_id: String,
    pub token: String,
}

impl fmt::Debug for DeviceIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceIdentity")
            .field("device_id", &self.device_id)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// A failure to mint an identity. A persistent registry does not return the
/// one-time token unless its digest was atomically committed first.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RegistryError {
    #[error("{operation}: {message}")]
    Io {
        operation: &'static str,
        message: String,
    },
    #[error("invalid device identity store: {message}")]
    InvalidStore { message: String },
    #[error("device identity sequence is exhausted")]
    SequenceExhausted,
}

/// Digest-to-device records plus the last issued monotonic id. Authentication
/// scans the vector; it is deliberately not keyed by digest.
pub struct Registry {
    path: Option<PathBuf>,
    state: Mutex<RegistryState>,
    store_load_failed: bool,
}

struct RegistryState {
    next_sequence: u64,
    tokens: Vec<TokenRecord>,
    failed_store_pending_repair: bool,
}

struct TokenRecord {
    digest: [u8; 32],
    device_id: DeviceId,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedRegistry {
    schema_version: u32,
    /// Despite its name, this stores the last issued sequence; minting adds
    /// one. The persisted field name is fixed by the on-disk format.
    next_sequence: u64,
    devices: Vec<PersistedDevice>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedDevice {
    device_id: DeviceId,
    token_sha256: String,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            path: None,
            state: Mutex::new(RegistryState::empty()),
            store_load_failed: false,
        }
    }

    /// Loads a persistent registry. Missing state is a normal empty registry;
    /// unreadable or invalid state is diagnosed and also becomes empty so the
    /// server remains available for re-minting.
    #[must_use]
    pub(crate) fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let (state, store_load_failed) = match load_store(&path) {
            Ok(Some(state)) => (state, false),
            Ok(None) => (RegistryState::empty(), false),
            Err(error) => {
                tracing::warn!(
                    store_path = %path.display(),
                    %error,
                    "device identity store failed to load; starting with an empty registry; \
                     existing devices must be re-minted"
                );
                let recovered_sequence = match scan_config_high_water(&path) {
                    Ok(sequence) => sequence,
                    Err(scan_error) => {
                        tracing::warn!(
                            store_path = %path.display(),
                            error = %scan_error,
                            "device config high-water scan failed after identity-store load failure; \
                             minting will retry before assigning an id"
                        );
                        0
                    }
                };
                (RegistryState::after_failed_load(recovered_sequence), true)
            }
        };
        Self {
            path: Some(path),
            state: Mutex::new(state),
            store_load_failed,
        }
    }

    /// Mints a new device identity: a monotonic `dev-NNNN` id and a random
    /// 32-byte token rendered as 64 lowercase hex characters. Persistent
    /// registries commit the digest before releasing the plaintext token.
    pub fn mint(&self) -> Result<DeviceIdentity, RegistryError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(path) = &self.path
            && state.failed_store_pending_repair
        {
            state.next_sequence = state.next_sequence.max(scan_config_high_water(path)?);
            archive_failed_store(path)?;
        }

        let sequence = state
            .next_sequence
            .checked_add(1)
            .ok_or(RegistryError::SequenceExhausted)?;
        let device_id = format!("dev-{sequence:04}");
        let token = random_token();
        let record = TokenRecord {
            digest: token_digest(&token),
            device_id: device_id.clone(),
        };

        if let Some(path) = &self.path {
            save_store(path, &state, sequence, &record)?;
            state.failed_store_pending_repair = false;
        }

        state.next_sequence = sequence;
        state.tokens.push(record);
        Ok(DeviceIdentity { device_id, token })
    }

    /// Authenticates a presented bearer token against every minted token.
    ///
    /// A *match* short-circuits via `Iterator::find`, same as any lookup --
    /// that's fine, since which token matched is exactly the information
    /// being asked for. The property this guards is about *rejection*: a
    /// wrong token is compared against the entire table with
    /// [`constant_time_eq`] rather than `==`, so how many of its leading
    /// bytes happen to match some real token never shows up as a timing
    /// difference. At the small device counts this registry is designed for,
    /// scanning the whole table on a miss is cheap.
    pub fn authenticate(&self, token: &str) -> Option<DeviceId> {
        let presented_digest = token_digest(token);
        // A panic while some other request held the lock must not turn every
        // *subsequent* authentication into a panic too -- that would be a
        // latent, self-inflicted denial of service on an internet-facing
        // endpoint. The registry holds no invariant a panic mid-mutation
        // could leave inconsistent (it's an insert or a scan, nothing more),
        // so recovering the poisoned guard and carrying on is safe.
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .tokens
            .iter()
            .find(|candidate| constant_time_eq(&candidate.digest, &presented_digest))
            .map(|candidate| candidate.device_id.clone())
    }

    /// Every minted device id, sorted. Ids only -- a digest never leaves this
    /// type, and the management surface reports presence, never credentials.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn device_ids(&self) -> Vec<String> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut ids: Vec<String> = state
            .tokens
            .iter()
            .map(|record| record.device_id.clone())
            .collect();
        ids.sort_unstable();
        ids
    }

    /// Whether `device_id` belongs to a loaded or newly minted identity.
    /// Admin paths use this check before deriving a per-device config path,
    /// so an arbitrary URL segment never reaches the filesystem.
    pub fn contains_device(&self, device_id: &str) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .tokens
            .iter()
            .any(|candidate| candidate.device_id == device_id)
    }

    /// Revokes a device identity. Persistent registries commit the removal
    /// before changing the in-memory authentication table.
    pub fn revoke(&self, device_id: &str) -> Result<bool, RegistryError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(index) = state
            .tokens
            .iter()
            .position(|record| record.device_id == device_id)
        else {
            return Ok(false);
        };
        let mut tokens = state
            .tokens
            .iter()
            .map(|record| TokenRecord {
                digest: record.digest,
                device_id: record.device_id.clone(),
            })
            .collect::<Vec<_>>();
        tokens.remove(index);
        if let Some(path) = &self.path {
            save_records(path, state.next_sequence, &tokens)?;
        }
        state.tokens = tokens;
        Ok(true)
    }

    /// Whether this process discarded an unreadable or invalid store at boot.
    /// The auth layer uses this only to choose static diagnostic wording; no
    /// token or digest is ever included in the warning.
    #[must_use]
    pub const fn store_load_failed(&self) -> bool {
        self.store_load_failed
    }
}

impl RegistryState {
    const fn empty() -> Self {
        Self {
            next_sequence: 0,
            tokens: Vec::new(),
            failed_store_pending_repair: false,
        }
    }

    const fn after_failed_load(next_sequence: u64) -> Self {
        Self {
            next_sequence,
            tokens: Vec::new(),
            failed_store_pending_repair: true,
        }
    }
}

fn load_store(path: &Path) -> Result<Option<RegistryState>, RegistryError> {
    let Some(bytes) =
        secure_file::read_bounded(path, MAX_REGISTRY_FILE_BYTES).map_err(bounded_read_error)?
    else {
        return Ok(None);
    };
    decode_store(&bytes).map(Some)
}

fn decode_store(bytes: &[u8]) -> Result<RegistryState, RegistryError> {
    let persisted: PersistedRegistry = serde_json::from_slice(bytes)
        .map_err(|error| invalid_store(format!("invalid JSON: {error}")))?;
    if persisted.schema_version != REGISTRY_SCHEMA_VERSION {
        return Err(invalid_store(format!(
            "unsupported schema version {}; expected {REGISTRY_SCHEMA_VERSION}",
            persisted.schema_version
        )));
    }

    let mut tokens: Vec<TokenRecord> = Vec::with_capacity(persisted.devices.len());
    let mut greatest_sequence = 0;
    for device in persisted.devices {
        let sequence = parse_device_sequence(&device.device_id)?;
        greatest_sequence = greatest_sequence.max(sequence);
        let digest = decode_digest(&device.token_sha256)?;
        if tokens
            .iter()
            .any(|candidate| candidate.device_id == device.device_id || candidate.digest == digest)
        {
            return Err(invalid_store("duplicate device id or token digest"));
        }
        tokens.push(TokenRecord {
            digest,
            device_id: device.device_id,
        });
    }
    if persisted.next_sequence < greatest_sequence {
        return Err(invalid_store(
            "next_sequence is lower than an existing device id",
        ));
    }

    Ok(RegistryState {
        next_sequence: persisted.next_sequence,
        tokens,
        failed_store_pending_repair: false,
    })
}

/// Finds the highest canonical `dev-NNNN.json` config filename. After a
/// failed identity-store load, these files are the surviving authority for ids
/// that must not be reused, even though their old bearer tokens are discarded.
fn scan_config_high_water(store_path: &Path) -> Result<u64, RegistryError> {
    let parent = usable_parent(store_path)?;
    let mut high_water = scan_device_config_directory(parent)?;
    let accounts = parent.join("accounts");
    let account_entries = match fs::read_dir(&accounts) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(high_water),
        Err(error) => return Err(io_error("scan account config ids", &error)),
    };
    for entry in account_entries {
        let entry = entry.map_err(|error| io_error("read account config id", &error))?;
        if !entry
            .file_type()
            .map_err(|error| io_error("inspect account config directory", &error))?
            .is_dir()
        {
            continue;
        }
        let devices = entry.path().join("devices");
        high_water = high_water.max(scan_device_config_directory(&devices)?);
    }
    Ok(high_water)
}

fn scan_device_config_directory(directory: &Path) -> Result<u64, RegistryError> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(io_error("scan device config ids", &error)),
    };
    let mut high_water = 0;
    for entry in entries {
        let entry = entry.map_err(|error| io_error("read device config id", &error))?;
        let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(device_id) = file_name.strip_suffix(".json") else {
            continue;
        };
        if let Ok(sequence) = parse_device_sequence(device_id) {
            high_water = high_water.max(sequence);
        }
    }
    Ok(high_water)
}

/// Moves failed input out of the canonical path before the first replacement
/// commit. An absent source is harmless (an operator may remove it after the
/// failed load); every other metadata or rename failure blocks minting so the
/// original cannot be overwritten.
fn archive_failed_store(path: &Path) -> Result<(), RegistryError> {
    let archived = archive_failed_store_with(path, |from, to| fs::rename(from, to))?;
    if let Some(archived) = archived {
        tracing::warn!(
            store_path = %path.display(),
            archived_path = %archived.display(),
            "failed device identity store archived before replacement"
        );
        if let Err(error) = secure_file::sync_parent(usable_parent(path)?) {
            tracing::warn!(
                store_path = %path.display(),
                error = %error.source,
                "failed identity-store archive was renamed but its directory could not be synced"
            );
        }
    }
    Ok(())
}

fn archive_failed_store_with(
    path: &Path,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<Option<PathBuf>, RegistryError> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("inspect failed device identity store", &error)),
    }

    let parent = usable_parent(path)?;
    let file_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| invalid_store("device identity store has no UTF-8 filename"))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut archive = None;
    for suffix in 0..1_000 {
        let suffix = if suffix == 0 {
            String::new()
        } else {
            format!("-{suffix}")
        };
        let candidate = parent.join(format!("{file_name}.corrupt-{timestamp}{suffix}"));
        match fs::symlink_metadata(&candidate) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                archive = Some(candidate);
                break;
            }
            Err(error) => {
                return Err(io_error("inspect device identity store archive", &error));
            }
        }
    }
    let archive = archive.ok_or_else(|| invalid_store("no unused archive filename available"))?;
    rename(path, &archive)
        .map_err(|error| io_error("archive failed device identity store", &error))?;
    Ok(Some(archive))
}

fn save_store(
    path: &Path,
    state: &RegistryState,
    next_sequence: u64,
    new_record: &TokenRecord,
) -> Result<(), RegistryError> {
    let records = state
        .tokens
        .iter()
        .map(|record| TokenRecord {
            digest: record.digest,
            device_id: record.device_id.clone(),
        })
        .chain(std::iter::once(TokenRecord {
            digest: new_record.digest,
            device_id: new_record.device_id.clone(),
        }))
        .collect::<Vec<_>>();
    save_records(path, next_sequence, &records)
}

fn save_records(
    path: &Path,
    next_sequence: u64,
    records: &[TokenRecord],
) -> Result<(), RegistryError> {
    let parent = usable_parent(path)?;
    secure_file::create_directory(parent).map_err(file_io_error)?;

    let devices = records
        .iter()
        .map(|record| PersistedDevice {
            device_id: record.device_id.clone(),
            token_sha256: protocol::digest_hex(&record.digest),
        })
        .collect();
    let persisted = PersistedRegistry {
        schema_version: REGISTRY_SCHEMA_VERSION,
        next_sequence,
        devices,
    };
    let bytes = serde_json::to_vec_pretty(&persisted)
        .map_err(|error| invalid_store(format!("failed to encode JSON: {error}")))?;
    if bytes.len() > MAX_REGISTRY_FILE_BYTES {
        return Err(invalid_store(format!(
            "encoded file exceeds the {MAX_REGISTRY_FILE_BYTES}-byte limit"
        )));
    }

    secure_file::write_and_replace(path, &bytes).map_err(file_io_error)?;

    if let Err(error) = secure_file::sync_parent(parent) {
        tracing::warn!(
            store_path = %path.display(),
            error = %error.source,
            "device identity store was replaced but its directory could not be synced"
        );
    }
    Ok(())
}

fn parse_device_sequence(device_id: &str) -> Result<u64, RegistryError> {
    let Some(digits) = device_id.strip_prefix("dev-") else {
        return Err(invalid_store("device id does not start with dev-"));
    };
    if digits.len() < 4 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid_store("device id has an invalid sequence"));
    }
    let sequence = digits
        .parse::<u64>()
        .map_err(|_| invalid_store("device id sequence is out of range"))?;
    if sequence == 0 || format!("dev-{sequence:04}") != device_id {
        return Err(invalid_store("device id is not canonical"));
    }
    Ok(sequence)
}

fn decode_digest(hex: &str) -> Result<[u8; 32], RegistryError> {
    match decode_credential_digest(hex) {
        Ok(digest) => Ok(digest),
        Err(DigestDecodeError::Length) => {
            Err(invalid_store("token digest is not 64 lowercase hex bytes"))
        }
        Err(DigestDecodeError::Character) => {
            Err(invalid_store("token digest is not lowercase hex"))
        }
    }
}

fn usable_parent(path: &Path) -> Result<&Path, RegistryError> {
    secure_file::usable_parent(path)
        .ok_or_else(|| invalid_store(format!("{} has no parent directory", path.display())))
}

fn io_error(operation: &'static str, error: &io::Error) -> RegistryError {
    RegistryError::Io {
        operation,
        message: error.to_string(),
    }
}

fn bounded_read_error(error: BoundedReadError) -> RegistryError {
    match error {
        BoundedReadError::Io(error) => file_io_error(error),
        BoundedReadError::TooLarge { maximum } => {
            invalid_store(format!("file exceeds the {maximum}-byte limit"))
        }
    }
}

fn file_io_error(error: FileIoError) -> RegistryError {
    let FileIoError { operation, source } = error;
    io_error(registry_file_operation(operation), &source)
}

const fn registry_file_operation(operation: FileOperation) -> &'static str {
    match operation {
        FileOperation::CreateDirectory => "create device identity store directory",
        FileOperation::Open => "open device identity store",
        FileOperation::Read => "read device identity store",
        FileOperation::CreateTemporary => "create temporary device identity store",
        FileOperation::WriteTemporary => "write temporary device identity store",
        FileOperation::FinishTemporary => "finish temporary device identity store",
        FileOperation::SyncAndReplace => "sync and replace device identity store",
        FileOperation::SyncDirectory => "sync device identity store directory",
    }
}

fn invalid_store(message: impl Into<String>) -> RegistryError {
    RegistryError::InvalidStore {
        message: message.into(),
    }
}

/// Compares two byte strings without short-circuiting on the first mismatch,
/// so the comparison's timing does not leak how many leading bytes agree.
///
/// Lengths are checked up front: our tokens are always the same fixed
/// length, so a length mismatch only tells an attacker their guess had the
/// wrong shape, never anything about the secret's content. Shared by device
/// token authentication ([`Registry::authenticate`]) and, via
/// `ServerState::verify_admin_token`, the single admin token -- the crate
/// has exactly one way to compare a presented secret against a real one.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {

    #[test]
    fn device_ids_lists_every_minted_identity_and_no_digest() {
        let registry = Registry::new();
        let first = registry.mint().expect("mint");
        let second = registry.mint().expect("mint");

        let ids = registry.device_ids();

        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&first.device_id));
        assert!(ids.contains(&second.device_id));
        // The listing exists for a dashboard, so assert what it must NOT carry.
        let joined = ids.join(" ");
        assert!(!joined.contains(&first.token));
        assert!(!joined.contains(&second.token));
    }
    use super::*;

    #[test]
    fn mint_assigns_monotonic_ids() {
        let registry = Registry::new();
        let first = registry.mint().expect("first mint");
        let second = registry.mint().expect("second mint");
        assert_eq!(first.device_id, "dev-0001");
        assert_eq!(second.device_id, "dev-0002");
        assert_ne!(first.token, second.token);
    }

    #[test]
    fn authenticate_finds_a_minted_token() {
        let registry = Registry::new();
        let identity = registry.mint().expect("mint identity");
        assert_eq!(
            registry.authenticate(&identity.token),
            Some(identity.device_id)
        );
    }

    #[test]
    fn authenticate_rejects_an_unknown_token() {
        let registry = Registry::new();
        registry.mint().expect("mint identity");
        assert_eq!(registry.authenticate("not-a-real-token"), None);
    }

    #[test]
    fn constant_time_eq_matches_equal_bytes() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn debug_redacts_the_token() {
        let identity = DeviceIdentity {
            device_id: "dev-0001".to_string(),
            token: "super-secret-value".to_string(),
        };
        let rendered = format!("{identity:?}");
        assert!(!rendered.contains("super-secret-value"));
        assert!(rendered.contains("dev-0001"));
    }

    #[test]
    fn secure_file_operations_keep_registry_error_wording() {
        assert_eq!(
            registry_file_operation(FileOperation::CreateDirectory),
            "create device identity store directory"
        );
        assert_eq!(
            registry_file_operation(FileOperation::Open),
            "open device identity store"
        );
        assert_eq!(
            registry_file_operation(FileOperation::Read),
            "read device identity store"
        );
        assert_eq!(
            registry_file_operation(FileOperation::CreateTemporary),
            "create temporary device identity store"
        );
        assert_eq!(
            registry_file_operation(FileOperation::WriteTemporary),
            "write temporary device identity store"
        );
        assert_eq!(
            registry_file_operation(FileOperation::FinishTemporary),
            "finish temporary device identity store"
        );
        assert_eq!(
            registry_file_operation(FileOperation::SyncAndReplace),
            "sync and replace device identity store"
        );
        assert_eq!(
            registry_file_operation(FileOperation::SyncDirectory),
            "sync device identity store directory"
        );
    }

    #[test]
    fn failed_archive_rename_is_reported_without_touching_the_original() {
        // Catches swallowing a rename error in the discard path: mint must be
        // able to stop before replacement while the original bytes remain.
        let temp = tempfile::tempdir().expect("archive test temp dir");
        let store_path = temp.path().join(DEVICE_IDENTITY_STORE_FILE);
        let original = b"still potentially recoverable";
        fs::write(&store_path, original).expect("write failed store");

        let error = archive_failed_store_with(&store_path, |_, _| {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
        })
        .expect_err("archive rename failure was ignored");

        assert!(matches!(
            error,
            RegistryError::Io {
                operation: "archive failed device identity store",
                ..
            }
        ));
        assert_eq!(fs::read(&store_path).expect("read failed store"), original);
        assert!(
            fs::read_dir(temp.path())
                .expect("read archive test dir")
                .all(|entry| entry.expect("read archive entry").path() == store_path)
        );
    }

    #[test]
    fn revoke_removes_the_identity_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DEVICE_IDENTITY_STORE_FILE);
        let registry = Registry::load(&path);
        let minted = registry.mint().unwrap();
        assert!(registry.revoke(&minted.device_id).unwrap());
        assert!(registry.authenticate(&minted.token).is_none());
        assert!(!Registry::load(&path).contains_device(&minted.device_id));
        assert!(
            !registry.revoke(&minted.device_id).unwrap(),
            "second revoke is a no-op"
        );
    }

    #[test]
    fn high_water_scan_sees_per_account_device_files() {
        let dir = tempfile::tempdir().unwrap();
        let devices = dir.path().join("accounts/acc_x/devices");
        std::fs::create_dir_all(&devices).unwrap();
        std::fs::write(devices.join("dev-0042.json"), b"{}").unwrap();
        std::fs::write(dir.path().join(DEVICE_IDENTITY_STORE_FILE), b"not json").unwrap();
        let registry = Registry::load(dir.path().join(DEVICE_IDENTITY_STORE_FILE));
        assert_eq!(registry.mint().unwrap().device_id, "dev-0043");
    }
}
