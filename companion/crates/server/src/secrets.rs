//! Encrypted-at-rest storage for per-integration OAuth secrets (spec §1.1, §6).
//! Server-crate only; never depended on by `app-core`.
#![allow(
    dead_code,
    reason = "these primitives are wired into the store by later foundation tasks"
)]

use base64::prelude::{BASE64_STANDARD, Engine as _};
use chacha20poly1305::aead::{Aead, KeyInit, OsRng, Payload};
use chacha20poly1305::{AeadCore, XChaCha20Poly1305, XNonce};
use zeroize::ZeroizeOnDrop;

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

#[cfg(test)]
mod tests {
    use super::*;

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
