use std::time::Duration;

use chrono::Utc;
use tokio::task::JoinHandle;

use super::DataCardSpec;
use super::faces_package::{self, FaceCommand, FaceRenderError};
use crate::image_ingest::{CanonicalFrame, canonical_frame_from_png};
use crate::image_sources::AcceptOutcome;
use crate::{ImageNotificationOrigin, ServerState};

/// A refresh no faster than this, whatever a spec asks for, for every face.
const MIN_REFRESH: Duration = Duration::from_secs(60);
/// An unreachable source is retried on its own interval, but never slower than
/// this, so a card that failed once during a network blip does not sit stale
/// for a day.
const MAX_REFRESH: Duration = Duration::from_hours(6);

pub(super) fn spawn_refresher(
    runtime: &tokio::runtime::Handle,
    state: ServerState,
    faces: FaceCommand,
    spec: DataCardSpec,
) -> JoinHandle<()> {
    let refresh = clamped_refresh(spec.refresh_seconds);
    runtime.spawn(async move { refresh_loop(state, faces, spec, refresh).await })
}

fn clamped_refresh(refresh_seconds: u64) -> Duration {
    Duration::from_secs(refresh_seconds).clamp(MIN_REFRESH, MAX_REFRESH)
}

/// Why one refresh published nothing. Every variant keeps the stored frame
/// without renewing its freshness; they differ in who has to act.
#[derive(Debug, PartialEq, Eq)]
enum RefreshFailure {
    /// The owner must change a setting.
    Configuration(String),
    /// Try again next tick.
    Transient(String),
    /// The package answered, but not with a frame the asset path accepts. That is
    /// a bug in `companion/faces`, not in any feed.
    NotAFrame(String),
}

/// Runs the faces package once and turns its PNG into the canonical frame.
///
/// Blocking -- it waits on a subprocess -- so it must be called from
/// `spawn_blocking`. The PNG goes through the same ingest function an external
/// producer's POST does, which is what makes "the server is just another
/// producer" true rather than merely intended.
fn render_frame(
    faces: &FaceCommand,
    spec: &DataCardSpec,
) -> Result<CanonicalFrame, RefreshFailure> {
    let png =
        faces_package::render(faces, &spec.face.kind, &spec.face.settings).map_err(|error| {
            match error {
                FaceRenderError::Configuration(message) => RefreshFailure::Configuration(message),
                FaceRenderError::Transient(message) => RefreshFailure::Transient(message),
            }
        })?;
    canonical_frame_from_png(&png).map_err(|error| RefreshFailure::NotAFrame(error.to_string()))
}

async fn refresh_loop(
    state: ServerState,
    faces: FaceCommand,
    spec: DataCardSpec,
    refresh: Duration,
) {
    let source_id = spec.source_id.clone();
    let kind = spec.face.kind.clone();
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

        let render_faces = faces.clone();
        let render_spec = spec.clone();
        let rendered =
            tokio::task::spawn_blocking(move || render_frame(&render_faces, &render_spec)).await;
        let frame = match rendered {
            Ok(Ok(frame)) => frame,
            Ok(Err(RefreshFailure::Configuration(error))) => {
                tracing::warn!(target: "server::data_cards", source_id = %source_id, kind, %error,
                    "the face's settings need the owner's attention; keeping the stored frame unchanged");
                continue;
            }
            Ok(Err(RefreshFailure::Transient(error))) => {
                tracing::warn!(target: "server::data_cards", source_id = %source_id, kind, %error,
                    "the data fetch failed; keeping the stored frame unchanged");
                continue;
            }
            Ok(Err(RefreshFailure::NotAFrame(error))) => {
                tracing::error!(target: "server::data_cards", source_id = %source_id, kind, %error,
                    "the faces package did not produce an acceptable frame");
                continue;
            }
            Err(error) => {
                tracing::error!(target: "server::data_cards", source_id = %source_id, kind, %error, "the render task panicked");
                return;
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
    use std::collections::BTreeMap;

    use super::*;
    use crate::data_cards::FaceSpec;

    fn spec(steer: &str) -> DataCardSpec {
        DataCardSpec {
            source_id: "source".into(),
            refresh_seconds: 900,
            face: FaceSpec {
                kind: "weather".into(),
                settings: BTreeMap::from([("location".into(), steer.into())]),
            },
        }
    }

    #[test]
    fn a_rendered_png_becomes_the_frame_a_producers_post_would() {
        let frame = render_frame(&faces_package::fake(), &spec("Dubai")).expect("a frame");
        let posted = canonical_frame_from_png(
            &std::fs::read(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/fake-face.png"
            ))
            .expect("the fixture is readable"),
        )
        .expect("the fixture is a valid frame");
        assert_eq!(frame.digest, posted.digest);
    }

    #[test]
    fn output_the_asset_path_would_refuse_is_our_bug_not_the_feeds() {
        for steer in ["answer-with-garbage", "answer-with-wrong-size"] {
            assert!(matches!(
                render_frame(&faces_package::fake(), &spec(steer)),
                Err(RefreshFailure::NotAFrame(_))
            ));
        }
    }

    #[test]
    fn the_two_refusals_keep_their_meaning_through_the_worker() {
        assert!(matches!(
            render_frame(&faces_package::fake(), &spec("refuse-as-configuration")),
            Err(RefreshFailure::Configuration(_))
        ));
        assert!(matches!(
            render_frame(&faces_package::fake(), &spec("refuse-as-transient")),
            Err(RefreshFailure::Transient(_))
        ));
    }

    #[test]
    fn a_failed_refresh_keeps_the_stored_frame_and_does_not_renew_its_freshness() {
        use crate::image_sources::ImageSourceStore;
        use chrono::TimeZone as _;

        let directory = tempfile::tempdir().expect("temp directory");
        let store = ImageSourceStore::new(directory.path().to_path_buf()).expect("store");
        let source = store.mint("News").expect("source");
        let at = |seconds: i64| Utc.timestamp_opt(1_700_000_000 + seconds, 0).unwrap();
        // What the refresh loop does with one tick: render, and accept only a frame.
        let publish = |steer: &str, now| {
            render_frame(&faces_package::fake(), &spec(steer))
                .map(|frame| store.accept(&source.id, frame, now).expect("accept"))
        };

        for seconds in [0, 60, 120, 180] {
            publish("Dubai", at(seconds)).expect("a successful refresh");
        }
        let before = store.frame(&source.id, at(180)).unwrap();
        let disk_before = std::fs::read(directory.path().join("image-sources.json")).unwrap();

        for seconds in [240, 600, 1_200] {
            assert!(publish("refuse-as-transient", at(seconds)).is_err());
            assert!(publish("refuse-as-configuration", at(seconds)).is_err());
        }
        let after = store.frame(&source.id, at(1_200)).unwrap();
        assert_eq!(
            after.digest, before.digest,
            "the pixels are the last good ones"
        );
        assert_eq!(store.summaries(at(1_200))[0].last_push, Some(at(180)));
        assert_eq!(
            std::fs::read(directory.path().join("image-sources.json")).unwrap(),
            disk_before,
            "a failure writes nothing"
        );
        assert!(
            after.stale,
            "and the face is allowed to go stale, which is the truth"
        );

        // An identical face is a no-op for bytes and still counts as liveness.
        assert_eq!(publish("Dubai", at(1_300)), Ok(AcceptOutcome::Unchanged));
        assert_eq!(store.summaries(at(1_300))[0].last_push, Some(at(1_300)));
        assert!(!store.frame(&source.id, at(1_300)).unwrap().stale);
        assert!(matches!(
            publish("alternate-frame", at(1_400)),
            Ok(AcceptOutcome::Changed { .. })
        ));
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

    #[test]
    fn refresh_is_clamped_at_both_ends() {
        assert_eq!(clamped_refresh(1), MIN_REFRESH);
        assert_eq!(clamped_refresh(900), Duration::from_mins(15));
        assert_eq!(clamped_refresh(u64::MAX), MAX_REFRESH);
    }
}
