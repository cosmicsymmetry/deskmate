//! Dev-only physical framebuffer diff. Pushes every device-representable case
//! in the synthetic
//! `lvgl_sim::cases::scene_cases()`, six-face `lvgl_sim::cases::face_scene_cases()`,
//! and produced-date overflow matrices to a physically connected device running a
//! `DESKMATE_DEV_DIAG=1`
//! build, requests a 0x7E framebuffer capture, and byte-compares the
//! reassembled pixels against `Simulator::render_scene` for the identical
//! case. This only runs against real hardware: 0x7E/0x7F are dev-build-only
//! message ids, absent from the release protocol and from
//! `docs/protocol/v2.md`.
//!
//! Requires a device flashed from `idf.py -C firmware -DDESKMATE_DEV_DIAG=1
//! build`. Against a plain release build every case fails with a timeout
//! (the device silently ignores the unrecognized 0x7E message type, exactly
//! like any other unsupported byte the untrusted-input rules require it to
//! tolerate).
//!
//! ## Real asset provisioning
//!
//! `push_case_assets` provisions every asset a case's scene names, over the
//! real `AssetBegin`/`AssetChunk`/`AssetCommit` wire path -- the first time
//! this harness (or any test in this repo) exercises the device's
//! asset-transfer path rather than excluding any case that needed one.
//!
//! ## Sequencing and settle time
//!
//! Each case applies a single-card config, activates that card, pushes any
//! timer, pushes the scene, and *finally* time-syncs the device
//! to the case's pinned instant. Time-sync updates the clock context read by
//! the 250 ms scene-binding tick; [`SETTLE`] gives that tick time to repaint
//! while staying comfortably under the one full second that would roll the
//! synced clock's displayed second over.

use std::env;
use std::process;
use std::thread;
use std::time::Duration;

use device::framebuffer_capture::capture_framebuffer;
use device::{DeviceClient, Transport, connect};
use lvgl_sim::scene::{SceneRenderRequest, SceneTimer};
use lvgl_sim::{SimOrientation, Simulator, cases};
use protocol::{
    ActivateCard, ApplyConfig, AssetBegin, AssetChunk, AssetCommit, CardConfig,
    MAX_ASSET_CHUNK_BYTES, Message, PushScene, PushTimer, TYPE_ACTIVATE_CARD, TYPE_APPLY_CONFIG,
    TYPE_ASSET_BEGIN, TYPE_ASSET_CHUNK, TYPE_ASSET_COMMIT, TYPE_PUSH_SCENE, TYPE_PUSH_TIMER,
    TapAction, TimeSync,
};
/// Gives the 250 ms scene-binding tick margin to redraw before capture while
/// staying clear of the 1 s mark that rolls a just-synced second over.
const SETTLE: Duration = Duration::from_millis(300);
/// Time given to the device's asynchronous scene teardown after an
/// orientation-changing `ApplyConfig`, before re-rendering an asset-bearing
/// scene. See the call site for why an image/glyph case needs it and a light
/// case does not.
const CONFIG_SETTLE: Duration = Duration::from_millis(700);

fn parse_port() -> Result<Option<String>, String> {
    let mut arguments = env::args().skip(1);
    let mut port = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--port" => {
                port = Some(
                    arguments
                        .next()
                        .ok_or_else(|| "--port requires a value".to_owned())?,
                );
            }
            other => return Err(format!("unknown option: {other}")),
        }
    }
    Ok(port)
}

/// Returns the reason a scene case cannot be pushed by this harness.
///
/// Every case uses the supported binding vocabulary. The only exclusion is
/// the timing race below, which no wire change can fix.
fn exclusion_reason(name: &str, _request: &SceneRenderRequest) -> Option<&'static str> {
    if name.starts_with("progress-ring--running-mid-countdown--") {
        Some(
            "the device keeps ticking a running timer through \
             scene_view_tick_bindings after the scene is pushed, so its frame \
             moves while the simulator's fixed fake tick freezes it -- the \
             MM:SS label flips a second as soon as push-to-capture latency \
             crosses 1000ms, making the comparison a race rather than a check. \
             No running value avoids this. The case is kept for the \
             deterministic simulator goldens, which do cover the running arc \
             hue; paused-mid-countdown covers the same geometry here, and \
             running-at-zero the running palette",
        )
    } else {
        None
    }
}

/// The firmware's rotation field: 90 is the default mount (`Landscape`),
/// 270 is the flipped mount (`board_display_set_rotation_180`).
fn rotation_degrees(orientation: SimOrientation) -> u16 {
    match orientation {
        SimOrientation::Landscape => 90,
        SimOrientation::LandscapeFlipped => 270,
    }
}

/// `lv_snapshot_take` (`firmware/main/link/dev_capture.c`) captures the
/// *pre-flush* logical frame, which is orientation-independent: the 90/270
/// software-rotation transform runs only in the flush path the snapshot
/// bypasses (spec §3.1's claim boundary). `Simulator::render`'s flipped
/// output already includes that transform as a 180-degree pixel reversal
/// (`companion/crates/lvgl-sim/csrc/sim_shim.c`), matching what the panel
/// would physically show, so the raw capture needs the identical reversal
/// before comparison.
fn maybe_flip(pixels: Vec<u16>, orientation: SimOrientation) -> Vec<u16> {
    match orientation {
        SimOrientation::Landscape => pixels,
        SimOrientation::LandscapeFlipped => pixels.into_iter().rev().collect(),
    }
}

fn bytes_to_pixels(bytes: &[u8]) -> Vec<u16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect()
}

/// Differing pixel count and the largest single-channel delta among them,
/// each channel widened to its 8-bit-equivalent scale (matching
/// `Simulator::render_png`'s R5/G6/B5 -> 8 expansion) so the number means
/// the same thing a human comparing two PNGs would see.
fn diff_pixels(expected: &[u16], actual: &[u16]) -> (usize, u32) {
    let mut differing = 0usize;
    let mut max_delta = 0u32;
    for (&expected, &actual) in expected.iter().zip(actual.iter()) {
        if expected == actual {
            continue;
        }
        differing += 1;
        let channels = |pixel: u16| -> (u32, u32, u32) {
            (
                (u32::from(pixel >> 11) & 0x1f) << 3,
                (u32::from(pixel >> 5) & 0x3f) << 2,
                (u32::from(pixel) & 0x1f) << 3,
            )
        };
        let (er, eg, eb) = channels(expected);
        let (ar, ag, ab) = channels(actual);
        max_delta = max_delta
            .max(er.abs_diff(ar))
            .max(eg.abs_diff(ag))
            .max(eb.abs_diff(ab));
    }
    (differing, max_delta)
}

fn apply_case_config(
    client: &mut DeviceClient<impl Transport>,
    revision: u32,
    request: &SceneRenderRequest,
) -> Result<(), String> {
    let config = ApplyConfig {
        revision,
        rotation: rotation_degrees(request.orientation),
        cards: vec![CardConfig {
            card_id: "diff".into(),
            tap_action: TapAction::None,
        }],
    };
    match client
        .request(&Message::ApplyConfig(config))
        .map_err(|error| format!("apply config: {error}"))?
    {
        Message::Ack(ack)
            if ack.acknowledged_type == TYPE_APPLY_CONFIG && ack.revision == Some(revision) =>
        {
            Ok(())
        }
        message => Err(format!("unexpected apply-config response: {message:?}")),
    }
}

fn activate_case_card(client: &mut DeviceClient<impl Transport>) -> Result<(), String> {
    match client
        .request(&Message::ActivateCard(ActivateCard {
            card_id: "diff".into(),
        }))
        .map_err(|error| format!("activate card: {error}"))?
    {
        Message::Ack(ack) if ack.acknowledged_type == TYPE_ACTIVATE_CARD => Ok(()),
        message => Err(format!("unexpected activate-card response: {message:?}")),
    }
}

/// Pushes the timer a case's `timer.*` bindings resolve against, if it has
/// one. A case with no timer pushes nothing: protocol v2's device models
/// nothing else per card.
fn push_case_timer(
    client: &mut DeviceClient<impl Transport>,
    revision: u32,
    request: &SceneRenderRequest,
) -> Result<(), String> {
    let Some(SceneTimer {
        total_ms,
        remaining_ms,
        running,
    }) = request.timer
    else {
        return Ok(());
    };
    let ack = client
        .push_timer(PushTimer {
            card_id: "diff".into(),
            revision,
            total_ms,
            remaining_ms,
            running,
        })
        .map_err(|error| format!("push timer: {error}"))?;
    if ack.acknowledged_type != TYPE_PUSH_TIMER || ack.revision != Some(revision) {
        return Err(format!(
            "push acknowledgement revision mismatch: expected {revision}, received {:?}",
            ack.revision
        ));
    }
    Ok(())
}

/// Provisions every asset `request.scene` names, over the real
/// `AssetBegin`/`AssetChunk`/`AssetCommit` wire path. `AssetBegin`'s
/// `already_present` lets a case skip
/// chunking an asset a previous case already committed under the same
/// digest, which is common across asset-bearing case variants.
fn push_case_assets(
    client: &mut DeviceClient<impl Transport>,
    request: &SceneRenderRequest,
) -> Result<(), String> {
    for asset in &request.assets {
        let digest = asset.digest;
        let total_length = u32::try_from(asset.bytes.len()).map_err(|_| {
            format!(
                "asset {digest:02x?} is {} bytes, over the wire's u32 length limit",
                asset.bytes.len()
            )
        })?;

        let ack = match client
            .request(&Message::AssetBegin(AssetBegin {
                digest,
                kind: asset.kind,
                total_length,
                // The comparison harness uses durable raw assets so the same
                // path works across every asset-transfer-capable device and
                // the bytes remain available throughout capture.
                volatile: false,
                encoding: protocol::ASSET_ENCODING_RAW,
                decoded_length: None,
            }))
            .map_err(|error| format!("asset begin {digest:02x?}: {error}"))?
        {
            Message::Ack(ack) if ack.acknowledged_type == TYPE_ASSET_BEGIN => ack,
            message => return Err(format!("unexpected asset-begin response: {message:?}")),
        };
        if ack.already_present == Some(true) {
            continue;
        }

        let mut offset: u32 = 0;
        for chunk in asset.bytes.chunks(MAX_ASSET_CHUNK_BYTES) {
            match client
                .request(&Message::AssetChunk(AssetChunk {
                    digest,
                    offset,
                    data: chunk.to_vec(),
                }))
                .map_err(|error| format!("asset chunk {digest:02x?} at {offset}: {error}"))?
            {
                Message::Ack(ack) if ack.acknowledged_type == TYPE_ASSET_CHUNK => {}
                message => return Err(format!("unexpected asset-chunk response: {message:?}")),
            }
            offset += u32::try_from(chunk.len())
                .expect("chunk length is bounded by MAX_ASSET_CHUNK_BYTES");
        }

        match client
            .request(&Message::AssetCommit(AssetCommit { digest }))
            .map_err(|error| format!("asset commit {digest:02x?}: {error}"))?
        {
            Message::Ack(ack) if ack.acknowledged_type == TYPE_ASSET_COMMIT => {}
            message => return Err(format!("unexpected asset-commit response: {message:?}")),
        }
    }
    Ok(())
}

fn sync_case_time(
    client: &mut DeviceClient<impl Transport>,
    request: &SceneRenderRequest,
) -> Result<(), String> {
    client
        .time_sync(TimeSync {
            unix_seconds: request.now_unix_seconds,
            utc_offset_minutes: request.utc_offset_minutes,
        })
        .map_err(|error| format!("time sync: {error}"))?;
    Ok(())
}

fn push_case_scene(
    client: &mut DeviceClient<impl Transport>,
    revision: u32,
    request: &SceneRenderRequest,
) -> Result<(), String> {
    match client
        .request(&Message::PushScene(PushScene {
            card_id: "diff".into(),
            revision,
            scene: request.scene.clone(),
        }))
        .map_err(|error| format!("push scene: {error}"))?
    {
        Message::Ack(ack)
            if ack.acknowledged_type == TYPE_PUSH_SCENE && ack.revision == Some(revision) =>
        {
            Ok(())
        }
        message => Err(format!("unexpected push-scene response: {message:?}")),
    }
}

enum CaseOutcome {
    Identical,
    Differ { count: usize, max_delta: u32 },
}

#[allow(
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::similar_names
)]
fn run_case<T: Transport>(
    mut client: DeviceClient<T>,
    sim: &mut Simulator,
    config_revision: u32,
    data_revision: u32,
    scene_revision: u32,
    request_id: u32,
    request: &SceneRenderRequest,
    name: &str,
) -> (DeviceClient<T>, Result<CaseOutcome, String>) {
    let setup: Result<(), String> = (|| {
        apply_case_config(&mut client, config_revision, request)?;
        // An asset-bearing scene (image/glyph nodes) allocates image buffers
        // the device frees only when the *previous* scene is torn down, which
        // happens asynchronously on the UI tick. The matrix flips orientation
        // (a fresh ApplyConfig + full re-render) between a case and its
        // counterpart, so back-to-back heavy-image renders can outpace that
        // teardown: the new render fails to allocate and the device refuses
        // the push (InvalidPayload "scene could not be rendered"). This is a
        // harness-pacing artifact, not a renderer defect -- the identical
        // scene renders byte-exact at either orientation run alone. Giving the
        // teardown a moment removes the race; 500 ms sufficed in hardware
        // testing, and this leaves margin. Light, asset-free cases never hit
        // it, so they never pay it.
        if !request.assets.is_empty() {
            thread::sleep(CONFIG_SETTLE);
        }
        activate_case_card(&mut client)?;
        push_case_timer(&mut client, data_revision, request)?;
        // Before the scene that names these assets by digest, so the device
        // never has to resolve a digest it has not been given bytes for yet.
        push_case_assets(&mut client, request)?;
        push_case_scene(&mut client, scene_revision, request)?;
        // Last: see the module doc for why time-sync is sequenced after the
        // config/push commands rather than before them.
        sync_case_time(&mut client, request)?;
        Ok(())
    })();
    if let Err(error) = setup {
        return (client, Err(error));
    }

    thread::sleep(SETTLE);

    let (client, capture) = capture_framebuffer(client, request_id);
    let raw = match capture {
        Ok(raw) => raw,
        Err(error) => return (client, Err(error.to_string())),
    };

    let actual = maybe_flip(bytes_to_pixels(&raw), request.orientation);
    let expected = match sim.render_scene(request) {
        Ok(expected) => expected,
        Err(error) => return (client, Err(format!("simulator render failed: {error}"))),
    };
    if actual.len() != expected.len() {
        return (
            client,
            Err(format!(
                "pixel count mismatch: device produced {}, simulator produced {}",
                actual.len(),
                expected.len()
            )),
        );
    }

    let (differing, max_delta) = diff_pixels(&expected, &actual);
    if differing > 0 {
        const W: usize = 448;
        let (mut minx, mut miny, mut maxx, mut maxy) = (W, 368usize, 0usize, 0usize);
        for (i, (&e, &a)) in expected.iter().zip(actual.iter()).enumerate() {
            if e != a {
                let (x, y) = (i % W, i / W);
                minx = minx.min(x);
                miny = miny.min(y);
                maxx = maxx.max(x);
                maxy = maxy.max(y);
            }
        }
        eprintln!("  [{name}] diff bbox x={minx}..={maxx} y={miny}..={maxy} ({differing} px)");
    }
    let outcome = if differing == 0 {
        CaseOutcome::Identical
    } else {
        CaseOutcome::Differ {
            count: differing,
            max_delta,
        }
    };
    (client, Ok(outcome))
}

fn run() -> Result<(), String> {
    let port = parse_port()?;
    let connected = connect(port.as_deref()).map_err(|error| error.to_string())?;
    println!("connected port={}", connected.port_name);

    let mut client = connected.client;
    let mut config_revision = connected.initial_status.config_revision;
    let mut data_revision = connected.initial_status.latest_revision;
    let mut scene_revision = connected.initial_status.latest_revision;
    // The 0x7E request ID only needs to be nonzero and distinct from
    // whatever the structured DeviceClient used most recently; the device
    // does not track a cross-request sequence, only per-request
    // correlation, so counting up here (independent of DeviceClient's own
    // internal counter, which resets every time it is rebuilt around the
    // transport in `capture_framebuffer`) is enough.
    let mut capture_request_id: u32 = 1;

    let mut sim = Simulator::new().map_err(|error| format!("simulator init failed: {error}"))?;

    let mut identical = 0usize;
    let mut differing = 0usize;
    let mut errored = 0usize;
    let mut excluded = 0usize;

    for (name, request) in cases::scene_cases()
        .into_iter()
        .chain(cases::face_scene_cases())
        .chain(cases::date_truncation_scene_cases())
    {
        if let Some(reason) = exclusion_reason(&name, &request) {
            println!("{name}: excluded ({reason})");
            excluded += 1;
            continue;
        }

        config_revision = match config_revision.checked_add(1) {
            Some(revision) => revision,
            None => return Err(format!("{name}: config revision exhausted")),
        };
        data_revision = match data_revision.checked_add(1) {
            Some(revision) => revision,
            None => return Err(format!("{name}: data revision exhausted")),
        };
        scene_revision = match scene_revision.checked_add(1) {
            Some(revision) => revision,
            None => return Err(format!("{name}: scene revision exhausted")),
        };
        capture_request_id = capture_request_id.wrapping_add(1).max(1);

        let (returned_client, result) = run_case(
            client,
            &mut sim,
            config_revision,
            data_revision,
            scene_revision,
            capture_request_id,
            &request,
            &name,
        );
        client = returned_client;

        match result {
            Ok(CaseOutcome::Identical) => {
                println!("{name}: identical");
                identical += 1;
            }
            Ok(CaseOutcome::Differ { count, max_delta }) => {
                println!("{name}: {count} pixels differ (max channel delta {max_delta})");
                differing += 1;
            }
            Err(error) => {
                println!("{name}: ERROR {error}");
                errored += 1;
            }
        }
    }

    let total = identical + differing + errored + excluded;
    println!(
        "SUMMARY total={total} identical={identical} differing={differing} errored={errored} excluded={excluded}"
    );

    if differing > 0 || errored > 0 {
        return Err(format!(
            "{differing} case(s) differed and {errored} case(s) errored -- see output above"
        ));
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("framebuffer diff failed: {error}");
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the software inventory separately from the last 96/10/86 hardware
    /// observation; the next board session must measure it afresh.
    ///
    /// Protocol v2 closed the four `field.*` exclusions: there is no field
    /// namespace, so `scene-text` and `scene-label` bind `date` and
    /// `timer.status` instead and run on hardware like every other row. One
    /// exclusion is left, and it is a timing race no wire change can fix.
    #[test]
    fn gate_b_inventory_is_the_real_counted_split_not_an_assumed_one() {
        let requests: Vec<_> = cases::scene_cases()
            .into_iter()
            .chain(cases::face_scene_cases())
            .chain(cases::date_truncation_scene_cases())
            .collect();

        let running_mid_countdown = requests
            .iter()
            .filter(|(name, _)| name.starts_with("progress-ring--running-mid-countdown--"))
            .count();
        let other_exclusions = requests
            .iter()
            .filter(|(name, request)| {
                !name.starts_with("progress-ring--running-mid-countdown--")
                    && exclusion_reason(name, request).is_some()
            })
            .count();
        let excluded = requests
            .iter()
            .filter(|(name, request)| exclusion_reason(name, request).is_some())
            .count();

        // Keep these explicit so a changed case inventory cannot silently alter
        // the hardware-coverage split.
        assert_eq!(
            requests.len(),
            44,
            "42 scene + face rows + 2 date-overflow rows"
        );
        assert_eq!(running_mid_countdown, 2);
        assert_eq!(
            other_exclusions, 0,
            "the running-timer race is the only reason a row cannot be pushed to \
             hardware"
        );
        assert_eq!(excluded, 2);
        assert_eq!(requests.len() - excluded, 42);

        // The RGB565-image and runtime-font-asset rows (scene-image,
        // scene-glyph, both orientations = 4 rows) are no longer excluded.
        for name in [
            "scene-image--landscape",
            "scene-image--flipped",
            "scene-glyph--landscape",
            "scene-glyph--flipped",
        ] {
            let (_, request) = requests
                .iter()
                .find(|(request_name, _)| request_name == name)
                .unwrap_or_else(|| panic!("missing case {name}"));
            assert_eq!(
                exclusion_reason(name, request),
                None,
                "{name} must be included now that assets are provisioned"
            );
        }

        // Every host-rendered face in the current case inventory.
        for prefix in ["digital-clock--", "analog-clock--", "progress-ring--"] {
            assert!(
                requests.iter().any(|(name, _)| name.starts_with(prefix)),
                "missing real-face coverage for {prefix}"
            );
        }
        // The produced date that overflows its 176 px box, at both mountings.
        // It binds `date` and carries no timer, so nothing excludes it.
        let overflow = requests
            .iter()
            .filter(|(name, _)| name.starts_with("digital-clock--date-overflow--"))
            .collect::<Vec<_>>();
        assert_eq!(
            overflow.len(),
            2,
            "date overflow must cover both orientations"
        );
        assert!(overflow.iter().all(|(name, request)| {
            exclusion_reason(name, request).is_none() && request.timer.is_none()
        }));
    }
}
