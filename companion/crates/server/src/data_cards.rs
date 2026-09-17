//! Server-rendered data cards: the server acting as its own picture producer.
//!
//! # What this is, and what it deliberately is not
//!
//! A weather, RSS or token card here is an **image source the server pushes to
//! itself**. The owner creates one picture card pointing at the source; a task
//! in this module fetches the data on an interval, renders the face, and hands
//! the frame to [`crate::image_sources::ImageSourceStore::accept`] -- the exact
//! call an external producer's PNG arrives through.
//!
//! Everything downstream is therefore already built and already proven on
//! hardware: the durable asset, the keep-set, the device notify, the
//! staleness inference, the GC. This module adds a producer, not a delivery
//! path.
//!
//! What it is not is a new card *kind*. `docs/config/v10.md` still has three
//! (`clock`, `pomodoro`, `picture`) and nothing here changes that. Native
//! kinds would put these cards in the companion window with their own editors
//! instead of requiring a hand-written spec file plus a picture card pointed at
//! a source id -- that is schema v11's job, and it is a strictly cosmetic
//! improvement on top of this: the fetch, the faces and the frames do not
//! change.
//!
//! # Why the specs live in a file rather than in the config document
//!
//! Because the config document is the *companion's* document, and the
//! companion has no UI for these yet. Putting a half-schema in it -- fields no
//! window can author and no migration can repair -- is how this repository
//! ended up deleting two card families. A server-side file is honest about
//! where the authority currently sits and costs nothing to delete when the
//! window grows the editors.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use providers::Provider as _;
use providers::rss::{RssOptions, RssProvider};
use providers::token::{TokenOptions, TokenProvider};
use providers::weather::{WeatherOptions, WeatherProvider, WeatherUnits};
use serde::{Deserialize, Serialize};

use crate::ServerState;
use crate::egress_client::EgressHttpClient;
use crate::face_render::frame_from_svg;
use crate::faces::{adapt, rss, token, weather};
use crate::image_sources::AcceptOutcome;

/// The weather face draws six hourly columns.
const HOURLY_COLUMNS: usize = 6;
/// A refresh no faster than this, whatever a spec asks for. The individual
/// providers have their own floors too; this one bounds the whole loop.
const MIN_REFRESH: Duration = Duration::from_secs(60);
/// An unreachable source is retried on its own interval, but never slower than
/// this, so a card that failed once during a network blip does not sit stale
/// for a day.
const MAX_REFRESH: Duration = Duration::from_hours(6);
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
pub struct DataCardSpec {
    /// The image source this card pushes to, as `POST /v1/images` minted it.
    /// The owner's picture card names the same id.
    pub source_id: String,
    #[serde(default = "default_refresh_seconds")]
    pub refresh_seconds: u64,
    pub face: FaceSpec,
}

const fn default_refresh_seconds() -> u64 {
    900
}

/// Which face, and what it needs to know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum FaceSpec {
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
pub enum Units {
    #[default]
    Metric,
    Imperial,
}

/// The server-owned settings contract rendered by the companion. The app treats
/// `kind` as opaque metadata and renders only this descriptor's field types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FaceDescriptor {
    pub kind: String,
    pub label: String,
    pub fields: Vec<FaceFieldDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum FaceFieldDescriptor {
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
pub struct FaceFieldOption {
    pub value: String,
    pub label: String,
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
pub fn creatable_faces() -> Vec<FaceDescriptor> {
    [
        FaceSpec::Weather {
            location: String::new(),
            units: Units::Metric,
        },
        FaceSpec::Rss {
            url: String::new(),
            title: String::new(),
        },
        FaceSpec::Token {
            coin_id: String::new(),
            currency: default_currency(),
            api_key: None,
        },
    ]
    .iter()
    .map(face_descriptor)
    .collect()
}

/// A blank face of `kind`, or `None` if this server cannot draw that kind.
fn blank_face(kind: &str) -> Option<FaceSpec> {
    creatable_faces()
        .into_iter()
        .find(|descriptor| descriptor.kind == kind)
        .map(|_| match kind {
            "weather" => FaceSpec::Weather {
                location: String::new(),
                units: Units::Metric,
            },
            "rss" => FaceSpec::Rss {
                url: String::new(),
                title: String::new(),
            },
            _ => FaceSpec::Token {
                coin_id: String::new(),
                currency: default_currency(),
                api_key: None,
            },
        })
}

/// Whether a face has everything it needs to be fetched.
///
/// A card added from the menu starts blank, and a blank weather face has no city
/// to ask about. Fetching anyway would fail every cycle and fill the journal with
/// errors the owner cannot act on until they type one. So an incomplete face is
/// stored but not refreshed, and the card shows the ordinary "no frame yet"
/// state until its settings are filled in.
pub(crate) fn face_is_complete(face: &FaceSpec) -> bool {
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

pub fn face_descriptor(face: &FaceSpec) -> FaceDescriptor {
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
    pub(crate) spec_path: PathBuf,
    pub(crate) specs: Vec<DataCardSpec>,
    pub(crate) tasks: HashMap<String, tokio::task::JoinHandle<()>>,
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

impl From<Units> for WeatherUnits {
    fn from(units: Units) -> Self {
        match units {
            Units::Metric => Self::Metric,
            Units::Imperial => Self::Imperial,
        }
    }
}

/// Reads the spec file, or returns nothing when there is none.
///
/// A missing file is the ordinary case -- most deployments have no
/// server-rendered cards -- so it is not an error. A *malformed* file is, and
/// loudly: silently running with zero cards because a comma was missing would
/// present as "the panel stopped updating" with nothing in the log.
pub fn load_specs(path: &Path) -> Result<Vec<DataCardSpec>, String> {
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

/// Renders one spec's face, fetching whatever it needs.
///
/// Synchronous and network-touching, so it must be called from
/// `spawn_blocking` -- [`EgressHttpClient`] blocks on the runtime. The
/// provider is passed in and handed back so its last-good state survives
/// across refreshes: without that, one failed fetch would blank a face that
/// has a perfectly good previous reading.
fn render_face(provider: &mut FaceProvider) -> Result<(String, Option<String>), String> {
    let now = Utc::now();
    match provider {
        FaceProvider::Weather { provider, units } => {
            let snapshot = provider.refresh(now);
            let face = adapt::weather_face(&snapshot.value, *units, HOURLY_COLUMNS);
            // A refresh that failed with no previous reading has nothing to
            // draw: the location is empty and the temperature is zero, which
            // would render a confident "0°" for a card that has never worked.
            if snapshot.error.is_some() && snapshot.refreshed_at.is_none() {
                return Err(snapshot.error.unwrap_or_default());
            }
            Ok((weather::render(&face), snapshot.error))
        }
        FaceProvider::Rss { provider, title } => {
            let snapshot = provider.refresh(now);
            if snapshot.error.is_some() && snapshot.refreshed_at.is_none() {
                return Err(snapshot.error.unwrap_or_default());
            }
            let face = adapt::rss_face(&snapshot.value, title, now);
            Ok((rss::render(&face), snapshot.error))
        }
        FaceProvider::Token { provider } => {
            let snapshot = provider.refresh(now);
            if snapshot.error.is_some() && snapshot.refreshed_at.is_none() {
                return Err(snapshot.error.unwrap_or_default());
            }
            let face = adapt::token_face(&snapshot.value, "24h");
            Ok((token::render(&face), snapshot.error))
        }
    }
}

/// One live provider, with the extra the adapter needs beside it.
enum FaceProvider {
    Weather {
        provider: Box<WeatherProvider<EgressHttpClient>>,
        units: WeatherUnits,
    },
    Rss {
        provider: Box<RssProvider<EgressHttpClient>>,
        title: String,
    },
    Token {
        provider: Box<TokenProvider<EgressHttpClient>>,
    },
}

impl FaceProvider {
    /// Builds the provider for a spec.
    ///
    /// Constructed inside the runtime because [`EgressHttpClient::new`]
    /// captures the current handle.
    fn build(spec: &DataCardSpec, refresh: Duration) -> Self {
        match &spec.face {
            FaceSpec::Weather { location, units } => Self::Weather {
                provider: Box::new(WeatherProvider::new(
                    EgressHttpClient::new(),
                    WeatherOptions {
                        location: location.clone(),
                        units: (*units).into(),
                        refresh_interval: refresh,
                        title: String::new(),
                    },
                )),
                units: (*units).into(),
            },
            FaceSpec::Rss { url, title } => Self::Rss {
                provider: Box::new(RssProvider::new(
                    EgressHttpClient::new(),
                    RssOptions {
                        url: url.clone(),
                        // Four is what the face can show: one lead plus three
                        // followers. Asking for more would parse items nothing
                        // draws.
                        maximum_items: 4,
                        refresh_interval: refresh,
                        title: String::new(),
                    },
                )),
                title: title.clone(),
            },
            FaceSpec::Token {
                coin_id,
                currency,
                api_key,
            } => Self::Token {
                provider: Box::new(TokenProvider::new(
                    EgressHttpClient::new(),
                    TokenOptions {
                        coin_id: coin_id.clone(),
                        currency: currency.clone(),
                        refresh_interval: refresh,
                        api_key: api_key.clone(),
                    },
                )),
            },
        }
    }

    const fn label(&self) -> &'static str {
        match self {
            Self::Weather { .. } => "weather",
            Self::Rss { .. } => "rss",
            Self::Token { .. } => "token",
        }
    }
}

/// Retains the spec path, parsed specs, and one task handle per source in
/// [`ServerState`]. Each card gets its own task so a slow feed cannot delay a
/// token price, and retaining the handles lets a settings update replace only
/// the source it changed.
pub fn spawn_refreshers(state: &ServerState, spec_path: PathBuf, specs: Vec<DataCardSpec>) {
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
        data_cards
            .tasks
            .insert(spec.source_id.clone(), spawn_refresher(state.clone(), spec));
    }
}

fn spawn_refresher(state: ServerState, spec: DataCardSpec) -> tokio::task::JoinHandle<()> {
    let refresh = Duration::from_secs(spec.refresh_seconds).clamp(MIN_REFRESH, MAX_REFRESH);
    tokio::spawn(async move { refresh_loop(state, spec, refresh).await })
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
    let bytes = serde_json::to_vec_pretty(&updated_specs)
        .map_err(|error| FaceUpdateError::Encode(error.to_string()))?;
    app_core::secure_file::write_and_replace(&data_cards.spec_path, &bytes).map_err(|error| {
        let (operation, message) = error.into_strings("data-card spec");
        FaceUpdateError::Persist(format!("{operation}: {message}"))
    })?;

    let updated_spec = updated_specs[index].clone();
    data_cards.specs = updated_specs;
    if let Some(task) = data_cards.tasks.remove(source_id) {
        task.abort();
    }
    data_cards.tasks.insert(
        source_id.to_owned(),
        runtime.spawn(refresh_loop(
            state.clone(),
            updated_spec.clone(),
            Duration::from_secs(updated_spec.refresh_seconds).clamp(MIN_REFRESH, MAX_REFRESH),
        )),
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
    let bytes = serde_json::to_vec_pretty(&updated_specs)
        .map_err(|error| FaceUpdateError::Encode(error.to_string()))?;
    app_core::secure_file::write_and_replace(&data_cards.spec_path, &bytes).map_err(|error| {
        let (operation, message) = error.into_strings("data-card spec");
        FaceUpdateError::Persist(format!("{operation}: {message}"))
    })?;
    data_cards.specs = updated_specs;

    // A blank face is not fetched; see `face_is_complete`.
    if face_is_complete(&spec.face) {
        data_cards.tasks.insert(
            source_id.to_owned(),
            runtime.spawn(refresh_loop(
                state.clone(),
                spec.clone(),
                Duration::from_secs(spec.refresh_seconds).clamp(MIN_REFRESH, MAX_REFRESH),
            )),
        );
    }
    Ok(face_descriptor(&spec.face))
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

async fn refresh_loop(state: ServerState, spec: DataCardSpec, refresh: Duration) {
    let mut provider = FaceProvider::build(&spec, refresh);
    let source_id = spec.source_id.clone();
    let kind = provider.label();
    tracing::info!(
        source_id = %source_id,
        kind,
        refresh_seconds = refresh.as_secs(),
        "server-rendered card refreshing"
    );

    let mut ticker = tokio::time::interval(refresh);
    // The first tick fires immediately, which is what fills a freshly started
    // server's panels instead of leaving them blank for fifteen minutes.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        ticker.tick().await;

        // The whole fetch-and-render is blocking: the egress client blocks on
        // the runtime, and rasterizing is CPU work that has no business on an
        // async worker.
        let outcome =
            tokio::task::spawn_blocking(move || (render_face(&mut provider), provider)).await;
        let (rendered, returned) = match outcome {
            Ok(pair) => pair,
            Err(error) => {
                tracing::error!(source_id = %source_id, kind, %error, "the render task panicked");
                return;
            }
        };
        provider = returned;

        let svg = match rendered {
            Ok((svg, data_error)) => {
                if let Some(error) = data_error {
                    // The face still renders -- from the last good reading --
                    // and the store's own push history is what marks it stale.
                    tracing::warn!(
                        source_id = %source_id,
                        kind,
                        %error,
                        "the data fetch failed; redrawing the last good reading"
                    );
                }
                svg
            }
            Err(error) => {
                tracing::warn!(
                    source_id = %source_id,
                    kind,
                    %error,
                    "the card has no reading yet; nothing to draw"
                );
                continue;
            }
        };

        let frame = match frame_from_svg(&svg) {
            Ok(frame) => frame,
            Err(error) => {
                // This is our SVG, so this is our bug, not the feed's.
                tracing::error!(source_id = %source_id, kind, %error, "the authored face did not rasterize");
                continue;
            }
        };

        let accept_state = state.clone();
        let accept_source = source_id.clone();
        let accepted = tokio::task::spawn_blocking(move || {
            accept_state
                .image_sources()
                .accept(&accept_source, frame, Utc::now())
        })
        .await;

        match accepted {
            Ok(Ok(AcceptOutcome::Changed { digest })) => {
                let runtimes = crate::images::live_runtimes(&state);
                let notified = source_id.clone();
                tokio::task::spawn_blocking(move || {
                    if let Err(error) = crate::images::notify_runtimes(runtimes, &notified, digest)
                    {
                        tracing::warn!(
                            source_id = %notified,
                            %error,
                            "the frame is stored but the device was not notified; the next synchronize reconciles it"
                        );
                    }
                });
            }
            Ok(Ok(AcceptOutcome::Unchanged)) => {
                tracing::debug!(source_id = %source_id, kind, "the face is unchanged");
            }
            Ok(Err(error)) => {
                tracing::warn!(source_id = %source_id, kind, %error, "the frame was not stored");
            }
            Err(error) => {
                tracing::error!(source_id = %source_id, kind, %error, "the store task panicked");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_refresh_interval_is_clamped_at_both_ends() {
        let fast = Duration::from_secs(1).clamp(MIN_REFRESH, MAX_REFRESH);
        let slow = Duration::from_secs(999_999).clamp(MIN_REFRESH, MAX_REFRESH);
        assert_eq!(fast, MIN_REFRESH);
        assert_eq!(slow, MAX_REFRESH);
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
