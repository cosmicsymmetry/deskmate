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
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::registry::constant_time_eq;

pub(crate) const STORE_FILE: &str = "producer-credentials.json";
const STORE_SCHEMA_VERSION: u32 = 1;
const MAX_STORE_FILE_BYTES: usize = 64 * 1_024;

#[derive(Debug, thiserror::Error)]
pub enum ProducerCredentialError {
    #[error("producer credential store i/o failed: {message}")]
    Io { message: String },
    #[error("producer credential store is malformed: {message}")]
    Malformed { message: String },
}

/// A freshly minted credential. The plaintext exists only in this value and is
/// never recoverable afterwards.
pub struct MintedProducerCredential {
    pub integration_id: String,
    pub token: String,
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

pub struct ProducerCredentialStore {
    root: PathBuf,
    state: Mutex<Vec<Credential>>,
}

impl ProducerCredentialStore {
    /// Loads the store, treating an absent file as an empty one.
    ///
    /// # Errors
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn open(root: PathBuf) -> Result<Self, ProducerCredentialError> {
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
    pub fn mint(
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
    pub fn authenticate(&self, token: &str) -> Option<String> {
        let presented = token_digest(token);
        let state = self.lock();
        state
            .iter()
            .find(|credential| constant_time_eq(&credential.token_digest, &presented))
            .map(|credential| credential.integration_id.clone())
    }

    /// Removes `integration_id`'s credential. Reports whether one existed.
    ///
    /// # Errors
    /// Returns an error if the store cannot be persisted.
    pub fn revoke(&self, integration_id: &str) -> Result<bool, ProducerCredentialError> {
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
    let bytes = serde_json::to_vec_pretty(&persisted).map_err(|error| {
        ProducerCredentialError::Io {
            message: format!("serializing producer credentials: {error}"),
        }
    })?;
    secure_file::write_and_replace(&root.join(STORE_FILE), &bytes).map_err(|error| {
        ProducerCredentialError::Io {
            message: format!("writing producer credentials: {error:?}"),
        }
    })
}

fn token_digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// A random 32-byte credential, rendered as 64 lowercase hex characters --
/// the same shape and entropy as an image-source token.
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    protocol::digest_hex(&bytes)
}

fn decode_digest(encoded: &str) -> Result<[u8; 32], ProducerCredentialError> {
    if encoded.len() != 64 || !encoded.is_ascii() {
        return Err(ProducerCredentialError::Malformed {
            message: String::from("token digest is not 64 lowercase hexadecimal bytes"),
        });
    }
    let mut digest = [0u8; 32];
    for (output, pair) in digest.iter_mut().zip(encoded.as_bytes().as_chunks::<2>().0) {
        *output = (decode_hex_digit(pair[0])? << 4) | decode_hex_digit(pair[1])?;
    }
    Ok(digest)
}

fn decode_hex_digit(byte: u8) -> Result<u8, ProducerCredentialError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(ProducerCredentialError::Malformed {
            message: String::from("token digest is not lowercase hexadecimal"),
        }),
    }
}
