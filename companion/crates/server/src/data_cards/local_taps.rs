//! Per-push ring snapshots. Indexes refer to the pushed snapshot, never to the
//! newly rotated selection, so 1, 2, 0 remains meaningful without another push.
use std::sync::Arc;

use super::{DataCardSpec, face_state::FaceStateStore, faces_package, worker};
use crate::{ImageNotificationOrigin, ServerState, accounts::AccountSpace};
use app_core::{ImageTapRing, ImageTapStep};

fn context(
    state: &ServerState,
    space: &AccountSpace,
    source: &str,
) -> Option<(
    DataCardSpec,
    Arc<FaceStateStore>,
    faces_package::FaceCommand,
)> {
    let (spec, stored) = {
        let cards = space
            .data_cards
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            cards
                .specs
                .iter()
                .find(|spec| spec.source_id == source)?
                .clone(),
            Arc::clone(&cards.face_state),
        )
    };
    let faces = state
        .inner
        .face_catalog
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .faces
        .clone()?;
    Some((spec, stored, faces))
}

pub(crate) fn request_local_tap_staging(
    state: &ServerState,
    space: &Arc<AccountSpace>,
    source: &str,
    runtime: &tokio::runtime::Handle,
) {
    let state = state.clone();
    let space = Arc::clone(space);
    let source = source.to_owned();
    std::mem::drop(runtime.spawn_blocking(move || {
        if let Some((spec, stored, faces)) = context(&state, &space, &source) {
            worker::stage_local_taps(&state, &space, &faces, &stored, &spec);
        }
    }));
}

/// Iterates the selector with the state returned by EACH preceding step. The
/// zero-tap answer names the current view without guessing face-specific state.
#[allow(clippy::type_complexity)]
pub(super) fn compute_ring(
    initial: Option<serde_json::Value>,
    mut select: impl FnMut(
        Option<&serde_json::Value>,
        u32,
    ) -> Result<faces_package::TapSelection, faces_package::FaceRenderError>,
) -> Result<(Vec<(String, Option<serde_json::Value>)>, bool), faces_package::FaceRenderError> {
    let start = select(initial.as_ref(), 0)?;
    let mut state = start.state.unwrap_or(initial);
    let mut steps = vec![(start.view, state.clone())];
    for _ in 0..protocol::MAX_TAP_VIEWS {
        let selection = select(state.as_ref(), 1)?;
        if steps.iter().any(|(view, _)| *view == selection.view) {
            let wraps = selection.view == steps[0].0;
            return Ok((steps, wraps));
        }
        if let Some(next) = selection.state {
            state = next;
        }
        steps.push((selection.view, state.clone()));
    }
    // A capped sequence must stop so its next tap can use the server path.
    Ok((steps, false))
}

pub(crate) fn tapped_local(
    state: &ServerState,
    space: &Arc<AccountSpace>,
    source: &str,
    ring: &ImageTapRing,
    index: u8,
) -> bool {
    let Some(step) = ring.steps.get(usize::from(index)) else {
        return false;
    };
    let Some((spec, stored, faces)) = context(state, space, source) else {
        return false;
    };
    let transition = stored.transition(source);
    let mut generation = transition
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if ring.plugin
        && (index != 1
            || !space
                .image_sources
                .promote_local_next(source, ring)
                .unwrap_or(false))
    {
        return false;
    }
    // Built-in selection already moved synchronously on the runtime thread.
    // Do not rewind it here while later device events await durable state commits.
    stored.put(source, step.state.clone());
    *generation = generation.wrapping_add(1);
    if !ring.plugin {
        // Recompute the next-push ring from the committed state using the same
        // selector, retaining only frames already in this pushed snapshot.
        if let Ok((steps, wraps)) = compute_ring(step.state.clone(), |value, taps| {
            faces_package::tap(&faces, &spec.face.kind, &spec.face.settings, value, taps)
        }) {
            let mut next = ImageTapRing {
                generation: *generation,
                steps: Vec::new(),
                wrap: wraps,
                plugin: false,
            };
            for (view, state) in steps {
                let Some(previous) = ring.steps.iter().find(|step| step.selector_view == view)
                else {
                    next.wrap = false;
                    break;
                };
                let mut step = previous.clone();
                step.state = state;
                next.steps.push(step);
            }
            space.image_sources.set_local_tap_ring(source, next);
        }
        return true;
    }
    drop(generation);
    // Promotion is already committed. Only the background prefetch may render.
    let state = state.clone();
    let space = Arc::clone(space);
    std::mem::drop(tokio::task::spawn_blocking(move || {
        worker::stage_local_taps(&state, &space, &faces, &stored, &spec);
        if let Some(frame) = space
            .image_sources
            .frame(&spec.source_id, chrono::Utc::now())
        {
            state.notify_image_source_changed(
                &space.account_id,
                spec.source_id,
                frame.digest,
                ImageNotificationOrigin::ServerRenderedRefresh,
            );
        }
    }));
    true
}

pub(super) fn step(
    view: String,
    digest: [u8; 32],
    state: Option<serde_json::Value>,
) -> ImageTapStep {
    ImageTapStep {
        selector_view: view.clone(),
        view,
        digest,
        state,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_cards::FaceSpec;
    use crate::image_ingest::canonical_frame_from_png;
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn ring_stops_on_repeat_and_caps_four_steps_with_each_returned_state() {
        for pages in [1_u64, 3, 9] {
            let (steps, wraps) = compute_ring(Some(serde_json::json!(0)), |value, taps| {
                let page = (value.unwrap().as_u64().unwrap() + u64::from(taps)) % pages;
                Ok(faces_package::TapSelection {
                    view: page.to_string(),
                    state: Some(Some(serde_json::json!(page))),
                })
            })
            .unwrap();
            assert_eq!(steps.len(), usize::try_from(pages).unwrap().min(5));
            assert_eq!(wraps, pages < 5);
            for (index, (_, state)) in steps.iter().enumerate() {
                assert_eq!(state, &Some(serde_json::json!(index)));
            }
        }
    }

    struct Rig {
        state: ServerState,
        space: Arc<AccountSpace>,
        spec: DataCardSpec,
        stored: Arc<FaceStateStore>,
        faces: faces_package::FaceCommand,
        _dir: tempfile::TempDir,
    }

    impl Rig {
        fn new(plugin: bool) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let script = dir.path().join("faces");
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support");
            // Real subprocess seam, distinct bytes and returned state for next.
            std::fs::write(&script, format!(r#"#!/bin/sh
case "$1" in
describe) printf '[{{"kind":"probe","label":"Probe","fields":[],"tap":"Next","origin":"{origin}","views":{selector},"selector":{selector}}}]' ;;
tap|render)
request=$(cat)
page=$(printf '%s' "$request" | sed -n 's/.*"page":\([0-9]*\).*/\1/p')
page=${{page:-0}}
taps=$(printf '%s' "$request" | sed -n 's/.*"taps":\([0-9]*\).*/\1/p')
if [ "$1" = tap ]; then
 page=$(( (page + ${{taps:-0}}) % 3 ))
 view="page-$page"
 [ "$page" = 0 ] && view=""
 printf '{{"view":"%s","state":{{"page":%s}}}}' "$view" "$page"
else
 page=$(( page + ${{taps:-0}} ))
 image=fake-face.png
 [ "$((page % 2))" = 1 ] && image=fake-face-alt.png
 printf '{{"png":"%s","state":{{"page":%s}}}}' "$(base64 < '{root}'/"$image" | tr -d '\n')" "$page"
fi ;;
esac
"#, origin=if plugin {"plugin"} else {"builtin"}, selector=if plugin {"false"} else {"true"}, root=root.display())).unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
            let state = ServerState::in_memory();
            let account = state
                .identity()
                .create_account("local@example.com", true, true, chrono::Utc::now())
                .unwrap();
            let space = state.account_space(&account.id);
            let source = space.image_sources.mint("Probe").unwrap().id;
            let faces = faces_package::FaceCommand::program(script);
            {
                let mut catalog = state.inner.face_catalog.lock().unwrap();
                catalog.faces = Some(faces.clone());
                catalog.catalog = None;
            }
            let spec = DataCardSpec {
                source_id: source.clone(),
                refresh_seconds: 900,
                face: FaceSpec {
                    kind: "probe".into(),
                    settings: BTreeMap::new(),
                },
            };
            let stored = {
                let mut cards = space.data_cards.lock().unwrap();
                cards.specs.push(spec.clone());
                Arc::clone(&cards.face_state)
            };
            stored.put(&source, Some(serde_json::json!({"page": 0})));
            let png = std::fs::read(root.join("fake-face.png")).unwrap();
            space
                .image_sources
                .accept_server_rendered(
                    &source,
                    canonical_frame_from_png(&png).unwrap(),
                    chrono::Utc::now(),
                )
                .unwrap();
            space.image_sources.enable_local_taps(&source);
            Self {
                state,
                space,
                spec,
                stored,
                faces,
                _dir: dir,
            }
        }
        fn stage(&self) {
            worker::stage_local_taps(
                &self.state,
                &self.space,
                &self.faces,
                &self.stored,
                &self.spec,
            );
        }
        fn ring(&self) -> ImageTapRing {
            self.space
                .image_sources
                .local_tap_ring(&self.spec.source_id)
                .unwrap()
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            self.state.shutdown();
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn local_index_commits_without_push_and_original_indexes_survive_rotation_and_wrap() {
        let fixture = Rig::new(false);
        fixture.stage();
        let ring = fixture.ring();
        assert_eq!(ring.steps.len(), 3);
        assert!(ring.wrap);
        let notifications = fixture.state.image_notifications_for_test().len();
        for index in [1_u8, 2, 0] {
            let step = &ring.steps[usize::from(index)];
            assert!(
                fixture
                    .space
                    .image_sources
                    .select_digest(&fixture.spec.source_id, step.digest)
            );
            assert!(tapped_local(
                &fixture.state,
                &fixture.space,
                &fixture.spec.source_id,
                &ring,
                index
            ));
            assert_eq!(fixture.stored.get(&fixture.spec.source_id), step.state);
        }
        assert_eq!(
            fixture.state.image_notifications_for_test().len(),
            notifications
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ring_budget_truncation_stops_instead_of_wrapping_over_missing_steps() {
        let fixture = Rig::new(false);
        let other = fixture.space.image_sources.mint("Other").unwrap().id;
        let png = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/fake-face.png"),
        )
        .unwrap();
        for index in 0..6 {
            fixture
                .space
                .image_sources
                .accept_staged_view(
                    &other,
                    &format!("other-{index}"),
                    canonical_frame_from_png(&png).unwrap(),
                    chrono::Utc::now(),
                )
                .unwrap();
        }
        fixture.stage();
        let ring = fixture.ring();
        assert_eq!(ring.steps.len(), 2);
        assert!(!ring.wrap);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_existing_full_staging_budget_is_reused_for_local_views() {
        let fixture = Rig::new(false);
        let png = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/fake-face.png"),
        )
        .unwrap();
        for index in 0..7 {
            fixture
                .space
                .image_sources
                .accept_staged_view(
                    &fixture.spec.source_id,
                    &format!("legacy-{index}"),
                    canonical_frame_from_png(&png).unwrap(),
                    chrono::Utc::now(),
                )
                .unwrap();
        }
        fixture.stage();
        assert_eq!(fixture.ring().steps.len(), 3);
        assert!(fixture.ring().wrap);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn plugin_promotes_state_and_frame_once_then_prefetches_another_next() {
        let fixture = Rig::new(true);
        fixture.stage();
        let ring = fixture.ring();
        assert_eq!(ring.steps.len(), 2);
        assert!(!ring.wrap);
        assert_eq!(
            fixture.stored.get(&fixture.spec.source_id),
            Some(serde_json::json!({"page":0})),
            "prefetch must not commit state"
        );
        assert!(tapped_local(
            &fixture.state,
            &fixture.space,
            &fixture.spec.source_id,
            &ring,
            1
        ));
        assert_eq!(
            fixture.stored.get(&fixture.spec.source_id),
            Some(serde_json::json!({"page":1}))
        );
        assert_eq!(
            fixture
                .space
                .image_sources
                .frame(&fixture.spec.source_id, chrono::Utc::now())
                .unwrap()
                .digest,
            ring.steps[1].digest
        );
        assert!(
            !tapped_local(
                &fixture.state,
                &fixture.space,
                &fixture.spec.source_id,
                &ring,
                1
            ),
            "repeated terminal index must render through the ordinary path"
        );
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if let Some(next) = fixture
                    .space
                    .image_sources
                    .local_tap_ring(&fixture.spec.source_id)
                    && next.generation != ring.generation
                {
                    assert_eq!(next.steps[1].state, Some(serde_json::json!({"page":2})));
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
}
