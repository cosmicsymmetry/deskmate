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

use std::path::Path;
use std::time::Duration;

use chrono::Utc;
use providers::Provider as _;
use providers::rss::{RssOptions, RssProvider};
use providers::token::{TokenOptions, TokenProvider};
use providers::weather::{WeatherOptions, WeatherProvider, WeatherUnits};
use serde::Deserialize;

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

/// One server-rendered card.
///
/// The face config is a nested object rather than flattened into this struct,
/// and that is not a style choice: serde's `deny_unknown_fields` and
/// `flatten` do not work together -- the outer struct rejects every flattened
/// field as unknown. Flattening therefore meant giving up the typo protection,
/// which is the one thing this file most needs, because a spec is hand-written
/// and a silently-defaulted field presents as a card that is subtly wrong
/// forever. Nesting costs one level of braces and keeps both sides strict.
#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Units {
    #[default]
    Metric,
    Imperial,
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
    for spec in &specs {
        if spec.source_id.trim().is_empty() {
            return Err(format!("{}: a spec has an empty source_id", path.display()));
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

/// Starts one refresh task per spec.
///
/// Each card gets its own task so a slow feed cannot delay a token price, and
/// so one card's repeated failure is visible as one card's problem.
pub fn spawn_refreshers(state: &ServerState, specs: Vec<DataCardSpec>) {
    for spec in specs {
        let refresh = Duration::from_secs(spec.refresh_seconds).clamp(MIN_REFRESH, MAX_REFRESH);
        let state = state.clone();
        tokio::spawn(async move { refresh_loop(state, spec, refresh).await });
    }
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
}
