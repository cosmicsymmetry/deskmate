//! Building and pushing the face of one card.
//!
//! The pushed-field accessors, scene construction, render negotiation, the
//! scene push, and the durable asset reconciliation a
//! scene's digests require.
// These runtime internals intentionally share the parent module's worker
// types and helpers; a glob keeps that internal seam in one place.
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::state::{CardField, CardFieldValue};

pub(super) fn field_text<'a>(fields: &'a [CardField], key: &str) -> &'a str {
    fields
        .iter()
        .find_map(|field| match (&*field.key, &field.value) {
            (candidate, CardFieldValue::Text { value }) if candidate == key => Some(value.as_str()),
            _ => None,
        })
        .unwrap_or("")
}

pub(super) fn field_integer(fields: &[CardField], key: &str) -> i64 {
    fields
        .iter()
        .find_map(|field| match (&*field.key, &field.value) {
            (candidate, CardFieldValue::Integer { value }) if candidate == key => Some(*value),
            _ => None,
        })
        .unwrap_or(0)
}

pub(super) fn field_boolean(fields: &[CardField], key: &str) -> bool {
    fields
        .iter()
        .find_map(|field| match (&*field.key, &field.value) {
            (candidate, CardFieldValue::Boolean { value }) if candidate == key => Some(*value),
            _ => None,
        })
        .unwrap_or(false)
}

pub(super) fn build_card_scene(
    config: &AppConfig,
    card_id: &str,
    fields: &[CardField],
    image_source_host: Option<&mut dyn ImageSourceHost>,
    revision: u32,
) -> Result<PushScene, String> {
    let card = config
        .cards
        .iter()
        .find(|card| card.id() == card_id)
        .ok_or_else(|| format!("card {card_id:?} is not present in the active configuration"))?;
    let scene = if let CardSettings::Picture { source_id, .. } = card {
        let host = image_source_host.ok_or_else(|| {
            format!("card {card_id:?} cannot render because no image source host is configured")
        })?;
        match host.image_source_frame(source_id) {
            Some(frame) => with_stale_footer(frame_face_scene(revision, frame.digest), frame.stale),
            // No frame at all, so there is nothing to badge.
            None => waiting_for_first_picture_scene(revision),
        }
    } else {
        build_template_card_scene(card, card_id, fields, revision)?
    };
    let push = PushScene {
        tap_views: Vec::new(),
        tap_wrap: true,
        card_id: card_id.to_owned(),
        revision,
        scene,
    };
    validate_message(&Message::PushScene(push.clone()))
        .map_err(|error| format!("the host-built scene is invalid: {error}"))?;
    Ok(push)
}

/// The scene the device would receive for one card, built for display rather
/// than for pushing: the revision is fixed at 1 and nothing is minted, marked
/// dirty, or sent.
///
/// This keeps preview and panel rendering on the same scene-building path, so
/// the preview shows what the panel shows by construction.
///
/// A `Picture` card has no locally renderable face -- its frames live on the
/// server, where producers push them, and this process has no
/// [`ImageSourceHost`] -- so it is refused rather than drawn blank.
pub fn preview_card_scene(
    config: &AppConfig,
    card_id: &str,
    fields: &[crate::CardField],
) -> Result<protocol::Scene, String> {
    let card = config
        .cards
        .iter()
        .find(|card| card.id() == card_id)
        .ok_or_else(|| format!("card {card_id:?} is not present in the active configuration"))?;
    if matches!(card, CardSettings::Picture { .. }) {
        return Err(format!(
            "card {card_id:?} is a picture card; its frame lives on the server"
        ));
    }
    build_card_scene(config, card_id, fields, None, 1).map(|push| push.scene)
}

pub(super) fn waiting_for_first_picture_scene(revision: u32) -> protocol::Scene {
    let tier = protocol::SceneFontTier::Caption;
    let caption = BakedFontMetrics::SHIPPED.tier(tier);
    let top = (protocol::SCENE_CANVAS_HEIGHT - caption.line_height) / 2;
    protocol::Scene {
        revision,
        background: 0,
        nodes: vec![protocol::SceneNode::Text(protocol::SceneText {
            x: 0,
            baseline_y: top + BakedFontMetrics::SHIPPED.baseline_offset(tier),
            w: protocol::SCENE_CANVAS_WIDTH,
            align: protocol::SceneAlign::Center,
            font: protocol::SceneFont::Baked(tier),
            color: 0x00f5_f5f7,
            running_color: None,
            value: protocol::SceneValue::Literal("Waiting for the first picture".into()),
            ellipsize: false,
        })],
    }
}

pub(super) fn build_template_card_scene(
    card: &CardSettings,
    card_id: &str,
    fields: &[CardField],
    revision: u32,
) -> Result<protocol::Scene, String> {
    let scene = match card.template() {
        Some(DisplayTemplate::DigitalClock) => build_digital_clock_scene(&ClockCard {
            revision,
            show_seconds: field_boolean(fields, "show_seconds"),
        }),
        Some(DisplayTemplate::AnalogClock) => build_analog_clock_scene(&AnalogClockCard {
            revision,
            show_seconds: field_boolean(fields, "show_seconds"),
        }),
        Some(DisplayTemplate::ProgressRing) => build_progress_ring_scene(&ProgressRingCard {
            revision,
            label: field_text(fields, "label"),
            duration_seconds: field_integer(fields, "duration_seconds"),
        }),
        None => {
            return Err(format!("card {card_id:?} has no display template"));
        }
    };
    Ok(scene)
}

pub(super) fn record_scene_refusal(state: &mut WorkerState, card_id: String, message: String) {
    state.push_rejections.insert(
        card_id.clone(),
        CardError {
            kind: CardErrorKind::SceneRefused,
            card_id,
            message,
        },
    );
}

/// Classifies every device-layer outcome for an automatic scene. This match is
/// deliberately exhaustive: a future `DeviceError` variant must choose an explicit
/// retry and visibility policy rather than inheriting silent retry behavior.
pub(super) fn handle_automatic_scene_error(
    state: &mut WorkerState,
    card_id: String,
    error: DeviceError,
    reconnect_interval: Duration,
) {
    match error {
        error if is_wrong_tier(&error) => mark_ownership_refused(state),
        DeviceError::Rejected(error) if error.code == protocol::ErrorCode::Busy => {
            // Firmware uses Busy for zero-timeout LVGL lock contention and OTA
            // takeover. The scene is still valid and no user edit can fix it;
            // retain the event so the next render phase tries it again.
            state.active_scene_dirty = true;
        }
        DeviceError::Rejected(error) => record_scene_refusal(
            state,
            card_id,
            format!(
                "the display refused this card's scene ({:?}): {}",
                error.code, error.diagnostic
            ),
        ),
        DeviceError::MissingCapabilities {
            required,
            available,
        } => record_scene_refusal(
            state,
            card_id,
            format!(
                "the display no longer advertises declarative scene rendering (required {required:#018x}, available {available:#018x}); reconnect to use its legacy widget renderer"
            ),
        ),
        DeviceError::Timeout
        | DeviceError::Transport(_)
        | DeviceError::MalformedResponse(_)
        | DeviceError::UnexpectedMessage => {
            // Timeouts, transport failures, and malformed/unexpected responses
            // describe the link, not the card. Keep the scene pending and let
            // ordinary status polling decide whether connection state changes.
            state.active_scene_dirty = true;
        }
        error @ DeviceError::NoDevice => {
            // Unlike a dropped response, NoDevice is already a definitive
            // connection observation. Enter the normal reconnect path now.
            mark_disconnected(state, &error, Instant::now(), reconnect_interval);
        }
        error @ (DeviceError::VersionMismatch(_)
        | DeviceError::InvalidRequest
        | DeviceError::RevisionExhausted) => {
            // These are terminal host/session faults, not defects in this card's
            // content and not transient link loss. Make them globally visible and
            // do not retry the identical request forever.
            state.runtime = RuntimeState::Error {
                message: format!("automatic scene delivery failed: {error}"),
            };
        }
    }
}

/// Validates the scene against the device's render profile.
pub(super) fn native_requirements(
    state: &mut WorkerState,
    scene: &protocol::Scene,
) -> Result<render_negotiation::RenderRequirements, String> {
    let requirements = render_negotiation::analyze_scene(scene);
    let profile = render_negotiation::DeviceRenderProfile {
        capabilities: state.device.capability_bits(),
        confirmed_assets: state.confirmed_resident_assets.clone(),
        installable_assets: if requirements.asset_digests.is_empty() {
            BTreeSet::new()
        } else {
            state
                .image_source_host
                .as_mut()
                .map(|host| {
                    host.desired_assets()
                        .iter()
                        .map(|asset| asset.digest)
                        .collect()
                })
                .unwrap_or_default()
        },
    };
    render_negotiation::validate_native_scene(&requirements, &profile)?;
    Ok(requirements)
}

/// Rebuilds the active card only after a host-owned event marks it dirty.
/// Device-side bindings keep clock and pomodoro facts moving between these
/// event-driven pushes; picture faces are rebuilt when their producer changes
/// the digest or when the server-inferred staleness state flips.
#[allow(clippy::too_many_lines)] // Scene construction, ring negotiation and one atomic push.
pub(super) fn push_active_scene(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
    reconnect_interval: Duration,
) {
    if ownership_was_refused(state) || state.needs_full_sync || !state.active_scene_dirty {
        return;
    }
    let Some(card_id) = state.active_card.clone() else {
        state.active_scene_dirty = false;
        return;
    };

    let Some(revision) = state.next_scene_revision.checked_add(1) else {
        state.active_scene_dirty = false;
        record_scene_refusal(
            state,
            card_id,
            "this card cannot be rendered because the scene revision counter is exhausted".into(),
        );
        return;
    };
    let fields = state
        .latest_fields
        .get(&card_id)
        .cloned()
        .unwrap_or_default();
    let mut push = match build_card_scene(
        &state.config,
        &card_id,
        &fields,
        state.image_source_host.as_deref_mut(),
        revision,
    ) {
        Ok(candidate) => candidate,
        Err(message) => {
            state.active_scene_dirty = false;
            record_scene_refusal(state, card_id, message);
            return;
        }
    };
    let ring = if state.device.capability_bits() & protocol::CAPABILITY_LOCAL_TAP_VIEWS != 0 {
        picture_source(&state.config, &card_id)
            .map(str::to_owned)
            .and_then(|source| {
                state
                    .image_source_host
                    .as_mut()
                    .and_then(|host| host.local_tap_ring(&source))
            })
    } else {
        None
    };
    let ring = ring.filter(|ring| {
        ring.steps.len() > 1
            && ring.steps.len() <= protocol::MAX_TAP_VIEWS + 1
            && built_picture_face(&state.config, &card_id, &push)
                .1
                .is_some_and(|(digest, _)| ring.steps[0].digest == digest)
    });
    let is_stale = built_picture_face(&state.config, &card_id, &push)
        .1
        .is_some_and(|(_, stale)| stale);
    if let Some(ring) = &ring {
        push.tap_views = ring
            .steps
            .iter()
            .skip(1)
            .map(|step| with_stale_footer(frame_face_scene(revision, step.digest), is_stale))
            .collect();
        push.tap_wrap = ring.wrap;
    }
    let (is_picture_card, picture_face) = built_picture_face(&state.config, &card_id, &push);
    if is_picture_card {
        match picture_face {
            Some(face) => {
                state
                    .last_evaluated_picture_face
                    .insert(card_id.clone(), face);
            }
            None => {
                state.last_evaluated_picture_face.remove(&card_id);
            }
        }
    }
    // Analyze every freshly-built scene against this device. In particular,
    // a picture digest must be confirmed or installable before its scene can
    // name it; otherwise the panel would accept a face it cannot draw.
    let mut requirements = match native_requirements(state, &push.scene) {
        Ok(requirements) => requirements,
        Err(message) => {
            state.active_scene_dirty = false;
            record_scene_refusal(state, card_id, message);
            return;
        }
    };
    for view in &push.tap_views {
        match native_requirements(state, view) {
            Ok(view_requirements) => requirements
                .asset_digests
                .extend(view_requirements.asset_digests),
            Err(message) => {
                record_scene_refusal(state, card_id, message);
                return;
            }
        }
    }
    let ring_card = card_id.clone();
    execute_native_push(
        state,
        device,
        card_id,
        revision,
        push,
        &requirements,
        reconnect_interval,
    );
    if !state.active_scene_dirty && !state.push_rejections.contains_key(&ring_card) {
        state.local_tap_ring = ring.map(|ring| (ring_card, ring, 0));
    }
}

pub(super) fn execute_native_push(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
    card_id: String,
    revision: u32,
    push: PushScene,
    requirements: &render_negotiation::RenderRequirements,
    reconnect_interval: Duration,
) {
    state.next_scene_revision = revision;
    if !requirements.asset_digests.is_empty()
        && let Err(error) =
            ensure_durable_assets_for_scene(state, device, &requirements.asset_digests)
    {
        state.active_scene_dirty = false;
        handle_automatic_asset_error(state, card_id, error, reconnect_interval);
        return;
    }
    state.active_scene_dirty = false;
    let visible_assets = render_negotiation::analyze_scene(&push.scene).asset_digests;
    match device.push_scene(push) {
        Ok(()) => {
            state.local_tap_ring = None;
            state.live_scene_assets = visible_assets;
            clear_scene_refusal(state, &card_id);
        }
        Err(error) => {
            handle_automatic_scene_error(state, card_id, error, reconnect_interval);
        }
    }
}

pub(super) fn built_picture_face(
    config: &AppConfig,
    card_id: &str,
    push: &PushScene,
) -> (bool, Option<([u8; protocol::ASSET_DIGEST_LEN], bool)>) {
    let is_picture = config
        .cards
        .iter()
        .any(|card| matches!(card, CardSettings::Picture { id, .. } if id == card_id));
    if !is_picture {
        return (false, None);
    }
    let face = push.scene.nodes.iter().find_map(|node| match node {
        protocol::SceneNode::Image(image) => Some((image.digest, push.scene.nodes.len() > 1)),
        _ => None,
    });
    (true, face)
}

pub(super) fn clear_scene_refusal(state: &mut WorkerState, card_id: &str) {
    if state
        .push_rejections
        .get(card_id)
        .is_some_and(|error| error.kind == CardErrorKind::SceneRefused)
    {
        state.push_rejections.remove(card_id);
    }
}

pub(super) fn ensure_durable_assets_for_scene(
    state: &mut WorkerState,
    device: &mut dyn RuntimeDevice,
    required: &BTreeSet<[u8; protocol::ASSET_DIGEST_LEN]>,
) -> Result<(), AssetSyncError> {
    if required
        .iter()
        .all(|digest| state.confirmed_resident_assets.contains(digest))
    {
        return Ok(());
    }
    let desired = state
        .image_source_host
        .as_deref_mut()
        .map(ImageSourceHost::desired_assets)
        .unwrap_or_default();
    if let Some(digest) = required.iter().find(|digest| {
        !state.confirmed_resident_assets.contains(*digest)
            && !desired.iter().any(|asset| asset.digest == **digest)
    }) {
        return Err(AssetSyncError::MissingRequiredAsset { digest: *digest });
    }
    let release = claim_asset_release(state, Instant::now());
    let keep_set = reconcile_assets(state, device, &desired, release)?;
    state.confirmed_resident_assets = keep_set.into_iter().collect();
    Ok(())
}

/// The one definition of "a frame as a face": a single full-canvas image node
/// naming a digest the device holds.
pub(super) fn frame_face_scene(
    revision: u32,
    digest: [u8; protocol::ASSET_DIGEST_LEN],
) -> protocol::Scene {
    protocol::Scene {
        revision,
        background: 0,
        nodes: vec![protocol::SceneNode::Image(protocol::SceneImage {
            x: 0,
            y: 0,
            w: protocol::SCENE_CANVAS_WIDTH,
            h: protocol::SCENE_CANVAS_HEIGHT,
            digest,
            recolor: false,
            color: 0,
        })],
    }
}

pub(super) fn handle_automatic_asset_error(
    state: &mut WorkerState,
    card_id: String,
    error: AssetSyncError,
    reconnect_interval: Duration,
) {
    match error {
        AssetSyncError::Begin { source, .. }
        | AssetSyncError::Chunk { source, .. }
        | AssetSyncError::Commit { source, .. }
        | AssetSyncError::Release { source } => {
            handle_automatic_scene_error(state, card_id, source, reconnect_interval);
        }
        // Host-side refusals: the device was never asked, and asking again with
        // the same inputs would fail identically. These are refusals to record,
        // not transport failures to retry.
        AssetSyncError::TooManyDesiredAssets { .. }
        | AssetSyncError::AssetTooLarge { .. }
        | AssetSyncError::MissingRequiredAsset { .. }
        | AssetSyncError::VolatileKind { .. }
        | AssetSyncError::VolatileUnsupported => {
            record_scene_refusal(state, card_id, error.to_string());
        }
    }
}
