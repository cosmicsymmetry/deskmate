//! Durable image-source credentials, canonical frames, and observed push cadence.
//!
//! Tokens follow the device-identity registry's contract: only SHA-256 digests
//! are persisted, and the plaintext is returned exactly once. Frames are read
//! once at startup into `Arc<[u8]>` and stay there, so later asset reconciliation
//! neither re-reads nor re-hashes bytes.

use std::collections::BTreeMap;
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

use crate::credential::{DigestDecodeError, decode_digest as decode_credential_digest};
use crate::credential::{random_token, token_digest};
use crate::image_ingest::{CANONICAL_FRAME_BYTES, CanonicalFrame};
use crate::image_staleness::{PUSH_TIME_RING, is_stale};
use crate::registry::constant_time_eq;

pub(crate) const IMAGE_SOURCE_STORE_FILE: &str = "image-sources.json";
pub(crate) const IMAGE_FRAME_DIRECTORY: &str = "image-frames";
pub(crate) const MIN_PUSH_INTERVAL: Duration = Duration::from_secs(5);

const STORE_SCHEMA_VERSION: u32 = 1;
const MAX_STORE_FILE_BYTES: usize = 64 * 1_024;
const SOURCE_ID_RANDOM_HEX_LEN: usize = 24;
/// Long enough for a readable view name, short enough that a source's frame
/// paths stay well inside any filesystem's component limit.
const MAX_VIEW_ID_LEN: usize = 32;
/// Legacy disk-cache bound, applied before pruning to the resident budget.
const MAX_VIEWS_PER_SOURCE: usize = 16;
/// Reserve a resting frame for every source the config can contain, including
/// sources minted later. The remaining seven slots are shared staged views.
/// This bounds the complete account set even when refreshers run concurrently.
const RESIDENT_FRAME_BUDGET: usize = 15;
const STAGED_VIEW_BUDGET: usize = RESIDENT_FRAME_BUDGET - MAX_IMAGE_SOURCES;
const _: () = assert!(MAX_IMAGE_SOURCES <= protocol::MAX_ASSET_DIGESTS);

pub(crate) struct ImageSourceStore {
    root: PathBuf,
    state: Mutex<ImageSourceState>,
}

/// Supplies the runtime with the complete durable picture keep-set and the
/// current frame for one configured source. The store's `Arc` bytes are passed
/// through unchanged, preserving the digest provenance established at ingest.
pub(crate) struct ServerImageSourceHost {
    store: Arc<ImageSourceStore>,
}

impl ServerImageSourceHost {
    pub(crate) fn new(store: Arc<ImageSourceStore>) -> Self {
        Self { store }
    }
}

impl app_core::ImageSourceHost for ServerImageSourceHost {
    fn desired_assets(&mut self) -> Vec<app_core::DesiredAsset> {
        self.store.desired_assets()
    }

    fn image_source_frame(&mut self, source_id: &str) -> Option<app_core::ImageSourceFrame> {
        self.store.frame(source_id, Utc::now())
    }
}

#[derive(Clone)]
struct ImageSourceState {
    sources: Vec<SourceRecord>,
}

/// What the management surface shows about one image source. Carries no bytes
/// and no credential -- only what a person needs to tell "the producer is
/// broken" from "the integration needs reconnecting".
pub(crate) struct SourceSummary {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) has_frame: bool,
    pub(crate) stale: bool,
    pub(crate) last_push: Option<DateTime<Utc>>,
}

#[derive(Clone)]
struct SourceRecord {
    id: String,
    name: String,
    token_digest: [u8; 32],
    recent_push_times: Vec<DateTime<Utc>>,
    /// One frame per view the face offers, keyed by [`ViewId`]. The resting view
    /// is [`RESTING_VIEW`] and is the only entry an external producer ever
    /// writes.
    frames: BTreeMap<ViewId, StoredFrame>,
    /// Which view the device should be showing. Not persisted: after a restart
    /// the resting view is the right answer, and a volatile frame did not
    /// survive the reboot either.
    selected: ViewId,
}

/// A face's name for one of the pictures it can draw. Opaque to the server
/// except that it becomes part of a filename, which is why [`valid_view_id`]
/// exists.
pub(crate) type ViewId = String;

/// The view every source has, and the only one a producer's POST writes.
///
/// The empty string rather than a word, so a face that declares no views and the
/// frame a source has always had are the same key here and the same path on
/// disk. That is what keeps every existing frame on the live VM readable.
pub(crate) const RESTING_VIEW: &str = "";

/// A view id may become `image-frames/<source>--<view>.bin`, so it is bounded and
/// restricted before it ever reaches the filesystem. The faces package is our
/// own code, but it is also the one input here that is not the server's.
fn valid_view_id(view: &str) -> bool {
    view == RESTING_VIEW
        || (view.len() <= MAX_VIEW_ID_LEN
            && view
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
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
    #[error("the view name is not a valid frame key")]
    InvalidView,
    #[error("the account's resident-frame staging budget has been reached")]
    StagingCapacity,
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
            frames: BTreeMap::new(),
            selected: RESTING_VIEW.to_owned(),
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

        remove_frames(&self.root, &removed.id)
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

    /// Accepts an external producer's frame, no faster than `MIN_PUSH_INTERVAL`.
    pub(crate) fn accept(
        &self,
        id: &str,
        frame: CanonicalFrame,
        now: DateTime<Utc>,
    ) -> Result<AcceptOutcome, ImageSourceError> {
        self.accept_into_view(id, RESTING_VIEW, frame, now, true)
    }

    /// Accepts a frame the SERVER drew, without the producer rate limit.
    ///
    /// `MIN_PUSH_INTERVAL` bounds what an external producer can do to this server
    /// over HTTP. The faces path is not that: one refresher per card renders
    /// strictly in sequence, and a tap can only ask for the next render once the
    /// previous one has finished. Applying the producer limit here would make a
    /// second tap within five seconds draw nothing -- the render happens, the frame
    /// is refused as `TooSoon`, and the panel silently keeps the old picture while the
    /// face's new state is discarded with it. Tapping twice in a row is the normal
    /// way to use a tappable face, so the limit would break the feature it guards.
    pub(crate) fn accept_server_rendered(
        &self,
        id: &str,
        frame: CanonicalFrame,
        now: DateTime<Utc>,
    ) -> Result<AcceptOutcome, ImageSourceError> {
        self.accept_into_view(id, RESTING_VIEW, frame, now, false)
    }

    /// Accepts a frame the SERVER drew for one named view, without the producer
    /// rate limit. Staging a view does not change what the device is showing --
    /// only [`ImageSourceStore::select_view`] does that -- so this returns
    /// whether the frame is new, not whether the panel should be told.
    pub(crate) fn accept_staged_view(
        &self,
        id: &str,
        view: &str,
        frame: CanonicalFrame,
        now: DateTime<Utc>,
    ) -> Result<AcceptOutcome, ImageSourceError> {
        self.accept_into_view(id, view, frame, now, false)
    }

    fn accept_into_view(
        &self,
        id: &str,
        view: &str,
        frame: CanonicalFrame,
        now: DateTime<Utc>,
        rate_limited: bool,
    ) -> Result<AcceptOutcome, ImageSourceError> {
        if !valid_view_id(view) {
            return Err(ImageSourceError::InvalidView);
        }
        let mut state = self.lock();
        let index = state
            .sources
            .iter()
            .position(|source| source.id == id)
            .ok_or(ImageSourceError::UnknownToken)?;
        let source = &state.sources[index];

        if rate_limited && let Some(last) = source.recent_push_times.last() {
            let minimum = chrono::Duration::from_std(MIN_PUSH_INTERVAL)
                .expect("the fixed minimum push interval fits chrono");
            if now - *last < minimum {
                return Err(ImageSourceError::TooSoon);
            }
        }
        if view != RESTING_VIEW
            && !source.frames.contains_key(view)
            && state
                .sources
                .iter()
                .map(|source| source.frames.keys().filter(|view| !view.is_empty()).count())
                .sum::<usize>()
                >= STAGED_VIEW_BUDGET
        {
            return Err(ImageSourceError::StagingCapacity);
        }

        let mut recent_push_times = source.recent_push_times.clone();
        recent_push_times.push(now);
        if recent_push_times.len() > PUSH_TIME_RING {
            recent_push_times.remove(0);
        }

        let unchanged = source
            .frames
            .get(view)
            .is_some_and(|stored| stored.digest == frame.digest);
        let mut candidate = state.clone();
        candidate.sources[index].recent_push_times = recent_push_times;

        if unchanged {
            save_store(&self.root, &candidate)?;
            *state = candidate;
            return Ok(AcceptOutcome::Unchanged);
        }

        write_frame(&self.root, id, view, &frame.bytes)?;
        let digest = frame.digest;
        candidate.sources[index].frames.insert(
            view.to_owned(),
            StoredFrame {
                digest,
                bytes: Arc::from(frame.bytes),
            },
        );
        save_store(&self.root, &candidate)?;
        *state = candidate;
        Ok(AcceptOutcome::Changed { digest })
    }

    /// Points the source at one of its staged views and reports that view's
    /// digest, so the caller can tell the runtime which frame to draw.
    ///
    /// Returns `None` when the view is not staged, which is the ordinary answer
    /// for a view past the card's share of the pool: the caller renders it on
    /// demand instead.
    pub(crate) fn select_view(&self, id: &str, view: &str) -> Option<[u8; 32]> {
        let mut state = self.lock();
        let source = state.sources.iter_mut().find(|source| source.id == id)?;
        let digest = source.frames.get(view)?.digest;
        view.clone_into(&mut source.selected);
        Some(digest)
    }

    /// The frame the device should be showing: the selected view, or the resting
    /// view when the selection has nothing staged behind it.
    pub(crate) fn frame(&self, id: &str, now: DateTime<Utc>) -> Option<app_core::ImageSourceFrame> {
        let state = self.lock();
        let source = state.sources.iter().find(|source| source.id == id)?;
        let frame = source
            .frames
            .get(&source.selected)
            .or_else(|| source.frames.get(RESTING_VIEW))?;
        Some(app_core::ImageSourceFrame {
            digest: frame.digest,
            bytes: Arc::clone(&frame.bytes),
            stale: is_stale(&source.recent_push_times, now),
        })
    }

    /// Liveness metadata for the management surface.
    ///
    /// Deliberately separate from [`ImageSourceStore::desired_assets`], which
    /// clones an `Arc` per frame for the push path: a dashboard that listed
    /// sources through it would hold every canonical frame alive to render a
    /// table of names.
    pub(crate) fn summaries(&self, now: DateTime<Utc>) -> Vec<SourceSummary> {
        self.lock()
            .sources
            .iter()
            .map(|source| SourceSummary {
                id: source.id.clone(),
                name: source.name.clone(),
                has_frame: !source.frames.is_empty(),
                stale: is_stale(&source.recent_push_times, now),
                last_push: source.recent_push_times.last().copied(),
            })
            .collect()
    }

    fn desired_assets(&self) -> Vec<app_core::DesiredAsset> {
        let state = self.lock();
        let mut desired = Vec::new();
        // Every staged view, not only the one on screen: a tap is one PushScene
        // precisely because the frame it names is already resident, and a frame
        // absent from this set is one the runtime refuses to name.
        for frame in state
            .sources
            .iter()
            .flat_map(|source| source.frames.values())
        {
            if desired
                .iter()
                .any(|asset: &app_core::DesiredAsset| asset.digest == frame.digest)
            {
                continue;
            }
            desired.push(app_core::DesiredAsset {
                digest: frame.digest,
                kind: protocol::AssetKind::Image,
                bytes: Arc::clone(&frame.bytes),
            });
        }
        desired
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ImageSourceState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(test)]
    pub(crate) fn forget_staged_view_for_test(&self, id: &str, view: &str) {
        assert!(!view.is_empty());
        self.lock()
            .sources
            .iter_mut()
            .find(|source| source.id == id)
            .unwrap()
            .frames
            .remove(view);
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

        let frames = load_frames(root, &persisted_source.id)?;
        sources.push(SourceRecord {
            id: persisted_source.id,
            name: persisted_source.name,
            token_digest,
            recent_push_times,
            frames,
            selected: RESTING_VIEW.to_owned(),
        });
    }

    // Old servers used a per-source cap, which could exceed fifteen in a large
    // loop. Load resting frames first and retain only the bounded extra cache.
    // Excess cache files remain on disk; no user data or config is deleted.
    let mut remaining = STAGED_VIEW_BUDGET;
    for source in &mut sources {
        source.frames.retain(|view, _| {
            if view.is_empty() {
                return true;
            }
            if remaining == 0 {
                return false;
            }
            remaining -= 1;
            true
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

/// Every frame this source has on disk, keyed by view.
///
/// **This is why a per-view store needs no schema bump.** A frame was never
/// recorded in `image-sources.json` -- the file is probed and the bytes re-hashed
/// -- so extra views are discovered by listing the directory, and a store written
/// before views existed loads as exactly one resting frame.
fn load_frames(root: &Path, id: &str) -> Result<BTreeMap<ViewId, StoredFrame>, ImageSourceError> {
    let mut frames = BTreeMap::new();
    if let Some(frame) = load_frame(root, id, RESTING_VIEW)? {
        frames.insert(RESTING_VIEW.to_owned(), frame);
    }
    let directory = root.join(IMAGE_FRAME_DIRECTORY);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(frames),
        Err(error) => {
            return Err(storage_error(format!(
                "read the image-frame directory: {error}"
            )));
        }
    };
    let prefix = format!("{id}--");
    for entry in entries {
        let entry = entry.map_err(|error| {
            storage_error(format!("read an image-frame directory entry: {error}"))
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(view) = name
            .strip_prefix(&prefix)
            .and_then(|rest| rest.strip_suffix(".bin"))
        else {
            continue;
        };
        // A file whose name is not a view this server would ever write is left
        // alone rather than loaded or deleted: it is not ours to interpret.
        if !valid_view_id(view) || view == RESTING_VIEW {
            continue;
        }
        if frames.len() >= MAX_VIEWS_PER_SOURCE {
            return Err(storage_error(format!(
                "image source {id} has more than {MAX_VIEWS_PER_SOURCE} stored views"
            )));
        }
        if let Some(frame) = load_frame(root, id, view)? {
            frames.insert(view.to_owned(), frame);
        }
    }
    Ok(frames)
}

fn load_frame(root: &Path, id: &str, view: &str) -> Result<Option<StoredFrame>, ImageSourceError> {
    let path = frame_path(root, id, view);
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

fn write_frame(root: &Path, id: &str, view: &str, bytes: &[u8]) -> Result<(), ImageSourceError> {
    let directory = root.join(IMAGE_FRAME_DIRECTORY);
    secure_file::create_directory(&directory)
        .map_err(|error| file_error("image-frame directory", error))?;
    let path = frame_path(root, id, view);
    secure_file::write_and_replace_binary(&path, bytes)
        .map_err(|error| file_error("image-source frame", error))?;
    sync_parent_best_effort(&directory, &path);
    Ok(())
}

/// Removes every view's frame for one source, which is what revoking it means.
fn remove_frames(root: &Path, id: &str) -> Result<(), ImageSourceError> {
    let views: Vec<ViewId> = load_frames(root, id)?.into_keys().collect();
    for view in views {
        let path = frame_path(root, id, &view);
        match fs::remove_file(&path) {
            Ok(()) => sync_parent_best_effort(&root.join(IMAGE_FRAME_DIRECTORY), &path),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(storage_error(format!("remove image-source frame: {error}")));
            }
        }
    }
    Ok(())
}

/// The resting view keeps the path it has always had, so every frame already on
/// the live VM stays where it is and every producer's POST still lands there.
fn frame_path(root: &Path, id: &str, view: &str) -> PathBuf {
    let directory = root.join(IMAGE_FRAME_DIRECTORY);
    if view == RESTING_VIEW {
        directory.join(format!("{id}.bin"))
    } else {
        directory.join(format!("{id}--{view}.bin"))
    }
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

fn decode_digest(encoded: &str) -> Result<[u8; 32], ImageSourceError> {
    match decode_credential_digest(encoded) {
        Ok(digest) => Ok(digest),
        Err(DigestDecodeError::Length) => Err(storage_error(
            "token digest is not 64 lowercase hexadecimal bytes",
        )),
        Err(DigestDecodeError::Character) => {
            Err(storage_error("token digest is not lowercase hexadecimal"))
        }
    }
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

    #[test]
    fn page_four_fits_but_concurrent_staging_reserves_all_eight_resting_frames() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(ImageSourceStore::new(dir.path().to_path_buf()).unwrap());
        let source = store.mint("Four pages").unwrap();
        store
            .accept_server_rendered(&source.id, canonical_frame(1), at(0))
            .unwrap();
        for page in 2..=4 {
            store
                .accept_staged_view(
                    &source.id,
                    &format!("page-{page}"),
                    canonical_frame(page),
                    at(0),
                )
                .unwrap();
        }
        assert!(store.select_view(&source.id, "page-4").is_some());
        let mut tasks = Vec::new();
        for index in 0..12 {
            let store = store.clone();
            let id = source.id.clone();
            tasks.push(std::thread::spawn(move || {
                store.accept_staged_view(
                    &id,
                    &format!("extra-{index}"),
                    canonical_frame(10 + index),
                    at(0),
                )
            }));
        }
        let accepted = tasks
            .into_iter()
            .filter_map(|task| task.join().unwrap().ok())
            .count();
        assert_eq!(accepted, STAGED_VIEW_BUDGET - 3);
        // Staging cannot steal the resting slots of sources minted in the future.
        for index in 1..MAX_IMAGE_SOURCES {
            let source = store.mint(&format!("Future {index}")).unwrap();
            store
                .accept_server_rendered(
                    &source.id,
                    canonical_frame(u8::try_from(30 + index).unwrap()),
                    at(0),
                )
                .unwrap();
        }
        assert_eq!(store.desired_assets().len(), RESIDENT_FRAME_BUDGET);
        // Replacing an existing view consumes no new resident-cache allocation.
        assert!(
            store
                .accept_staged_view(&source.id, "page-4", canonical_frame(90), at(1))
                .is_ok()
        );
        let reloaded = ImageSourceStore::new(dir.path().to_path_buf()).unwrap();
        assert_eq!(reloaded.desired_assets().len(), RESIDENT_FRAME_BUDGET);
        assert!(reloaded.select_view(&source.id, "page-4").is_some());
    }

    #[test]
    fn an_older_oversized_staging_cache_is_bounded_on_load_without_deleting_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = ImageSourceStore::new(dir.path().to_path_buf()).unwrap();
        let mut ids = Vec::new();
        for index in 0..MAX_IMAGE_SOURCES {
            let source = store.mint(&format!("Source {index}")).unwrap();
            let frame = canonical_frame(u8::try_from(index).unwrap());
            store
                .accept_server_rendered(&source.id, frame, at(0))
                .unwrap();
            // Reproduce old on-disk files, independently of today's admission guard.
            for page in 2..=4 {
                write_frame(
                    dir.path(),
                    &source.id,
                    &format!("page-{page}"),
                    &canonical_frame(u8::try_from(20 + index * 4 + page).unwrap()).bytes,
                )
                .unwrap();
            }
            ids.push(source.id);
        }
        let reloaded = ImageSourceStore::new(dir.path().to_path_buf()).unwrap();
        assert_eq!(reloaded.desired_assets().len(), RESIDENT_FRAME_BUDGET);
        for id in &ids {
            assert!(reloaded.frame(id, at(0)).is_some());
            assert!(frame_path(dir.path(), id, "page-4").exists());
        }
    }

    #[test]
    fn one_sources_legacy_cache_accepts_nine_through_sixteen_disk_frames() {
        for count in 9..=16 {
            let dir = tempfile::tempdir().unwrap();
            let store = ImageSourceStore::new(dir.path().to_path_buf()).unwrap();
            let source = store.mint("Legacy pages").unwrap();
            store
                .accept_server_rendered(&source.id, canonical_frame(0), at(0))
                .unwrap();
            for page in 1..count {
                write_frame(
                    dir.path(),
                    &source.id,
                    &format!("page-{page:02}"),
                    &canonical_frame(page).bytes,
                )
                .unwrap();
            }
            let reloaded = ImageSourceStore::new(dir.path().to_path_buf()).unwrap();
            assert_eq!(reloaded.desired_assets().len(), 1 + STAGED_VIEW_BUDGET);
            assert_eq!(
                reloaded.frame(&source.id, at(0)).unwrap().digest,
                canonical_frame(0).digest
            );
            for page in 1..count {
                assert!(frame_path(dir.path(), &source.id, &format!("page-{page:02}")).exists());
            }
        }
    }

    #[test]
    fn a_server_rendered_frame_is_not_held_to_the_producer_rate_limit() {
        // Two taps two seconds apart are ordinary use. Refusing the second as
        // TooSoon would leave the panel showing the old page with no error anywhere
        // the owner can see, and would throw away the face's advanced state too.
        let temp = tempfile::tempdir().expect("temp dir");
        let store = ImageSourceStore::new(temp.path().to_path_buf()).expect("store");
        let source = store.mint("Status panel").expect("mint source");
        assert!(
            store
                .accept_server_rendered(&source.id, canonical_frame(1), at(0))
                .is_ok()
        );
        assert!(
            store
                .accept_server_rendered(&source.id, canonical_frame(2), at(2))
                .is_ok()
        );
        // An external producer POSTing at the same cadence is still refused.
        assert_eq!(
            store.accept(&source.id, canonical_frame(3), at(3)),
            Err(ImageSourceError::TooSoon)
        );
    }

    #[test]
    fn summaries_report_liveness_without_carrying_frame_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ImageSourceStore::new(dir.path().to_path_buf()).expect("store");
        let minted = store.mint("kitchen").expect("mint");

        let summaries = store.summaries(Utc::now());

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, minted.id);
        assert_eq!(summaries[0].name, "kitchen");
        assert!(!summaries[0].has_frame, "nothing has been pushed yet");
        assert_eq!(summaries[0].last_push, None);
        assert!(
            !summaries[0].stale,
            "a source with no push history has no deadline to be past"
        );
    }
    use std::fs;

    use app_core::ImageSourceHost as _;
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

        assert!(store.desired_assets().is_empty());
        assert!(!frame_path.exists());
    }

    #[test]
    fn host_desired_assets_keep_first_digest_order_bytes_and_arcs_including_stale_frames() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(ImageSourceStore::new(temp.path().to_path_buf()).expect("store"));
        let first = store.mint("First").expect("mint first");
        let duplicate = store.mint("Duplicate").expect("mint duplicate");
        let distinct = store.mint("Distinct").expect("mint distinct");
        let _unpushed = store.mint("Unpushed").expect("mint unpushed");
        let shared = canonical_frame(0x31);
        for second in [0, 5, 10, 15] {
            store
                .accept(&first.id, shared.clone(), at(second))
                .expect("record first-source cadence");
        }
        store
            .accept(&duplicate.id, shared.clone(), at(0))
            .expect("accept duplicate");
        let other = canonical_frame(0xa7);
        store
            .accept(&distinct.id, other.clone(), at(0))
            .expect("accept distinct");

        let retained = store.frame(&first.id, at(3_600)).expect("first frame");
        assert!(
            retained.stale,
            "the stale frame must remain in the keep-set"
        );
        let mut host = ServerImageSourceHost::new(Arc::clone(&store));
        let desired = host.desired_assets();

        assert_eq!(
            desired.len(),
            2,
            "duplicate and unpushed sources add nothing"
        );
        assert_eq!(desired[0].digest, shared.digest);
        assert_eq!(desired[0].kind, protocol::AssetKind::Image);
        assert_eq!(desired[0].bytes.as_ref(), shared.bytes.as_slice());
        assert!(Arc::ptr_eq(&desired[0].bytes, &retained.bytes));
        assert_eq!(desired[1].digest, other.digest);
        assert_eq!(desired[1].kind, protocol::AssetKind::Image);
        assert_eq!(desired[1].bytes.as_ref(), other.bytes.as_slice());
    }

    #[test]
    fn revoking_the_first_duplicate_keeps_the_surviving_digest_desired() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(ImageSourceStore::new(temp.path().to_path_buf()).expect("store"));
        let first = store.mint("First").expect("mint first");
        let duplicate = store.mint("Duplicate").expect("mint duplicate");
        let shared = canonical_frame(0x42);
        store
            .accept(&first.id, shared.clone(), at(0))
            .expect("accept first");
        store
            .accept(&duplicate.id, shared.clone(), at(0))
            .expect("accept duplicate");
        let surviving = store.frame(&duplicate.id, at(1)).expect("duplicate frame");

        store.revoke(&first.id).expect("revoke first duplicate");

        let mut host = ServerImageSourceHost::new(store);
        let desired = host.desired_assets();
        assert_eq!(desired.len(), 1);
        assert_eq!(desired[0].digest, shared.digest);
        assert_eq!(desired[0].bytes.as_ref(), shared.bytes.as_slice());
        assert!(Arc::ptr_eq(&desired[0].bytes, &surviving.bytes));
    }
}
