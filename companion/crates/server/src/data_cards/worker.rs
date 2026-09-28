use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::task::JoinHandle;

use super::face_state::FaceStateStore;
use super::faces_package::{self, FaceCommand, FaceRenderError, RenderRequest};
use super::{DataCardSpec, RefreshOutcome, TapSignal, record_outcome};
use crate::accounts::AccountSpace;
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
    space: std::sync::Arc<AccountSpace>,
    faces: FaceCommand,
    face_state: Arc<FaceStateStore>,
    taps: Arc<TapSignal>,
    spec: DataCardSpec,
) -> JoinHandle<()> {
    let refresh = clamped_refresh(spec.refresh_seconds);
    spawn_refresher_with_refresh(
        runtime,
        RefresherJob::new(state, space, faces, face_state, taps, spec),
        refresh,
    )
}

pub(super) struct RefresherJob {
    state: ServerState,
    space: Arc<AccountSpace>,
    faces: FaceCommand,
    face_state: Arc<FaceStateStore>,
    taps: Arc<TapSignal>,
    spec: DataCardSpec,
}

impl RefresherJob {
    pub(super) fn new(
        state: ServerState,
        space: Arc<AccountSpace>,
        faces: FaceCommand,
        face_state: Arc<FaceStateStore>,
        taps: Arc<TapSignal>,
        spec: DataCardSpec,
    ) -> Self {
        Self {
            state,
            space,
            faces,
            face_state,
            taps,
            spec,
        }
    }
}

fn spawn_refresher_with_refresh(
    runtime: &tokio::runtime::Handle,
    job: RefresherJob,
    refresh: Duration,
) -> JoinHandle<()> {
    runtime.spawn(async move {
        refresh_loop(
            job.state,
            job.space,
            job.faces,
            job.face_state,
            job.taps,
            job.spec,
            refresh,
        )
        .await;
    })
}

#[cfg(test)]
pub(super) fn spawn_refresher_for_test(
    runtime: &tokio::runtime::Handle,
    job: RefresherJob,
    refresh: Duration,
) -> JoinHandle<()> {
    spawn_refresher_with_refresh(runtime, job, refresh)
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

#[allow(clippy::option_option)] // Absent keeps state; present null clears it.
struct RenderedFrame {
    frame: CanonicalFrame,
    state: Option<Option<serde_json::Value>>,
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
    state: Option<&serde_json::Value>,
    taps: u32,
) -> Result<RenderedFrame, RefreshFailure> {
    let rendered = faces_package::render(
        faces,
        RenderRequest {
            kind: &spec.face.kind,
            settings: &spec.face.settings,
            state,
            taps,
        },
    )
    .map_err(|error| match error {
        FaceRenderError::Configuration(message) => RefreshFailure::Configuration(message),
        FaceRenderError::Malformed(message) => RefreshFailure::NotAFrame(message),
        FaceRenderError::Transient(message) => RefreshFailure::Transient(message),
    })?;
    let faces_package::Rendered { png, state } = rendered;
    let frame = canonical_frame_from_png(&png)
        .map_err(|error| RefreshFailure::NotAFrame(error.to_string()))?;
    Ok(RenderedFrame { frame, state })
}

/// How long to wait before the next attempt.
///
/// A drawn face waits its whole interval, and so does one whose settings were
/// refused -- nothing changes until the owner edits them, and an edit restarts this
/// task. A TRANSIENT failure is retried soon, backing off to the interval: a keyless
/// API answers a burst with 429 for about a minute, and waiting fifteen for that left
/// a freshly created card on "Waiting for the first picture" for a quarter of an hour
/// with nothing wrong that a minute would not fix.
fn next_attempt(
    refresh: Duration,
    outcome: &RefreshOutcome,
    consecutive_failures: u32,
) -> Duration {
    match outcome {
        RefreshOutcome::Drawn | RefreshOutcome::NeedsAttention(_) => refresh,
        RefreshOutcome::Retrying(_) => {
            let doublings = consecutive_failures.saturating_sub(1).min(16);
            MIN_REFRESH.saturating_mul(1 << doublings).min(refresh)
        }
    }
}

/// One refresh: render, accept, notify. `None` means a blocking task panicked, which
/// is a bug rather than an outcome, and ends the refresher.
async fn refresh_once(
    state: &ServerState,
    space: &std::sync::Arc<AccountSpace>,
    faces: &FaceCommand,
    face_state: &Arc<FaceStateStore>,
    spec: &DataCardSpec,
    taps: u32,
) -> Option<RefreshOutcome> {
    let (source_id, kind) = (&spec.source_id, &spec.face.kind);
    let previous_state = face_state.get(source_id);
    let render_faces = faces.clone();
    let render_spec = spec.clone();
    let rendered = tokio::task::spawn_blocking(move || {
        render_frame(&render_faces, &render_spec, previous_state.as_ref(), taps)
    })
    .await;
    let rendered = match rendered {
        Ok(Ok(rendered)) => rendered,
        Ok(Err(RefreshFailure::Configuration(error))) => {
            tracing::warn!(target: "server::data_cards", source_id = %source_id, kind, %error,
                "the face's settings need the owner's attention; keeping the stored frame unchanged");
            return Some(RefreshOutcome::NeedsAttention(error));
        }
        Ok(Err(RefreshFailure::Transient(error))) => {
            tracing::warn!(target: "server::data_cards", source_id = %source_id, kind, %error,
                "the data fetch failed; keeping the stored frame unchanged");
            return Some(RefreshOutcome::Retrying(error));
        }
        Ok(Err(RefreshFailure::NotAFrame(error))) => {
            tracing::error!(target: "server::data_cards", source_id = %source_id, kind, %error,
                "the faces package did not produce an acceptable frame");
            return Some(RefreshOutcome::Retrying(format!(
                "the face could not be drawn: {error}"
            )));
        }
        Err(error) => {
            tracing::error!(target: "server::data_cards", source_id = %source_id, kind, %error, "the render task panicked");
            return None;
        }
    };

    let accept_source = source_id.clone();
    let accept_space = Arc::clone(space);
    let next_face_state = rendered.state;
    let face_state = Arc::clone(face_state);
    let accepted = tokio::task::spawn_blocking(move || {
        let outcome = accept_space.image_sources.accept_server_rendered(
            &accept_source,
            rendered.frame,
            Utc::now(),
        );
        if outcome.is_ok()
            && let Some(next_face_state) = next_face_state
        {
            face_state.put(&accept_source, next_face_state);
        }
        outcome
    })
    .await;
    match accepted {
        Ok(Ok(outcome)) => {
            if !notify_image_source_outcome(state, space, source_id, &outcome) {
                tracing::debug!(target: "server::data_cards", source_id = %source_id, kind, "the face is unchanged");
            }
            Some(RefreshOutcome::Drawn)
        }
        Ok(Err(error)) => {
            tracing::warn!(target: "server::data_cards", source_id = %source_id, kind, %error, "the frame was not stored");
            Some(RefreshOutcome::Retrying(format!(
                "the frame was not stored: {error}"
            )))
        }
        Err(error) => {
            tracing::error!(target: "server::data_cards", source_id = %source_id, kind, %error, "the store task panicked");
            None
        }
    }
}

async fn refresh_loop(
    state: ServerState,
    space: std::sync::Arc<AccountSpace>,
    faces: FaceCommand,
    face_state: Arc<FaceStateStore>,
    taps: Arc<TapSignal>,
    spec: DataCardSpec,
    refresh: Duration,
) {
    tracing::info!(target: "server::data_cards",
        source_id = %spec.source_id,
        kind = %spec.face.kind,
        refresh_seconds = refresh.as_secs(),
        "server-rendered card refreshing"
    );
    // `record_outcome` checks this id against the retained handle, so a refresher that
    // was replaced mid-render cannot overwrite its successor's status.
    let task = tokio::task::id();
    let mut consecutive_failures = 0_u32;
    let mut tap_count = 0;
    // The first attempt is immediate, which is what fills a freshly started server's
    // panels instead of leaving them blank for fifteen minutes.
    loop {
        let Some(outcome) =
            refresh_once(&state, &space, &faces, &face_state, &spec, tap_count).await
        else {
            return;
        };
        consecutive_failures = match outcome {
            RefreshOutcome::Retrying(_) => consecutive_failures.saturating_add(1),
            RefreshOutcome::Drawn | RefreshOutcome::NeedsAttention(_) => 0,
        };
        let wait = next_attempt(refresh, &outcome, consecutive_failures);
        record_outcome(&space, &spec.source_id, task, outcome);
        tap_count = tokio::select! {
            () = tokio::time::sleep(wait) => 0,
            () = taps.notify.notified() => taps.take().max(1),
        };
    }
}

fn notify_image_source_outcome(
    state: &ServerState,
    space: &AccountSpace,
    source_id: &str,
    outcome: &AcceptOutcome,
) -> bool {
    match outcome {
        AcceptOutcome::Changed { digest } => {
            state.notify_image_source_changed(
                &space.account_id,
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

    fn test_space(state: &ServerState) -> Arc<AccountSpace> {
        let account = state
            .identity()
            .create_account("owner@example.com", true, true, Utc::now())
            .expect("create account");
        state.account_space(&account.id)
    }

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
        let rendered =
            render_frame(&faces_package::fake(), &spec("Dubai"), None, 0).expect("a frame");
        let posted = canonical_frame_from_png(
            &std::fs::read(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/fake-face.png"
            ))
            .expect("the fixture is readable"),
        )
        .expect("the fixture is a valid frame");
        assert_eq!(rendered.frame.digest, posted.digest);
    }

    #[test]
    fn output_the_asset_path_would_refuse_is_our_bug_not_the_feeds() {
        for steer in ["answer-with-garbage", "answer-with-wrong-size"] {
            assert!(matches!(
                render_frame(&faces_package::fake(), &spec(steer), None, 0),
                Err(RefreshFailure::NotAFrame(_))
            ));
        }
    }

    #[test]
    fn the_two_refusals_keep_their_meaning_through_the_worker() {
        assert!(matches!(
            render_frame(
                &faces_package::fake(),
                &spec("refuse-as-configuration"),
                None,
                0,
            ),
            Err(RefreshFailure::Configuration(_))
        ));
        assert!(matches!(
            render_frame(
                &faces_package::fake(),
                &spec("refuse-as-transient"),
                None,
                0,
            ),
            Err(RefreshFailure::Transient(_))
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refresh_passes_stored_state_to_the_faces_package() {
        let directory = tempfile::tempdir().expect("temp directory");
        let face_state = Arc::new(FaceStateStore::load(
            directory.path().join("face-state.json"),
        ));
        face_state.put("source", Some(serde_json::json!({ "page": 2 })));
        let state = ServerState::in_memory();
        let space = test_space(&state);

        let outcome = refresh_once(
            &state,
            &space,
            &faces_package::fake(),
            &face_state,
            &spec("echo-the-request"),
            0,
        )
        .await
        .expect("the refresh task did not panic");
        let RefreshOutcome::Retrying(message) = outcome else {
            panic!("the fake package refuses this request on purpose");
        };
        assert!(message.contains(r#""state":{"page":2}"#), "{message}");
        assert_eq!(
            face_state.get("source"),
            Some(serde_json::json!({ "page": 2 })),
            "a failed render keeps the prior state"
        );
        state.shutdown();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn only_an_accepted_frame_applies_the_returned_state() {
        let directory = tempfile::tempdir().expect("temp directory");
        let face_state = Arc::new(FaceStateStore::load(
            directory.path().join("face-state.json"),
        ));
        let state = ServerState::in_memory();
        let space = test_space(&state);
        let source = space.image_sources.mint("Weather").expect("a source");

        let mut accepted_spec = spec("answer-with-an-envelope");
        accepted_spec.source_id.clone_from(&source.id);
        assert_eq!(
            refresh_once(
                &state,
                &space,
                &faces_package::fake(),
                &face_state,
                &accepted_spec,
                0,
            )
            .await,
            Some(RefreshOutcome::Drawn)
        );
        assert_eq!(
            face_state.get(&source.id),
            Some(serde_json::json!({ "page": 3 }))
        );

        let mut refused_spec = spec("answer-with-an-envelope");
        refused_spec.source_id = "missing-source".into();
        face_state.put(
            &refused_spec.source_id,
            Some(serde_json::json!({ "page": 2 })),
        );
        assert!(matches!(
            refresh_once(
                &state,
                &space,
                &faces_package::fake(),
                &face_state,
                &refused_spec,
                0,
            )
            .await,
            Some(RefreshOutcome::Retrying(_))
        ));
        assert_eq!(
            face_state.get(&refused_spec.source_id),
            Some(serde_json::json!({ "page": 2 })),
            "an unaccepted frame must not advance state"
        );

        let clear_source = space
            .image_sources
            .mint("Forecast")
            .expect("another source");
        face_state.put(
            &clear_source.id,
            Some(serde_json::json!({ "view": "days" })),
        );
        let mut clear_spec = spec("answer-with-a-null-state");
        clear_spec.source_id.clone_from(&clear_source.id);
        assert_eq!(
            refresh_once(
                &state,
                &space,
                &faces_package::fake(),
                &face_state,
                &clear_spec,
                0,
            )
            .await,
            Some(RefreshOutcome::Drawn)
        );
        assert_eq!(face_state.get(&clear_source.id), None);
        state.shutdown();
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
            render_frame(&faces_package::fake(), &spec(steer), None, 0).map(|rendered| {
                store
                    .accept(&source.id, rendered.frame, now)
                    .expect("accept")
            })
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
        let account = state
            .identity()
            .create_account("owner@example.com", true, true, Utc::now())
            .unwrap();
        let space = state.account_space(&account.id);
        let digest = [0x5a; protocol::ASSET_DIGEST_LEN];

        assert!(notify_image_source_outcome(
            &state,
            &space,
            "server-face",
            &AcceptOutcome::Changed { digest },
        ));
        assert_eq!(
            state.image_notifications_for_test(),
            [(
                account.id,
                "server-face".to_owned(),
                digest,
                ImageNotificationOrigin::ServerRenderedRefresh,
                Vec::new(),
            )]
        );

        assert!(!notify_image_source_outcome(
            &state,
            &space,
            "server-face",
            &AcceptOutcome::Unchanged,
        ));
        assert_eq!(state.image_notifications_for_test().len(), 1);
    }

    #[test]
    fn a_transient_failure_is_retried_soon_and_backs_off_to_the_interval() {
        let refresh = Duration::from_mins(15);
        let retrying = RefreshOutcome::Retrying("HTTP 429".into());
        let waits: Vec<u64> = (1..=6)
            .map(|failures| next_attempt(refresh, &retrying, failures).as_secs())
            .collect();
        assert_eq!(waits, [60, 120, 240, 480, 900, 900]);
        assert_eq!(
            next_attempt(refresh, &retrying, u32::MAX),
            refresh,
            "no overflow"
        );
        // A refresh faster than the retry floor is never made slower by a failure.
        assert_eq!(next_attempt(MIN_REFRESH, &retrying, 3), MIN_REFRESH);
        // Refused settings wait for the owner, not for the clock.
        let refused = RefreshOutcome::NeedsAttention("no such coin".into());
        assert_eq!(next_attempt(refresh, &refused, 0), refresh);
        assert_eq!(next_attempt(refresh, &RefreshOutcome::Drawn, 0), refresh);
    }

    #[test]
    fn refresh_is_clamped_at_both_ends() {
        assert_eq!(clamped_refresh(1), MIN_REFRESH);
        assert_eq!(clamped_refresh(900), Duration::from_mins(15));
        assert_eq!(clamped_refresh(u64::MAX), MAX_REFRESH);
    }
}
