//! Server-rendered data cards: the server acting as its own picture producer.
//!
//! A weather, Hacker News, RSS or token face belongs to an ordinary picture card.
//! Each is an image source the server pushes to itself: a refresh task asks the
//! faces package for a frame and hands it to
//! [`crate::image_sources::ImageSourceStore::accept`]. Delivery then uses the
//! same durable assets, device notifications and staleness inference as an
//! external producer's PNG.
//!
//! # What lives here and what does not
//!
//! The server owns the *bookkeeping*: the persisted specs, the settings contract
//! the browser edits them through, validation, and the refresh schedule. It does
//! not know what any face fetches or how it is drawn. That is
//! `companion/faces/` -- TypeScript, run as a subprocess through
//! [`faces_package`] -- and it is why this file names no face kind: the catalog
//! comes from the package's `describe`, so adding a face is adding a file there.
//! No card kind, no schema change, no Rust change, no binary redeploy.
//!
//! Settings absent from a face's descriptor (a token's `api_key`) can be edited in
//! the spec file while the server is stopped; they are passed through untouched.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::ServerState;

mod face_state;
mod faces_package;
mod worker;

use face_state::FaceStateStore;
pub use faces_package::FaceCommand;
use faces_package::{CatalogFace, CatalogField};

/// A field value arrives over an internet-facing admin route. This comfortably
/// covers feed URLs while preventing a tiny settings document from becoming an
/// unbounded allocation surface.
const MAX_FACE_FIELD_VALUE_BYTES: usize = 2_048;

/// How often the catalog is re-read. The faces directory is rsync'd like the web
/// `dist/`, so a new face appears in the add menu without a restart.
const CATALOG_RELOAD: Duration = Duration::from_secs(60);

/// One server-rendered card.
///
/// The face config is a nested object rather than flattened into this struct,
/// and that is not a style choice: serde's `deny_unknown_fields` and
/// `flatten` do not work together -- the outer struct rejects every flattened
/// field as unknown. Nesting costs one level of braces and keeps the outer
/// struct strict, so `"refresh_second"` does not quietly become the default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DataCardSpec {
    /// The image source this card pushes to, as `POST /v1/images` minted it.
    /// The owner's picture card names the same id.
    source_id: String,
    #[serde(default = "default_refresh_seconds")]
    refresh_seconds: u64,
    face: FaceSpec,
}

const fn default_refresh_seconds() -> u64 {
    900
}

/// Which face, and the settings it was given.
///
/// Open-ended on purpose. The file shape is unchanged from when this was a closed
/// Rust enum -- `{"kind": "token", "coin_id": "solana", ...}` -- so a spec file
/// written by an older server loads without a migration. What changed is who
/// checks the settings: a mistyped key is no longer a serde refusal at startup
/// but the faces package's complaint at the first refresh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FaceSpec {
    kind: String,
    #[serde(flatten)]
    settings: BTreeMap<String, serde_json::Value>,
}

/// The server-owned settings contract rendered by the companion. The app treats
/// `kind` as opaque metadata and renders only this descriptor's field types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct FaceDescriptor {
    pub(crate) kind: String,
    pub(crate) label: String,
    /// What the window tells the owner a tap does. Absent means taps are ignored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tap: Option<String>,
    pub(crate) fields: Vec<FaceFieldDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum FaceFieldDescriptor {
    Text {
        key: String,
        label: String,
        value: String,
        placeholder: String,
    },
    Url {
        key: String,
        label: String,
        value: String,
        placeholder: String,
    },
    Enum {
        key: String,
        label: String,
        value: String,
        options: Vec<FaceFieldOption>,
    },
}

impl FaceFieldDescriptor {
    fn key(&self) -> &str {
        match self {
            Self::Text { key, .. } | Self::Url { key, .. } | Self::Enum { key, .. } => key,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FaceFieldOption {
    pub(crate) value: String,
    pub(crate) label: String,
}

/// What the owner needs to know about one face right now, in the window, without
/// reading the server's journal.
///
/// Until 2026-09-21 a face that could not be drawn said so only in a log line on the
/// VM. The panel showed "Waiting for the first picture", the window showed "Saved to
/// the server", and a coin ID typed as a ticker looked exactly like a broken device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct FaceStatus {
    pub(crate) state: FaceState,
    /// The faces package's own sentence, for the two states that have one.
    pub(crate) message: Option<String>,
    /// When the last refresh finished, whatever its outcome, in Unix seconds.
    pub(crate) at_unix_seconds: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FaceState {
    /// A required field is blank, so nothing is being fetched.
    NeedsSettings,
    /// A refresher is running and has not finished its first attempt.
    Drawing,
    /// The last refresh produced a frame.
    Drawn,
    /// The faces package refused the settings; retrying cannot help.
    NeedsAttention,
    /// The last refresh failed for a reason that may pass; the stored frame stays.
    Retrying,
    /// The faces package does not draw this kind (or there is no package).
    Unavailable,
}

/// How one refresh ended, as the worker reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RefreshOutcome {
    Drawn,
    NeedsAttention(String),
    Retrying(String),
}

/// A catalog face's descriptor with `settings` filled in: what the owner typed
/// where they typed something, the field's default where they did not.
fn describe_face(
    face: &CatalogFace,
    settings: &BTreeMap<String, serde_json::Value>,
) -> FaceDescriptor {
    let value_of = |field: &CatalogField| {
        settings
            .get(field.key())
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| field.default_value())
            .to_owned()
    };
    FaceDescriptor {
        kind: face.kind.clone(),
        label: face.label.clone(),
        tap: face.tap.clone(),
        fields: face
            .fields
            .iter()
            .map(|field| match field {
                CatalogField::Text {
                    key,
                    label,
                    placeholder,
                    ..
                } => FaceFieldDescriptor::Text {
                    key: key.clone(),
                    label: label.clone(),
                    value: value_of(field),
                    placeholder: placeholder.clone(),
                },
                CatalogField::Url {
                    key,
                    label,
                    placeholder,
                    ..
                } => FaceFieldDescriptor::Url {
                    key: key.clone(),
                    label: label.clone(),
                    value: value_of(field),
                    placeholder: placeholder.clone(),
                },
                CatalogField::Enum {
                    key,
                    label,
                    options,
                    ..
                } => FaceFieldDescriptor::Enum {
                    key: key.clone(),
                    label: label.clone(),
                    value: value_of(field),
                    options: options.clone(),
                },
            })
            .collect(),
    }
}

/// A blank face of a catalog kind: every field at its default.
fn blank_face(face: &CatalogFace) -> FaceSpec {
    FaceSpec {
        kind: face.kind.clone(),
        settings: face
            .fields
            .iter()
            .map(|field| {
                (
                    field.key().to_owned(),
                    serde_json::Value::String(field.default_value().to_owned()),
                )
            })
            .collect(),
    }
}

/// Whether a face has everything it needs to be fetched.
///
/// A card added from the menu starts blank, and a blank weather face has no city
/// to ask about. Fetching anyway would fail every cycle and fill the journal with
/// errors the owner cannot act on until they type one. So an incomplete face is
/// stored but not refreshed, and the card shows the ordinary "no frame yet"
/// state until its settings are filled in. A face whose fields all have defaults
/// -- Hacker News -- is complete the moment it is created.
fn face_is_complete(descriptor: &FaceDescriptor) -> bool {
    descriptor.fields.iter().all(|field| match field {
        FaceFieldDescriptor::Text { value, .. } | FaceFieldDescriptor::Url { value, .. } => {
            !value.trim().is_empty()
        }
        FaceFieldDescriptor::Enum { .. } => true,
    })
}

/// Server-owned data-card state. The path, parsed specs, and every live task
/// stay together so an update can atomically replace the persisted spec and the
/// one refresher that consumes it.
pub(crate) struct DataCardState {
    spec_path: PathBuf,
    face_state: Arc<FaceStateStore>,
    specs: Vec<DataCardSpec>,
    tasks: HashMap<String, tokio::task::JoinHandle<()>>,
    /// How to run the faces package. `None` means this deployment has none, and
    /// then there is no catalog, no add-menu entry and no refresher.
    faces: Option<FaceCommand>,
    /// `None` until first needed. Production loads it at startup, before the
    /// listener binds, so no request handler ever pays for the subprocess.
    catalog: Option<Arc<Vec<CatalogFace>>>,
    catalog_reloader: Option<tokio::task::JoinHandle<()>>,
    /// The last refresh outcome per source. Forgotten whenever that source's
    /// refresher is replaced or removed, so a status never describes settings that
    /// are no longer the ones in force.
    outcomes: HashMap<String, (RefreshOutcome, chrono::DateTime<chrono::Utc>)>,
}

impl DataCardState {
    pub(crate) fn new(spec_path: PathBuf, faces: Option<FaceCommand>) -> Self {
        let face_state = Arc::new(FaceStateStore::load(
            spec_path.with_file_name("face-state.json"),
        ));
        Self {
            spec_path,
            face_state,
            specs: Vec::new(),
            tasks: HashMap::new(),
            faces,
            catalog: None,
            catalog_reloader: None,
            outcomes: HashMap::new(),
        }
    }

    fn catalog(&mut self) -> Arc<Vec<CatalogFace>> {
        if self.catalog.is_none() {
            self.catalog = Some(Arc::new(load_catalog(self.faces.as_ref())));
        }
        Arc::clone(self.catalog.as_ref().expect("just loaded"))
    }

    /// The descriptor for one spec, or `None` when the package no longer draws
    /// that kind -- the spec is kept, because the package can come back.
    fn descriptor(&mut self, face: &FaceSpec) -> Option<FaceDescriptor> {
        self.catalog()
            .iter()
            .find(|candidate| candidate.kind == face.kind)
            .map(|candidate| describe_face(candidate, &face.settings))
    }

    /// Starts the refresher for `spec` if its face is complete and drawable.
    fn start_if_complete(
        &mut self,
        runtime: &tokio::runtime::Handle,
        state: &ServerState,
        spec: &DataCardSpec,
    ) {
        let Some(faces) = self.faces.clone() else {
            return;
        };
        match self.descriptor(&spec.face) {
            Some(descriptor) if face_is_complete(&descriptor) => {
                self.tasks.insert(
                    spec.source_id.clone(),
                    worker::spawn_refresher(
                        runtime,
                        state.clone(),
                        faces,
                        Arc::clone(&self.face_state),
                        spec.clone(),
                    ),
                );
            }
            // A card added from the menu is stored blank and stays unfetched until
            // its settings are filled in, so restarting must not start it either.
            Some(_) => tracing::info!(
                source_id = %spec.source_id,
                "a server-rendered card has no settings yet and is not being fetched"
            ),
            None => tracing::warn!(
                source_id = %spec.source_id,
                kind = %spec.face.kind,
                "the faces package does not draw this kind; the card is kept but not refreshed"
            ),
        }
    }

    #[cfg(test)]
    pub(crate) fn insert_task_for_test(
        &mut self,
        source_id: String,
        task: tokio::task::JoinHandle<()>,
    ) {
        self.tasks.insert(source_id, task);
    }

    #[cfg(test)]
    pub(crate) fn spec_source_ids_for_test(&self) -> impl Iterator<Item = &str> {
        self.specs.iter().map(|spec| spec.source_id.as_str())
    }

    #[cfg(test)]
    pub(crate) fn has_task_for_test(&self, source_id: &str) -> bool {
        self.tasks.contains_key(source_id)
    }
}

/// Tells the server how to run the faces package. Called once at startup, before
/// [`start_data_cards`]; a deployment that never calls it has no server faces.
pub fn set_faces(state: &ServerState, faces: FaceCommand) {
    let mut data_cards = state
        .inner
        .data_cards
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    data_cards.faces = Some(faces);
    data_cards.catalog = None;
}

#[cfg(test)]
pub(crate) fn fake_faces() -> FaceCommand {
    faces_package::fake()
}

/// Reads the catalog, or returns an empty one and says why. An unreadable
/// catalog is not fatal: the device link, the clock and every external producer
/// work without it, and refusing to start would take them down with it.
fn load_catalog(faces: Option<&FaceCommand>) -> Vec<CatalogFace> {
    let Some(faces) = faces else {
        return Vec::new();
    };
    match faces_package::describe(faces) {
        Ok(catalog) => catalog,
        Err(error) => {
            tracing::error!(target: "server::data_cards", %error,
                "the faces package could not be read; no server-rendered face is available");
            Vec::new()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum FaceUpdateError {
    #[error("this image source is fed by an external producer and has no server settings")]
    NoFace,
    #[error("unknown face field {0:?}")]
    UnknownField(String),
    #[error("{field}: {message}")]
    InvalidField { field: String, message: String },
    #[error("the data-card spec could not be encoded: {0}")]
    Encode(String),
    #[error("the data-card spec could not be saved: {0}")]
    Persist(String),
}

/// Loads the server-owned specs before listener binding and starts their refreshers.
///
/// # Errors
/// Returns the existing load error when the spec file cannot be read or parsed.
pub fn start_data_cards(state: &ServerState, spec_path: PathBuf) -> Result<(), String> {
    let specs = load_specs(&spec_path)?;
    if specs.is_empty() {
        tracing::info!(target: "server", "no server-rendered data cards configured");
    }
    spawn_refreshers(state, spec_path, specs);
    Ok(())
}

/// Reads the spec file, or returns nothing when there is none.
///
/// A missing file is the ordinary case -- most deployments have no
/// server-rendered cards -- so it is not an error. A *malformed* file is, and
/// loudly: silently running with zero cards because a comma was missing would
/// present as "the panel stopped updating" with nothing in the log.
fn load_specs(path: &Path) -> Result<Vec<DataCardSpec>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let specs: Vec<DataCardSpec> =
        serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut source_ids = HashSet::with_capacity(specs.len());
    for spec in &specs {
        if spec.source_id.trim().is_empty() {
            return Err(format!("{}: a spec has an empty source_id", path.display()));
        }
        if spec.face.kind.trim().is_empty() {
            return Err(format!(
                "{}: the spec for {:?} has an empty face kind",
                path.display(),
                spec.source_id
            ));
        }
        if !source_ids.insert(spec.source_id.as_str()) {
            return Err(format!(
                "{}: source_id {:?} has more than one spec",
                path.display(),
                spec.source_id
            ));
        }
    }
    Ok(specs)
}

/// Retains the spec path, parsed specs, and one task handle per source in
/// [`ServerState`]. Each card gets its own task so a slow feed cannot delay a
/// token price, and retaining the handles lets a settings update replace only
/// the source it changed.
fn spawn_refreshers(state: &ServerState, spec_path: PathBuf, specs: Vec<DataCardSpec>) {
    // Drop specs whose image source no longer exists, before anything is
    // spawned for them. Until `revoke_source` learned to remove a face, every
    // revoke left its spec behind and its refresher fetching on schedule: the
    // live server was found with eight specs for five sources, four of them
    // fetching weather every fifteen minutes for sources gone for days, each
    // failing with "unknown or revoked image source". A spec with no source can
    // do nothing but fail, so this is self-healing rather than a migration --
    // and it is cheap, running once at startup.
    let live: std::collections::BTreeSet<String> = state
        .image_sources()
        .summaries(chrono::Utc::now())
        .into_iter()
        .map(|source| source.id)
        .collect();
    let (specs, orphaned): (Vec<DataCardSpec>, Vec<DataCardSpec>) = specs
        .into_iter()
        .partition(|spec| live.contains(&spec.source_id));
    for spec in &orphaned {
        tracing::warn!(
            source_id = %spec.source_id,
            "dropping a server-rendered card whose image source no longer exists"
        );
    }
    if !orphaned.is_empty()
        && let Ok(bytes) = serde_json::to_vec_pretty(&specs)
        && let Err(error) = app_core::secure_file::write_and_replace(&spec_path, &bytes)
    {
        let (operation, message) = error.into_strings("data-card spec");
        // Not fatal: the in-memory set below is already correct, so this start
        // is healthy either way and the next one tries again.
        tracing::warn!(%operation, %message, "could not persist the pruned data-card specs");
    }

    let runtime = tokio::runtime::Handle::current();
    let mut data_cards = state
        .inner
        .data_cards
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (_, task) in data_cards.tasks.drain() {
        task.abort();
    }
    data_cards.outcomes.clear();
    data_cards.face_state = Arc::new(FaceStateStore::load(
        spec_path.with_file_name("face-state.json"),
    ));
    data_cards.spec_path = spec_path;
    data_cards.specs = specs;
    // Read fresh at every start, here, so no request handler pays for it later.
    data_cards.catalog = None;
    for spec in data_cards.specs.clone() {
        data_cards.start_if_complete(&runtime, state, &spec);
    }

    if let Some(reloader) = data_cards.catalog_reloader.take() {
        reloader.abort();
    }
    if let Some(faces) = data_cards.faces.clone() {
        data_cards.catalog_reloader = Some(runtime.spawn(reload_catalog(state.clone(), faces)));
    }
}

/// Re-reads the catalog on an interval, off the lock, and swaps it in only when
/// the read succeeded -- a package caught mid-rsync must not empty the add menu.
async fn reload_catalog(state: ServerState, faces: FaceCommand) {
    let mut ticker = tokio::time::interval(CATALOG_RELOAD);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await;
    loop {
        ticker.tick().await;
        let command = faces.clone();
        let Ok(Ok(catalog)) =
            tokio::task::spawn_blocking(move || faces_package::describe(&command)).await
        else {
            continue;
        };
        state
            .inner
            .data_cards
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .catalog = Some(Arc::new(catalog));
    }
}

/// Stops every retained data-card refresher. Called from the server's existing
/// graceful shutdown path before the runtime state is released.
pub(crate) fn stop_refreshers(state: &ServerState) {
    let mut data_cards = state
        .inner
        .data_cards
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (_, task) in data_cards.tasks.drain() {
        task.abort();
    }
    if let Some(reloader) = data_cards.catalog_reloader.take() {
        reloader.abort();
    }
}

/// The faces this server can create, as blank defaults.
///
/// The companion's add menu is built from this list, which is why the menu can
/// offer "Weather" without the app knowing what a weather face is -- and why a
/// face added to `companion/faces/` appears in the window with no app change.
#[must_use]
pub(crate) fn creatable_faces(state: &ServerState) -> Vec<FaceDescriptor> {
    state
        .inner
        .data_cards
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .catalog()
        .iter()
        .map(|face| describe_face(face, &BTreeMap::new()))
        .collect()
}

pub(crate) fn descriptor_for_source(
    state: &ServerState,
    source_id: &str,
) -> Option<FaceDescriptor> {
    let mut data_cards = state
        .inner
        .data_cards
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let face = data_cards
        .specs
        .iter()
        .find(|spec| spec.source_id == source_id)?
        .face
        .clone();
    data_cards.descriptor(&face)
}

/// Whether `source_id` belongs to a server-rendered face that declares a tap.
#[must_use]
#[allow(dead_code)] // Task 6 consumes this when it installs the server's CardTapSink.
pub(crate) fn face_takes_taps(state: &ServerState, source_id: &str) -> bool {
    descriptor_for_source(state, source_id).is_some_and(|face| face.tap.is_some())
}

/// What to tell the owner about `source_id`'s face, or `None` for a source that has
/// no server face (an external producer's).
pub(crate) fn status_for_source(state: &ServerState, source_id: &str) -> Option<FaceStatus> {
    let mut data_cards = state
        .inner
        .data_cards
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let face = data_cards
        .specs
        .iter()
        .find(|spec| spec.source_id == source_id)?
        .face
        .clone();
    let plain = |state| FaceStatus {
        state,
        message: None,
        at_unix_seconds: None,
    };
    let Some(descriptor) = data_cards.descriptor(&face) else {
        return Some(plain(FaceState::Unavailable));
    };
    if !face_is_complete(&descriptor) {
        return Some(plain(FaceState::NeedsSettings));
    }
    Some(match data_cards.outcomes.get(source_id) {
        None if data_cards.tasks.contains_key(source_id) => plain(FaceState::Drawing),
        // Complete, drawable, and nothing running: a server with no runtime to spawn
        // on, which in practice is only a unit test.
        None => plain(FaceState::Unavailable),
        Some((RefreshOutcome::Drawn, at)) => FaceStatus {
            state: FaceState::Drawn,
            message: None,
            at_unix_seconds: Some(at.timestamp()),
        },
        Some((RefreshOutcome::NeedsAttention(message), at)) => FaceStatus {
            state: FaceState::NeedsAttention,
            message: Some(message.clone()),
            at_unix_seconds: Some(at.timestamp()),
        },
        Some((RefreshOutcome::Retrying(message), at)) => FaceStatus {
            state: FaceState::Retrying,
            message: Some(message.clone()),
            at_unix_seconds: Some(at.timestamp()),
        },
    })
}

/// Called by a refresher when one refresh finishes. Ignored for a source whose
/// refresher has since been replaced or removed: an aborted task can still be
/// inside its blocking render, and its verdict is about settings no longer in force.
pub(super) fn record_outcome(
    state: &ServerState,
    source_id: &str,
    task: tokio::task::Id,
    outcome: RefreshOutcome,
) {
    let mut data_cards = state
        .inner
        .data_cards
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if data_cards
        .tasks
        .get(source_id)
        .map(tokio::task::JoinHandle::id)
        == Some(task)
    {
        data_cards
            .outcomes
            .insert(source_id.to_owned(), (outcome, chrono::Utc::now()));
    }
}

/// Validates and persists one face update, then aborts that source's refresher
/// and replaces it only when complete. The descriptor drives validation, so this
/// function knows field *types* and nothing about any face.
pub(crate) fn update_face_fields(
    state: &ServerState,
    runtime: &tokio::runtime::Handle,
    source_id: &str,
    fields: &BTreeMap<String, String>,
) -> Result<FaceDescriptor, FaceUpdateError> {
    let mut data_cards = state
        .inner
        .data_cards
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let index = data_cards
        .specs
        .iter()
        .position(|spec| spec.source_id == source_id)
        .ok_or(FaceUpdateError::NoFace)?;
    let current = data_cards.specs[index].face.clone();
    let descriptor = data_cards
        .descriptor(&current)
        .ok_or(FaceUpdateError::NoFace)?;
    validate_face_fields(&descriptor, fields)?;

    let mut updated_specs = data_cards.specs.clone();
    for (key, value) in fields {
        updated_specs[index]
            .face
            .settings
            .insert(key.clone(), serde_json::Value::String(value.clone()));
    }
    persist_specs(&data_cards.spec_path, &updated_specs)?;

    let updated_spec = updated_specs[index].clone();
    data_cards.specs = updated_specs;
    if let Some(task) = data_cards.tasks.remove(source_id) {
        task.abort();
    }
    data_cards.outcomes.remove(source_id);
    data_cards.start_if_complete(runtime, state, &updated_spec);
    data_cards
        .descriptor(&updated_spec.face)
        .ok_or(FaceUpdateError::NoFace)
}

/// Attaches a blank face of `kind` to a freshly minted source.
///
/// Called right after `POST /v1/images` mints the source, so the companion's
/// "Weather" menu item is one round trip: mint, attach, add a card. The spec is
/// persisted immediately -- a face that exists only in memory would vanish on the
/// next restart and leave a card pointing at a source nothing draws. A face that
/// is complete at birth starts refreshing here; there is no later save to do it.
pub(crate) fn create_face(
    state: &ServerState,
    source_id: &str,
    kind: &str,
) -> Result<FaceDescriptor, FaceUpdateError> {
    let mut data_cards = state
        .inner
        .data_cards
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let face = data_cards
        .catalog()
        .iter()
        .find(|candidate| candidate.kind == kind)
        .map(blank_face)
        .ok_or_else(|| FaceUpdateError::UnknownField(kind.to_owned()))?;
    if data_cards
        .specs
        .iter()
        .any(|spec| spec.source_id == source_id)
    {
        return Err(FaceUpdateError::InvalidField {
            field: "source_id".into(),
            message: "this source already has a face".into(),
        });
    }

    let spec = DataCardSpec {
        source_id: source_id.to_owned(),
        refresh_seconds: default_refresh_seconds(),
        face,
    };
    let mut updated_specs = data_cards.specs.clone();
    updated_specs.push(spec.clone());
    persist_specs(&data_cards.spec_path, &updated_specs)?;
    data_cards.specs = updated_specs;

    // Blocking-pool threads keep the runtime context, so this finds the handle
    // from inside `spawn_blocking`. A plain thread has none, and then the face
    // simply waits for the next start like any other.
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        data_cards.start_if_complete(&runtime, state, &spec);
    }
    data_cards
        .descriptor(&spec.face)
        .ok_or_else(|| FaceUpdateError::UnknownField(kind.to_owned()))
}

/// Forgets a source's face: drops the persisted spec and stops its refresher.
///
/// Idempotent: a source with no face is not an error, because most sources are
/// fed by an external producer and never had one.
pub(crate) fn remove_face(state: &ServerState, source_id: &str) -> Result<(), FaceUpdateError> {
    let mut data_cards = state
        .inner
        .data_cards
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Revocation has already removed the source, so its task must stop even
    // when the durable spec replacement fails and needs an internal retry.
    if let Some(task) = data_cards.tasks.remove(source_id) {
        task.abort();
    }
    data_cards.outcomes.remove(source_id);
    if !data_cards
        .specs
        .iter()
        .any(|spec| spec.source_id == source_id)
    {
        data_cards.face_state.remove(source_id);
        return Ok(());
    }

    let updated_specs: Vec<DataCardSpec> = data_cards
        .specs
        .iter()
        .filter(|spec| spec.source_id != source_id)
        .cloned()
        .collect();
    persist_specs(&data_cards.spec_path, &updated_specs)?;
    // Only replace the in-memory specs after persistence succeeds, retaining
    // the original spec for a retry if the write fails.
    data_cards.specs = updated_specs;
    data_cards.face_state.remove(source_id);
    Ok(())
}

fn persist_specs(path: &Path, specs: &[DataCardSpec]) -> Result<(), FaceUpdateError> {
    let bytes = serde_json::to_vec_pretty(specs)
        .map_err(|error| FaceUpdateError::Encode(error.to_string()))?;
    app_core::secure_file::write_and_replace(path, &bytes).map_err(|error| {
        let (operation, message) = error.into_strings("data-card spec");
        FaceUpdateError::Persist(format!("{operation}: {message}"))
    })
}

fn validate_face_fields(
    descriptor: &FaceDescriptor,
    fields: &BTreeMap<String, String>,
) -> Result<(), FaceUpdateError> {
    for (key, value) in fields {
        let field = descriptor
            .fields
            .iter()
            .find(|field| field.key() == key)
            .ok_or_else(|| FaceUpdateError::UnknownField(key.clone()))?;
        if value.len() > MAX_FACE_FIELD_VALUE_BYTES {
            return Err(FaceUpdateError::InvalidField {
                field: key.clone(),
                message: format!("must be at most {MAX_FACE_FIELD_VALUE_BYTES} UTF-8 bytes"),
            });
        }
        if value.chars().any(char::is_control) {
            return Err(FaceUpdateError::InvalidField {
                field: key.clone(),
                message: "must not contain control characters".into(),
            });
        }
        if value.trim().is_empty() {
            return Err(FaceUpdateError::InvalidField {
                field: key.clone(),
                message: "must not be empty".into(),
            });
        }
        match field {
            FaceFieldDescriptor::Text { .. } => {}
            FaceFieldDescriptor::Url { .. } => {
                let parsed = url::Url::parse(value).map_err(|_| FaceUpdateError::InvalidField {
                    field: key.clone(),
                    message: "must be an absolute HTTP or HTTPS URL".into(),
                })?;
                if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
                    return Err(FaceUpdateError::InvalidField {
                        field: key.clone(),
                        message: "must be an absolute HTTP or HTTPS URL".into(),
                    });
                }
            }
            FaceFieldDescriptor::Enum { options, .. } => {
                if !options.iter().any(|option| option.value == *value) {
                    return Err(FaceUpdateError::InvalidField {
                        field: key.clone(),
                        message: "has an unsupported value".into(),
                    });
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A face of `kind` with exactly these settings -- the shape a spec file holds.
    fn face(kind: &str, settings: serde_json::Value) -> FaceSpec {
        let serde_json::Value::Object(settings) = settings else {
            panic!("settings are an object");
        };
        FaceSpec {
            kind: kind.to_owned(),
            settings: settings.into_iter().collect(),
        }
    }

    fn weather(location: &str) -> FaceSpec {
        face(
            "weather",
            serde_json::json!({"location": location, "units": "metric"}),
        )
    }

    /// A blank face of `kind`, as the add menu would create it.
    fn blank(kind: &str) -> FaceSpec {
        let catalog = faces_package::describe(&fake_faces()).expect("the fake catalog");
        blank_face(
            catalog
                .iter()
                .find(|face| face.kind == kind)
                .expect("a known kind"),
        )
    }

    fn descriptor_of(face: &FaceSpec) -> FaceDescriptor {
        let catalog = faces_package::describe(&fake_faces()).expect("the fake catalog");
        describe_face(
            catalog
                .iter()
                .find(|candidate| candidate.kind == face.kind)
                .expect("a known kind"),
            &face.settings,
        )
    }

    fn write(contents: &str) -> tempfile::TempDir {
        let directory = tempfile::tempdir().expect("a temp directory");
        std::fs::write(directory.path().join("cards.json"), contents).expect("the spec is written");
        directory
    }

    #[test]
    fn a_missing_spec_file_is_no_cards_rather_than_an_error() {
        // The ordinary deployment. An error here would make every server
        // without server-rendered cards fail to start.
        let specs = load_specs(Path::new("/nonexistent/deskmate/cards.json"))
            .expect("a missing file is not an error");
        assert!(specs.is_empty());
    }

    #[tokio::test]
    async fn missing_file_startup_retains_override_and_clears_previous_state() {
        let directory = tempfile::tempdir().expect("a temp directory");
        let path = directory.path().join("override.json");
        let state = ServerState::in_memory();
        let task = tokio::spawn(std::future::pending::<()>());
        let abort = task.abort_handle();
        {
            let mut retained = state.inner.data_cards.lock().expect("data cards");
            retained.specs.push(DataCardSpec {
                source_id: "old".into(),
                refresh_seconds: 900,
                face: blank("weather"),
            });
            retained.tasks.insert("old".into(), task);
        }

        start_data_cards(&state, path.clone()).expect("missing specs are valid");
        tokio::task::yield_now().await;
        {
            let retained = state.inner.data_cards.lock().expect("data cards");
            assert_eq!(retained.spec_path, path);
            assert!(retained.specs.is_empty());
            assert!(retained.tasks.is_empty());
        }
        assert!(abort.is_finished(), "empty startup still drains old tasks");
        assert!(!path.exists(), "startup does not create a missing file");
        state.shutdown();
    }

    #[tokio::test]
    async fn malformed_file_startup_leaves_retained_state_and_bytes_unchanged() {
        let directory = write("[{ malformed");
        let path = directory.path().join("cards.json");
        let bytes = std::fs::read(&path).expect("original bytes");
        let expected_error = load_specs(&path).expect_err("malformed specs");
        let state = ServerState::in_memory();
        let original = vec![DataCardSpec {
            source_id: "old".into(),
            refresh_seconds: 900,
            face: blank("weather"),
        }];
        let task = tokio::spawn(std::future::pending::<()>());
        let task_id = task.id();
        let abort = task.abort_handle();
        let original_path = {
            let mut retained = state.inner.data_cards.lock().expect("data cards");
            retained.specs = original.clone();
            retained.tasks.insert("old".into(), task);
            retained.spec_path.clone()
        };

        assert_eq!(start_data_cards(&state, path.clone()), Err(expected_error));
        tokio::task::yield_now().await;
        {
            let retained = state.inner.data_cards.lock().expect("data cards");
            assert_eq!(retained.spec_path, original_path);
            assert_eq!(retained.specs, original);
            assert_eq!(retained.tasks.len(), 1);
            assert_eq!(retained.tasks["old"].id(), task_id);
        }
        assert!(
            !abort.is_finished(),
            "malformed startup leaves the task live"
        );
        assert_eq!(std::fs::read(&path).expect("preserved bytes"), bytes);
        state.shutdown();
    }

    #[test]
    fn a_spec_file_written_by_the_rust_faces_loads_without_a_migration() {
        // The exact shape the live server's file holds: the closed enum's tag and
        // fields, `api_key: null` included. It must load, and round-trip unchanged.
        let written = serde_json::json!([
            {"source_id": "abc", "refresh_seconds": 900,
             "face": {"kind": "weather", "location": "Dubai", "units": "imperial"}},
            {"source_id": "def", "refresh_seconds": 900,
             "face": {"kind": "rss", "url": "https://example.test/feed", "title": "News"}},
            {"source_id": "ghi", "refresh_seconds": 300,
             "face": {"kind": "token", "coin_id": "solana", "currency": "usd", "api_key": null}}
        ]);
        let directory = write(&written.to_string());
        let specs = load_specs(&directory.path().join("cards.json")).expect("the specs parse");
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[2].refresh_seconds, 300);
        assert_eq!(specs[0].face.settings["units"], "imperial");
        assert_eq!(specs[2].face.settings["api_key"], serde_json::Value::Null);
        assert_eq!(
            serde_json::to_value(&specs).expect("specs serialize"),
            written
        );
    }

    #[test]
    fn refresh_seconds_defaults_and_an_absent_setting_is_absent_not_blank() {
        let directory =
            write(r#"[{"source_id": "ghi", "face": {"kind": "token", "coin_id": "solana"}}]"#);
        let specs = load_specs(&directory.path().join("cards.json")).expect("the spec parses");
        assert_eq!(specs[0].refresh_seconds, 900);
        assert_eq!(specs[0].face.settings.get("currency"), None);
        // ...and the descriptor shows the field's default where nothing was typed.
        assert!(matches!(
            &descriptor_of(&specs[0].face).fields[1],
            FaceFieldDescriptor::Text { key, value, .. } if key == "currency" && value == "usd"
        ));
    }

    #[test]
    fn a_malformed_spec_file_is_a_loud_error_rather_than_zero_cards() {
        for contents in [
            "[{ malformed",
            r#"[{"source_id": "abc"}]"#,
            r#"[{"source_id": "abc", "face": {}}]"#,
        ] {
            let directory = write(contents);
            assert!(
                load_specs(&directory.path().join("cards.json")).is_err(),
                "{contents} must not load as zero cards"
            );
        }
    }

    #[test]
    fn an_unknown_outer_field_is_refused() {
        // The outer struct stays strict, which is why the face is nested rather
        // than flattened into it: a typo here would otherwise be a silent default.
        let directory = write(
            r#"[{"source_id": "abc", "refresh_second": 60, "face": {"kind": "weather", "location": "Dubai"}}]"#,
        );
        assert!(
            load_specs(&directory.path().join("cards.json")).is_err(),
            "\"refresh_second\" is not \"refresh_seconds\""
        );
    }

    #[tokio::test]
    async fn a_kind_the_package_does_not_draw_is_kept_but_never_refreshed() {
        // Kinds are the faces package's to define, so the file cannot refuse one. A
        // spec outliving its face must survive a start untouched -- the package can
        // come back -- while doing nothing in the meantime.
        let state = ServerState::in_memory();
        let source = state.image_sources().mint("Calendar").expect("a source");
        let directory = write(&format!(
            r#"[{{"source_id": "{}", "face": {{"kind": "calendar", "url": "https://example.test/cal"}}}}]"#,
            source.id
        ));
        let path = directory.path().join("cards.json");
        let bytes = std::fs::read(&path).expect("original bytes");

        start_data_cards(&state, path.clone()).expect("an unknown kind is not a load error");
        {
            let retained = state.inner.data_cards.lock().expect("data cards");
            assert_eq!(retained.specs.len(), 1, "the spec is kept");
            assert!(retained.tasks.is_empty(), "and nothing refreshes it");
        }
        assert_eq!(descriptor_for_source(&state, &source.id), None);
        assert_eq!(
            status_for_source(&state, &source.id).map(|status| status.state),
            Some(FaceState::Unavailable)
        );
        assert!(matches!(
            update_face_fields(
                &state,
                &tokio::runtime::Handle::current(),
                &source.id,
                &BTreeMap::from([("url".into(), "https://example.test/other".into())]),
            ),
            Err(FaceUpdateError::NoFace)
        ));
        assert_eq!(std::fs::read(&path).expect("preserved bytes"), bytes);
        state.shutdown();
    }

    #[test]
    fn an_empty_source_id_or_kind_is_refused() {
        for (contents, names) in [
            (
                r#"[{"source_id": "  ", "face": {"kind": "weather", "location": "Dubai"}}]"#,
                "source_id",
            ),
            (
                r#"[{"source_id": "abc", "face": {"kind": " ", "location": "Dubai"}}]"#,
                "kind",
            ),
        ] {
            let directory = write(contents);
            let error = load_specs(&directory.path().join("cards.json")).expect_err("refused");
            assert!(error.contains(names), "the message names {names}: {error}");
        }
    }

    #[tokio::test]
    async fn a_server_with_no_faces_package_offers_no_faces() {
        let state = ServerState::in_memory();
        {
            let mut retained = state.inner.data_cards.lock().expect("data cards");
            retained.faces = None;
            retained.catalog = None;
        }
        assert!(creatable_faces(&state).is_empty());
        assert!(matches!(
            create_face(&state, "source", "weather"),
            Err(FaceUpdateError::UnknownField(kind)) if kind == "weather"
        ));
        state.shutdown();
    }

    #[test]
    fn a_faces_tap_sentence_reaches_the_descriptor_and_absence_means_no_taps() {
        let state = ServerState::in_memory();
        set_faces(&state, faces_package::fake());

        let descriptors = creatable_faces(&state);
        let headlines = descriptors
            .iter()
            .find(|face| face.kind == "headlines")
            .expect("headlines");
        assert_eq!(
            headlines.tap.as_deref(),
            Some("Tap the panel for the next stories.")
        );
        assert_eq!(
            serde_json::to_value(headlines).unwrap()["tap"],
            "Tap the panel for the next stories."
        );
        let weather = descriptors
            .iter()
            .find(|face| face.kind == "weather")
            .expect("weather");
        assert_eq!(weather.tap, None);
        assert_eq!(
            serde_json::to_value(weather).unwrap().get("tap"),
            None,
            "a face without a tap sentence adds no contract key"
        );

        create_face(&state, "tappable", "headlines").expect("attach tappable face");
        create_face(&state, "untappable", "weather").expect("attach untappable face");
        assert!(face_takes_taps(&state, "tappable"));
        assert!(!face_takes_taps(&state, "untappable"));
        assert!(!face_takes_taps(&state, "external-or-unknown"));
        state.shutdown();
    }

    /// Polls until the face's status satisfies `done`, the way the window does.
    async fn status_when(
        state: &ServerState,
        source_id: &str,
        done: impl Fn(&FaceStatus) -> bool,
    ) -> FaceStatus {
        for _ in 0..200 {
            if let Some(status) = status_for_source(state, source_id)
                && done(&status)
            {
                return status;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!(
            "the status never settled: {:?}",
            status_for_source(state, source_id)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_window_is_told_why_a_face_is_not_drawing() {
        // The failure this exists for: a coin ID typed as a ticker. The panel said
        // "Waiting for the first picture", the window said "Saved to the server", and
        // the reason was a log line on a VM.
        let state = ServerState::in_memory();
        let runtime = tokio::runtime::Handle::current();
        let source = state.image_sources().mint("Token price").expect("a source");
        let field = |value: &str| BTreeMap::from([("coin_id".to_owned(), value.to_owned())]);

        assert_eq!(
            status_for_source(&state, "an-external-producers-source"),
            None
        );
        create_face(&state, &source.id, "token").expect("create");
        assert_eq!(
            status_for_source(&state, &source.id).map(|status| status.state),
            Some(FaceState::NeedsSettings),
            "a blank coin ID is not an error yet, it is an unfinished form"
        );

        update_face_fields(
            &state,
            &runtime,
            &source.id,
            &field("refuse-as-configuration"),
        )
        .expect("the field is well-formed; only the faces package can judge the coin");
        let refused =
            status_when(&state, &source.id, |s| s.state == FaceState::NeedsAttention).await;
        assert_eq!(
            refused.message.as_deref(),
            Some("the coin was not found; check the coin ID"),
            "the package's own sentence reaches the owner verbatim"
        );
        assert!(refused.at_unix_seconds.is_some());

        update_face_fields(&state, &runtime, &source.id, &field("solana")).expect("update");
        let drawn = status_when(&state, &source.id, |s| s.state == FaceState::Drawn).await;
        assert_eq!(drawn.message, None);

        // New settings start from a clean slate: "drawn" was about the OLD coin.
        update_face_fields(&state, &runtime, &source.id, &field("refuse-as-transient"))
            .expect("update");
        let first = status_for_source(&state, &source.id).expect("a status");
        assert_ne!(
            first.state,
            FaceState::Drawn,
            "a stale verdict must not survive new settings"
        );
        let retrying = status_when(&state, &source.id, |s| s.state == FaceState::Retrying).await;
        assert_eq!(
            retrying.message.as_deref(),
            Some("api.example returned HTTP 503")
        );
        stop_refreshers(&state);
        state.shutdown();
    }

    #[tokio::test]
    async fn a_replaced_refreshers_late_verdict_is_ignored() {
        // An aborted task can still be inside its blocking render. When it finishes,
        // its verdict is about settings that are no longer in force.
        let state = ServerState::in_memory();
        let current = tokio::spawn(std::future::pending::<()>());
        let replaced = tokio::spawn(std::future::pending::<()>());
        let (current_id, replaced_id) = (current.id(), replaced.id());
        state
            .inner
            .data_cards
            .lock()
            .expect("data cards")
            .tasks
            .insert("target".into(), current);

        record_outcome(&state, "target", replaced_id, RefreshOutcome::Drawn);
        assert!(
            state
                .inner
                .data_cards
                .lock()
                .expect("data cards")
                .outcomes
                .is_empty()
        );
        record_outcome(&state, "target", current_id, RefreshOutcome::Drawn);
        assert_eq!(
            state
                .inner
                .data_cards
                .lock()
                .expect("data cards")
                .outcomes
                .len(),
            1
        );

        remove_face(&state, "target").expect("remove");
        assert!(
            state
                .inner
                .data_cards
                .lock()
                .expect("data cards")
                .outcomes
                .is_empty()
        );
        replaced.abort();
        state.shutdown();
    }

    #[tokio::test]
    async fn a_face_that_is_complete_at_birth_starts_refreshing_without_a_save() {
        // Every field of `headlines` has a default, so there is no later settings
        // save to start it: creating it has to.
        let state = ServerState::in_memory();
        let descriptor = create_face(&state, "target", "headlines").expect("create");
        assert!(face_is_complete(&descriptor));
        assert!(
            state
                .inner
                .data_cards
                .lock()
                .expect("data cards")
                .tasks
                .contains_key("target")
        );
        stop_refreshers(&state);
        state.shutdown();
    }

    #[test]
    fn descriptor_fields_are_derived_from_a_real_face_spec() {
        let descriptor = descriptor_of(&weather("Dubai"));

        assert_eq!(descriptor.kind, "weather");
        assert_eq!(descriptor.label, "Weather");
        assert!(matches!(
            &descriptor.fields[0],
            FaceFieldDescriptor::Text { key, value, .. }
                if key == "location" && value == "Dubai"
        ));
        assert!(matches!(
            &descriptor.fields[1],
            FaceFieldDescriptor::Enum { key, value, options, .. }
                if key == "units" && value == "metric" && options.len() == 2
        ));
    }

    #[test]
    fn an_unknown_face_field_key_is_refused() {
        let descriptor = descriptor_of(&weather("Dubai"));
        let fields = BTreeMap::from([("locaton".into(), "Berlin".into())]);

        assert!(matches!(
            validate_face_fields(&descriptor, &fields),
            Err(FaceUpdateError::UnknownField(key)) if key == "locaton"
        ));
    }

    #[test]
    fn a_non_http_face_url_is_refused() {
        let descriptor = descriptor_of(&face(
            "rss",
            serde_json::json!({"url": "https://example.test/feed", "title": "News"}),
        ));
        let fields = BTreeMap::from([("url".into(), "file:///etc/passwd".into())]);

        assert!(matches!(
            validate_face_fields(&descriptor, &fields),
            Err(FaceUpdateError::InvalidField { field, .. }) if field == "url"
        ));
    }

    #[tokio::test]
    async fn blank_faces_persist_their_defaults_without_starting_tasks() {
        let state = ServerState::in_memory();
        for (kind, expected) in [
            (
                "weather",
                serde_json::json!({"kind":"weather", "location":"", "units":"metric"}),
            ),
            (
                "rss",
                serde_json::json!({"kind":"rss", "url":"", "title":""}),
            ),
            (
                "token",
                serde_json::json!({"kind":"token", "coin_id":"", "currency":"usd"}),
            ),
        ] {
            let descriptor = create_face(&state, kind, kind).expect("create blank face");
            assert_eq!(
                descriptor,
                creatable_faces(&state)
                    .into_iter()
                    .find(|face| face.kind == kind)
                    .unwrap()
            );
            let retained = state.inner.data_cards.lock().unwrap();
            assert!(retained.tasks.is_empty());
            let persisted = load_specs(&retained.spec_path).unwrap();
            let spec = persisted.last().unwrap();
            assert_eq!(spec.source_id, kind);
            assert_eq!(spec.refresh_seconds, 900);
            assert_eq!(serde_json::to_value(&spec.face).unwrap(), expected);
            assert_eq!(persisted, retained.specs);
        }
        state.shutdown();
    }

    async fn assert_partial_update_stays_dormant(kind: &str, partial: BTreeMap<String, String>) {
        let state = ServerState::in_memory();
        let runtime = tokio::runtime::Handle::current();
        create_face(&state, "target", kind).unwrap();
        update_face_fields(&state, &runtime, "target", &partial).unwrap();
        {
            let retained = state.inner.data_cards.lock().unwrap();
            assert!(
                retained.tasks.is_empty(),
                "incomplete {kind} must remain dormant"
            );
            assert_eq!(load_specs(&retained.spec_path).unwrap(), retained.specs);
            let face = serde_json::to_value(&retained.specs[0].face).unwrap();
            for (key, value) in &partial {
                assert_eq!(face[key], *value);
            }
        }
        let remaining = match kind {
            "weather" => BTreeMap::from([("location".into(), "Berlin".into())]),
            "rss" if partial.contains_key("url") => {
                BTreeMap::from([("title".into(), "News".into())])
            }
            "rss" => BTreeMap::from([
                ("url".into(), "https://example.test/feed".into()),
                ("title".into(), "News".into()),
            ]),
            "token" => BTreeMap::from([("coin_id".into(), "solana".into())]),
            _ => unreachable!(),
        };
        update_face_fields(&state, &runtime, "target", &remaining).unwrap();
        let (first_id, first_abort) = {
            let retained = state.inner.data_cards.lock().unwrap();
            assert_eq!(retained.tasks.len(), 1);
            (
                retained.tasks["target"].id(),
                retained.tasks["target"].abort_handle(),
            )
        };
        update_face_fields(&state, &runtime, "target", &BTreeMap::new()).unwrap();
        {
            let retained = state.inner.data_cards.lock().unwrap();
            assert_eq!(retained.tasks.len(), 1);
            assert_ne!(retained.tasks["target"].id(), first_id);
        }
        // No refresher gets polled on this current-thread runtime before abortion.
        stop_refreshers(&state);
        tokio::task::yield_now().await;
        assert!(first_abort.is_finished());
        state.shutdown();
    }

    #[tokio::test]
    async fn empty_updates_leave_all_blank_kinds_dormant() {
        for kind in ["weather", "rss", "token"] {
            assert_partial_update_stays_dormant(kind, BTreeMap::new()).await;
        }
    }

    #[tokio::test]
    async fn partial_fields_persist_without_starting_incomplete_faces() {
        for (kind, key, value) in [
            ("weather", "units", "imperial"),
            ("rss", "title", "News"),
            ("rss", "url", "https://example.test/feed"),
            ("token", "currency", "eur"),
        ] {
            assert_partial_update_stays_dormant(kind, BTreeMap::from([(key.into(), value.into())]))
                .await;
        }
    }

    #[tokio::test]
    async fn public_token_updates_preserve_private_key_and_refresh_interval() {
        let state = ServerState::in_memory();
        let runtime = tokio::runtime::Handle::current();
        {
            let mut retained = state.inner.data_cards.lock().unwrap();
            retained.specs.push(DataCardSpec {
                source_id: "target".into(),
                refresh_seconds: 3_600,
                face: face(
                    "token",
                    serde_json::json!({"coin_id": "solana", "currency": "usd", "api_key": "secret"}),
                ),
            });
        }
        update_face_fields(
            &state,
            &runtime,
            "target",
            &BTreeMap::from([("currency".into(), "eur".into())]),
        )
        .unwrap();
        {
            let retained = state.inner.data_cards.lock().unwrap();
            assert_eq!(retained.tasks.len(), 1);
            let persisted = load_specs(&retained.spec_path).unwrap();
            assert_eq!(persisted[0].refresh_seconds, 3_600);
            // `api_key` is in no descriptor, so no update can name it -- or drop it.
            assert_eq!(persisted[0].face.settings["currency"], "eur");
            assert_eq!(persisted[0].face.settings["api_key"], "secret");
        }
        stop_refreshers(&state);
        state.shutdown();
    }

    #[tokio::test]
    async fn invalid_updates_preserve_existing_specs_tasks_and_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cards.json");
        let state = ServerState::in_memory();
        let task = tokio::spawn(std::future::pending::<()>());
        let task_id = task.id();
        let abort = task.abort_handle();
        let original = vec![DataCardSpec {
            source_id: "target".into(),
            refresh_seconds: 900,
            face: weather("Berlin"),
        }];
        persist_specs(&path, &original).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        {
            let mut retained = state.inner.data_cards.lock().unwrap();
            retained.spec_path = path.clone();
            retained.specs = original.clone();
            retained.tasks.insert("target".into(), task);
        }
        for (key, value) in [("location", " "), ("units", "kelvin"), ("unknown", "value")] {
            assert!(
                update_face_fields(
                    &state,
                    &tokio::runtime::Handle::current(),
                    "target",
                    &BTreeMap::from([(key.into(), value.into())])
                )
                .is_err()
            );
            let retained = state.inner.data_cards.lock().unwrap();
            assert_eq!(retained.specs, original);
            assert_eq!(retained.tasks.len(), 1);
            assert_eq!(retained.tasks["target"].id(), task_id);
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        tokio::task::yield_now().await;
        assert!(!abort.is_finished());
        state.shutdown();
    }

    #[tokio::test]
    async fn revoked_source_cancels_before_failed_persistence_and_can_retry() {
        let directory = tempfile::tempdir().unwrap();
        let parent = directory.path().join("specs");
        std::fs::create_dir(&parent).unwrap();
        let path = parent.join("cards.json");
        let backup = directory.path().join("backup");
        let state = ServerState::in_memory();
        let removed = state.image_sources().mint("Removed").unwrap();
        let retained = state.image_sources().mint("Retained").unwrap();
        let original: Vec<_> = [&removed.id, &retained.id]
            .into_iter()
            .map(|id| DataCardSpec {
                source_id: id.clone(),
                refresh_seconds: 900,
                face: blank("weather"),
            })
            .collect();
        persist_specs(&path, &original).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let task = tokio::spawn(std::future::pending::<()>());
        let abort = task.abort_handle();
        {
            let mut cards = state.inner.data_cards.lock().unwrap();
            cards.spec_path = path.clone();
            cards.specs = original.clone();
            cards.tasks.insert(removed.id.clone(), task);
        }
        tokio::task::yield_now().await;
        state.image_sources().revoke(&removed.id).unwrap();
        std::fs::rename(&parent, &backup).unwrap();
        std::fs::write(&parent, b"not a directory").unwrap();
        assert!(matches!(
            remove_face(&state, &removed.id),
            Err(FaceUpdateError::Persist(_))
        ));
        {
            let cards = state.inner.data_cards.lock().unwrap();
            assert!(
                !cards.tasks.contains_key(&removed.id),
                "revoked refresher must be removed even if persistence fails"
            );
            assert_eq!(cards.specs, original);
        }
        tokio::task::yield_now().await;
        assert!(abort.is_finished(), "the executor processes cancellation");
        assert_eq!(std::fs::read(backup.join("cards.json")).unwrap(), bytes);
        assert_eq!(std::fs::read(&parent).unwrap(), b"not a directory");
        std::fs::remove_file(&parent).unwrap();
        std::fs::rename(&backup, &parent).unwrap();
        remove_face(&state, &removed.id).expect("internal retry persists the removal");
        assert_eq!(load_specs(&path).unwrap(), original[1..]);
        assert_eq!(state.inner.data_cards.lock().unwrap().specs, original[1..]);
        assert!(std::fs::read(&path).unwrap().ends_with(b"\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        state.shutdown();
    }

    #[tokio::test]
    async fn removing_an_absent_spec_still_cancels_its_task() {
        let state = ServerState::in_memory();
        let task = tokio::spawn(std::future::pending::<()>());
        let abort = task.abort_handle();
        state
            .inner
            .data_cards
            .lock()
            .unwrap()
            .tasks
            .insert("absent".into(), task);
        remove_face(&state, "absent").unwrap();
        assert!(state.inner.data_cards.lock().unwrap().tasks.is_empty());
        tokio::task::yield_now().await;
        assert!(abort.is_finished());
        state.shutdown();
    }

    #[tokio::test]
    async fn successful_removal_cancels_only_the_matching_task_and_spec() {
        let state = ServerState::in_memory();
        for id in ["removed", "retained"] {
            create_face(&state, id, "weather").unwrap();
        }
        let removed = tokio::spawn(std::future::pending::<()>());
        let abort = removed.abort_handle();
        let retained = tokio::spawn(std::future::pending::<()>());
        let retained_id = retained.id();
        {
            let mut cards = state.inner.data_cards.lock().unwrap();
            cards.tasks.insert("removed".into(), removed);
            cards.tasks.insert("retained".into(), retained);
        }
        remove_face(&state, "removed").unwrap();
        {
            let cards = state.inner.data_cards.lock().unwrap();
            assert_eq!(cards.specs.len(), 1);
            assert_eq!(cards.specs[0].source_id, "retained");
            assert_eq!(load_specs(&cards.spec_path).unwrap(), cards.specs);
            assert_eq!(cards.tasks.len(), 1);
            assert_eq!(cards.tasks["retained"].id(), retained_id);
        }
        tokio::task::yield_now().await;
        assert!(abort.is_finished());
        state.shutdown();
    }

    #[tokio::test]
    async fn face_state_lives_beside_the_specs_and_is_removed_with_its_face() {
        let state = ServerState::in_memory();
        create_face(&state, "target", "weather").expect("create face");
        let (path, face_state) = {
            let cards = state.inner.data_cards.lock().expect("data cards");
            (
                cards.spec_path.with_file_name("face-state.json"),
                Arc::clone(&cards.face_state),
            )
        };
        face_state.put("target", Some(serde_json::json!({ "page": 3 })));
        assert_eq!(
            FaceStateStore::load(path.clone()).get("target"),
            Some(serde_json::json!({ "page": 3 }))
        );

        remove_face(&state, "target").expect("remove face");
        assert_eq!(face_state.get("target"), None);
        assert_eq!(FaceStateStore::load(path).get("target"), None);
        state.shutdown();
    }

    #[tokio::test]
    async fn startup_prunes_revoked_sources_even_when_pruning_cannot_be_persisted() {
        for blocked in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("cards.json");
            let state = ServerState::in_memory();
            let removed = state.image_sources().mint("Removed").unwrap();
            let retained = state.image_sources().mint("Retained").unwrap();
            let original: Vec<_> = [&removed.id, &retained.id]
                .into_iter()
                .map(|id| DataCardSpec {
                    source_id: id.clone(),
                    refresh_seconds: 900,
                    face: blank("weather"),
                })
                .collect();
            persist_specs(&path, &original).unwrap();
            state.image_sources().revoke(&removed.id).unwrap();
            if blocked {
                let blocker = directory.path().join("not-a-directory");
                std::fs::write(&blocker, b"blocked").unwrap();
                spawn_refreshers(
                    &state,
                    blocker.join("cards.json"),
                    load_specs(&path).unwrap(),
                );
                assert_eq!(load_specs(&path).unwrap(), original);
            } else {
                start_data_cards(&state, path.clone()).unwrap();
                assert_eq!(load_specs(&path).unwrap(), original[1..]);
            }
            {
                let cards = state.inner.data_cards.lock().unwrap();
                assert_eq!(cards.specs, original[1..]);
                assert!(cards.tasks.is_empty());
            }
            state.shutdown();
        }
    }

    async fn assert_failed_mutation_preserves_state(operation: &str) {
        let directory = tempfile::tempdir().expect("a temp directory");
        // A regular file cannot contain the spec, even when tests run as root.
        let blocker = directory.path().join("not-a-directory");
        std::fs::write(&blocker, b"retained bytes").expect("write blocker");
        let path = blocker.join("cards.json");
        let state = ServerState::in_memory();
        let original = vec![DataCardSpec {
            source_id: "target".into(),
            refresh_seconds: 900,
            face: weather("Dubai"),
        }];
        let task = tokio::spawn(std::future::pending::<()>());
        let task_id = task.id();
        let abort = task.abort_handle();
        {
            let mut retained = state.inner.data_cards.lock().expect("data cards");
            retained.spec_path = path.clone();
            retained.specs = original.clone();
            retained.tasks.insert("target".into(), task);
        }
        let runtime = tokio::runtime::Handle::current();
        let result = match operation {
            "update" => update_face_fields(
                &state,
                &runtime,
                "target",
                &BTreeMap::from([("location".into(), "Berlin".into())]),
            )
            .map(|_| ()),
            "create" => create_face(&state, "new-source", "weather").map(|_| ()),
            "remove" => remove_face(&state, "target"),
            _ => panic!("unknown test operation"),
        };
        assert!(matches!(result, Err(FaceUpdateError::Persist(_))));
        tokio::task::yield_now().await;
        {
            let retained = state.inner.data_cards.lock().expect("data cards");
            assert_eq!(retained.spec_path, path);
            assert_eq!(retained.specs, original);
            if operation == "remove" {
                assert!(
                    retained.tasks.is_empty(),
                    "revoked tasks must stop despite persistence failure"
                );
            } else {
                assert_eq!(retained.tasks.len(), 1);
                assert_eq!(retained.tasks["target"].id(), task_id);
                assert!(!retained.tasks["target"].is_finished());
            }
        }
        assert_eq!(abort.is_finished(), operation == "remove");
        assert_eq!(
            std::fs::read(blocker).expect("read blocker"),
            b"retained bytes"
        );
        assert!(!path.exists());
        state.shutdown();
    }

    #[tokio::test]
    async fn update_persistence_failure_preserves_specs_and_tasks() {
        assert_failed_mutation_preserves_state("update").await;
    }

    #[tokio::test]
    async fn create_persistence_failure_preserves_specs_and_tasks() {
        assert_failed_mutation_preserves_state("create").await;
    }

    #[tokio::test]
    async fn remove_persistence_failure_preserves_specs_and_cancels_tasks() {
        assert_failed_mutation_preserves_state("remove").await;
    }

    #[tokio::test]
    async fn rewriting_one_spec_preserves_every_other_entry() {
        let state = ServerState::in_memory();
        let path = {
            state
                .inner
                .data_cards
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .spec_path
                .clone()
        };
        let untouched = DataCardSpec {
            source_id: "untouched".into(),
            refresh_seconds: 3_600,
            face: face(
                "token",
                serde_json::json!({"coin_id": "solana", "currency": "eur", "api_key": "still-secret"}),
            ),
        };
        let original = vec![
            DataCardSpec {
                source_id: "target".into(),
                refresh_seconds: 600,
                face: face(
                    "rss",
                    serde_json::json!({"url": "https://example.test/old.xml", "title": "Old title"}),
                ),
            },
            untouched.clone(),
        ];
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&original).expect("encode fixture"),
        )
        .expect("write fixture");
        {
            let mut retained = state
                .inner
                .data_cards
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            retained.specs = original;
        }

        let fields = BTreeMap::from([
            ("url".into(), "https://example.test/new.xml".into()),
            ("title".into(), "New title".into()),
        ]);
        update_face_fields(
            &state,
            &tokio::runtime::Handle::current(),
            "target",
            &fields,
        )
        .expect("update face");

        let rewritten = load_specs(&path).expect("rewritten specs parse");
        assert_eq!(rewritten.len(), 2);
        assert_eq!(rewritten[1], untouched);
        assert_eq!(
            rewritten[0].face,
            face(
                "rss",
                serde_json::json!({"url": "https://example.test/new.xml", "title": "New title"}),
            )
        );
        state.shutdown();
    }
}
