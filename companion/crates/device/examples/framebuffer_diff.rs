//! Dev-only physical framebuffer diff (V1 reset design spec §3.2.2/§3.2.3,
//! Task 10). Pushes every device-representable case in the synthetic
//! `lvgl_sim::cases::scene_cases()`, six-face `lvgl_sim::cases::face_scene_cases()`,
//! and (Task 8, plugin-manifest stage) `lvgl_sim::cases::plugin_scene_cases()`
//! matrices to a physically connected device running a `DESKMATE_DEV_DIAG=1`
//! build, requests a 0x7E framebuffer capture, and byte-compares the
//! reassembled pixels against `Simulator::render_scene` for the identical
//! case. This only runs against real hardware: 0x7E/0x7F are dev-build-only
//! message ids, absent from the release protocol and from
//! `docs/protocol/v1.md`.
//!
//! Requires a device flashed from `idf.py -C firmware -DDESKMATE_DEV_DIAG=1
//! build`. Against a plain release build every case fails with a timeout
//! (the device silently ignores the unrecognized 0x7E message type, exactly
//! like any other unsupported byte the untrusted-input rules require it to
//! tolerate).
//!
//! ## Real asset provisioning (Task 8, plugin-manifest stage)
//!
//! `push_case_assets` provisions every asset a case's scene names, over the
//! real `AssetBegin`/`AssetChunk`/`AssetCommit` wire path -- the first time
//! this harness (or any test in this repo) exercises the device's
//! asset-transfer path rather than excluding any case that needed one. See
//! `exclusion_reason`'s doc for exactly which of the three historical
//! exclusion reasons this closes, and which one it does not.
//!
//! ## The row-list truncation-boundary exclusion
//!
//! `row-list--truncation-boundary--*` pins `row0_title` at the wire's
//! generic per-field ceiling (`PROTOCOL_MAX_FIELD_TEXT_LENGTH` = 128,
//! `firmware/main/core/protocol_message.h`), which exceeds that specific
//! field's own registry maximum of 96
//! (`s_row_list_fields` in `firmware/main/core/template_fields.c`). A push
//! that violates a field's registry maximum is rejected by the device, so
//! this case cannot be exercised against real hardware as written — see the
//! Task 5 ledger note carried into the Task 10 brief. It is excluded here
//! (not clamped): clamping would silently change what the case name means,
//! and the golden PNG suite already pins the 128-char behavior against the
//! host renderer, which has no such wire ceiling to violate.
//!
//! ## Sequencing and settle time
//!
//! Each case applies a single-card config, activates its one screen, pushes
//! any timer snapshot, pushes the scene, and *finally* time-syncs the device
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
use lvgl_sim::scene::{SceneAsset, SceneRenderRequest, SceneTimer};
use lvgl_sim::{SimOrientation, Simulator, cases};
use protocol::{
    ActivateScreen, ApplyConfig, AssetBegin, AssetChunk, AssetCommit, AssetKind, Field, FieldValue,
    InterruptPolicy, MAX_ASSET_CHUNK_BYTES, Message, PushData, PushScene, ScreenConfig, SizeClass,
    TYPE_ACTIVATE_SCREEN, TYPE_APPLY_CONFIG, TYPE_ASSET_BEGIN, TYPE_ASSET_CHUNK, TYPE_ASSET_COMMIT,
    TYPE_PUSH_SCENE, TapAction, TemplateKind, TimeSync, WidgetConfig,
};
/// Gives the 250 ms scene-binding tick margin to redraw before capture while
/// staying clear of the 1 s mark that rolls a just-synced second over.
const SETTLE: Duration = Duration::from_millis(300);

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

/// Returns the reason a scene case cannot be pushed by this harness. The two
/// historical face-row exclusions are reachable through `face_scene_cases()`;
/// keep them explicit so their known flakes cannot silently return.
fn exclusion_reason(name: &str, request: &SceneRenderRequest) -> Option<&'static str> {
    if name.starts_with("row-list--truncation-boundary--") {
        Some(
            "row0_title is 128 chars (the wire's generic field ceiling), \
             exceeding row-list's own registry maximum of 96 for that \
             field -- the device rejects this push",
        )
    } else if name.starts_with("progress-ring--running-mid-countdown--") {
        Some(
            "a running ring keeps counting down from lv_tick_get() after its \
             fields are pushed, so the device frame moves while the simulator's \
             fixed fake tick freezes it -- the MM:SS label flips a second as \
             soon as push-to-capture latency crosses 1000ms, making the \
             comparison a race rather than a check. No running value avoids \
             this. The case is kept for the deterministic simulator goldens, \
             which do cover the running arc hue; paused-mid-countdown covers \
             the same geometry here, and running-at-zero the running palette",
        )
    } else if request.fields.iter().any(|(field_name, _)| {
        !matches!(
            field_name.as_str(),
            "title" | "show_seconds" | "stale" | "error"
        )
    }) {
        // Task 8 (plugin-manifest stage): `push_case_assets` below now
        // provisions real RGB565 image and runtime font assets, and
        // `push_case_fields` now pushes `request.fields` as PushData too --
        // so the two asset-shaped exclusions this function used to name are
        // gone. What is left is genuinely un-closable without a firmware
        // change: `apply_case_config` always selects `TemplateKind::DigitalClock`
        // for a no-timer case (this crate's own choice, unrelated to Task 8),
        // whose PushData registry (`s_digital_clock_fields` in
        // `firmware/main/core/template_fields.c`) is exactly
        // `title`/`show_seconds`/`stale`/`error`. A schema-v6 plugin card's
        // `WidgetConfig.template` is *also* always `DigitalClock` on the wire
        // (`app-core/src/config.rs`'s `wire_config`, an established, deliberate
        // decision this task did not make and does not reopen) -- so a plugin
        // does not get a registry of its own to "bring" either. The two
        // scene-node synthetic cases below (`scene-text`/`scene-label`) bind
        // `field.status`, a name no registry accepts; the curated `aqi`
        // plugin instead binds `field.title`, which this registry does
        // accept, and that is what gives `field.*` its hardware coverage.
        Some(
            "the scene binds a field.* name outside DigitalClock's PushData registry \
             (title/show_seconds/stale/error) -- apply_case_config always selects \
             DigitalClock's template for a no-timer case, so any other field name \
             is rejected by the device",
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
        widgets: vec![WidgetConfig {
            widget_id: "diff".into(),
            /* ApplyConfig still registers the bounded PushData field schema;
             * it no longer selects a C renderer. Timer scene cases need the
             * ProgressRing registry, while asset-free geometry needs no data. */
            template: if request.timer.is_some() {
                TemplateKind::ProgressRing
            } else {
                TemplateKind::DigitalClock
            },
            size_class: SizeClass::Full,
            tap_action: TapAction::None,
            interrupt_policy: InterruptPolicy::Disabled,
        }],
        screens: vec![ScreenConfig {
            screen_id: "diff-screen".into(),
            widget_id: "diff".into(),
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

fn activate_case_screen(client: &mut DeviceClient<impl Transport>) -> Result<(), String> {
    match client
        .request(&Message::ActivateScreen(ActivateScreen {
            screen_id: "diff-screen".into(),
        }))
        .map_err(|error| format!("activate screen: {error}"))?
    {
        Message::Ack(ack) if ack.acknowledged_type == TYPE_ACTIVATE_SCREEN => Ok(()),
        message => Err(format!("unexpected activate-screen response: {message:?}")),
    }
}

/// Pushes both of the two things a case's `PushData` can carry: a
/// `ProgressRing` timer snapshot (`request.timer`), and (Task 8,
/// plugin-manifest stage) `request.fields` -- the `field.*` binding values a
/// plugin's scene resolves against, e.g. `aqi`'s `field.title`. A case with
/// neither pushes nothing at all, exactly as before Task 8.
fn push_case_fields(
    client: &mut DeviceClient<impl Transport>,
    revision: u32,
    request: &SceneRenderRequest,
) -> Result<(), String> {
    let mut fields = Vec::new();
    if let Some(SceneTimer {
        total_ms,
        remaining_ms,
        running,
    }) = request.timer
    {
        fields.push(Field {
            key: "duration_seconds".into(),
            value: FieldValue::Integer(i64::from(total_ms / 1_000)),
        });
        fields.push(Field {
            key: "remaining_seconds".into(),
            value: FieldValue::Integer(i64::from(remaining_ms / 1_000)),
        });
        fields.push(Field {
            key: "running".into(),
            value: FieldValue::Boolean(running),
        });
    }
    // `exclusion_reason` already refused any case whose field name is not
    // one of DigitalClock's registered fields, so every name reaching here
    // is one `apply_case_config`'s always-DigitalClock template accepts.
    for (name, value) in &request.fields {
        fields.push(Field {
            key: name.clone(),
            value: FieldValue::Text(value.clone()),
        });
    }
    if fields.is_empty() {
        return Ok(());
    }
    let ack = client
        .push_data(PushData {
            widget_id: "diff".into(),
            revision,
            fields,
        })
        .map_err(|error| format!("push data: {error}"))?;
    if ack.revision != Some(revision) {
        return Err(format!(
            "push acknowledgement revision mismatch: expected {revision}, received {:?}",
            ack.revision
        ));
    }
    Ok(())
}

/// Rebuilds the raw wire bytes one [`SceneAsset`] resolves to, and the
/// [`AssetKind`] to declare it under.
///
/// `SceneAsset::Font`'s `bytes` are already the exact TTF file content --
/// see `lvgl_sim::cases::compile_plugin_scene`'s doc for why this harness
/// always uses `AssetKind::Font` here even for a manifest asset declared
/// `kind = "icon-font"`: `firmware/main/core/asset_store.c`'s
/// `asset_kind_is_valid` accepts either for a font-shaped blob, and the
/// simulator's own `SceneAsset` enum has no separate icon-font variant to
/// preserve the distinction through in the first place.
///
/// `SceneAsset::Image`'s `width`/`height`/`pixels` are the *decoded* form
/// `lvgl_sim::cases::decode_rgb565_asset` produced from the plugin's
/// committed `.rgb565` file; this is that decode's exact inverse, so the
/// re-encoded bytes are byte-identical to the committed file
/// `plugin::resolve_assets` hashed to get `digest` in the first place.
fn asset_wire_bytes(asset: &SceneAsset) -> (Vec<u8>, AssetKind) {
    match asset {
        SceneAsset::Font { bytes, .. } => ((*bytes).to_vec(), AssetKind::Font),
        SceneAsset::Image {
            width,
            height,
            pixels,
            ..
        } => {
            // `lv_image_header_t`: magic(8) | cf(8) | flags(16), then
            // w(16) | h(16), then stride(16) | reserved_2(16), all
            // little-endian -- see `companion/plugins/agenda/manifest.toml`'s
            // doc comment and `sim_build_rgb565_image`
            // (`crates/lvgl-sim/csrc/sim_shim.c`) for the same layout.
            const MAGIC: u32 = 0x19;
            const COLOR_FORMAT_RGB565: u32 = 0x12;
            let stride = width * 2;
            let word0 = MAGIC | (COLOR_FORMAT_RGB565 << 8);
            let word1 = (width & 0xFFFF) | ((height & 0xFFFF) << 16);
            let word2 = stride & 0xFFFF;
            let mut bytes = Vec::with_capacity(12 + pixels.len() * 2);
            bytes.extend_from_slice(&word0.to_le_bytes());
            bytes.extend_from_slice(&word1.to_le_bytes());
            bytes.extend_from_slice(&word2.to_le_bytes());
            for pixel in pixels {
                bytes.extend_from_slice(&pixel.to_le_bytes());
            }
            (bytes, AssetKind::Image)
        }
    }
}

/// Provisions every asset `request.scene` names, over the real
/// `AssetBegin`/`AssetChunk`/`AssetCommit` wire path -- Task 8
/// (plugin-manifest stage)'s first exercise of the device's asset-transfer
/// path in this harness. `AssetBegin`'s `already_present` lets a case skip
/// chunking an asset a previous case already committed under the same
/// digest, which is common here: the two glyph-bearing `aqi` states below
/// each name the same `icons.ttf` digest.
fn push_case_assets(
    client: &mut DeviceClient<impl Transport>,
    request: &SceneRenderRequest,
) -> Result<(), String> {
    for asset in &request.assets {
        let digest = match asset {
            SceneAsset::Font { digest, .. } | SceneAsset::Image { digest, .. } => *digest,
        };
        let (bytes, kind) = asset_wire_bytes(asset);
        let total_length = u32::try_from(bytes.len()).map_err(|_| {
            format!(
                "asset {digest:02x?} is {} bytes, over the wire's u32 length limit",
                bytes.len()
            )
        })?;

        let ack = match client
            .request(&Message::AssetBegin(AssetBegin {
                digest,
                kind,
                total_length,
                // Stage 1 has no volatile (PSRAM) tier; the device refuses
                // `volatile: true` outright (see `server::asset_sync`'s
                // own comment on this).
                volatile: false,
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
        for chunk in bytes.chunks(MAX_ASSET_CHUNK_BYTES) {
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

#[allow(clippy::too_many_lines)]
fn run_case<T: Transport>(
    mut client: DeviceClient<T>,
    sim: &mut Simulator,
    config_revision: u32,
    data_revision: u32,
    scene_revision: u32,
    request_id: u32,
    request: &SceneRenderRequest,
) -> (DeviceClient<T>, Result<CaseOutcome, String>) {
    let setup: Result<(), String> = (|| {
        apply_case_config(&mut client, config_revision, request)?;
        activate_case_screen(&mut client)?;
        push_case_fields(&mut client, data_revision, request)?;
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
        .chain(cases::plugin_scene_cases())
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

    /// Task 8 (plugin-manifest stage) added `plugin_scene_cases()` (16 rows:
    /// two curated plugins x four data states x two orientations) and, in
    /// the same change, `push_case_assets`/`push_case_fields` closed two of
    /// `exclusion_reason`'s three pre-Task-8 reasons -- the RGB565-image and
    /// runtime-font-asset ones -- by actually provisioning them, rather than
    /// refusing every case that needed one. The third reason (a `field.*`
    /// name no `TemplateKind::DigitalClock` PushData push can carry) is
    /// NOT closed: see `exclusion_reason`'s own doc for why a plugin does
    /// not get a registry of its own to "bring" on the wire. This test
    /// pins the real, counted-not-assumed per-reason split rather than a
    /// single total, the way the stage's own 58/54/4 invariant should have
    /// been pinned before it went stale and misled a whole stage.
    #[test]
    fn gate_b_inventory_is_the_real_counted_split_not_an_assumed_one() {
        let requests: Vec<_> = cases::scene_cases()
            .into_iter()
            .chain(cases::face_scene_cases())
            .chain(cases::plugin_scene_cases())
            .collect();

        let truncation_boundary = requests
            .iter()
            .filter(|(name, _)| name.starts_with("row-list--truncation-boundary--"))
            .count();
        let running_mid_countdown = requests
            .iter()
            .filter(|(name, _)| name.starts_with("progress-ring--running-mid-countdown--"))
            .count();
        let field_registry_mismatch = requests
            .iter()
            .filter(|(name, request)| {
                !name.starts_with("row-list--truncation-boundary--")
                    && !name.starts_with("progress-ring--running-mid-countdown--")
                    && exclusion_reason(name, request).is_some()
            })
            .count();
        let excluded = requests
            .iter()
            .filter(|(name, request)| exclusion_reason(name, request).is_some())
            .count();

        // The counted-not-assumed numbers this task's report must state.
        assert_eq!(requests.len(), 92, "76 pre-Task-8 rows + 16 plugin rows");
        assert_eq!(truncation_boundary, 2);
        assert_eq!(running_mid_countdown, 2);
        assert_eq!(
            field_registry_mismatch, 4,
            "scene-text and scene-label, both orientations -- field.status, which no \
             registry (built-in or plugin-forced-to-DigitalClock) accepts"
        );
        assert_eq!(
            excluded,
            truncation_boundary + running_mid_countdown + field_registry_mismatch
        );
        assert_eq!(excluded, 8);
        assert_eq!(requests.len() - excluded, 84);

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

        for prefix in [
            "digital-clock--",
            "analog-clock--",
            "progress-ring--",
            "row-list--",
            "big-number-label--",
            "icon-badge-text--",
        ] {
            assert!(
                requests.iter().any(|(name, _)| name.starts_with(prefix)),
                "missing real-face coverage for {prefix}"
            );
        }
        for prefix in ["plugin-aqi--", "plugin-agenda--"] {
            assert_eq!(
                requests
                    .iter()
                    .filter(|(name, _)| name.starts_with(prefix))
                    .count(),
                8,
                "missing curated-plugin coverage for {prefix}"
            );
            assert!(
                requests
                    .iter()
                    .filter(|(name, _)| name.starts_with(prefix))
                    .all(|(name, request)| exclusion_reason(name, request).is_none()),
                "every {prefix} row must be includable, not excluded"
            );
        }
    }
}
