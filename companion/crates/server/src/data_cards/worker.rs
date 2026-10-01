use std::path::Path;
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
///
/// `pub(super)`: `cadence_for_new_spec` in the parent module clamps a NEW
/// spec's stored cadence to this same range, so the persisted file never
/// claims a cadence this scheduler would silently override at run time.
pub(super) const MIN_REFRESH: Duration = Duration::from_secs(60);
/// An unreachable source is retried on its own interval, but never slower than
/// this, so a card that failed once during a network blip does not sit stale
/// for a day. See [`MIN_REFRESH`] for why this is `pub(super)`.
pub(super) const MAX_REFRESH: Duration = Duration::from_hours(6);

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
///
/// `account_dir` is this refresh's account's own directory (`AccountSpace::root`),
/// forwarded to the child as `DESKMATE_CONFIG_DIR` so a plugin can read its own
/// stored credential and never another account's.
fn render_frame(
    faces: &FaceCommand,
    account_dir: &Path,
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
            view: None,
        },
        account_dir,
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

/// Draws one named view of a face, for staging.
///
/// The view goes to the package as `view`, never as a tap count: a staged view is
/// not a tap, and telling the face otherwise would stamp "the owner just tapped"
/// into its state and hold the wrong view on the next scheduled refresh.
fn render_view_frame(
    faces: &FaceCommand,
    account_dir: &Path,
    spec: &DataCardSpec,
    state: Option<&serde_json::Value>,
    view: &str,
) -> Result<RenderedFrame, RefreshFailure> {
    let rendered = faces_package::render(
        faces,
        RenderRequest {
            kind: &spec.face.kind,
            settings: &spec.face.settings,
            state,
            taps: 0,
            view: Some(view),
        },
        account_dir,
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
    let started = std::time::Instant::now();
    if taps > 0 {
        tracing::info!(target: "server::tap_latency", account_id = %space.account_id,
            source_id = %source_id, taps, unix_us = chrono::Utc::now().timestamp_micros(),
            "tap render started");
    }
    let transition = face_state.transition(source_id);
    let (previous_state, generation) = {
        let guard = transition
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (face_state.get(source_id), *guard)
    };
    let render_faces = faces.clone();
    let render_spec = spec.clone();
    let render_account_dir = space.root.clone();
    let rendered = tokio::task::spawn_blocking(move || {
        render_frame(
            &render_faces,
            &render_account_dir,
            &render_spec,
            previous_state.as_ref(),
            taps,
        )
    })
    .await;
    if taps > 0 {
        tracing::info!(target: "server::tap_latency", account_id = %space.account_id,
            source_id = %source_id, taps, unix_us = chrono::Utc::now().timestamp_micros(),
            elapsed_us = started.elapsed().as_micros(), ok = matches!(&rendered, Ok(Ok(_))),
            "tap render completed");
    }
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
    let face_state_for_staging = Arc::clone(face_state);
    let next_face_state = rendered.state;
    let face_state = Arc::clone(face_state);
    let accepted = tokio::task::spawn_blocking(move || {
        let mut guard = transition
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *guard != generation {
            // Selection is allowed during this render. Its newer frame and
            // durable state win; never rewind the page when stale work finishes.
            return Ok(None);
        }
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
        if outcome.is_ok() {
            *guard = guard.wrapping_add(1);
        }
        outcome.map(Some)
    })
    .await;
    match accepted {
        Ok(Ok(None)) => Some(RefreshOutcome::Drawn),
        Ok(Ok(Some(outcome))) => {
            if !notify_image_source_outcome(state, space, source_id, &outcome) {
                tracing::debug!(target: "server::data_cards", source_id = %source_id, kind, "the face is unchanged");
            }
            stage_other_views(space, faces, face_state_for_staging, spec).await;
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

/// How many frames one source may hold, resting view included.
///
/// The real ceilings are the device's fifteen resident slots and the wire's
/// 32-digest `AssetRelease`, and both are shared across every picture card. Four
/// cards at four views each would exceed the first and approach the second, so
/// this is deliberately conservative: it keeps a four-card loop inside both with
/// room to spare. The spec's proportional budget -- `floor(15 / picture_cards)`
/// with the remainder to the cards that declared the most -- needs the device's
/// card count here, which this worker does not have; a flat cap is the honest
/// version of it until it does.
const MAX_STAGED_FRAMES_PER_SOURCE: usize = 3;

/// Draws the face's other views and stores each, so a tap on one costs a scene
/// and nothing else.
///
/// Every failure here is swallowed on purpose. The resting frame is already
/// stored and already on its way to the panel by the time this runs; staging is
/// an optimisation for a tap that may never come, and a face that cannot draw
/// its second view must not turn a delivered refresh into a failed one. A view
/// that is not staged is simply rendered on the tap that asks for it, which is
/// what happened before any of this existed.
async fn stage_other_views(
    space: &Arc<AccountSpace>,
    faces: &FaceCommand,
    face_state: Arc<FaceStateStore>,
    spec: &DataCardSpec,
) {
    let source_id = spec.source_id.clone();
    let stored = face_state.get(&source_id);
    let views_faces = faces.clone();
    let views_spec = spec.clone();
    let views_state = stored.clone();
    let Ok(Ok(views)) = tokio::task::spawn_blocking(move || {
        faces_package::views(
            &views_faces,
            &views_spec.face.kind,
            &views_spec.face.settings,
            views_state.as_ref(),
        )
    })
    .await
    else {
        // An older faces package answers non-zero here, which reads as "one
        // resting view" -- exactly what such a package can draw.
        return;
    };

    for view in views
        .into_iter()
        .filter(|view| !view.is_empty())
        .take(MAX_STAGED_FRAMES_PER_SOURCE.saturating_sub(1))
    {
        let render_faces = faces.clone();
        let render_spec = spec.clone();
        let render_state = stored.clone();
        let render_view = view.clone();
        let render_account_dir = space.root.clone();
        let drawn = tokio::task::spawn_blocking(move || {
            render_view_frame(
                &render_faces,
                &render_account_dir,
                &render_spec,
                render_state.as_ref(),
                &render_view,
            )
        })
        .await;
        let rendered = match drawn {
            Ok(Ok(rendered)) => rendered,
            Ok(Err(failure)) => {
                // Staging is optional, but a view that cannot be drawn should
                // leave a trace: silently skipping it makes "why is this tap
                // still slow?" unanswerable.
                tracing::warn!(target: "server::data_cards",
                    source_id = %source_id, view = %view, error = ?failure,
                    "a view could not be drawn for staging; a tap on it will render instead");
                continue;
            }
            Err(error) => {
                tracing::error!(target: "server::data_cards",
                    source_id = %source_id, view = %view, %error, "the staging render panicked");
                continue;
            }
        };
        let accept_space = Arc::clone(space);
        let accept_source = source_id.clone();
        let accept_view = view.clone();
        let staged = tokio::task::spawn_blocking(move || {
            accept_space.image_sources.accept_staged_view(
                &accept_source,
                &accept_view,
                rendered.frame,
                Utc::now(),
            )
        })
        .await;
        match staged {
            Ok(Ok(_)) => tracing::debug!(target: "server::data_cards",
                source_id = %source_id, view = %view, "staged a view for a tap"),
            Ok(Err(error)) => tracing::warn!(target: "server::data_cards",
                source_id = %source_id, view = %view, %error,
                "a view was drawn but not stored; a tap on it will render instead"),
            Err(error) => tracing::error!(target: "server::data_cards",
                source_id = %source_id, view = %view, %error, "the staging task panicked"),
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

    /// A stand-in for the calling refresher's `AccountSpace::root`, for tests that
    /// do not care which account it is, only that some directory is threaded
    /// through. The fake package only ever echoes it; it need not exist on disk.
    fn test_account_dir() -> &'static Path {
        Path::new("/test-account")
    }

    #[test]
    fn a_rendered_png_becomes_the_frame_a_producers_post_would() {
        let rendered = render_frame(
            &faces_package::fake(),
            test_account_dir(),
            &spec("Dubai"),
            None,
            0,
        )
        .expect("a frame");
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
                render_frame(
                    &faces_package::fake(),
                    test_account_dir(),
                    &spec(steer),
                    None,
                    0,
                ),
                Err(RefreshFailure::NotAFrame(_))
            ));
        }
    }

    #[test]
    fn the_two_refusals_keep_their_meaning_through_the_worker() {
        assert!(matches!(
            render_frame(
                &faces_package::fake(),
                test_account_dir(),
                &spec("refuse-as-configuration"),
                None,
                0,
            ),
            Err(RefreshFailure::Configuration(_))
        ));
        assert!(matches!(
            render_frame(
                &faces_package::fake(),
                test_account_dir(),
                &spec("refuse-as-transient"),
                None,
                0,
            ),
            Err(RefreshFailure::Transient(_))
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refresh_scopes_the_child_to_this_accounts_own_directory() {
        // End-to-end regression for the dropped `DESKMATE_CONFIG_DIR`: the server
        // clears the child's environment, and a plugin reads its stored credential
        // from `$DESKMATE_CONFIG_DIR/plugin-secrets.json` inside the child itself.
        // If `refresh_once` ever stops threading `space.root` through to the
        // subprocess, this fails -- it does not stop at "some path was passed" the
        // way the faces_package-level tests do, it pins the REAL account directory
        // this refresher was created for.
        let directory = tempfile::tempdir().expect("temp directory");
        let face_state = Arc::new(FaceStateStore::load(
            directory.path().join("face-state.json"),
        ));
        let state = ServerState::in_memory();
        let space = test_space(&state);

        let outcome = refresh_once(
            &state,
            &space,
            &faces_package::fake(),
            &face_state,
            &spec("echo-the-config-dir"),
            0,
        )
        .await
        .expect("the refresh task did not panic");
        let RefreshOutcome::Retrying(message) = outcome else {
            panic!("the fake package refuses this request on purpose");
        };
        assert_eq!(
            message,
            format!("DESKMATE_CONFIG_DIR={}", space.root.display())
        );
        state.shutdown();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn one_accounts_render_never_sees_another_accounts_secrets_file() {
        // The directory-passing tests above prove the right STRING reaches the
        // child. They do not prove the property that actually matters: with two
        // real accounts' `plugin-secrets.json` sitting on disk at once, an
        // account's render reads only its own file and never the other's, in the
        // request, the answer, or an error. `echo-the-secrets-file` in
        // fake-faces.sh stands in for a plugin's own `readPluginSecrets` call --
        // it reads `$DESKMATE_CONFIG_DIR/plugin-secrets.json` and reports the raw
        // bytes -- so this exercises the same isolation a real plugin gets.
        const SECRET_A: &str = "unique-secret-for-account-A-jsw8x2";
        const SECRET_B: &str = "unique-secret-for-account-B-r7fqe9";

        let directory = tempfile::tempdir().expect("temp directory");
        let face_state = Arc::new(FaceStateStore::load(
            directory.path().join("face-state.json"),
        ));
        let state = ServerState::in_memory();
        let space_a = test_space(&state);
        let account_b = state
            .identity()
            .create_account("other-owner@example.com", true, true, Utc::now())
            .expect("create a second account");
        let space_b = state.account_space(&account_b.id);
        assert_ne!(
            space_a.root, space_b.root,
            "two distinct on-disk directories"
        );

        std::fs::write(
            space_a.root.join("plugin-secrets.json"),
            format!(r#"{{"acct-marker":{{"token":"{SECRET_A}"}}}}"#),
        )
        .expect("write account A's secrets file");
        std::fs::write(
            space_b.root.join("plugin-secrets.json"),
            format!(r#"{{"acct-marker":{{"token":"{SECRET_B}"}}}}"#),
        )
        .expect("write account B's secrets file");

        // Nothing about the request Rust builds ever carries a secret -- it is
        // `{kind, settings, state?, event?}` -- so checking the message below
        // also stands for "not in the request", the third place the task asked
        // to check alongside the answer and the error.
        let message_a = render_secrets_message(&state, &space_a, &face_state).await;
        assert!(message_a.contains(SECRET_A), "{message_a}");
        assert!(
            !message_a.contains(SECRET_B),
            "account A's render must never see account B's secret: {message_a}"
        );

        let message_b = render_secrets_message(&state, &space_b, &face_state).await;
        assert!(message_b.contains(SECRET_B), "{message_b}");
        assert!(
            !message_b.contains(SECRET_A),
            "account B's render must never see account A's secret: {message_b}"
        );

        state.shutdown();
    }

    /// Runs `echo-the-secrets-file` for one account and returns the fake's
    /// message: whatever `$DESKMATE_CONFIG_DIR/plugin-secrets.json` held in
    /// that account's own directory, or `SECRETS=<none>` if it had none.
    async fn render_secrets_message(
        state: &ServerState,
        space: &Arc<AccountSpace>,
        face_state: &Arc<FaceStateStore>,
    ) -> String {
        let outcome = refresh_once(
            state,
            space,
            &faces_package::fake(),
            face_state,
            &spec("echo-the-secrets-file"),
            0,
        )
        .await
        .expect("the refresh task did not panic");
        let RefreshOutcome::Retrying(message) = outcome else {
            panic!("the fake package refuses this request on purpose");
        };
        message
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
            render_frame(
                &faces_package::fake(),
                test_account_dir(),
                &spec(steer),
                None,
                0,
            )
            .map(|rendered| {
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
