//! Dev-only physical framebuffer diff (V1 reset design spec §3.2.2/§3.2.3,
//! Task 10). Pushes every case in `lvgl_sim::cases::golden_cases()` — the
//! same table the golden-frame PNG suite pins — to a physically connected
//! device running a `DESKMATE_DEV_DIAG=1` build, requests a 0x7E
//! framebuffer capture, and byte-compares the reassembled pixels against
//! `Simulator::render` for the identical case. This only runs against real
//! hardware: 0x7E/0x7F are dev-build-only message ids, absent from the
//! release protocol and from `docs/protocol/v1.md`.
//!
//! Requires a device flashed from `idf.py -C firmware -DDESKMATE_DEV_DIAG=1
//! build`. Against a plain release build every case fails with a timeout
//! (the device silently ignores the unrecognized 0x7E message type, exactly
//! like any other unsupported byte the untrusted-input rules require it to
//! tolerate).
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
//! Each case applies a single-widget config, activates its one screen,
//! pushes the case's fields, and *finally* time-syncs the device to the
//! case's pinned instant — time-sync last because `dispatch_time_sync`
//! (`firmware/main/link/protocol_task.c`) enqueues the redraw the clock
//! templates need, and every enqueued UI command drains in FIFO order
//! (`firmware/main/ui/ui_runtime.c`), so time-sync's command is guaranteed
//! to land after the config/push commands ahead of it. The UI command timer
//! polls every 20 ms; this waits [`SETTLE`] (well under the 20 ms bound's
//! nearest order of magnitude, and comfortably under the one full second
//! that would roll the synced clock's displayed second over) before
//! requesting the capture.

use std::env;
use std::process;
use std::thread;
use std::time::Duration;

use device::framebuffer_capture::capture_framebuffer;
use device::{DeviceClient, Transport, connect};
use lvgl_sim::{RenderRequest, SimFieldValue, SimOrientation, SimTemplate, Simulator, cases};
use protocol::{
    ActivateScreen, ApplyConfig, Field, FieldValue, InterruptPolicy, Message, PushData,
    ScreenConfig, SizeClass, TYPE_ACTIVATE_SCREEN, TYPE_APPLY_CONFIG, TapAction, TemplateKind,
    TimeSync, WidgetConfig,
};
/// Gives the UI command queue (20 ms poll) generous margin to drain the
/// config/push/time-sync commands and redraw before the capture request,
/// while staying well clear of the 1 s mark that would roll the just-synced
/// clock's displayed second over.
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

/// The row-list truncation-boundary case cannot be pushed to real hardware
/// as written -- see this file's module doc. Returns the reason to log when
/// a case name matches it.
fn exclusion_reason(name: &str) -> Option<&'static str> {
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
    } else {
        None
    }
}

fn template_kind(template: SimTemplate) -> TemplateKind {
    match template {
        SimTemplate::DigitalClock => TemplateKind::DigitalClock,
        SimTemplate::ProgressRing => TemplateKind::ProgressRing,
        SimTemplate::RowList => TemplateKind::RowList,
        SimTemplate::AnalogClock => TemplateKind::AnalogClock,
        SimTemplate::BigNumberLabel => TemplateKind::BigNumberLabel,
        SimTemplate::IconBadgeText => TemplateKind::IconBadgeText,
    }
}

fn field_value(value: &SimFieldValue) -> FieldValue {
    match value {
        SimFieldValue::Text(text) => FieldValue::Text(text.clone()),
        SimFieldValue::Integer(value) => FieldValue::Integer(*value),
        SimFieldValue::Boolean(value) => FieldValue::Boolean(*value),
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
    request: &RenderRequest,
) -> Result<(), String> {
    let config = ApplyConfig {
        revision,
        rotation: rotation_degrees(request.orientation),
        widgets: vec![WidgetConfig {
            widget_id: "diff".into(),
            template: template_kind(request.template),
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

fn push_case_fields(
    client: &mut DeviceClient<impl Transport>,
    revision: u32,
    request: &RenderRequest,
) -> Result<(), String> {
    let fields = request
        .fields
        .iter()
        .map(|field| Field {
            key: field.name.clone(),
            value: field_value(&field.value),
        })
        .collect();
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

fn sync_case_time(
    client: &mut DeviceClient<impl Transport>,
    request: &RenderRequest,
) -> Result<(), String> {
    client
        .time_sync(TimeSync {
            unix_seconds: request.now_unix_seconds,
            utc_offset_minutes: request.utc_offset_minutes,
        })
        .map_err(|error| format!("time sync: {error}"))?;
    Ok(())
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
    request_id: u32,
    request: &RenderRequest,
) -> (DeviceClient<T>, Result<CaseOutcome, String>) {
    let setup: Result<(), String> = (|| {
        apply_case_config(&mut client, config_revision, request)?;
        activate_case_screen(&mut client)?;
        push_case_fields(&mut client, data_revision, request)?;
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
    let expected = match sim.render(request) {
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

    for (name, request) in cases::golden_cases() {
        if let Some(reason) = exclusion_reason(&name) {
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
        capture_request_id = capture_request_id.wrapping_add(1).max(1);

        let (returned_client, result) = run_case(
            client,
            &mut sim,
            config_revision,
            data_revision,
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
