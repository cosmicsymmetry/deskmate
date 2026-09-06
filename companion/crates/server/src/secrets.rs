//! Encrypted-at-rest storage for per-integration OAuth secrets (spec §1.1, §6).
//! Server-crate only; never depended on by `app-core`.
#![allow(
    dead_code,
    reason = "these primitives are wired into the store by later foundation tasks"
)]

use base64::prelude::{BASE64_STANDARD, Engine as _};
use chacha20poly1305::aead::{Aead, KeyInit, OsRng, Payload};
use chacha20poly1305::{AeadCore, XChaCha20Poly1305, XNonce};
use std::path::{Path, PathBuf};
use zeroize::ZeroizeOnDrop;

/// Environment variable naming a `0o600` keyfile that holds the base64 master key.
pub const ENV_KEY_FILE: &str = "DESKMATE_SECRETS_KEY_FILE";
/// Fallback environment variable carrying the base64 master key inline. Documented
/// as second-choice: env values are visible in `ps`/`/proc`/`systemctl show` in a
/// way a keyfile is not (spec §1.1).
pub const ENV_KEY: &str = "DESKMATE_SECRETS_KEY";

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
    pub fn from_base64(value: &str) -> Result<Self, KeyError> {
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
    pub fn from_keyfile(config_dir: &Path, keyfile: &Path) -> Result<Self, KeyError> {
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

        let bytes = std::fs::read(&canonical_keyfile).map_err(|error| KeyError::Io {
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
pub fn acquire_key(
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
pub fn acquire_key_from_env(config_dir: &Path) -> Result<SecretsKey, KeyError> {
    let keyfile = std::env::var_os(ENV_KEY_FILE).map(PathBuf::from);
    let inline = std::env::var(ENV_KEY).ok();
    acquire_key(config_dir, keyfile.as_deref(), inline.as_deref())
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
}
