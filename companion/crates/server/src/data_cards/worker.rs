use std::time::Duration;

use chrono::Utc;
use providers::http::HttpClient;
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

/// A refresh no faster than this, whatever a spec asks for, for every provider.
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
    let refresh = clamped_refresh(spec.refresh_seconds);
    runtime.spawn(async move { refresh_loop(state, spec, refresh).await })
}

fn clamped_refresh(refresh_seconds: u64) -> Duration {
    Duration::from_secs(refresh_seconds).clamp(MIN_REFRESH, MAX_REFRESH)
}

/// Renders one spec's face, fetching whatever it needs.
///
/// Synchronous and network-touching, so it must be called from
/// `spawn_blocking` -- [`EgressHttpClient`] blocks on the runtime. The
/// provider is passed in and handed back so its last-good state survives
/// across refreshes: without that, one failed fetch would blank a face that
/// has a perfectly good previous reading.
fn render_face<C: HttpClient>(
    provider: &mut FaceProvider<C>,
) -> Result<(String, Option<String>), String> {
    let now = Utc::now();
    match provider {
        FaceProvider::Weather { provider } => {
            let snapshot = provider.refresh();
            // A refresh that failed with no previous reading has nothing to
            // draw: a default reading would render a confident "0°" for a
            // card that has never worked.
            let Some(value) = snapshot.value else {
                return Err(snapshot.error.unwrap_or_default());
            };
            let face = adapt::weather_face(&value);
            Ok((weather::render(&face), snapshot.error))
        }
        FaceProvider::Rss { provider, title } => {
            let snapshot = provider.refresh();
            let Some(value) = snapshot.value else {
                return Err(snapshot.error.unwrap_or_default());
            };
            let face = adapt::rss_face(&value, title, now);
            Ok((rss::render(&face), snapshot.error))
        }
        FaceProvider::Token { provider } => {
            let snapshot = provider.refresh();
            let Some(value) = snapshot.value else {
                return Err(snapshot.error.unwrap_or_default());
            };
            let face = adapt::token_face(&value);
            Ok((token::render(&face), snapshot.error))
        }
    }
}

/// One live provider, with the extra the adapter needs beside it.
enum FaceProvider<C> {
    Weather {
        provider: Box<WeatherProvider<C>>,
    },
    Rss {
        provider: Box<RssProvider<C>>,
        title: String,
    },
    Token {
        provider: Box<TokenProvider<C>>,
    },
}

impl FaceProvider<EgressHttpClient> {
    /// Builds the provider for a spec.
    ///
    /// Constructed inside the runtime because [`EgressHttpClient::new`]
    /// captures the current handle.
    fn build(spec: &DataCardSpec) -> Self {
        match &spec.face {
            FaceSpec::Weather { location, units } => Self::Weather {
                provider: Box::new(WeatherProvider::new(
                    EgressHttpClient::new(),
                    WeatherOptions {
                        location: location.clone(),
                        units: (*units).into(),
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
    let mut provider = FaceProvider::build(&spec);
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
    use std::collections::VecDeque;

    use providers::ProviderError;

    use super::*;

    struct FakeClient {
        responses: VecDeque<Result<String, ProviderError>>,
    }

    impl HttpClient for FakeClient {
        fn get_text(&mut self, _url: &str) -> Result<String, ProviderError> {
            self.responses.pop_front().expect("a response is queued")
        }
    }

    fn assert_cached_face_retains_error(mut provider: FaceProvider<FakeClient>, expected: &str) {
        let (first_face, first_error) =
            render_face(&mut provider).expect("the first fetch succeeds");
        assert!(first_error.is_none());
        frame_from_svg(&first_face).expect("the reading renders a valid frame");

        let (cached_face, error) =
            render_face(&mut provider).expect("a failed fetch still renders the cached reading");
        assert_eq!(cached_face, first_face);
        assert_eq!(error.as_deref(), Some(expected));
    }

    #[test]
    fn cached_weather_renders_with_the_refresh_error() {
        let client = FakeClient {
            responses: VecDeque::from([
                Ok(include_str!("../../../providers/tests/fixtures/weather-location.json").into()),
                Ok(include_str!("../../../providers/tests/fixtures/weather-current.json").into()),
                Err(ProviderError::Timeout),
            ]),
        };
        assert_cached_face_retains_error(
            FaceProvider::Weather {
                provider: Box::new(WeatherProvider::new(
                    client,
                    WeatherOptions {
                        location: "Tbilisi".into(),
                        units: WeatherUnits::Metric,
                    },
                )),
            },
            "provider request timed out",
        );
    }

    #[test]
    fn cached_rss_renders_with_the_refresh_error() {
        let client = FakeClient {
            responses: VecDeque::from([
                Ok(
                    "<rss><channel><item><title>Cached headline</title></item></channel></rss>"
                        .into(),
                ),
                Err(ProviderError::HttpStatus(503)),
            ]),
        };
        assert_cached_face_retains_error(
            FaceProvider::Rss {
                provider: Box::new(RssProvider::new(
                    client,
                    RssOptions {
                        url: "https://example.test/feed".into(),
                        maximum_items: 4,
                    },
                )),
                title: "News".into(),
            },
            "provider returned HTTP 503",
        );
    }

    #[test]
    fn cached_token_renders_with_the_refresh_error() {
        let client = FakeClient {
            responses: VecDeque::from([
                Ok(r#"[{"symbol":"sol","name":"Solana","current_price":142.37}]"#.into()),
                Ok(r#"{"prices":[[1,140.0],[2,142.37]]}"#.into()),
                Err(ProviderError::HttpStatus(429)),
            ]),
        };
        assert_cached_face_retains_error(
            FaceProvider::Token {
                provider: Box::new(TokenProvider::new(
                    client,
                    TokenOptions {
                        coin_id: "solana".into(),
                        currency: "usd".into(),
                        api_key: None,
                    },
                )),
            },
            "provider returned HTTP 429",
        );
    }

    #[test]
    fn the_refresh_interval_is_clamped_at_both_ends() {
        for (seconds, expected) in [
            (0, 60),
            (1, 60),
            (60, 60),
            (900, 900),
            (21_600, 21_600),
            (999_999, 21_600),
            (u64::MAX, 21_600),
        ] {
            assert_eq!(clamped_refresh(seconds), Duration::from_secs(expected));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn first_refresh_failures_return_the_existing_error_without_a_face() {
        for (face, expected) in [
            (
                FaceSpec::Weather {
                    location: String::new(),
                    units: Units::Metric,
                },
                "invalid provider configuration: weather location is empty",
            ),
            (
                FaceSpec::Token {
                    coin_id: "Solana".into(),
                    currency: "usd".into(),
                    api_key: None,
                },
                "invalid provider configuration: the token's coin id may use only lowercase letters, digits and h",
            ),
            (
                FaceSpec::Rss {
                    url: "http://192.168.1.1/feed.xml".into(),
                    title: "News".into(),
                },
                "invalid provider configuration: RFC1918 private",
            ),
        ] {
            let result = tokio::task::spawn_blocking(move || {
                let spec = DataCardSpec {
                    source_id: "test".into(),
                    refresh_seconds: 900,
                    face,
                };
                render_face(&mut FaceProvider::build(&spec))
            })
            .await
            .expect("the blocking refresh runs");
            assert_eq!(result, Err(expected.to_owned()));
        }
    }
}
