//! Server-rendered data cards: the server acting as its own picture producer.
//!
//! Weather, RSS and token faces belong to ordinary picture cards. Each face is
//! an image source the server pushes to itself: a refresh task fetches data,
//! renders the face, and hands the frame to
//! [`crate::image_sources::ImageSourceStore::accept`]. Delivery then uses the
//! same durable assets, device notifications and staleness inference as an
//! external producer's PNG.
//!
//! The server owns the persisted specs separately from the device config.
//! The browser creates and edits faces through server-provided descriptors,
//! so adding a face does not require a new card kind or a config schema change.
//! Settings absent from those descriptors can be edited in the spec file while
//! the server is stopped.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ServerState;

mod worker;

/// A field value arrives over an internet-facing admin route. This comfortably
/// covers feed URLs while preventing a tiny settings document from becoming an
/// unbounded allocation surface.
const MAX_FACE_FIELD_VALUE_BYTES: usize = 2_048;

/// One server-rendered card.
///
/// The face config is a nested object rather than flattened into this struct,
/// and that is not a style choice: serde's `deny_unknown_fields` and
/// `flatten` do not work together -- the outer struct rejects every flattened
/// field as unknown. Flattening therefore meant giving up the typo protection,
/// which is the one thing this file most needs, because a spec is hand-written
/// and a silently-defaulted field presents as a card that is subtly wrong
/// forever. Nesting costs one level of braces and keeps both sides strict.
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

/// Which face, and what it needs to know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum FaceSpec {
    Weather {
        /// A place name, geocoded by the provider. Not coordinates: the owner
        /// types a city, and the geocoder's own display name is what the face
        /// shows, so the panel says what the forecast is actually for.
        location: String,
        #[serde(default)]
        units: Units,
    },
    Rss {
        url: String,
        /// The eyebrow. Feeds name themselves inconsistently and often at
        /// length, so the owner gets to say what this feed is called.
        title: String,
    },
    Token {
        /// `CoinGecko`'s coin id, e.g. `solana`.
        coin_id: String,
        #[serde(default = "default_currency")]
        currency: String,
        #[serde(default)]
        api_key: Option<String>,
    },
}

fn default_currency() -> String {
    "usd".to_owned()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Units {
    #[default]
    Metric,
    Imperial,
}

/// The server-owned settings contract rendered by the companion. The app treats
/// `kind` as opaque metadata and renders only this descriptor's field types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct FaceDescriptor {
    pub(crate) kind: String,
    pub(crate) label: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct FaceFieldOption {
    pub(crate) value: String,
    pub(crate) label: String,
}

/// The only function that teaches the settings UI about a face. Adding a face
/// means adding one server-side descriptor here; the companion never switches on
/// `kind` and needs no release of its own.
#[must_use]
/// The faces this server knows how to create, as blank defaults.
///
/// The companion's add menu is built from this list, which is why the menu can
/// offer "Weather" without the app knowing what a weather face is. Adding a
/// fourth face here is what makes it appear in the window -- no app change.
pub(crate) fn creatable_faces() -> Vec<FaceDescriptor> {
    ["weather", "rss", "token"]
        .into_iter()
        .filter_map(blank_face)
        .map(|face| face_descriptor(&face))
        .collect()
}

/// A blank face of `kind`, or `None` if this server cannot draw that kind.
fn blank_face(kind: &str) -> Option<FaceSpec> {
    match kind {
        "weather" => Some(FaceSpec::Weather {
            location: String::new(),
            units: Units::Metric,
        }),
        "rss" => Some(FaceSpec::Rss {
            url: String::new(),
            title: String::new(),
        }),
        "token" => Some(FaceSpec::Token {
            coin_id: String::new(),
            currency: default_currency(),
            api_key: None,
        }),
        _ => None,
    }
}

/// Whether a face has everything it needs to be fetched.
///
/// A card added from the menu starts blank, and a blank weather face has no city
/// to ask about. Fetching anyway would fail every cycle and fill the journal with
/// errors the owner cannot act on until they type one. So an incomplete face is
/// stored but not refreshed, and the card shows the ordinary "no frame yet"
/// state until its settings are filled in.
fn face_is_complete(face: &FaceSpec) -> bool {
    face_descriptor(face)
        .fields
        .iter()
        .all(|field| match field {
            FaceFieldDescriptor::Text { value, .. } | FaceFieldDescriptor::Url { value, .. } => {
                !value.trim().is_empty()
            }
            FaceFieldDescriptor::Enum { .. } => true,
        })
}

fn face_descriptor(face: &FaceSpec) -> FaceDescriptor {
    let text = |key: &str, label: &str, value: &str, placeholder: &str| FaceFieldDescriptor::Text {
        key: key.to_owned(),
        label: label.to_owned(),
        value: value.to_owned(),
        placeholder: placeholder.to_owned(),
    };
    let url = |key: &str, label: &str, value: &str, placeholder: &str| FaceFieldDescriptor::Url {
        key: key.to_owned(),
        label: label.to_owned(),
        value: value.to_owned(),
        placeholder: placeholder.to_owned(),
    };
    match face {
        FaceSpec::Weather { location, units } => FaceDescriptor {
            kind: "weather".into(),
            label: "Weather".into(),
            fields: vec![
                text("location", "Location", location, "Dubai"),
                FaceFieldDescriptor::Enum {
                    key: "units".into(),
                    label: "Units".into(),
                    value: match units {
                        Units::Metric => "metric",
                        Units::Imperial => "imperial",
                    }
                    .into(),
                    options: vec![
                        FaceFieldOption {
                            value: "metric".into(),
                            label: "Metric".into(),
                        },
                        FaceFieldOption {
                            value: "imperial".into(),
                            label: "Imperial".into(),
                        },
                    ],
                },
            ],
        },
        FaceSpec::Rss {
            url: feed_url,
            title,
        } => FaceDescriptor {
            kind: "rss".into(),
            label: "RSS feed".into(),
            fields: vec![
                url("url", "Feed URL", feed_url, "https://example.com/feed.xml"),
                text("title", "Title", title, "News"),
            ],
        },
        FaceSpec::Token {
            coin_id, currency, ..
        } => FaceDescriptor {
            kind: "token".into(),
            label: "Token price".into(),
            // `api_key` is deliberately absent: the descriptor is a readable
            // response and its field vocabulary has no secret type. An existing
            // key remains in the spec unchanged when these public fields change.
            fields: vec![
                text("coin_id", "Coin ID", coin_id, "solana"),
                text("currency", "Currency", currency, "usd"),
            ],
        },
    }
}

/// Server-owned data-card state. The path, parsed specs, and every live task
/// stay together so an update can atomically replace the persisted spec and the
/// one refresher that consumes it.
pub(crate) struct DataCardState {
    spec_path: PathBuf,
    specs: Vec<DataCardSpec>,
    tasks: HashMap<String, tokio::task::JoinHandle<()>>,
}

impl DataCardState {
    pub(crate) fn new(spec_path: PathBuf) -> Self {
        Self {
            spec_path,
            specs: Vec::new(),
            tasks: HashMap::new(),
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

    let mut data_cards = state
        .inner
        .data_cards
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (_, task) in data_cards.tasks.drain() {
        task.abort();
    }
    data_cards.spec_path = spec_path;
    data_cards.specs = specs;
    let specs = data_cards.specs.clone();
    for spec in specs {
        // A card added from the menu is stored blank and stays unfetched until
        // its settings are filled in, so restarting must not start it either.
        if !face_is_complete(&spec.face) {
            tracing::info!(
                source_id = %spec.source_id,
                "a server-rendered card has no settings yet and is not being fetched"
            );
            continue;
        }
        data_cards.tasks.insert(
            spec.source_id.clone(),
            worker::spawn_refresher(&tokio::runtime::Handle::current(), state.clone(), spec),
        );
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
}

pub(crate) fn descriptor_for_source(
    state: &ServerState,
    source_id: &str,
) -> Option<FaceDescriptor> {
    state
        .inner
        .data_cards
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .specs
        .iter()
        .find(|spec| spec.source_id == source_id)
        .map(|spec| face_descriptor(&spec.face))
}

/// Validates and persists one face update, then aborts and replaces exactly
/// that source's refresher. The descriptor drives validation and mutation, so
/// face-specific knowledge is confined to [`face_descriptor`].
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
    let current = &data_cards.specs[index];
    let descriptor = face_descriptor(&current.face);
    validate_face_fields(&descriptor, fields)?;

    let mut face_value = serde_json::to_value(&current.face)
        .map_err(|error| FaceUpdateError::Encode(error.to_string()))?;
    let object = face_value
        .as_object_mut()
        .ok_or_else(|| FaceUpdateError::Encode("a face did not serialize as an object".into()))?;
    for (key, value) in fields {
        object.insert(key.clone(), serde_json::Value::String(value.clone()));
    }
    let updated_face: FaceSpec = serde_json::from_value(face_value)
        .map_err(|error| FaceUpdateError::Encode(error.to_string()))?;
    let mut updated_specs = data_cards.specs.clone();
    updated_specs[index].face = updated_face;
    persist_specs(&data_cards.spec_path, &updated_specs)?;

    let updated_spec = updated_specs[index].clone();
    data_cards.specs = updated_specs;
    if let Some(task) = data_cards.tasks.remove(source_id) {
        task.abort();
    }
    data_cards.tasks.insert(
        source_id.to_owned(),
        worker::spawn_refresher(runtime, state.clone(), updated_spec.clone()),
    );
    Ok(face_descriptor(&updated_spec.face))
}

/// Attaches a blank face of `kind` to a freshly minted source.
///
/// Called right after `POST /v1/images` mints the source, so the companion's
/// "Weather" menu item is one round trip: mint, attach, add a card. The spec is
/// persisted immediately -- a face that exists only in memory would vanish on the
/// next restart and leave a card pointing at a source nothing draws.
pub(crate) fn create_face(
    state: &ServerState,
    runtime: &tokio::runtime::Handle,
    source_id: &str,
    kind: &str,
) -> Result<FaceDescriptor, FaceUpdateError> {
    let face = blank_face(kind).ok_or_else(|| FaceUpdateError::UnknownField(kind.to_owned()))?;
    let mut data_cards = state
        .inner
        .data_cards
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    // A blank face is not fetched; see `face_is_complete`.
    if face_is_complete(&spec.face) {
        data_cards.tasks.insert(
            source_id.to_owned(),
            worker::spawn_refresher(runtime, state.clone(), spec.clone()),
        );
    }
    Ok(face_descriptor(&spec.face))
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
    if !data_cards
        .specs
        .iter()
        .any(|spec| spec.source_id == source_id)
    {
        // Stop the task anyway: a spec can have been removed by an earlier call
        // whose task abort did not happen, and leaving a live refresher for a
        // spec nobody holds is exactly the state this function exists to end.
        if let Some(task) = data_cards.tasks.remove(source_id) {
            task.abort();
        }
        return Ok(());
    }

    let updated_specs: Vec<DataCardSpec> = data_cards
        .specs
        .iter()
        .filter(|spec| spec.source_id != source_id)
        .cloned()
        .collect();
    persist_specs(&data_cards.spec_path, &updated_specs)?;
    // The file is the durable record, so it is replaced before the in-memory
    // copy and the task are -- a crash between the two leaves a spec that is
    // gone from disk and a task that dies with the process.
    data_cards.specs = updated_specs;
    if let Some(task) = data_cards.tasks.remove(source_id) {
        task.abort();
    }
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
    use providers::weather::WeatherUnits;

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
                face: blank_face("weather").expect("weather"),
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
            face: blank_face("weather").expect("weather"),
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
    fn the_three_faces_parse_with_their_defaults() {
        let directory = write(
            r#"[
                {"source_id": "abc", "face": {"kind": "weather", "location": "Dubai"}},
                {"source_id": "def", "face": {"kind": "rss", "url": "https://example.test/feed", "title": "News"}},
                {"source_id": "ghi", "face": {"kind": "token", "coin_id": "solana"}}
            ]"#,
        );
        let specs = load_specs(&directory.path().join("cards.json")).expect("the specs parse");
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].refresh_seconds, 900);
        assert!(matches!(
            specs[0].face,
            FaceSpec::Weather {
                units: Units::Metric,
                ..
            }
        ));
        match &specs[2].face {
            FaceSpec::Token {
                currency, api_key, ..
            } => {
                assert_eq!(currency, "usd");
                assert!(api_key.is_none());
            }
            other => panic!("expected a token card, got {other:?}"),
        }
    }

    #[test]
    fn a_malformed_spec_file_is_a_loud_error_rather_than_zero_cards() {
        let directory = write(r#"[{"source_id": "abc", "face": {"kind": "weather"}}]"#);
        let error = load_specs(&directory.path().join("cards.json"))
            .expect_err("a weather card without a location is refused");
        assert!(
            error.contains("location"),
            "the message names the field: {error}"
        );
    }

    #[test]
    fn an_unknown_field_is_refused_rather_than_silently_ignored() {
        // `deny_unknown_fields` is what turns a typo into a startup error
        // instead of a card that quietly uses a default forever.
        let directory = write(
            r#"[{"source_id": "abc", "face": {"kind": "weather", "location": "Dubai", "unit": "imperial"}}]"#,
        );
        assert!(
            load_specs(&directory.path().join("cards.json")).is_err(),
            "\"unit\" is not \"units\" and must not be accepted"
        );
    }

    #[test]
    fn an_unknown_outer_field_is_refused_too() {
        // The regression `flatten` caused: with it, `deny_unknown_fields` on
        // this struct rejected the face's own fields, so it had to be removed
        // and every typo -- inner and outer -- became a silent default.
        let directory = write(
            r#"[{"source_id": "abc", "refresh_second": 60, "face": {"kind": "weather", "location": "Dubai"}}]"#,
        );
        assert!(
            load_specs(&directory.path().join("cards.json")).is_err(),
            "\"refresh_second\" is not \"refresh_seconds\""
        );
    }

    #[test]
    fn an_unknown_kind_is_refused() {
        let directory =
            write(r#"[{"source_id": "abc", "face": {"kind": "calendar", "url": "x"}}]"#);
        assert!(
            load_specs(&directory.path().join("cards.json")).is_err(),
            "the retired card kinds do not come back through this file"
        );
    }

    #[test]
    fn an_empty_source_id_is_refused() {
        let directory =
            write(r#"[{"source_id": "  ", "face": {"kind": "weather", "location": "Dubai"}}]"#);
        let error = load_specs(&directory.path().join("cards.json"))
            .expect_err("a blank source id is refused");
        assert!(error.contains("source_id"));
    }

    #[test]
    fn imperial_units_reach_the_provider() {
        let directory = write(
            r#"[{"source_id": "abc", "face": {"kind": "weather", "location": "Austin", "units": "imperial"}}]"#,
        );
        let specs = load_specs(&directory.path().join("cards.json")).expect("the spec parses");
        match &specs[0].face {
            FaceSpec::Weather { units, .. } => {
                assert!(matches!(WeatherUnits::from(*units), WeatherUnits::Imperial));
            }
            other => panic!("expected a weather card, got {other:?}"),
        }
    }

    #[test]
    fn descriptor_fields_are_derived_from_a_real_face_spec() {
        let face = FaceSpec::Weather {
            location: "Dubai".into(),
            units: Units::Metric,
        };

        let descriptor = face_descriptor(&face);

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
        let descriptor = face_descriptor(&FaceSpec::Weather {
            location: "Dubai".into(),
            units: Units::Metric,
        });
        let fields = BTreeMap::from([("locaton".into(), "Berlin".into())]);

        assert!(matches!(
            validate_face_fields(&descriptor, &fields),
            Err(FaceUpdateError::UnknownField(key)) if key == "locaton"
        ));
    }

    #[test]
    fn a_non_http_face_url_is_refused() {
        let descriptor = face_descriptor(&FaceSpec::Rss {
            url: "https://example.test/feed".into(),
            title: "News".into(),
        });
        let fields = BTreeMap::from([("url".into(), "file:///etc/passwd".into())]);

        assert!(matches!(
            validate_face_fields(&descriptor, &fields),
            Err(FaceUpdateError::InvalidField { field, .. }) if field == "url"
        ));
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
            face: FaceSpec::Weather {
                location: "Dubai".into(),
                units: Units::Metric,
            },
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
            "create" => create_face(&state, &runtime, "new-source", "weather").map(|_| ()),
            "remove" => remove_face(&state, "target"),
            _ => panic!("unknown test operation"),
        };
        assert!(matches!(result, Err(FaceUpdateError::Persist(_))));
        tokio::task::yield_now().await;
        {
            let retained = state.inner.data_cards.lock().expect("data cards");
            assert_eq!(retained.spec_path, path);
            assert_eq!(retained.specs, original);
            assert_eq!(retained.tasks.len(), 1);
            assert_eq!(retained.tasks["target"].id(), task_id);
            assert!(!retained.tasks["target"].is_finished());
        }
        assert!(!abort.is_finished(), "the original task was not aborted");
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
    async fn remove_persistence_failure_preserves_specs_and_tasks() {
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
            face: FaceSpec::Token {
                coin_id: "solana".into(),
                currency: "eur".into(),
                api_key: Some("still-secret".into()),
            },
        };
        let original = vec![
            DataCardSpec {
                source_id: "target".into(),
                refresh_seconds: 600,
                face: FaceSpec::Rss {
                    url: "https://example.test/old.xml".into(),
                    title: "Old title".into(),
                },
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
        assert!(matches!(
            &rewritten[0].face,
            FaceSpec::Rss { url, title }
                if url == "https://example.test/new.xml" && title == "New title"
        ));
        state.shutdown();
    }
}
