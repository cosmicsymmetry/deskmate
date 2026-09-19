//! Encrypted-at-rest storage for per-integration OAuth secrets. Server-crate
//! only; never depended on by `app-core`.

use app_core::secure_file;
use base64::prelude::{BASE64_STANDARD, Engine as _};
use chacha20poly1305::aead::{Aead, KeyInit, OsRng, Payload};
use chacha20poly1305::{AeadCore, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use zeroize::ZeroizeOnDrop;

/// Environment variable naming a `0o600` keyfile that holds the base64 master key.
const ENV_KEY_FILE: &str = "DESKMATE_SECRETS_KEY_FILE";
/// Fallback environment variable carrying the base64 master key inline. Documented
/// as second-choice: env values are visible in `ps`/`/proc`/`systemctl show` in a
/// way a keyfile is not.
const ENV_KEY: &str = "DESKMATE_SECRETS_KEY";

const MAX_KEYFILE_BYTES: usize = 1_024;

/// Format magic + version for `secrets.enc`. The `1` is the format version; a
/// future rotation bumps it and `open` learns to accept both. It is also the
/// AEAD associated data, so flipping it fails authentication as well as the
/// prefix check.
const SECRETS_MAGIC: &[u8; 4] = b"DMS1";
const NONCE_LEN: usize = 24;
/// Generous ceiling: integration secrets are a small JSON map. Bounds a hostile
/// or corrupt file before it is base64-decoded.
const MAX_SECRETS_FILE_BYTES: usize = 262_144;

/// The at-rest secrets file, kept next to `device-identities.json` in the config
/// directory. The key that decrypts it lives outside the encrypted store.
pub const SECRETS_STORE_FILE: &str = "secrets.enc";

/// A 32-byte master key, held in memory only, wiped on drop. No `Clone`: the key
/// is moved into the one `IntegrationStore` that owns it.
#[derive(ZeroizeOnDrop)]
pub struct SecretsKey {
    bytes: [u8; 32],
}

impl SecretsKey {
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    pub(crate) fn bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
}

impl std::fmt::Debug for SecretsKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretsKey(redacted)")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
    #[error("failed to {operation}: {detail}")]
    Io { operation: String, detail: String },
    #[error("secrets file exceeds {maximum} bytes")]
    TooLarge { maximum: usize },
    #[error("secrets file is not a recognized Deskmate secrets envelope")]
    BadFormat,
    #[error("secrets could not be decrypted: wrong key or tampered file")]
    Decrypt,
    #[error("secrets could not be encrypted")]
    Encrypt,
    #[error("secrets could not be (de)serialized: {0}")]
    Serialize(String),
}

/// One integration's stored credentials. `deny_unknown_fields` so a format drift
/// is a loud decode error, not a silent dropped field.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationSecret {
    /// The identity provider, e.g. `"google"`.
    pub provider: String,
    /// The long-lived OAuth refresh token.
    pub refresh_token: String,
    /// The OAuth client secret, when the deployment stores it here rather than in a
    /// separate keyfile. `None` when supplied out-of-band.
    pub client_secret: Option<String>,
    /// The scopes granted at authorization time.
    pub scopes: Vec<String>,
    /// Unix seconds when these credentials were obtained.
    pub obtained_at: i64,
}

impl std::fmt::Debug for IntegrationSecret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let client_secret = self.client_secret.as_ref().map(|_| "<redacted>");
        formatter
            .debug_struct("IntegrationSecret")
            .field("provider", &self.provider)
            .field("refresh_token", &"<redacted>")
            .field("client_secret", &client_secret)
            .field("scopes", &self.scopes)
            .field("obtained_at", &self.obtained_at)
            .finish()
    }
}

/// Encrypted-at-rest map of `integration_id -> IntegrationSecret`. Cheap to share
/// behind an `Arc`; every mutation persists the whole sealed file atomically.
pub struct IntegrationStore {
    path: PathBuf,
    key: SecretsKey,
    secrets: Mutex<BTreeMap<String, IntegrationSecret>>,
    #[cfg(test)]
    persistence_fault: Option<PersistenceFault>,
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum PersistenceFault {
    BeforeReplacement,
    ParentSync,
}

impl IntegrationStore {
    /// Loads and decrypts an existing `secrets.enc`, or starts empty if none exists.
    /// A present-but-undecryptable file is a hard error (fail closed) rather than a
    /// silent reset that would discard the owner's integrations.
    pub fn open(path: PathBuf, key: SecretsKey) -> Result<Self, SecretsError> {
        let secrets = match secure_file::read_bounded(&path, MAX_SECRETS_FILE_BYTES) {
            Ok(Some(file_bytes)) => {
                let plaintext = open(&key, &file_bytes)?;
                serde_json::from_slice(&plaintext)
                    .map_err(|error| SecretsError::Serialize(error.to_string()))?
            }
            Ok(None) => BTreeMap::new(),
            Err(secure_file::BoundedReadError::TooLarge { maximum }) => {
                return Err(SecretsError::TooLarge { maximum });
            }
            Err(secure_file::BoundedReadError::Io(error)) => {
                let (operation, detail) = error.into_strings("secrets file");
                return Err(SecretsError::Io { operation, detail });
            }
        };
        Ok(Self {
            path,
            key,
            secrets: Mutex::new(secrets),
            #[cfg(test)]
            persistence_fault: None,
        })
    }

    pub fn get(&self, integration_id: &str) -> Option<IntegrationSecret> {
        self.lock().get(integration_id).cloned()
    }

    pub fn put(
        &self,
        integration_id: String,
        secret: IntegrationSecret,
    ) -> Result<(), SecretsError> {
        let mut secrets = self.lock();
        let mut candidate = secrets.clone();
        candidate.insert(integration_id, secret);
        self.commit_candidate(&mut secrets, candidate)
    }

    /// Removes an integration's secret. Returns whether one was present.
    pub fn remove(&self, integration_id: &str) -> Result<bool, SecretsError> {
        let mut secrets = self.lock();
        if !secrets.contains_key(integration_id) {
            return Ok(false);
        }
        let mut candidate = secrets.clone();
        candidate.remove(integration_id);
        self.commit_candidate(&mut secrets, candidate)?;
        Ok(true)
    }

    /// The integration ids present, sorted. Carries presence only, never secret
    /// values.
    pub fn integration_ids(&self) -> Vec<String> {
        self.lock().keys().cloned().collect()
    }

    fn commit_candidate(
        &self,
        live: &mut BTreeMap<String, IntegrationSecret>,
        candidate: BTreeMap<String, IntegrationSecret>,
    ) -> Result<(), SecretsError> {
        self.replace(&candidate)?;
        *live = candidate;
        self.sync_parent()
    }

    fn replace(&self, candidate: &BTreeMap<String, IntegrationSecret>) -> Result<(), SecretsError> {
        let plaintext = serde_json::to_vec(candidate)
            .map_err(|error| SecretsError::Serialize(error.to_string()))?;
        let sealed = seal(&self.key, &plaintext)?;

        if let Some(parent) = secure_file::usable_parent(&self.path) {
            secure_file::create_directory(parent).map_err(|error| {
                let (operation, detail) = error.into_strings("secrets directory");
                SecretsError::Io { operation, detail }
            })?;
        }
        #[cfg(test)]
        if matches!(
            self.persistence_fault,
            Some(PersistenceFault::BeforeReplacement)
        ) {
            return Err(SecretsError::Io {
                operation: "replace secrets file".to_string(),
                detail: "injected failure before replacement".to_string(),
            });
        }
        secure_file::write_and_replace(&self.path, &sealed).map_err(|error| {
            let (operation, detail) = error.into_strings("secrets file");
            SecretsError::Io { operation, detail }
        })?;
        Ok(())
    }

    fn sync_parent(&self) -> Result<(), SecretsError> {
        if let Some(parent) = secure_file::usable_parent(&self.path) {
            #[cfg(test)]
            if matches!(self.persistence_fault, Some(PersistenceFault::ParentSync)) {
                return Err(SecretsError::Io {
                    operation: "sync secrets directory".to_string(),
                    detail: "injected parent sync failure".to_string(),
                });
            }
            secure_file::sync_parent(parent).map_err(|error| {
                let (operation, detail) = error.into_strings("secrets directory");
                SecretsError::Io { operation, detail }
            })?;
        }
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, IntegrationSecret>> {
        self.secrets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error(
        "no secrets key configured: set {ENV_KEY_FILE} to a 0600 keyfile (preferred) \
         or {ENV_KEY} to the base64 key"
    )]
    NotConfigured,
    #[error("secrets keyfile not found: {0}")]
    KeyfileNotFound(PathBuf),
    #[error(
        "secrets keyfile {keyfile} is inside the config directory {config_dir}; the key \
         must live outside it so a config backup does not carry the key"
    )]
    KeyfileInsideConfigDir {
        keyfile: PathBuf,
        config_dir: PathBuf,
    },
    #[error("secrets keyfile is group/other-accessible (mode {mode:#o}); must be 0600")]
    KeyfileWorldReadable { mode: u32 },
    #[error("secrets key is not valid base64")]
    InvalidBase64,
    #[error("secrets key must decode to 32 bytes, got {got}")]
    WrongLength { got: usize },
    #[error("failed to {operation}: {detail}")]
    Io { operation: String, detail: String },
}

/// Encrypts `plaintext` into a single base64 line: `MAGIC || nonce || ciphertext`.
/// The returned bytes carry no trailing newline; `write_and_replace` adds one.
fn seal(key: &SecretsKey, plaintext: &[u8]) -> Result<Vec<u8>, SecretsError> {
    let cipher = XChaCha20Poly1305::new_from_slice(key.bytes())
        .expect("SecretsKey is 32 bytes by construction");
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: SECRETS_MAGIC,
            },
        )
        .map_err(|_| SecretsError::Encrypt)?;

    let mut buffer = Vec::with_capacity(SECRETS_MAGIC.len() + NONCE_LEN + ciphertext.len());
    buffer.extend_from_slice(SECRETS_MAGIC);
    buffer.extend_from_slice(nonce.as_slice());
    buffer.extend_from_slice(&ciphertext);
    Ok(BASE64_STANDARD.encode(&buffer).into_bytes())
}

/// Inverse of [`seal`]. Trims trailing ASCII whitespace (the persisted newline),
/// base64-decodes, checks the magic prefix, then authenticates and decrypts.
fn open(key: &SecretsKey, file_bytes: &[u8]) -> Result<Vec<u8>, SecretsError> {
    let trimmed = file_bytes.trim_ascii_end();
    let raw = BASE64_STANDARD
        .decode(trimmed)
        .map_err(|_| SecretsError::BadFormat)?;

    let header = SECRETS_MAGIC.len() + NONCE_LEN;
    if raw.len() < header || &raw[..SECRETS_MAGIC.len()] != SECRETS_MAGIC {
        return Err(SecretsError::BadFormat);
    }
    let nonce = XNonce::from_slice(&raw[SECRETS_MAGIC.len()..header]);
    let ciphertext = &raw[header..];

    let cipher = XChaCha20Poly1305::new_from_slice(key.bytes())
        .expect("SecretsKey is 32 bytes by construction");
    cipher
        .decrypt(
            nonce,
            Payload {
                msg: ciphertext,
                aad: SECRETS_MAGIC,
            },
        )
        .map_err(|_| SecretsError::Decrypt)
}

impl SecretsKey {
    /// Decodes a base64 string into a 32-byte key.
    fn from_base64(value: &str) -> Result<Self, KeyError> {
        let decoded = BASE64_STANDARD
            .decode(value.trim())
            .map_err(|_| KeyError::InvalidBase64)?;
        let bytes: [u8; 32] = decoded
            .as_slice()
            .try_into()
            .map_err(|_| KeyError::WrongLength { got: decoded.len() })?;
        Ok(Self::from_bytes(bytes))
    }

    /// Reads a base64 key from a keyfile after checking it is (a) not inside the
    /// config directory and (b) `0o600`.
    fn from_keyfile(config_dir: &Path, keyfile: &Path) -> Result<Self, KeyError> {
        let canonical_keyfile = std::fs::canonicalize(keyfile)
            .map_err(|_| KeyError::KeyfileNotFound(keyfile.to_path_buf()))?;

        // Containment check on canonical paths, so a symlink or `..` cannot smuggle
        // the key back inside the config directory.
        if let Ok(canonical_config) = std::fs::canonicalize(config_dir)
            && canonical_keyfile.starts_with(&canonical_config)
        {
            return Err(KeyError::KeyfileInsideConfigDir {
                keyfile: canonical_keyfile,
                config_dir: canonical_config,
            });
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(&canonical_keyfile).map_err(|error| KeyError::Io {
                operation: "stat the secrets keyfile".to_string(),
                detail: error.to_string(),
            })?;
            let mode = metadata.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(KeyError::KeyfileWorldReadable { mode });
            }
        }

        let file = std::fs::File::open(&canonical_keyfile).map_err(|error| KeyError::Io {
            operation: "read the secrets keyfile".to_string(),
            detail: error.to_string(),
        })?;
        let limit = u64::try_from(MAX_KEYFILE_BYTES + 1).expect("keyfile limit fits in u64");
        let mut bytes = Vec::with_capacity(MAX_KEYFILE_BYTES + 1);
        file.take(limit)
            .read_to_end(&mut bytes)
            .map_err(|error| KeyError::Io {
                operation: "read the secrets keyfile".to_string(),
                detail: error.to_string(),
            })?;
        if bytes.len() > MAX_KEYFILE_BYTES {
            return Err(KeyError::Io {
                operation: "read the secrets keyfile".to_string(),
                detail: format!("keyfile exceeds {MAX_KEYFILE_BYTES} bytes"),
            });
        }
        let text = String::from_utf8(bytes).map_err(|_| KeyError::InvalidBase64)?;
        Self::from_base64(&text)
    }
}

/// Resolves the master key from injected sources: keyfile wins over inline base64;
/// neither present is [`KeyError::NotConfigured`].
fn acquire_key(
    config_dir: &Path,
    keyfile: Option<&Path>,
    inline_base64: Option<&str>,
) -> Result<SecretsKey, KeyError> {
    if let Some(path) = keyfile {
        return SecretsKey::from_keyfile(config_dir, path);
    }
    if let Some(value) = inline_base64 {
        return SecretsKey::from_base64(value);
    }
    Err(KeyError::NotConfigured)
}

/// Reads [`ENV_KEY_FILE`]/[`ENV_KEY`] and delegates to [`acquire_key`]. The env is
/// read only here so the resolution logic stays pure and test-parallel-safe.
fn acquire_key_from_env(config_dir: &Path) -> Result<SecretsKey, KeyError> {
    let keyfile = std::env::var_os(ENV_KEY_FILE).map(PathBuf::from);
    let inline = std::env::var(ENV_KEY).ok();
    acquire_key(config_dir, keyfile.as_deref(), inline.as_deref())
}

#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error(
        "a secrets key is required: set {ENV_KEY_FILE} (preferred) or {ENV_KEY} before starting"
    )]
    KeyRequired,
    #[error(
        "{SECRETS_STORE_FILE} exists but no secrets key is configured; refusing to start. \
         Set {ENV_KEY_FILE} to the keyfile that decrypts it (spec fail-closed rule)"
    )]
    KeyRequiredForExistingSecrets,
    #[error("secrets key error: {0}")]
    Key(#[source] KeyError),
    #[error("secrets store error: {0}")]
    Store(#[source] SecretsError),
}

/// Opens the integration store from an injected key result, applying the
/// fail-closed startup rule. A missing key when a `secrets.enc`
/// already exists is the emphatic refusal; a missing key with no file yet is
/// still an error, because integrations cannot function without one and a
/// "works until you connect an account" failure is worse.
fn open_integration_store_with(
    config_dir: &Path,
    key: Result<SecretsKey, KeyError>,
) -> Result<IntegrationStore, StartupError> {
    let secrets_path = config_dir.join(SECRETS_STORE_FILE);
    let key = match key {
        Ok(key) => key,
        Err(KeyError::NotConfigured) => {
            return Err(if secrets_path.exists() {
                StartupError::KeyRequiredForExistingSecrets
            } else {
                StartupError::KeyRequired
            });
        }
        Err(other) => return Err(StartupError::Key(other)),
    };
    IntegrationStore::open(secrets_path, key).map_err(StartupError::Store)
}

/// Production entry point: reads the key from the environment, then applies
/// [`open_integration_store_with`].
pub fn open_integration_store(config_dir: &Path) -> Result<IntegrationStore, StartupError> {
    open_integration_store_with(config_dir, acquire_key_from_env(config_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn valid_key_base64() -> String {
        use base64::prelude::{BASE64_STANDARD, Engine as _};
        BASE64_STANDARD.encode([42u8; 32])
    }

    #[test]
    fn from_base64_accepts_thirty_two_bytes() {
        let key = SecretsKey::from_base64(&valid_key_base64()).expect("valid key");
        assert_eq!(key.bytes(), &[42u8; 32]);
    }

    #[test]
    fn from_base64_rejects_wrong_length() {
        use base64::prelude::{BASE64_STANDARD, Engine as _};
        let short = BASE64_STANDARD.encode([1u8; 16]);
        assert!(matches!(
            SecretsKey::from_base64(&short),
            Err(KeyError::WrongLength { got: 16 })
        ));
    }

    #[test]
    fn from_base64_rejects_garbage() {
        assert!(matches!(
            SecretsKey::from_base64("not base64 !!!"),
            Err(KeyError::InvalidBase64)
        ));
    }

    #[test]
    fn acquire_key_without_any_source_is_not_configured() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(
            acquire_key(dir.path(), None, None),
            Err(KeyError::NotConfigured)
        ));
    }

    #[test]
    fn acquire_key_reads_a_keyfile_outside_the_config_dir() {
        let config = tempfile::tempdir().expect("config dir");
        let keydir = tempfile::tempdir().expect("key dir");
        let keyfile = keydir.path().join("secrets.key");
        write_keyfile(&keyfile, &valid_key_base64(), 0o600);

        let key = acquire_key(config.path(), Some(&keyfile), None).expect("key loads");
        assert_eq!(key.bytes(), &[42u8; 32]);
    }

    #[test]
    fn acquire_key_prefers_the_keyfile_over_inline_base64() {
        use base64::prelude::{BASE64_STANDARD, Engine as _};
        let config = tempfile::tempdir().expect("config dir");
        let keydir = tempfile::tempdir().expect("key dir");
        let keyfile = keydir.path().join("secrets.key");
        write_keyfile(&keyfile, &BASE64_STANDARD.encode([7u8; 32]), 0o600);

        let key = acquire_key(config.path(), Some(&keyfile), Some(&valid_key_base64()))
            .expect("keyfile wins");
        assert_eq!(key.bytes(), &[7u8; 32]);
    }

    #[test]
    fn acquire_key_does_not_fall_back_when_the_keyfile_fails_to_load() {
        use base64::prelude::{BASE64_STANDARD, Engine as _};
        let config = tempfile::tempdir().expect("config dir");
        let keydir = tempfile::tempdir().expect("key dir");
        let keyfile = keydir.path().join("secrets.key");
        write_keyfile(&keyfile, &BASE64_STANDARD.encode([7u8; 16]), 0o600);

        assert!(matches!(
            acquire_key(config.path(), Some(&keyfile), Some(&valid_key_base64())),
            Err(KeyError::WrongLength { got: 16 })
        ));
    }

    #[test]
    fn acquire_key_rejects_a_keyfile_inside_the_config_dir() {
        let config = tempfile::tempdir().expect("config dir");
        let keyfile = config.path().join("secrets.key");
        write_keyfile(&keyfile, &valid_key_base64(), 0o600);

        assert!(matches!(
            acquire_key(config.path(), Some(&keyfile), None),
            Err(KeyError::KeyfileInsideConfigDir { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn acquire_key_rejects_a_world_readable_keyfile() {
        let config = tempfile::tempdir().expect("config dir");
        let keydir = tempfile::tempdir().expect("key dir");
        let keyfile = keydir.path().join("secrets.key");
        write_keyfile(&keyfile, &valid_key_base64(), 0o644);

        assert!(matches!(
            acquire_key(config.path(), Some(&keyfile), None),
            Err(KeyError::KeyfileWorldReadable { mode }) if mode & 0o077 != 0
        ));
    }

    #[test]
    fn acquire_key_reports_a_missing_keyfile() {
        let config = tempfile::tempdir().expect("config dir");
        let missing = config.path().parent().unwrap().join("does-not-exist.key");
        assert!(matches!(
            acquire_key(config.path(), Some(&missing), None),
            Err(KeyError::KeyfileNotFound(_))
        ));
    }

    #[test]
    fn acquire_key_accepts_a_keyfile_at_the_size_limit() {
        let config = tempfile::tempdir().expect("config dir");
        let keydir = tempfile::tempdir().expect("key dir");
        let keyfile = keydir.path().join("secrets.key");
        let contents = format!("{:width$}", valid_key_base64(), width = 1024);
        assert_eq!(contents.len(), 1024);
        write_keyfile(&keyfile, &contents, 0o600);

        let key = acquire_key(config.path(), Some(&keyfile), None).expect("key loads");
        assert_eq!(key.bytes(), &[42u8; 32]);
    }

    #[test]
    fn acquire_key_rejects_an_oversized_keyfile_without_inline_fallback() {
        let config = tempfile::tempdir().expect("config dir");
        let keydir = tempfile::tempdir().expect("key dir");
        let keyfile = keydir.path().join("secrets.key");
        let contents = format!("{:width$}", valid_key_base64(), width = 1025);
        assert_eq!(contents.len(), 1025);
        write_keyfile(&keyfile, &contents, 0o600);

        let error = acquire_key(config.path(), Some(&keyfile), Some(&valid_key_base64()))
            .expect_err("oversized file must not fall back to the inline key");
        assert!(matches!(
            error,
            KeyError::Io { operation, detail }
                if operation == "read the secrets keyfile" && detail == "keyfile exceeds 1024 bytes"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn acquire_key_checks_permissions_before_keyfile_size() {
        let config = tempfile::tempdir().expect("config dir");
        let keydir = tempfile::tempdir().expect("key dir");
        let keyfile = keydir.path().join("secrets.key");
        write_keyfile(&keyfile, &"x".repeat(1025), 0o644);

        assert!(matches!(
            acquire_key(config.path(), Some(&keyfile), Some(&valid_key_base64())),
            Err(KeyError::KeyfileWorldReadable { mode: 0o644 })
        ));
    }

    fn write_keyfile(path: &Path, contents: &str, mode: u32) {
        std::fs::write(path, contents).expect("write keyfile");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                .expect("chmod keyfile");
        }
        #[cfg(not(unix))]
        let _ = mode;
    }

    fn test_key(byte: u8) -> SecretsKey {
        SecretsKey::from_bytes([byte; 32])
    }

    #[test]
    fn seal_then_open_round_trips() {
        let key = test_key(7);
        let sealed = seal(&key, b"a refresh token").expect("seal");
        let opened = open(&key, &sealed).expect("open");
        assert_eq!(opened, b"a refresh token");
    }

    #[test]
    fn open_tolerates_a_trailing_newline() {
        // write_and_replace appends `\n`; open must strip trailing whitespace.
        let key = test_key(7);
        let mut sealed = seal(&key, b"payload").expect("seal");
        sealed.push(b'\n');
        assert_eq!(open(&key, &sealed).expect("open"), b"payload");
    }

    #[test]
    fn wrong_key_fails_closed() {
        let sealed = seal(&test_key(1), b"payload").expect("seal");
        let error = open(&test_key(2), &sealed).expect_err("wrong key must fail");
        assert!(matches!(error, SecretsError::Decrypt));
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        use base64::prelude::{BASE64_STANDARD, Engine as _};

        let key = test_key(9);
        let sealed = seal(&key, b"payload").expect("seal");
        let mut raw = BASE64_STANDARD.decode(&sealed).expect("decode");
        let last = raw.len() - 1;
        raw[last] ^= 0xff; // flip a ciphertext/tag byte
        let retampered = BASE64_STANDARD.encode(&raw).into_bytes();
        assert!(matches!(
            open(&key, &retampered),
            Err(SecretsError::Decrypt)
        ));
    }

    #[test]
    fn truncated_file_is_rejected() {
        let key = test_key(3);
        let sealed = seal(&key, b"payload").expect("seal");
        let truncated = &sealed[..sealed.len() / 2];
        assert!(open(&key, truncated).is_err());
    }

    #[test]
    fn wrong_magic_is_bad_format() {
        use base64::prelude::{BASE64_STANDARD, Engine as _};

        let key = test_key(4);
        let sealed = seal(&key, b"payload").expect("seal");
        let mut raw = BASE64_STANDARD.decode(&sealed).expect("decode");
        raw[0] ^= 0xff; // corrupt the magic prefix
        let broken = BASE64_STANDARD.encode(&raw).into_bytes();
        assert!(matches!(open(&key, &broken), Err(SecretsError::BadFormat)));
    }

    #[test]
    fn debug_redacts_the_key() {
        assert_eq!(format!("{:?}", test_key(5)), "SecretsKey(redacted)");
    }

    fn sample_secret() -> IntegrationSecret {
        IntegrationSecret {
            provider: "google".to_string(),
            refresh_token: "super-secret-refresh-token-value".to_string(),
            client_secret: Some("client-secret-value".to_string()),
            scopes: vec!["https://www.googleapis.com/auth/calendar.events.readonly".to_string()],
            obtained_at: 1_725_600_000,
        }
    }

    fn replacement_secret() -> IntegrationSecret {
        IntegrationSecret {
            refresh_token: "replacement-refresh-token".to_string(),
            obtained_at: 1_725_600_001,
            ..sample_secret()
        }
    }

    #[test]
    fn failed_put_before_replacement_preserves_live_and_durable_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SECRETS_STORE_FILE);
        let mut store = IntegrationStore::open(path.clone(), test_key(31)).expect("open");
        store
            .put("id".to_string(), sample_secret())
            .expect("seed old value");
        store.persistence_fault = Some(PersistenceFault::BeforeReplacement);

        store
            .put("id".to_string(), replacement_secret())
            .expect_err("replacement must fail before commit");

        assert_eq!(store.get("id"), Some(sample_secret()));
        let reopened = IntegrationStore::open(path, test_key(31)).expect("reopen");
        assert_eq!(reopened.get("id"), Some(sample_secret()));
    }

    #[test]
    fn failed_remove_before_replacement_preserves_live_and_durable_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SECRETS_STORE_FILE);
        let mut store = IntegrationStore::open(path.clone(), test_key(32)).expect("open");
        store
            .put("id".to_string(), sample_secret())
            .expect("seed old value");
        store.persistence_fault = Some(PersistenceFault::BeforeReplacement);
        assert!(!store.remove("absent").expect("absent remove skips write"));

        store
            .remove("id")
            .expect_err("removal must fail before commit");

        assert_eq!(store.get("id"), Some(sample_secret()));
        let reopened = IntegrationStore::open(path, test_key(32)).expect("reopen");
        assert_eq!(reopened.get("id"), Some(sample_secret()));
    }

    #[test]
    fn parent_sync_failure_keeps_replaced_live_and_durable_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SECRETS_STORE_FILE);
        let mut store = IntegrationStore::open(path.clone(), test_key(33)).expect("open");
        store
            .put("id".to_string(), sample_secret())
            .expect("seed old value");
        store.persistence_fault = Some(PersistenceFault::ParentSync);

        store
            .put("id".to_string(), replacement_secret())
            .expect_err("parent sync failure must be reported");

        assert_eq!(store.get("id"), Some(replacement_secret()));
        let reopened = IntegrationStore::open(path, test_key(33)).expect("reopen");
        assert_eq!(reopened.get("id"), Some(replacement_secret()));
    }

    #[test]
    fn debug_redacts_integration_secret_credentials() {
        let debug = format!("{:?}", sample_secret());

        assert!(debug.contains("provider"));
        assert!(debug.contains("google"));
        assert!(!debug.contains("super-secret-refresh-token-value"));
        assert!(!debug.contains("client-secret-value"));
    }

    #[test]
    fn put_then_get_round_trips_across_reopen() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SECRETS_STORE_FILE);

        let store = IntegrationStore::open(path.clone(), test_key(11)).expect("open empty");
        store
            .put("google-primary".to_string(), sample_secret())
            .expect("put");
        drop(store);

        let reopened = IntegrationStore::open(path, test_key(11)).expect("reopen");
        assert_eq!(reopened.get("google-primary"), Some(sample_secret()));
        assert_eq!(reopened.get("absent"), None);
    }

    #[test]
    fn remove_deletes_and_reports_presence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SECRETS_STORE_FILE);
        let store = IntegrationStore::open(path, test_key(12)).expect("open");

        store.put("id".to_string(), sample_secret()).expect("put");
        assert!(store.remove("id").expect("remove existing"));
        assert!(!store.remove("id").expect("remove absent"));
        assert_eq!(store.get("id"), None);
    }

    #[test]
    fn integration_ids_lists_keys_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SECRETS_STORE_FILE);
        let store = IntegrationStore::open(path, test_key(13)).expect("open");

        store.put("b".to_string(), sample_secret()).expect("put b");
        store.put("a".to_string(), sample_secret()).expect("put a");
        assert_eq!(
            store.integration_ids(),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn opening_with_the_wrong_key_fails_closed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SECRETS_STORE_FILE);
        IntegrationStore::open(path.clone(), test_key(1))
            .expect("open")
            .put("id".to_string(), sample_secret())
            .expect("put");

        assert!(matches!(
            IntegrationStore::open(path, test_key(2)),
            Err(SecretsError::Decrypt)
        ));
    }

    #[test]
    fn persisted_file_contains_no_plaintext_secret() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SECRETS_STORE_FILE);
        let store = IntegrationStore::open(path.clone(), test_key(14)).expect("open");
        store.put("id".to_string(), sample_secret()).expect("put");

        let on_disk = std::fs::read(&path).expect("read file");
        for needle in [
            b"super-secret-refresh-token-value".as_slice(),
            b"client-secret-value".as_slice(),
            b"google".as_slice(),
        ] {
            assert!(
                !on_disk.windows(needle.len()).any(|window| window == needle),
                "plaintext leaked to disk: {:?}",
                String::from_utf8_lossy(needle)
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn persisted_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SECRETS_STORE_FILE);
        IntegrationStore::open(path.clone(), test_key(15))
            .expect("open")
            .put("id".to_string(), sample_secret())
            .expect("put");

        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn startup_fails_closed_when_key_missing_and_secrets_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(SECRETS_STORE_FILE);
        // Materialize a real sealed secrets file first.
        IntegrationStore::open(path, test_key(21))
            .expect("open")
            .put("id".to_string(), sample_secret())
            .expect("put");

        let result = open_integration_store_with(dir.path(), Err(KeyError::NotConfigured));
        assert!(matches!(
            result,
            Err(StartupError::KeyRequiredForExistingSecrets)
        ));
    }

    #[test]
    fn startup_requires_a_key_even_with_no_secrets_yet() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = open_integration_store_with(dir.path(), Err(KeyError::NotConfigured));
        assert!(matches!(result, Err(StartupError::KeyRequired)));
    }

    #[test]
    fn startup_propagates_a_concrete_key_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result =
            open_integration_store_with(dir.path(), Err(KeyError::WrongLength { got: 10 }));
        assert!(matches!(
            result,
            Err(StartupError::Key(KeyError::WrongLength { got: 10 }))
        ));
    }

    #[test]
    fn startup_opens_the_store_with_a_valid_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = open_integration_store_with(dir.path(), Ok(test_key(22))).expect("store opens");
        assert!(store.integration_ids().is_empty());
    }
}
