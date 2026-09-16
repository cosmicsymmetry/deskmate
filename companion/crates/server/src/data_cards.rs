//! Server-rendered data cards: the server acting as its own picture producer.
//!
//! # What this is, and what it deliberately is not
//!
//! A weather, RSS or token card here is an **image source the server pushes to
//! itself**. The device config owns the face settings; a task in this module
//! fetches the data on an interval, renders the face, and hands
//! the frame to [`crate::image_sources::ImageSourceStore::accept`] -- the exact
//! call an external producer's PNG arrives through.
//!
//! Everything downstream is therefore already built and already proven on
//! hardware: the durable asset, the keep-set, the device notify, the
//! staleness inference, the GC. This module adds a producer, not a delivery
//! path.
//!
use std::time::Duration;

use app_core::{AppConfig, CardSettings, RefreshPolicy};
use chrono::Utc;
use providers::Provider as _;
use providers::rss::{RssOptions, RssProvider};
use providers::token::{TokenOptions, TokenProvider};
use providers::weather::{WeatherOptions, WeatherProvider, WeatherUnits};

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

/// The internal rendering input for one first-class data card. It stays
/// separate from `CardSettings` so the provider/render loop has one small,
/// stable vocabulary and cannot accidentally retain the whole device config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataCardSpec {
    /// A server-owned source derived from immutable card identity.
    pub source_id: String,
    pub refresh_seconds: u64,
    pub face: FaceSpec,
}

/// Which face, and what it needs to know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaceSpec {
    Weather {
        /// A place name, geocoded by the provider. Not coordinates: the owner
        /// types a city, and the geocoder's own display name is what the face
        /// shows, so the panel says what the forecast is actually for.
        location: String,
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
        currency: String,
        api_key: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Units {
    #[default]
    Metric,
    Imperial,
}

impl From<app_core::Units> for Units {
    fn from(units: app_core::Units) -> Self {
        match units {
            app_core::Units::Metric => Self::Metric,
            app_core::Units::Imperial => Self::Imperial,
        }
    }
}

impl From<Units> for WeatherUnits {
    fn from(units: Units) -> Self {
        match units {
            Units::Metric => Self::Metric,
            Units::Imperial => Self::Imperial,
        }
    }
}

/// Derives the server's complete refresher set from the validated device
/// document. One data-card object becomes one spec and one deterministic image
/// source; ordinary picture cards remain external producers and are skipped.
pub fn specs_from_config(config: &AppConfig) -> Vec<DataCardSpec> {
    config
        .cards
        .iter()
        .filter_map(|card| {
            let refresh_seconds = match card.refresh() {
                RefreshPolicy::Interval { minutes } => u64::from(minutes).saturating_mul(60),
                RefreshPolicy::DeviceLocal | RefreshPolicy::Manual => return None,
            };
            let source_id = card.source_id()?.into_owned();
            let face = match card {
                CardSettings::Weather {
                    location, units, ..
                } => FaceSpec::Weather {
                    location: location.clone(),
                    units: (*units).into(),
                },
                CardSettings::Rss {
                    feed_url,
                    feed_title,
                    ..
                } => FaceSpec::Rss {
                    url: feed_url.clone(),
                    title: feed_title.clone(),
                },
                CardSettings::Token {
                    coin_id,
                    currency,
                    api_key,
                    ..
                } => FaceSpec::Token {
                    coin_id: coin_id.clone(),
                    currency: currency.clone(),
                    api_key: api_key.clone(),
                },
                CardSettings::Clock { .. }
                | CardSettings::Pomodoro { .. }
                | CardSettings::Picture { .. } => return None,
            };
            Some(DataCardSpec {
                source_id,
                refresh_seconds,
                face,
            })
        })
        .collect()
}

/// Registers every deterministic source before its refresher can produce a
/// frame. The store creates these records without minting or returning a bearer
/// token: the producer is this process and calls `accept` directly.
pub(crate) fn ensure_sources(state: &ServerState, specs: &[DataCardSpec]) -> Result<(), String> {
    let sources: Vec<_> = specs
        .iter()
        .map(|spec| (spec.source_id.clone(), face_name(&spec.face).to_owned()))
        .collect();
    state
        .image_sources()
        .ensure_server_sources(&sources, protocol::MAX_ASSET_DIGESTS)
        .map_err(|error| error.to_string())
}

fn face_name(face: &FaceSpec) -> &'static str {
    match face {
        FaceSpec::Weather { .. } => "Weather",
        FaceSpec::Rss { .. } => "RSS",
        FaceSpec::Token { .. } => "Token",
    }
}

/// Rebuilds the refresher sets from every persisted device config at process
/// start. A board is normally powered off, so tying this work to a WebSocket
/// connection would leave its server-rendered cards frozen precisely during
/// the ordinary disconnected state.
pub async fn start_refreshers_from_stored_configs(state: &ServerState) {
    for device_id in state.registry_device_ids() {
        let device_config = state.configs().for_device(&device_id);
        let load_store = std::sync::Arc::clone(&device_config);
        let Ok(outcome) = tokio::task::spawn_blocking(move || load_store.store.load()).await else {
            tracing::error!(device_id = %device_id, "data-card config loader panicked");
            continue;
        };
        let config = outcome.into_config();
        let specs = specs_from_config(&config);
        let ensure_state = state.clone();
        let ensure_specs = specs.clone();
        match tokio::task::spawn_blocking(move || ensure_sources(&ensure_state, &ensure_specs))
            .await
        {
            Ok(Ok(())) => replace_refreshers(state, &device_id, specs),
            Ok(Err(error)) => tracing::error!(
                device_id = %device_id,
                %error,
                "server-rendered card sources could not be prepared"
            ),
            Err(_) => tracing::error!(
                device_id = %device_id,
                "data-card source preparation panicked"
            ),
        }
    }
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
pub fn spawn_refreshers(
    state: &ServerState,
    specs: Vec<DataCardSpec>,
) -> Vec<tokio::task::JoinHandle<()>> {
    specs
        .into_iter()
        .map(|spec| {
            let refresh = Duration::from_secs(spec.refresh_seconds).clamp(MIN_REFRESH, MAX_REFRESH);
            let state = state.clone();
            tokio::spawn(async move { refresh_loop(state, spec, refresh).await })
        })
        .collect()
}

/// Cancels the old tasks for one device and installs the set derived from its
/// newly accepted config. Task ownership is per device even though v2 is
/// single-tenant, so a later multi-device deployment cannot make one save
/// cancel another device's cards.
pub(crate) fn replace_refreshers(state: &ServerState, device_id: &str, specs: Vec<DataCardSpec>) {
    let handles = spawn_refreshers(state, specs);
    state.replace_data_card_refreshers(device_id, handles);
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
    use app_core::{CardAlert, Units as ConfigUnits, WidgetTapAction};

    #[test]
    fn the_three_specs_are_derived_from_real_config_card_types() {
        let config = AppConfig {
            cards: vec![
                CardSettings::Weather {
                    id: "outside".into(),
                    title: "Dubai".into(),
                    location: "Dubai".into(),
                    units: ConfigUnits::Imperial,
                    tap_action: WidgetTapAction::None,
                    refresh: RefreshPolicy::Interval { minutes: 15 },
                    alert: CardAlert::None,
                    dwell_seconds: None,
                },
                CardSettings::Rss {
                    id: "headlines".into(),
                    title: "News".into(),
                    feed_url: "https://example.test/feed".into(),
                    feed_title: "Hacker News".into(),
                    tap_action: WidgetTapAction::None,
                    refresh: RefreshPolicy::Interval { minutes: 30 },
                    alert: CardAlert::None,
                    dwell_seconds: None,
                },
                CardSettings::Token {
                    id: "market".into(),
                    title: "SOL".into(),
                    coin_id: "solana".into(),
                    currency: "eur".into(),
                    api_key: Some("demo-key".into()),
                    tap_action: WidgetTapAction::None,
                    refresh: RefreshPolicy::Interval { minutes: 5 },
                    alert: CardAlert::None,
                    dwell_seconds: None,
                },
            ],
            ..AppConfig::default()
        };
        config.validate().expect("the real config is valid");

        let specs = specs_from_config(&config);
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].source_id, "card-outside");
        assert_eq!(specs[0].refresh_seconds, 900);
        assert_eq!(
            specs[0].face,
            FaceSpec::Weather {
                location: "Dubai".into(),
                units: Units::Imperial,
            }
        );
        assert_eq!(specs[1].source_id, "card-headlines");
        assert_eq!(specs[1].refresh_seconds, 1_800);
        assert_eq!(
            specs[1].face,
            FaceSpec::Rss {
                url: "https://example.test/feed".into(),
                title: "Hacker News".into(),
            }
        );
        assert_eq!(specs[2].source_id, "card-market");
        assert_eq!(specs[2].refresh_seconds, 300);
        assert_eq!(
            specs[2].face,
            FaceSpec::Token {
                coin_id: "solana".into(),
                currency: "eur".into(),
                api_key: Some("demo-key".into()),
            }
        );
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
        assert!(matches!(
            WeatherUnits::from(Units::Imperial),
            WeatherUnits::Imperial
        ));
    }
}
