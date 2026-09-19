use std::time::Duration;

use chrono::Utc;
use providers::Provider as _;
use providers::rss::{RssOptions, RssProvider};
use providers::token::{TokenOptions, TokenProvider};
use providers::weather::{WeatherOptions, WeatherProvider, WeatherUnits};
use tokio::task::JoinHandle;

use super::{DataCardSpec, FaceSpec, Units};
use crate::ServerState;
use crate::egress_client::EgressHttpClient;
use crate::face_render::frame_from_svg;
use crate::faces::{adapt, rss, token, weather};
use crate::image_sources::AcceptOutcome;

/// A refresh no faster than this, whatever a spec asks for. The individual
/// providers have their own floors too; this one bounds the whole loop.
const MIN_REFRESH: Duration = Duration::from_secs(60);
/// An unreachable source is retried on its own interval, but never slower than
/// this, so a card that failed once during a network blip does not sit stale
/// for a day.
const MAX_REFRESH: Duration = Duration::from_hours(6);

impl From<Units> for WeatherUnits {
    fn from(units: Units) -> Self {
        match units {
            Units::Metric => Self::Metric,
            Units::Imperial => Self::Imperial,
        }
    }
}

pub(super) fn spawn_refresher(
    runtime: &tokio::runtime::Handle,
    state: ServerState,
    spec: DataCardSpec,
) -> JoinHandle<()> {
    let refresh = Duration::from_secs(spec.refresh_seconds).clamp(MIN_REFRESH, MAX_REFRESH);
    runtime.spawn(async move { refresh_loop(state, spec, refresh).await })
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
        FaceProvider::Weather { provider } => {
            let snapshot = provider.refresh(now);
            let face = adapt::weather_face(&snapshot.value);
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
            let face = adapt::token_face(&snapshot.value);
            Ok((token::render(&face), snapshot.error))
        }
    }
}

/// One live provider, with the extra the adapter needs beside it.
enum FaceProvider {
    Weather {
        provider: Box<WeatherProvider<EgressHttpClient>>,
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
                    },
                )),
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

async fn refresh_loop(state: ServerState, spec: DataCardSpec, refresh: Duration) {
    let mut provider = FaceProvider::build(&spec, refresh);
    let source_id = spec.source_id.clone();
    let kind = provider.label();
    tracing::info!(target: "server::data_cards",
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
        let outcome = tokio::task::spawn_blocking(move || {
            let rendered = render_face(&mut provider)
                .map(|(svg, data_error)| (frame_from_svg(&svg), data_error));
            (rendered, provider)
        })
        .await;
        let (rendered, returned) = match outcome {
            Ok(pair) => pair,
            Err(error) => {
                tracing::error!(target: "server::data_cards", source_id = %source_id, kind, %error, "the render task panicked");
                return;
            }
        };
        provider = returned;

        let rasterized = match rendered {
            Ok((frame, data_error)) => {
                if let Some(error) = data_error {
                    // The face still renders -- from the last good reading --
                    // and the store's own push history is what marks it stale.
                    tracing::warn!(target: "server::data_cards",
                        source_id = %source_id,
                        kind,
                        %error,
                        "the data fetch failed; redrawing the last good reading"
                    );
                }
                frame
            }
            Err(error) => {
                tracing::warn!(target: "server::data_cards",
                    source_id = %source_id,
                    kind,
                    %error,
                    "the card has no reading yet; nothing to draw"
                );
                continue;
            }
        };

        let frame = match rasterized {
            Ok(frame) => frame,
            Err(error) => {
                // This is our SVG, so this is our bug, not the feed's.
                tracing::error!(target: "server::data_cards", source_id = %source_id, kind, %error, "the authored face did not rasterize");
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
                        tracing::warn!(target: "server::data_cards",
                            source_id = %notified,
                            %error,
                            "the frame is stored but the device was not notified; the next synchronize reconciles it"
                        );
                    }
                });
            }
            Ok(Ok(AcceptOutcome::Unchanged)) => {
                tracing::debug!(target: "server::data_cards", source_id = %source_id, kind, "the face is unchanged");
            }
            Ok(Err(error)) => {
                tracing::warn!(target: "server::data_cards", source_id = %source_id, kind, %error, "the frame was not stored");
            }
            Err(error) => {
                tracing::error!(target: "server::data_cards", source_id = %source_id, kind, %error, "the store task panicked");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refresh_interval_is_clamped_at_both_ends() {
        let fast = Duration::from_secs(1).clamp(MIN_REFRESH, MAX_REFRESH);
        let slow = Duration::from_secs(999_999).clamp(MIN_REFRESH, MAX_REFRESH);
        assert_eq!(fast, MIN_REFRESH);
        assert_eq!(slow, MAX_REFRESH);
    }
}
