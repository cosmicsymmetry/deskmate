//! Producer credentials: a write-scoped secret that lets one external producer
//! ask for one integration's access token.
//!
//! Deliberately *not* the image-source token. Pushing a picture and minting a
//! Google access token are different privileges: a producer that renders a
//! calendar holds both, and revoking one must not revoke the other, so that
//! disconnecting an integration leaves its picture pushes working (and stale)
//! rather than silently failing with a 401 the operator cannot interpret.
//!
//! Follows the contract `image_sources.rs` and `registry.rs` already use --
//! only the SHA-256 digest is persisted, the plaintext is returned exactly
//! once, and rejection compares in constant time -- so the store file is worth
//! nothing to whoever reads a backup.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use app_core::secure_file;
use serde::{Deserialize, Serialize};

use crate::credential::{DigestDecodeError, decode_digest as decode_credential_digest};
use crate::credential::{random_token, token_digest};
use crate::registry::constant_time_eq;

pub(crate) const STORE_FILE: &str = "producer-credentials.json";
const STORE_SCHEMA_VERSION: u32 = 1;
const MAX_STORE_FILE_BYTES: usize = 64 * 1_024;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ProducerCredentialError {
    #[error("producer credential store i/o failed: {message}")]
    Io { message: String },
    #[error("producer credential store is malformed: {message}")]
    Malformed { message: String },
}

/// A freshly minted credential. The plaintext exists only in this value and is
/// never recoverable afterwards.
pub(crate) struct MintedProducerCredential {
    pub(crate) integration_id: String,
    pub(crate) token: String,
}

impl std::fmt::Debug for MintedProducerCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MintedProducerCredential")
            .field("integration_id", &self.integration_id)
            .field("token", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct PersistedCredential {
    integration_id: String,
    token_sha256: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct PersistedStore {
    schema_version: u32,
    credentials: Vec<PersistedCredential>,
}

#[derive(Clone)]
struct Credential {
    integration_id: String,
    token_digest: [u8; 32],
}

pub(crate) struct ProducerCredentialStore {
    root: PathBuf,
    state: Mutex<Vec<Credential>>,
}

impl ProducerCredentialStore {
    /// Loads the store, treating an absent file as an empty one.
    ///
    /// # Errors
    /// Returns an error if the file exists but cannot be read or parsed.
    pub(crate) fn open(root: PathBuf) -> Result<Self, ProducerCredentialError> {
        let credentials = load(&root)?;
        Ok(Self {
            root,
            state: Mutex::new(credentials),
        })
    }

    /// Mints a credential for `integration_id`, replacing any existing one.
    ///
    /// Replacement *is* rotation: the previous plaintext stops authenticating
    /// the moment this returns, so an operator re-minting after a suspected
    /// leak does not have to revoke first.
    ///
    /// # Errors
    /// Returns an error if the store cannot be persisted.
    pub(crate) fn mint(
        &self,
        integration_id: &str,
    ) -> Result<MintedProducerCredential, ProducerCredentialError> {
        let mut state = self.lock();
        let token = random_token();
        let mut candidate: Vec<Credential> = state
            .iter()
            .filter(|credential| credential.integration_id != integration_id)
            .cloned()
            .collect();
        candidate.push(Credential {
            integration_id: integration_id.to_owned(),
            token_digest: token_digest(&token),
        });
        save(&self.root, &candidate)?;
        *state = candidate;
        Ok(MintedProducerCredential {
            integration_id: integration_id.to_owned(),
            token,
        })
    }

    /// Resolves a presented credential to its integration id.
    ///
    /// A *match* short-circuits, which is fine: which credential matched is
    /// exactly what is being asked for. The property guarded here is about
    /// *rejection* -- a wrong credential is compared against every entry with
    /// [`constant_time_eq`] rather than `==`, so how many of its leading bytes
    /// happen to match a real digest never shows up as a timing difference.
    #[must_use]
    pub(crate) fn authenticate(&self, token: &str) -> Option<String> {
        let presented = token_digest(token);
        let state = self.lock();
        state
            .iter()
            .find(|credential| constant_time_eq(&credential.token_digest, &presented))
            .map(|credential| credential.integration_id.clone())
    }

    /// Whether a credential exists for `integration_id`. Presence only; the
    /// value is not recoverable from this type at all.
    #[must_use]
    pub(crate) fn has_credential(&self, integration_id: &str) -> bool {
        self.lock()
            .iter()
            .any(|credential| credential.integration_id == integration_id)
    }

    /// Removes `integration_id`'s credential. Reports whether one existed.
    ///
    /// # Errors
    /// Returns an error if the store cannot be persisted.
    pub(crate) fn revoke(&self, integration_id: &str) -> Result<bool, ProducerCredentialError> {
        let mut state = self.lock();
        let candidate: Vec<Credential> = state
            .iter()
            .filter(|credential| credential.integration_id != integration_id)
            .cloned()
            .collect();
        if candidate.len() == state.len() {
            return Ok(false);
        }
        save(&self.root, &candidate)?;
        *state = candidate;
        Ok(true)
    }

    /// A panic while some other request held the lock must not turn every
    /// *subsequent* authentication into a panic too -- that would be a latent,
    /// self-inflicted denial of service on an internet-facing endpoint. The
    /// store holds no invariant a panic mid-mutation could leave inconsistent
    /// (the on-disk write happens before the in-memory swap), so recovering the
    /// poisoned guard and carrying on is safe.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Credential>> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn load(root: &Path) -> Result<Vec<Credential>, ProducerCredentialError> {
    let path = root.join(STORE_FILE);
    let bytes = match secure_file::read_bounded(&path, MAX_STORE_FILE_BYTES) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(Vec::new()),
        // Matches `Registry::load` and `ImageSourceStore::load`: a fresh or
        // unusable root still constructs, and the first attempted write is what
        // reports that the configured data root is not a directory.
        Err(secure_file::BoundedReadError::Io(error))
            if error.source.kind() == io::ErrorKind::NotADirectory =>
        {
            return Ok(Vec::new());
        }
        Err(error) => {
            return Err(ProducerCredentialError::Io {
                message: format!("reading producer credentials: {error:?}"),
            });
        }
    };
    let persisted: PersistedStore =
        serde_json::from_slice(&bytes).map_err(|error| ProducerCredentialError::Malformed {
            message: format!("invalid JSON: {error}"),
        })?;
    if persisted.schema_version != STORE_SCHEMA_VERSION {
        return Err(ProducerCredentialError::Malformed {
            message: format!(
                "unsupported schema version {}; expected {STORE_SCHEMA_VERSION}",
                persisted.schema_version
            ),
        });
    }
    persisted
        .credentials
        .into_iter()
        .map(|credential| {
            Ok(Credential {
                integration_id: credential.integration_id,
                token_digest: decode_digest(&credential.token_sha256)?,
            })
        })
        .collect()
}

fn save(root: &Path, credentials: &[Credential]) -> Result<(), ProducerCredentialError> {
    secure_file::create_directory(root).map_err(|error| ProducerCredentialError::Io {
        message: format!("creating the producer credential directory: {error:?}"),
    })?;
    let persisted = PersistedStore {
        schema_version: STORE_SCHEMA_VERSION,
        credentials: credentials
            .iter()
            .map(|credential| PersistedCredential {
                integration_id: credential.integration_id.clone(),
                token_sha256: protocol::digest_hex(&credential.token_digest),
            })
            .collect(),
    };
    let bytes =
        serde_json::to_vec_pretty(&persisted).map_err(|error| ProducerCredentialError::Io {
            message: format!("serializing producer credentials: {error}"),
        })?;
    secure_file::write_and_replace(&root.join(STORE_FILE), &bytes).map_err(|error| {
        ProducerCredentialError::Io {
            message: format!("writing producer credentials: {error:?}"),
        }
    })
}

fn decode_digest(encoded: &str) -> Result<[u8; 32], ProducerCredentialError> {
    match decode_credential_digest(encoded) {
        Ok(digest) => Ok(digest),
        Err(DigestDecodeError::Length) => Err(ProducerCredentialError::Malformed {
            message: String::from("token digest is not 64 lowercase hexadecimal bytes"),
        }),
        Err(DigestDecodeError::Character) => Err(ProducerCredentialError::Malformed {
            message: String::from("token digest is not lowercase hexadecimal"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::ProducerCredentialStore;

    fn store() -> (tempfile::TempDir, ProducerCredentialStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ProducerCredentialStore::open(dir.path().to_path_buf()).expect("open");
        (dir, store)
    }

    #[test]
    fn a_minted_credential_authenticates_to_its_integration() {
        let (_dir, store) = store();
        let minted = store.mint("google").expect("mint");
        assert_eq!(store.authenticate(&minted.token).as_deref(), Some("google"));
    }

    #[test]
    fn an_unknown_credential_authenticates_to_nothing() {
        let (_dir, store) = store();
        store.mint("google").expect("mint");
        assert!(store.authenticate("not-a-real-token").is_none());
    }

    #[test]
    fn what_lands_on_disk_is_the_sha256_of_the_credential() {
        // Asserting only that the plaintext is ABSENT is too weak, and a mutation
        // probe proved it: a `token_digest` that copied the token's bytes verbatim
        // instead of hashing them passed that check, because the bytes are
        // hex-encoded on the way to disk and the substring never appears. So pin
        // the contract positively -- the stored value must BE the hash.
        use sha2::{Digest, Sha256};

        let (dir, store) = store();
        let minted = store.mint("google").expect("mint");
        let on_disk = std::fs::read_to_string(dir.path().join("producer-credentials.json"))
            .expect("store file");

        let digest: [u8; 32] = Sha256::digest(minted.token.as_bytes()).into();
        // `digest_hex` is only the encoding; the assertion that carries weight is
        // that the bytes are SHA-256 of the token.
        let expected = protocol::digest_hex(&digest);
        assert!(
            on_disk.contains(&expected),
            "the stored value is not SHA-256(token)"
        );
        assert!(
            !on_disk.contains(&minted.token),
            "the plaintext credential was persisted"
        );
    }

    #[test]
    fn minting_again_rotates_and_retires_the_previous_credential() {
        let (_dir, store) = store();
        let first = store.mint("google").expect("first mint");
        let second = store.mint("google").expect("second mint");
        assert_ne!(first.token, second.token);
        assert!(
            store.authenticate(&first.token).is_none(),
            "a rotated credential must stop working"
        );
        assert_eq!(store.authenticate(&second.token).as_deref(), Some("google"));
    }

    #[test]
    fn revoking_stops_the_credential_and_reports_whether_one_existed() {
        let (_dir, store) = store();
        let minted = store.mint("google").expect("mint");
        assert!(store.revoke("google").expect("revoke"));
        assert!(store.authenticate(&minted.token).is_none());
        assert!(!store.revoke("google").expect("second revoke"));
    }

    #[test]
    fn revoking_one_integration_leaves_another_alone() {
        // Revocation is scoped by integration id; a wider deletion would make
        // disconnecting one integration silently break every producer.
        let (_dir, store) = store();
        let google = store.mint("google").expect("mint google");
        let other = store.mint("dropbox").expect("mint dropbox");
        store.revoke("google").expect("revoke google");
        assert!(store.authenticate(&google.token).is_none());
        assert_eq!(
            store.authenticate(&other.token).as_deref(),
            Some("dropbox"),
            "revoking one integration must not touch another's credential"
        );
    }

    #[test]
    fn a_credential_survives_a_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let minted = {
            let store = ProducerCredentialStore::open(dir.path().to_path_buf()).expect("open");
            store.mint("google").expect("mint")
        };
        let reopened = ProducerCredentialStore::open(dir.path().to_path_buf()).expect("reopen");
        assert_eq!(
            reopened.authenticate(&minted.token).as_deref(),
            Some("google")
        );
    }

    #[test]
    fn opening_a_fresh_root_yields_an_empty_store() {
        let (_dir, store) = store();
        assert!(store.authenticate("anything").is_none());
        assert!(!store.revoke("google").expect("revoke on empty store"));
    }
}
