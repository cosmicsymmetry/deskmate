//! Durable image-source credentials, canonical frames, and observed push cadence.
//!
//! Tokens follow the device-identity registry's contract: only SHA-256 digests
//! are persisted, and the plaintext is returned exactly once. Frames are read
//! once at startup into `Arc<[u8]>` and stay there, so later asset reconciliation
//! neither re-reads nor re-hashes bytes.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use app_core::config::MAX_IMAGE_SOURCES;
use app_core::secure_file;
use chrono::{DateTime, SecondsFormat, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::image_ingest::CanonicalFrame;
use crate::image_staleness::{PUSH_TIME_RING, is_stale};
use crate::registry::constant_time_eq;

pub(crate) const IMAGE_SOURCE_STORE_FILE: &str = "image-sources.json";
pub(crate) const IMAGE_FRAME_DIRECTORY: &str = "image-frames";
pub(crate) const MIN_PUSH_INTERVAL: Duration = Duration::from_secs(5);

const STORE_SCHEMA_VERSION: u32 = 1;
const MAX_STORE_FILE_BYTES: usize = 64 * 1_024;
const LVGL_IMAGE_HEADER_BYTES: usize = 12;
const CANONICAL_FRAME_BYTES: usize = LVGL_IMAGE_HEADER_BYTES
    + protocol::SCENE_CANVAS_WIDTH as usize * protocol::SCENE_CANVAS_HEIGHT as usize * 2;
const SOURCE_ID_RANDOM_HEX_LEN: usize = 24;

pub(crate) struct ImageSourceStore {
    root: PathBuf,
    state: Mutex<ImageSourceState>,
}

#[derive(Clone)]
struct ImageSourceState {
    sources: Vec<SourceRecord>,
}

#[derive(Clone)]
struct SourceRecord {
    id: String,
    name: String,
    token_digest: [u8; 32],
    recent_push_times: Vec<DateTime<Utc>>,
    frame: Option<StoredFrame>,
}

#[derive(Clone)]
struct StoredFrame {
    digest: [u8; 32],
    bytes: Arc<[u8]>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct MintedSource {
    pub id: String,
    pub token: String,
}

impl fmt::Debug for MintedSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MintedSource")
            .field("id", &self.id)
            .field("token", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceFrame {
    pub digest: [u8; 32],
    pub bytes: Arc<[u8]>,
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AcceptOutcome {
    Changed { digest: [u8; 32] },
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ImageSourceError {
    #[error("unknown or revoked image source")]
    UnknownToken,
    #[error("the image-source capacity has been reached")]
    Capacity,
    #[error("image-source storage failed: {message}")]
    Io { message: String },
    #[error("the image source was pushed too recently")]
    TooSoon,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedStore {
    schema_version: u32,
    sources: Vec<PersistedSource>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedSource {
    id: String,
    name: String,
    token_sha256: String,
    recent_push_times: Vec<String>,
}

impl ImageSourceStore {
    pub(crate) fn new(root: PathBuf) -> Result<Self, ImageSourceError> {
        let state = load_store(&root)?;
        Ok(Self {
            root,
            state: Mutex::new(state),
        })
    }

    /// Mints a source id and a random 32-byte bearer token. The token's digest
    /// is committed before the one plaintext copy is returned.
    pub(crate) fn mint(&self, name: &str) -> Result<MintedSource, ImageSourceError> {
        let mut state = self.lock();
        if state.sources.len() >= MAX_IMAGE_SOURCES {
            return Err(ImageSourceError::Capacity);
        }

        let id = loop {
            let candidate = random_source_id();
            if state.sources.iter().all(|source| source.id != candidate) {
                break candidate;
            }
        };
        let token = random_token();
        let record = SourceRecord {
            id: id.clone(),
            name: name.to_owned(),
            token_digest: token_digest(&token),
            recent_push_times: Vec::new(),
            frame: None,
        };
        let mut candidate = state.clone();
        candidate.sources.push(record);
        save_store(&self.root, &candidate)?;
        *state = candidate;

        Ok(MintedSource { id, token })
    }

    pub(crate) fn revoke(&self, id: &str) -> Result<(), ImageSourceError> {
        let mut state = self.lock();
        let index = state
            .sources
            .iter()
            .position(|source| source.id == id)
            .ok_or(ImageSourceError::UnknownToken)?;

        let mut candidate = state.clone();
        let removed = candidate.sources.remove(index);
        save_store(&self.root, &candidate)?;
        *state = candidate;

        remove_frame(&self.root, &removed.id)
    }

    /// Authenticates against digest records only. Output-equivalence tests
    /// cannot distinguish this review-enforced guard from `==`; keep the
    /// constant-time comparison even though its timing property is not test-provable.
    pub(crate) fn authenticate(&self, token: &str) -> Option<String> {
        let presented_digest = token_digest(token);
        self.lock()
            .sources
            .iter()
            .find(|source| constant_time_eq(&source.token_digest, &presented_digest))
            .map(|source| source.id.clone())
    }

    pub(crate) fn accept(
        &self,
        id: &str,
        frame: CanonicalFrame,
        now: DateTime<Utc>,
    ) -> Result<AcceptOutcome, ImageSourceError> {
        let mut state = self.lock();
        let index = state
            .sources
            .iter()
            .position(|source| source.id == id)
            .ok_or(ImageSourceError::UnknownToken)?;
        let source = &state.sources[index];

        if let Some(last) = source.recent_push_times.last() {
            let minimum = chrono::Duration::from_std(MIN_PUSH_INTERVAL)
                .expect("the fixed minimum push interval fits chrono");
            if now - *last < minimum {
                return Err(ImageSourceError::TooSoon);
            }
        }

        let mut recent_push_times = source.recent_push_times.clone();
        recent_push_times.push(now);
        if recent_push_times.len() > PUSH_TIME_RING {
            recent_push_times.remove(0);
        }

        let unchanged = source
            .frame
            .as_ref()
            .is_some_and(|stored| stored.digest == frame.digest);
        let mut candidate = state.clone();
        candidate.sources[index].recent_push_times = recent_push_times;

        if unchanged {
            save_store(&self.root, &candidate)?;
            *state = candidate;
            return Ok(AcceptOutcome::Unchanged);
        }

        write_frame(&self.root, id, &frame.bytes)?;
        let digest = frame.digest;
        candidate.sources[index].frame = Some(StoredFrame {
            digest,
            bytes: Arc::from(frame.bytes),
        });
        save_store(&self.root, &candidate)?;
        *state = candidate;
        Ok(AcceptOutcome::Changed { digest })
    }

    pub(crate) fn frame(&self, id: &str, now: DateTime<Utc>) -> Option<SourceFrame> {
        let state = self.lock();
        let source = state.sources.iter().find(|source| source.id == id)?;
        source.frame.as_ref().map(|frame| SourceFrame {
            digest: frame.digest,
            bytes: Arc::clone(&frame.bytes),
            stale: is_stale(&source.recent_push_times, now),
        })
    }

    pub(crate) fn all_frames(&self, now: DateTime<Utc>) -> Vec<(String, SourceFrame)> {
        self.lock()
            .sources
            .iter()
            .filter_map(|source| {
                source.frame.as_ref().map(|frame| {
                    (
                        source.id.clone(),
                        SourceFrame {
                            digest: frame.digest,
                            bytes: Arc::clone(&frame.bytes),
                            stale: is_stale(&source.recent_push_times, now),
                        },
                    )
                })
            })
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ImageSourceState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn load_store(root: &Path) -> Result<ImageSourceState, ImageSourceError> {
    let path = root.join(IMAGE_SOURCE_STORE_FILE);
    let bytes = match secure_file::read_bounded(&path, MAX_STORE_FILE_BYTES) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(empty_state()),
        // Match Registry::load's fresh/unusable-root behaviour: constructing
        // ServerState still succeeds, while the first attempted persistent
        // mutation reports that the configured data root is not a directory.
        Err(secure_file::BoundedReadError::Io(error))
            if error.source.kind() == io::ErrorKind::NotADirectory =>
        {
            return Ok(empty_state());
        }
        Err(error) => return Err(bounded_read_error("image-source metadata", error)),
    };
    let persisted: PersistedStore = serde_json::from_slice(&bytes)
        .map_err(|error| storage_error(format!("invalid metadata JSON: {error}")))?;
    if persisted.schema_version != STORE_SCHEMA_VERSION {
        return Err(storage_error(format!(
            "unsupported metadata schema version {}; expected {STORE_SCHEMA_VERSION}",
            persisted.schema_version
        )));
    }
    if persisted.sources.len() > MAX_IMAGE_SOURCES {
        return Err(storage_error(format!(
            "metadata contains more than {MAX_IMAGE_SOURCES} sources"
        )));
    }

    let mut sources: Vec<SourceRecord> = Vec::with_capacity(persisted.sources.len());
    for persisted_source in persisted.sources {
        if !valid_source_id(&persisted_source.id) {
            return Err(storage_error("metadata contains an invalid source id"));
        }
        let token_digest = decode_digest(&persisted_source.token_sha256)?;
        if sources
            .iter()
            .any(|source| source.id == persisted_source.id || source.token_digest == token_digest)
        {
            return Err(storage_error(
                "metadata contains a duplicate source id or token digest",
            ));
        }
        if persisted_source.recent_push_times.len() > PUSH_TIME_RING {
            return Err(storage_error(format!(
                "metadata contains more than {PUSH_TIME_RING} recent push times"
            )));
        }
        let recent_push_times = persisted_source
            .recent_push_times
            .iter()
            .map(|value| {
                DateTime::parse_from_rfc3339(value)
                    .map(|parsed| parsed.with_timezone(&Utc))
                    .map_err(|error| storage_error(format!("invalid push time: {error}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if recent_push_times.windows(2).any(|pair| pair[0] > pair[1]) {
            return Err(storage_error("recent push times are not chronological"));
        }

        let frame = load_frame(root, &persisted_source.id)?;
        sources.push(SourceRecord {
            id: persisted_source.id,
            name: persisted_source.name,
            token_digest,
            recent_push_times,
            frame,
        });
    }

    Ok(ImageSourceState { sources })
}

fn empty_state() -> ImageSourceState {
    ImageSourceState {
        sources: Vec::new(),
    }
}

fn save_store(root: &Path, state: &ImageSourceState) -> Result<(), ImageSourceError> {
    secure_file::create_directory(root)
        .map_err(|error| file_error("image-source data directory", error))?;
    let sources = state
        .sources
        .iter()
        .map(|source| PersistedSource {
            id: source.id.clone(),
            name: source.name.clone(),
            token_sha256: protocol::digest_hex(&source.token_digest),
            recent_push_times: source
                .recent_push_times
                .iter()
                .map(|time| time.to_rfc3339_opts(SecondsFormat::Nanos, true))
                .collect(),
        })
        .collect();
    let persisted = PersistedStore {
        schema_version: STORE_SCHEMA_VERSION,
        sources,
    };
    let bytes = serde_json::to_vec_pretty(&persisted)
        .map_err(|error| storage_error(format!("failed to encode metadata JSON: {error}")))?;
    if bytes.len() > MAX_STORE_FILE_BYTES {
        return Err(storage_error(format!(
            "encoded metadata exceeds the {MAX_STORE_FILE_BYTES}-byte limit"
        )));
    }

    let path = root.join(IMAGE_SOURCE_STORE_FILE);
    secure_file::write_and_replace(&path, &bytes)
        .map_err(|error| file_error("image-source metadata", error))?;
    sync_parent_best_effort(root, &path);
    Ok(())
}

fn load_frame(root: &Path, id: &str) -> Result<Option<StoredFrame>, ImageSourceError> {
    let path = frame_path(root, id);
    let Some(bytes) = secure_file::read_bounded(&path, CANONICAL_FRAME_BYTES)
        .map_err(|error| bounded_read_error("image-source frame", error))?
    else {
        return Ok(None);
    };
    if bytes.len() != CANONICAL_FRAME_BYTES {
        return Err(storage_error(format!(
            "image-source frame has {} bytes; expected {CANONICAL_FRAME_BYTES}",
            bytes.len()
        )));
    }
    let digest = Sha256::digest(&bytes).into();
    Ok(Some(StoredFrame {
        digest,
        bytes: Arc::from(bytes),
    }))
}

fn write_frame(root: &Path, id: &str, bytes: &[u8]) -> Result<(), ImageSourceError> {
    let directory = root.join(IMAGE_FRAME_DIRECTORY);
    secure_file::create_directory(&directory)
        .map_err(|error| file_error("image-frame directory", error))?;
    let path = frame_path(root, id);
    secure_file::write_and_replace_binary(&path, bytes)
        .map_err(|error| file_error("image-source frame", error))?;
    sync_parent_best_effort(&directory, &path);
    Ok(())
}

fn remove_frame(root: &Path, id: &str) -> Result<(), ImageSourceError> {
    let path = frame_path(root, id);
    match fs::remove_file(&path) {
        Ok(()) => {
            sync_parent_best_effort(&root.join(IMAGE_FRAME_DIRECTORY), &path);
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(storage_error(format!("remove image-source frame: {error}"))),
    }
}

fn frame_path(root: &Path, id: &str) -> PathBuf {
    root.join(IMAGE_FRAME_DIRECTORY).join(format!("{id}.bin"))
}

fn sync_parent_best_effort(parent: &Path, path: &Path) {
    if let Err(error) = secure_file::sync_parent(parent) {
        tracing::warn!(
            path = %path.display(),
            message = %error.source,
            "image-source file was replaced but its directory could not be synced"
        );
    }
}

fn bounded_read_error(subject: &str, error: secure_file::BoundedReadError) -> ImageSourceError {
    match error {
        secure_file::BoundedReadError::Io(error) => file_error(subject, error),
        secure_file::BoundedReadError::TooLarge { maximum } => {
            storage_error(format!("{subject} exceeds the {maximum}-byte limit"))
        }
    }
}

fn file_error(subject: &str, error: secure_file::FileIoError) -> ImageSourceError {
    let (operation, message) = error.into_strings(subject);
    storage_error(format!("{operation}: {message}"))
}

fn storage_error(message: impl Into<String>) -> ImageSourceError {
    ImageSourceError::Io {
        message: message.into(),
    }
}

fn token_digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn decode_digest(encoded: &str) -> Result<[u8; 32], ImageSourceError> {
    if encoded.len() != 64 || !encoded.is_ascii() {
        return Err(storage_error(
            "token digest is not 64 lowercase hexadecimal bytes",
        ));
    }
    let mut digest = [0u8; 32];
    for (output, pair) in digest.iter_mut().zip(encoded.as_bytes().as_chunks::<2>().0) {
        let high = decode_hex_digit(pair[0])?;
        let low = decode_hex_digit(pair[1])?;
        *output = (high << 4) | low;
    }
    Ok(digest)
}

fn decode_hex_digit(byte: u8) -> Result<u8, ImageSourceError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(storage_error("token digest is not lowercase hexadecimal")),
    }
}

/// Generates a random 32-byte token, rendered as 64 lowercase hex characters.
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    protocol::digest_hex(&bytes)
}

fn random_source_id() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let encoded = protocol::digest_hex(&bytes);
    format!("image-{}", &encoded[..SOURCE_ID_RANDOM_HEX_LEN])
}

fn valid_source_id(id: &str) -> bool {
    id.len() == "image-".len() + SOURCE_ID_RANDOM_HEX_LEN
        && id.starts_with("image-")
        && id["image-".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use app_core::config::MAX_IMAGE_SOURCES;
    use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::image_ingest::CanonicalFrame;
    use crate::image_staleness::PUSH_TIME_RING;

    const CANONICAL_FRAME_BYTES: usize = 329_740;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + seconds, 0)
            .single()
            .expect("valid test instant")
    }

    fn canonical_frame(fill: u8) -> CanonicalFrame {
        let bytes = vec![fill; CANONICAL_FRAME_BYTES];
        let digest = Sha256::digest(&bytes).into();
        CanonicalFrame { digest, bytes }
    }

    fn recent_push_times(store: &ImageSourceStore, id: &str) -> Vec<DateTime<Utc>> {
        store
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .sources
            .iter()
            .find(|source| source.id == id)
            .expect("source exists")
            .recent_push_times
            .clone()
    }

    #[test]
    fn a_minted_token_authenticates_and_a_wrong_one_does_not() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = ImageSourceStore::new(temp.path().to_path_buf()).expect("store");
        let source = store.mint("Status panel").expect("mint source");

        assert_eq!(store.authenticate(&source.token), Some(source.id));
        assert_eq!(store.authenticate(&"00".repeat(32)), None);
    }

    #[test]
    fn a_revoked_token_stops_authenticating() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = ImageSourceStore::new(temp.path().to_path_buf()).expect("store");
        let source = store.mint("Status panel").expect("mint source");

        store.revoke(&source.id).expect("revoke source");

        assert_eq!(store.authenticate(&source.token), None);
    }

    #[test]
    fn the_plaintext_token_never_appears_in_the_store_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = ImageSourceStore::new(temp.path().to_path_buf()).expect("store");
        let source = store.mint("Status panel").expect("mint source");

        let persisted = fs::read_to_string(temp.path().join(IMAGE_SOURCE_STORE_FILE))
            .expect("read image source store");
        assert!(!persisted.contains(&source.token));
    }

    #[test]
    fn a_token_still_authenticates_after_the_store_is_reloaded() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = {
            let store = ImageSourceStore::new(temp.path().to_path_buf()).expect("first store");
            store.mint("Status panel").expect("mint source")
        };

        let reloaded = ImageSourceStore::new(temp.path().to_path_buf()).expect("reloaded store");
        assert_eq!(reloaded.authenticate(&source.token), Some(source.id));
    }

    #[test]
    fn the_same_picture_twice_is_accepted_once_but_counted_twice() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = ImageSourceStore::new(temp.path().to_path_buf()).expect("store");
        let source = store.mint("Status panel").expect("mint source");
        let frame = canonical_frame(0x2a);

        assert!(matches!(
            store.accept(&source.id, frame.clone(), at(0)),
            Ok(AcceptOutcome::Changed { digest }) if digest == frame.digest
        ));
        assert!(matches!(
            store.accept(&source.id, frame, at(5)),
            Ok(AcceptOutcome::Unchanged)
        ));
        assert_eq!(recent_push_times(&store, &source.id), vec![at(0), at(5)]);
    }

    #[test]
    fn a_second_push_inside_the_minimum_interval_is_refused() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = ImageSourceStore::new(temp.path().to_path_buf()).expect("store");
        let source = store.mint("Status panel").expect("mint source");

        store
            .accept(&source.id, canonical_frame(1), at(0))
            .expect("first push");
        assert!(matches!(
            store.accept(&source.id, canonical_frame(2), at(4)),
            Err(ImageSourceError::TooSoon)
        ));
        assert!(store.accept(&source.id, canonical_frame(3), at(6)).is_ok());
    }

    #[test]
    fn minting_past_the_maximum_is_refused() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = ImageSourceStore::new(temp.path().to_path_buf()).expect("store");

        for index in 0..MAX_IMAGE_SOURCES {
            store
                .mint(&format!("Source {index}"))
                .expect("mint within capacity");
        }
        assert!(matches!(
            store.mint("One too many"),
            Err(ImageSourceError::Capacity)
        ));
    }

    #[test]
    fn the_ring_keeps_only_the_most_recent_pushes() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = {
            let store = ImageSourceStore::new(temp.path().to_path_buf()).expect("first store");
            let source = store.mint("Status panel").expect("mint source");
            for index in 0..PUSH_TIME_RING + 3 {
                let fill = u8::try_from(index).expect("small bounded ring index");
                let seconds = i64::try_from(index).expect("small bounded ring index") * 5;
                store
                    .accept(&source.id, canonical_frame(fill), at(seconds))
                    .expect("push");
            }
            source
        };

        let reloaded = ImageSourceStore::new(temp.path().to_path_buf()).expect("reloaded store");
        let times = recent_push_times(&reloaded, &source.id);
        assert_eq!(times.len(), PUSH_TIME_RING);
        let newest_index = i64::try_from(PUSH_TIME_RING).expect("small fixed ring") + 2;
        assert_eq!(times.last(), Some(&at(newest_index * 5)));
    }

    #[test]
    fn a_frame_survives_a_restart_byte_for_byte() {
        let temp = tempfile::tempdir().expect("temp dir");
        let expected = canonical_frame(0xa5);
        let source = {
            let store = ImageSourceStore::new(temp.path().to_path_buf()).expect("first store");
            let source = store.mint("Status panel").expect("mint source");
            store
                .accept(&source.id, expected.clone(), at(0))
                .expect("accept frame");
            source
        };

        let reloaded = ImageSourceStore::new(temp.path().to_path_buf()).expect("reloaded store");
        let actual = reloaded.frame(&source.id, at(1)).expect("stored frame");
        assert_eq!(actual.bytes.as_ref(), expected.bytes.as_slice());
        assert_eq!(actual.digest, expected.digest);
    }

    #[test]
    fn a_source_that_has_gone_quiet_reports_stale() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = {
            let store = ImageSourceStore::new(temp.path().to_path_buf()).expect("first store");
            let source = store.mint("Status panel").expect("mint source");
            for index in 0..4 {
                store
                    .accept(
                        &source.id,
                        canonical_frame(index),
                        at(i64::from(index) * 10 * 60),
                    )
                    .expect("push");
            }
            source
        };
        let last_push = at(3 * 10 * 60);
        let reloaded = ImageSourceStore::new(temp.path().to_path_buf()).expect("reloaded store");
        let stored = reloaded
            .frame(&source.id, last_push + ChronoDuration::hours(5))
            .expect("stored frame");
        assert!(stored.stale);
    }

    #[test]
    fn revoking_a_source_drops_its_frame_from_the_desired_set() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = ImageSourceStore::new(temp.path().to_path_buf()).expect("store");
        let source = store.mint("Status panel").expect("mint source");
        store
            .accept(&source.id, canonical_frame(0x5a), at(0))
            .expect("accept frame");
        let frame_path = temp
            .path()
            .join(IMAGE_FRAME_DIRECTORY)
            .join(format!("{}.bin", source.id));
        assert!(frame_path.is_file());

        store.revoke(&source.id).expect("revoke source");

        assert!(store.all_frames(at(1)).is_empty());
        assert!(!frame_path.exists());
    }
}
