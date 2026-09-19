use std::time::Duration;

use chrono::Utc;
use providers::http::HttpClient;
use providers::rss::{RssOptions, RssProvider};
use providers::token::{TokenOptions, TokenProvider};
use providers::weather::{WeatherOptions, WeatherProvider, WeatherUnits};
use tokio::task::JoinHandle;

use super::{DataCardSpec, FaceSpec, Units, render_snapshot};
use crate::egress_client::EgressHttpClient;
use crate::face_render::frame_from_svg;
use crate::faces::{adapt, rss, token, weather};
use crate::image_sources::AcceptOutcome;
use crate::{ImageNotificationOrigin, ServerState};

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
/// provider is passed in and handed back so the same instance can move into
/// the next blocking task. Failed refreshes do not publish: the durable image
/// source retains the previous frame without renewing its freshness.
fn render_face<C: HttpClient>(provider: &mut FaceProvider<C>) -> Result<String, String> {
    let now = Utc::now();
    match provider {
        FaceProvider::Weather { provider } => render_snapshot(provider.refresh(), |value| {
            weather::render(&adapt::weather_face(value))
        }),
        FaceProvider::Rss { provider, title } => render_snapshot(provider.refresh(), |value| {
            rss::render(&adapt::rss_face(value, title, now))
        }),
        FaceProvider::Token { provider } => render_snapshot(provider.refresh(), |value| {
            token::render(&adapt::token_face(value))
        }),
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
            let rendered = render_face(&mut provider).map(|svg| frame_from_svg(&svg));
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
            Ok(frame) => frame,
            Err(error) => {
                tracing::warn!(target: "server::data_cards",
                    source_id = %source_id,
                    kind,
                    %error,
                    "the data fetch failed; keeping the stored frame unchanged"
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
            Ok(Ok(outcome)) => {
                if !notify_image_source_outcome(&state, &source_id, &outcome) {
                    tracing::debug!(target: "server::data_cards", source_id = %source_id, kind, "the face is unchanged");
                }
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

fn notify_image_source_outcome(
    state: &ServerState,
    source_id: &str,
    outcome: &AcceptOutcome,
) -> bool {
    match outcome {
        AcceptOutcome::Changed { digest } => {
            state.notify_image_source_changed(
                source_id.to_owned(),
                *digest,
                ImageNotificationOrigin::ServerRenderedRefresh,
            );
            true
        }
        AcceptOutcome::Unchanged => false,
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

    #[test]
    fn a_token_chart_failure_still_publishes_the_successful_quote() {
        let mut provider = FaceProvider::Token {
            provider: Box::new(TokenProvider::new(
                FakeClient {
                    responses: VecDeque::from([
                        Ok(r#"[{"symbol":"sol","name":"Solana","current_price":142.37}]"#.into()),
                        Err(ProviderError::Timeout),
                    ]),
                },
                TokenOptions {
                    coin_id: "solana".into(),
                    currency: "usd".into(),
                    api_key: None,
                },
            )),
        };
        let svg = render_face(&mut provider).expect("a quote needs no chart");
        assert!(svg.contains("142.37"));
        frame_from_svg(&svg).expect("the successful quote remains publishable");
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

    #[tokio::test]
    async fn changed_outcomes_notify_devices_and_unchanged_outcomes_do_not() {
        let state = ServerState::in_memory();
        let digest = [0x5a; protocol::ASSET_DIGEST_LEN];

        assert!(notify_image_source_outcome(
            &state,
            "server-face",
            &AcceptOutcome::Changed { digest },
        ));
        assert_eq!(
            state.image_notifications_for_test(),
            [(
                "server-face".to_owned(),
                digest,
                ImageNotificationOrigin::ServerRenderedRefresh,
            )]
        );

        assert!(!notify_image_source_outcome(
            &state,
            "server-face",
            &AcceptOutcome::Unchanged,
        ));
        assert_eq!(state.image_notifications_for_test().len(), 1);
    }
}
