//! Dev-only physical check for the scene interpreter's RGB565 output.
//!
//! This sends the parity gate's `DigitalClock` scenes and the asset-free,
//! representable entries from `lvgl_sim::cases::scene_cases()` to a device,
//! then byte-compares a 0x7E capture against `Simulator::render_scene()` for
//! the identical scene and render context. It must run against a device in
//! local tier flashed from a `DESKMATE_DEV_DIAG=1` build.
//!
//! Run from `companion/`:
//! `cargo run -p app-core --example scene_panel_check -- [--port /dev/cu.usbmodem101]`
//!
//! # What the 270-degree half can and cannot prove
//!
//! Firmware's dev capture re-renders the active object tree into a pre-flush
//! logical buffer. The panel's 90/270 software rotation happens later, in the
//! flush path, so the raw capture is orientation-independent. Like
//! `framebuffer_diff`, this check reverses the logical capture before comparing
//! a flipped simulator frame. It proves the hardware decoder/interpreter drew
//! the right logical pixels while the device was configured at both rotations;
//! it cannot prove the physical 270-degree flush transform byte for byte.
//!
//! # Sequencing and settle time
//!
//! `ApplyConfig` queues the standalone-card fallback while `PushScene` shows
//! synchronously and cancels that provisional fallback. This older harness
//! still lets the queue drain before the scene push. Time-sync is last: it
//! updates the device clock used by `time:` bindings, and the scene-binding
//! tick then redraws them. Both waits use 300 ms, generous against the 20 ms UI
//! queue poll and 250 ms scene tick while remaining below the one second that
//! would roll the pinned displayed second.

use std::env;
use std::path::Path;
use std::process;
use std::thread;
use std::time::Duration;

use app_core::{BakedFontMetrics, ClockCard, build_digital_clock_scene};
use chrono::{NaiveDate, NaiveDateTime};
use device::framebuffer_capture::{CaptureError, FRAME_BYTES, capture_framebuffer};
use device::{DeviceClient, DeviceError, Transport, connect};
use lvgl_sim::scene::{SceneAsset, SceneRenderRequest, SceneTimer};
use lvgl_sim::{LOGICAL_HEIGHT, LOGICAL_WIDTH, SimOrientation, Simulator, cases};
use protocol::{
    ApplyConfig, AssetKind, ErrorCode, Field, FieldValue, InterruptPolicy, Message, PushData,
    PushScene, ScreenConfig, SizeClass, TYPE_APPLY_CONFIG, TYPE_PUSH_SCENE, TapAction,
    TemplateKind, Tier, TimeSync, WidgetConfig,
};

// The capture module cannot import the simulator, so CI compiles this example to pin their dimensions together.
const _: () = assert!(
    device::framebuffer_capture::FRAME_WIDTH == lvgl_sim::LOGICAL_WIDTH as usize
        && device::framebuffer_capture::FRAME_HEIGHT == lvgl_sim::LOGICAL_HEIGHT as usize,
);

const CARD_ID: &str = "scene-check";
const SCREEN_ID: &str = "scene-check-screen";
const SETTLE: Duration = Duration::from_millis(300);

struct Instant {
    slug: &'static str,
    utc_offset_minutes: i16,
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

impl Instant {
    fn local_now(&self) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(self.year, self.month, self.day)
            .expect("a real date")
            .and_hms_opt(self.hour, self.minute, self.second)
            .expect("a real time")
    }

    fn now_unix_seconds(&self) -> i64 {
        self.local_now().and_utc().timestamp() - i64::from(self.utc_offset_minutes) * 60
    }
}

const INSTANTS: &[Instant] = &[
    Instant {
        slug: "wed-aug-12-1609",
        utc_offset_minutes: 240,
        year: 2026,
        month: 8,
        day: 12,
        hour: 16,
        minute: 9,
        second: 37,
    },
    Instant {
        slug: "sun-aug-2-midnight",
        utc_offset_minutes: -300,
        year: 2026,
        month: 8,
        day: 2,
        hour: 0,
        minute: 0,
        second: 0,
    },
    Instant {
        slug: "mon-aug-3-noon",
        utc_offset_minutes: 0,
        year: 2026,
        month: 8,
        day: 3,
        hour: 12,
        minute: 0,
        second: 0,
    },
    Instant {
        slug: "thu-jan-1-2359",
        utc_offset_minutes: 240,
        year: 2026,
        month: 1,
        day: 1,
        hour: 23,
        minute: 59,
        second: 59,
    },
    Instant {
        slug: "fri-dec-25-0905",
        utc_offset_minutes: 330,
        year: 2026,
        month: 12,
        day: 25,
        hour: 9,
        minute: 5,
        second: 3,
    },
    Instant {
        slug: "sat-feb-28-0315",
        utc_offset_minutes: -480,
        year: 2026,
        month: 2,
        day: 28,
        hour: 3,
        minute: 15,
        second: 0,
    },
    Instant {
        slug: "tue-jun-30-2133",
        utc_offset_minutes: 60,
        year: 2026,
        month: 6,
        day: 30,
        hour: 21,
        minute: 33,
        second: 7,
    },
];

struct CheckCase {
    name: String,
    request: SceneRenderRequest,
    template: TemplateKind,
    fields: Vec<Field>,
    exclusion: Option<String>,
}

fn orientation_slug(orientation: SimOrientation) -> &'static str {
    match orientation {
        SimOrientation::Landscape => "landscape",
        SimOrientation::LandscapeFlipped => "flipped",
    }
}

fn rotation_degrees(orientation: SimOrientation) -> u16 {
    match orientation {
        SimOrientation::Landscape => 90,
        SimOrientation::LandscapeFlipped => 270,
    }
}

fn digital_clock_cases() -> Vec<CheckCase> {
    let mut checks = Vec::new();
    for instant in INSTANTS {
        for show_seconds in [true, false] {
            for orientation in [SimOrientation::Landscape, SimOrientation::LandscapeFlipped] {
                let seconds_slug = if show_seconds {
                    "seconds"
                } else {
                    "no-seconds"
                };
                let name = format!(
                    "digital-clock--{}--{seconds_slug}--{}",
                    instant.slug,
                    orientation_slug(orientation)
                );
                let scene = build_digital_clock_scene(
                    &ClockCard {
                        revision: 1,
                        show_seconds,
                        local_now: instant.local_now(),
                    },
                    &BakedFontMetrics::SHIPPED,
                );
                checks.push(CheckCase {
                    name,
                    request: SceneRenderRequest {
                        scene,
                        assets: Vec::new(),
                        utc_offset_minutes: instant.utc_offset_minutes,
                        now_unix_seconds: instant.now_unix_seconds(),
                        timer: None,
                        fields: Vec::new(),
                        orientation,
                    },
                    template: TemplateKind::DigitalClock,
                    fields: Vec::new(),
                    exclusion: None,
                });
            }
        }
    }
    checks
}

fn asset_exclusion(assets: &[SceneAsset]) -> Option<String> {
    if assets.iter().any(|asset| asset.kind == AssetKind::Image) {
        Some(
            "the scene requires a registered RGB565 image asset; asset transfer is out of scope"
                .to_string(),
        )
    } else if assets
        .iter()
        .any(|asset| matches!(asset.kind, AssetKind::Font | AssetKind::IconFont))
    {
        Some(
            "the scene requires a registered runtime font asset; asset transfer is out of scope"
                .to_string(),
        )
    } else {
        None
    }
}

fn timer_fields(timer: SceneTimer) -> Vec<Field> {
    // The asset-free arc case reads timer.pct but not timer.remaining. A
    // stopped 100-second ProgressRing snapshot represents every integer
    // remaining percentage exactly and cannot move during push-to-capture
    // latency, so the device producer and simulator receive the same value.
    let duration = i64::from(timer.total_ms / 1_000);
    let remaining = i64::from(timer.remaining_ms / 1_000);
    vec![
        Field {
            key: "duration_seconds".into(),
            value: FieldValue::Integer(duration),
        },
        Field {
            key: "remaining_seconds".into(),
            value: FieldValue::Integer(remaining),
        },
        Field {
            key: "running".into(),
            value: FieldValue::Boolean(timer.running),
        },
    ]
}

fn scene_node_cases() -> Vec<CheckCase> {
    cases::scene_cases()
        .into_iter()
        .map(|(name, request)| {
            let exclusion = asset_exclusion(&request.assets).or_else(|| {
                (!request.fields.is_empty()).then(|| {
                    "field.status cannot be supplied to hardware: PushData retains only fields \
                     registered by a built-in template, and no template registers status"
                        .to_string()
                })
            });
            let (template, fields) = match request.timer {
                Some(timer) => (TemplateKind::ProgressRing, timer_fields(timer)),
                None => (TemplateKind::DigitalClock, Vec::new()),
            };
            CheckCase {
                name,
                request,
                template,
                fields,
                exclusion,
            }
        })
        .collect()
}

fn all_cases() -> Vec<CheckCase> {
    let mut checks = digital_clock_cases();
    checks.extend(scene_node_cases());
    checks
}

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

enum CaseError {
    WrongTier { operation: &'static str },
    Capture(CaptureError),
    Other(String),
}

impl std::fmt::Display for CaseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongTier { operation } => {
                write!(
                    formatter,
                    "{operation}: device rejected the request with WrongTier"
                )
            }
            Self::Capture(error) => write!(formatter, "capture: {error}"),
            Self::Other(message) => formatter.write_str(message),
        }
    }
}

fn device_error(operation: &'static str, error: DeviceError) -> CaseError {
    match error {
        DeviceError::Rejected(response) if response.code == ErrorCode::WrongTier => {
            CaseError::WrongTier { operation }
        }
        other => CaseError::Other(format!("{operation}: {other}")),
    }
}

fn apply_config(
    client: &mut DeviceClient<impl Transport>,
    revision: u32,
    case: &CheckCase,
) -> Result<(), CaseError> {
    let config = ApplyConfig {
        revision,
        rotation: rotation_degrees(case.request.orientation),
        widgets: vec![WidgetConfig {
            widget_id: CARD_ID.into(),
            template: case.template,
            size_class: SizeClass::Full,
            tap_action: TapAction::None,
            interrupt_policy: InterruptPolicy::Disabled,
        }],
        screens: vec![ScreenConfig {
            screen_id: SCREEN_ID.into(),
            widget_id: CARD_ID.into(),
        }],
    };
    match client
        .request(&Message::ApplyConfig(config))
        .map_err(|error| device_error("apply config", error))?
    {
        Message::Ack(ack)
            if ack.acknowledged_type == TYPE_APPLY_CONFIG && ack.revision == Some(revision) =>
        {
            Ok(())
        }
        message => Err(CaseError::Other(format!(
            "unexpected apply-config response: {message:?}"
        ))),
    }
}

fn push_fields(
    client: &mut DeviceClient<impl Transport>,
    revision: u32,
    fields: &[Field],
) -> Result<(), CaseError> {
    if fields.is_empty() {
        return Ok(());
    }
    let ack = client
        .push_data(PushData {
            widget_id: CARD_ID.into(),
            revision,
            fields: fields.to_vec(),
        })
        .map_err(|error| device_error("push data", error))?;
    if ack.revision == Some(revision) {
        Ok(())
    } else {
        Err(CaseError::Other(format!(
            "push acknowledgement revision mismatch: expected {revision}, received {:?}",
            ack.revision
        )))
    }
}

fn push_scene(
    client: &mut DeviceClient<impl Transport>,
    revision: u32,
    scene: &protocol::Scene,
) -> Result<(), CaseError> {
    match client
        .request(&Message::PushScene(PushScene {
            card_id: CARD_ID.into(),
            revision,
            scene: scene.clone(),
        }))
        .map_err(|error| device_error("push scene", error))?
    {
        Message::Ack(ack)
            if ack.acknowledged_type == TYPE_PUSH_SCENE && ack.revision == Some(revision) =>
        {
            Ok(())
        }
        message => Err(CaseError::Other(format!(
            "unexpected push-scene response: {message:?}"
        ))),
    }
}

fn sync_time(
    client: &mut DeviceClient<impl Transport>,
    request: &SceneRenderRequest,
) -> Result<(), CaseError> {
    client
        .time_sync(TimeSync {
            unix_seconds: request.now_unix_seconds,
            utc_offset_minutes: request.utc_offset_minutes,
        })
        .map_err(|error| device_error("time sync", error))?;
    Ok(())
}

fn bytes_to_pixels(bytes: &[u8]) -> Vec<u16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect()
}

fn orient_capture(pixels: Vec<u16>, orientation: SimOrientation) -> Vec<u16> {
    match orientation {
        SimOrientation::Landscape => pixels,
        SimOrientation::LandscapeFlipped => pixels.into_iter().rev().collect(),
    }
}

struct Difference {
    count: usize,
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
    samples: Vec<(u32, u32, u16, u16)>,
}

fn diff(expected: &[u16], device: &[u16]) -> Result<Option<Difference>, CaseError> {
    if expected.len() != device.len() {
        return Err(CaseError::Other(format!(
            "pixel count mismatch: simulator produced {}, device produced {}",
            expected.len(),
            device.len()
        )));
    }
    if expected.len() != (LOGICAL_WIDTH * LOGICAL_HEIGHT) as usize {
        return Err(CaseError::Other(format!(
            "simulator produced {} pixels instead of a full {}x{} frame",
            expected.len(),
            LOGICAL_WIDTH,
            LOGICAL_HEIGHT
        )));
    }
    let mut difference = Difference {
        count: 0,
        min_x: u32::MAX,
        min_y: u32::MAX,
        max_x: 0,
        max_y: 0,
        samples: Vec::new(),
    };
    for (index, (left, right)) in expected.iter().zip(device.iter()).enumerate() {
        if left == right {
            continue;
        }
        let index = u32::try_from(index).expect("a 448x368 frame indexes inside u32");
        let (x, y) = (index % LOGICAL_WIDTH, index / LOGICAL_WIDTH);
        difference.count += 1;
        difference.min_x = difference.min_x.min(x);
        difference.min_y = difference.min_y.min(y);
        difference.max_x = difference.max_x.max(x);
        difference.max_y = difference.max_y.max(y);
        if difference.samples.len() < 8 {
            difference.samples.push((x, y, *left, *right));
        }
    }
    Ok((difference.count > 0).then_some(difference))
}

fn dump(name: &str, expected: &[u16], device: &[u16]) -> String {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/scene-panel-check");
    if std::fs::create_dir_all(&directory).is_err() {
        return "could not create target/scene-panel-check to dump PNGs".to_string();
    }
    let mut written = Vec::new();
    for (suffix, pixels) in [("simulator", expected), ("device", device)] {
        let path = directory.join(format!("{name}--{suffix}.png"));
        if write_png(&path, pixels).is_ok() {
            written.push(path.display().to_string());
        }
    }
    if written.len() == 2 {
        format!("dumped: {}", written.join(", "))
    } else {
        format!("dumped only: {}", written.join(", "))
    }
}

fn write_png(path: &Path, pixels: &[u16]) -> std::io::Result<()> {
    let png = lvgl_sim::pixels_to_png(pixels)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    std::fs::write(path, png)
}

enum CaseOutcome {
    Identical,
    Differ {
        difference: Difference,
        dump: String,
    },
}

struct Revisions {
    config: u32,
    data: u32,
    scene: u32,
    capture_request_id: u32,
}

impl Revisions {
    fn increment(value: &mut u32, what: &str) -> Result<u32, CaseError> {
        *value = value
            .checked_add(1)
            .ok_or_else(|| CaseError::Other(format!("{what} revision exhausted")))?;
        Ok(*value)
    }

    fn next_config(&mut self) -> Result<u32, CaseError> {
        Self::increment(&mut self.config, "config")
    }

    fn next_data(&mut self) -> Result<u32, CaseError> {
        Self::increment(&mut self.data, "data")
    }

    fn next_scene(&mut self) -> Result<u32, CaseError> {
        Self::increment(&mut self.scene, "scene")
    }

    fn next_capture_request_id(&mut self) -> u32 {
        self.capture_request_id = self.capture_request_id.wrapping_add(1).max(1);
        self.capture_request_id
    }
}

#[allow(clippy::too_many_lines)]
fn run_case<T: Transport>(
    mut client: DeviceClient<T>,
    sim: &mut Simulator,
    revisions: &mut Revisions,
    case: &CheckCase,
) -> (DeviceClient<T>, Result<CaseOutcome, CaseError>) {
    let setup = (|| {
        let config_revision = revisions.next_config()?;
        apply_config(&mut client, config_revision, case)?;
        if !case.fields.is_empty() {
            let data_revision = revisions.next_data()?;
            push_fields(&mut client, data_revision, &case.fields)?;
        }

        // ApplyConfig/PushData are queued but PushScene is synchronous. Drain
        // the former first so a late template command cannot delete the scene.
        thread::sleep(SETTLE);

        let scene_revision = revisions.next_scene()?;
        push_scene(&mut client, scene_revision, &case.request.scene)?;
        // Last: the binding refresh caused by the device's scene tick must see
        // this case's pinned clock after all preceding UI work has landed.
        sync_time(&mut client, &case.request)?;
        Ok(())
    })();
    if let Err(error) = setup {
        return (client, Err(error));
    }

    thread::sleep(SETTLE);
    let capture_request_id = revisions.next_capture_request_id();
    let (client, capture) = capture_framebuffer(client, capture_request_id);
    let raw = match capture {
        Ok(raw) => raw,
        Err(error) => return (client, Err(CaseError::Capture(error))),
    };
    if raw.len() != FRAME_BYTES {
        return (
            client,
            Err(CaseError::Other(format!(
                "capture returned {} bytes instead of {FRAME_BYTES}",
                raw.len()
            ))),
        );
    }

    let device = orient_capture(bytes_to_pixels(&raw), case.request.orientation);
    let expected = match sim.render_scene(&case.request) {
        Ok(expected) => expected,
        Err(error) => {
            return (
                client,
                Err(CaseError::Other(format!(
                    "simulator render failed: {error}"
                ))),
            );
        }
    };
    let difference = match diff(&expected, &device) {
        Ok(difference) => difference,
        Err(error) => return (client, Err(error)),
    };
    let outcome = match difference {
        None => CaseOutcome::Identical,
        Some(difference) => CaseOutcome::Differ {
            dump: dump(&case.name, &expected, &device),
            difference,
        },
    };
    (client, Ok(outcome))
}

fn summary(total: usize, identical: usize, differing: usize, errored: usize, excluded: usize) {
    println!(
        "SUMMARY total={total} identical={identical} differing={differing} \
         errored={errored} excluded={excluded}"
    );
}

#[allow(clippy::too_many_lines)]
fn run() -> Result<(), String> {
    let port = parse_port()?;
    let connected = connect(port.as_deref()).map_err(|error| error.to_string())?;
    println!(
        "connected port={} tier={:?}",
        connected.port_name, connected.initial_status.tier
    );
    println!(
        "NOTE: 0x7E captures the pre-flush logical frame. Flipped comparisons reverse it \
         on the host and do not prove the panel's physical 270-degree flush transform."
    );

    let checks = all_cases();
    let excluded = checks
        .iter()
        .filter_map(|case| {
            case.exclusion.as_ref().map(|reason| {
                println!("{}: excluded ({reason})", case.name);
            })
        })
        .count();
    let runnable: Vec<&CheckCase> = checks
        .iter()
        .filter(|case| case.exclusion.is_none())
        .collect();
    let total = checks.len();

    if connected.initial_status.tier == Tier::Networked {
        eprintln!(
            "the device is in networked tier; this harness needs local tier because firmware \
             refuses ApplyConfig, PushScene, and TimeSync over USB with WrongTier"
        );
        summary(total, 0, 0, runnable.len(), excluded);
        return Err("device must be switched to local tier before this check".to_string());
    }

    let mut client = connected.client;
    let mut revisions = Revisions {
        config: connected.initial_status.config_revision,
        data: connected.initial_status.latest_revision,
        scene: connected
            .initial_status
            .latest_revision
            .max(connected.initial_status.config_revision),
        capture_request_id: 1,
    };
    let mut sim = Simulator::new().map_err(|error| format!("simulator init failed: {error}"))?;
    let mut identical = 0usize;
    let mut differing = 0usize;
    let mut errored = 0usize;

    for (index, case) in runnable.iter().enumerate() {
        let (returned_client, result) = run_case(client, &mut sim, &mut revisions, case);
        client = returned_client;
        match result {
            Ok(CaseOutcome::Identical) => {
                println!("{}: identical", case.name);
                identical += 1;
            }
            Ok(CaseOutcome::Differ { difference, dump }) => {
                let samples = difference
                    .samples
                    .iter()
                    .map(|(x, y, expected, actual)| {
                        format!("({x},{y}) {expected:#06x}/{actual:#06x}")
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                println!(
                    "{}: {} of {} pixels differ, bounding box x {}..={} y {}..={}\n  \
                     samples (simulator/device): {samples}\n  {dump}",
                    case.name,
                    difference.count,
                    (LOGICAL_WIDTH * LOGICAL_HEIGHT),
                    difference.min_x,
                    difference.max_x,
                    difference.min_y,
                    difference.max_y,
                );
                differing += 1;
            }
            Err(error) => {
                println!("{}: ERROR {error}", case.name);
                let fatal = match &error {
                    CaseError::WrongTier { .. } => {
                        eprintln!(
                            "the device is in networked tier; this harness needs local tier because \
                             firmware refuses ApplyConfig, PushScene, and TimeSync over USB"
                        );
                        true
                    }
                    CaseError::Capture(capture) if index == 0 && capture.is_timeout() => {
                        eprintln!(
                            "the first capture timed out; the device is probably not running a \
                             DESKMATE_DEV_DIAG=1 build. Flash a dev-diag image before retrying."
                        );
                        true
                    }
                    _ => false,
                };
                errored += 1;
                if fatal {
                    let remaining = runnable.len() - index - 1;
                    if remaining > 0 {
                        eprintln!(
                            "aborting {remaining} remaining case(s) after prerequisite failure"
                        );
                        errored += remaining;
                    }
                    break;
                }
            }
        }
    }

    summary(total, identical, differing, errored, excluded);
    if differing > 0 || errored > 0 {
        return Err(format!(
            "{differing} case(s) differed and {errored} case(s) errored -- see output above"
        ));
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("scene panel check failed: {error}");
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_timer_fields_preserve_remaining_percentage_semantics() {
        let fields = timer_fields(SceneTimer {
            total_ms: 100_000,
            remaining_ms: 60_000,
            running: false,
        });
        assert!(matches!(
            fields.get(1),
            Some(Field {
                key,
                value: FieldValue::Integer(60),
            }) if key == "remaining_seconds"
        ));
    }

    #[test]
    fn case_table_has_the_parity_matrix_and_only_explicit_scene_exclusions() {
        let checks = all_cases();
        assert_eq!(
            checks
                .iter()
                .filter(|case| case.name.starts_with("digital-clock--"))
                .count(),
            INSTANTS.len() * 4
        );

        let excluded = checks
            .iter()
            .filter_map(|case| {
                case.exclusion
                    .as_deref()
                    .map(|reason| (case.name.as_str(), reason))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            excluded,
            [
                (
                    "scene-text--landscape",
                    "field.status cannot be supplied to hardware: PushData retains only fields registered by a built-in template, and no template registers status",
                ),
                (
                    "scene-text--flipped",
                    "field.status cannot be supplied to hardware: PushData retains only fields registered by a built-in template, and no template registers status",
                ),
                (
                    "scene-image--landscape",
                    "the scene requires a registered RGB565 image asset; asset transfer is out of scope",
                ),
                (
                    "scene-image--flipped",
                    "the scene requires a registered RGB565 image asset; asset transfer is out of scope",
                ),
                (
                    "scene-glyph--landscape",
                    "the scene requires a registered runtime font asset; asset transfer is out of scope",
                ),
                (
                    "scene-glyph--flipped",
                    "the scene requires a registered runtime font asset; asset transfer is out of scope",
                ),
                (
                    "scene-label--landscape",
                    "field.status cannot be supplied to hardware: PushData retains only fields registered by a built-in template, and no template registers status",
                ),
                (
                    "scene-label--flipped",
                    "field.status cannot be supplied to hardware: PushData retains only fields registered by a built-in template, and no template registers status",
                ),
            ]
        );
        assert_eq!(checks.len(), 46);
        assert_eq!(checks.len() - excluded.len(), 38);
    }
}
