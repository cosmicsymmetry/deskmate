use std::env;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use device::{ConnectedSession, DeviceError, connect_session};
use protocol::{
    ActivateScreen, Field, FieldValue, InterruptPolicy, ScreenConfig, SizeClass, StatusResponse,
    TapAction, TemplateKind, WidgetConfig,
};

const DEFAULT_CONFIG_SWAPS: u32 = 100;
const DEFAULT_PATCHES: u32 = 1_000;
const DEFAULT_IDLE_SECONDS: u64 = 65;
const MAX_HEAP_REGRESSION_BYTES: u32 = 64 * 1024;

struct Options {
    port: Option<String>,
    config_swaps: u32,
    patches: u32,
    idle_seconds: u64,
    soak_seconds: u64,
}

fn parse_number<T: std::str::FromStr>(raw: Option<String>, option: &str) -> Result<T, String> {
    raw.ok_or_else(|| format!("{option} requires a value"))?
        .parse()
        .map_err(|_| format!("{option} requires a nonnegative integer"))
}

fn parse_options() -> Result<Options, String> {
    let mut arguments = env::args().skip(1);
    let mut options = Options {
        port: None,
        config_swaps: DEFAULT_CONFIG_SWAPS,
        patches: DEFAULT_PATCHES,
        idle_seconds: DEFAULT_IDLE_SECONDS,
        soak_seconds: 0,
    };
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--port" => {
                options.port = Some(
                    arguments
                        .next()
                        .ok_or_else(|| "--port requires a value".to_owned())?,
                );
            }
            "--config-swaps" => {
                options.config_swaps = parse_number(arguments.next(), "--config-swaps")?;
            }
            "--patches" => {
                options.patches = parse_number(arguments.next(), "--patches")?;
            }
            "--idle-seconds" => {
                options.idle_seconds = parse_number(arguments.next(), "--idle-seconds")?;
            }
            "--soak-seconds" => {
                options.soak_seconds = parse_number(arguments.next(), "--soak-seconds")?;
            }
            _ => return Err(format!("unknown option: {argument}")),
        }
    }
    Ok(options)
}

fn widget(
    id: &str,
    template: TemplateKind,
    size_class: SizeClass,
    tap_action: TapAction,
    interrupts: bool,
) -> WidgetConfig {
    WidgetConfig {
        widget_id: id.into(),
        template,
        size_class,
        tap_action,
        interrupt_policy: if interrupts {
            InterruptPolicy::Enabled
        } else {
            InterruptPolicy::Disabled
        },
    }
}

fn screen(id: &str, widget_id: &str) -> ScreenConfig {
    ScreenConfig {
        screen_id: id.into(),
        widget_id: widget_id.into(),
    }
}

fn layout(reverse: bool) -> (Vec<WidgetConfig>, Vec<ScreenConfig>) {
    let widgets = vec![
        widget(
            "clock",
            TemplateKind::DigitalClock,
            SizeClass::Full,
            TapAction::None,
            false,
        ),
        widget(
            "pomodoro",
            TemplateKind::ProgressRing,
            SizeClass::Standard,
            TapAction::StartPause,
            true,
        ),
        widget(
            "calendar",
            TemplateKind::RowList,
            SizeClass::Standard,
            TapAction::None,
            true,
        ),
    ];
    let mut screens = vec![
        screen("clock-screen", "clock"),
        screen("pomodoro-screen", "pomodoro"),
        screen("calendar-screen", "calendar"),
    ];
    if reverse {
        screens.reverse();
    }
    (widgets, screens)
}

fn text(key: &str, value: impl Into<String>) -> Field {
    Field {
        key: key.into(),
        value: FieldValue::Text(value.into()),
    }
}

fn integer(key: &str, value: i64) -> Field {
    Field {
        key: key.into(),
        value: FieldValue::Integer(value),
    }
}

fn boolean(key: &str, value: bool) -> Field {
    Field {
        key: key.into(),
        value: FieldValue::Boolean(value),
    }
}

fn patch_fields(connected: &ConnectedSession, iteration: u64) -> Result<(), DeviceError> {
    match iteration % 3 {
        0 => {
            connected.session.push_fields(
                "clock",
                vec![
                    text("title", format!("Stress {iteration:05}")),
                    boolean("show_seconds", iteration.is_multiple_of(2)),
                    boolean("stale", false),
                    text("error", ""),
                ],
            )?;
        }
        1 => {
            let remaining = i64::try_from(60 - iteration % 61).unwrap_or(0);
            connected.session.push_fields(
                "pomodoro",
                vec![
                    text("label", "Stress timer"),
                    integer("duration_seconds", 60),
                    integer("remaining_seconds", remaining),
                    boolean("running", iteration.is_multiple_of(2)),
                    boolean("stale", false),
                    text("error", ""),
                ],
            )?;
        }
        _ => {
            connected.session.push_fields(
                "calendar",
                vec![
                    text("title", "Stress calendar"),
                    text("row0_title", format!("Render sample {iteration:05}")),
                    text(
                        "row0_time",
                        format!("{:02}:{:02}", iteration % 24, iteration % 60),
                    ),
                    boolean("stale", false),
                    text("error", ""),
                ],
            )?;
        }
    }
    Ok(())
}

fn activate_for_iteration(connected: &ConnectedSession, iteration: u64) -> Result<(), DeviceError> {
    let screen_id = match iteration % 3 {
        0 => "clock-screen",
        1 => "pomodoro-screen",
        _ => "calendar-screen",
    };
    connected.session.activate_screen(ActivateScreen {
        screen_id: screen_id.into(),
    })?;
    Ok(())
}

fn checked_status(
    connected: &ConnectedSession,
    previous_uptime: &mut u64,
    heap_floor: &mut u32,
) -> Result<StatusResponse, String> {
    let status = connected
        .session
        .status()
        .map_err(|error| error.to_string())?;
    if status.uptime_ms < *previous_uptime {
        return Err("device uptime moved backwards (reset detected)".into());
    }
    if !status.online {
        return Err("device reported standalone mode during an active session".into());
    }
    *previous_uptime = status.uptime_ms;
    *heap_floor = (*heap_floor).min(status.free_heap);
    Ok(status)
}

fn drain_events(connected: &ConnectedSession, received: &mut u64) {
    while connected.session.try_recv_event().is_some() {
        *received += 1;
    }
}

fn run_config_swaps(
    connected: &ConnectedSession,
    count: u32,
    previous_uptime: &mut u64,
    heap_floor: &mut u32,
) -> Result<u32, String> {
    let mut samples = 0_u32;
    for swap in 0..count {
        let (widgets, screens) = layout(swap % 2 != 0);
        connected
            .session
            .apply_next_config(widgets, screens)
            .map_err(|error| format!("config swap {} failed: {error}", swap + 1))?;
        if swap % 10 == 9 || swap + 1 == count {
            thread::sleep(Duration::from_millis(25));
            let status = checked_status(connected, previous_uptime, heap_floor)?;
            samples += 1;
            println!(
                "configs={} heap={} ui_high_water={} ui_dropped={}",
                swap + 1,
                status.free_heap,
                status.ui_queue_high_water,
                status.dropped_ui_commands
            );
        }
    }
    Ok(samples)
}

fn run_patches(
    connected: &ConnectedSession,
    count: u32,
    previous_uptime: &mut u64,
    heap_floor: &mut u32,
    received_events: &mut u64,
) -> Result<u32, String> {
    let mut samples = 0_u32;
    for patch in 0..count {
        let iteration = u64::from(patch);
        patch_fields(connected, iteration)
            .map_err(|error| format!("field patch {} failed: {error}", patch + 1))?;
        if patch.is_multiple_of(10) {
            activate_for_iteration(connected, iteration)
                .map_err(|error| format!("screen activation failed: {error}"))?;
        }
        drain_events(connected, received_events);
        if patch % 100 == 99 || patch + 1 == count {
            thread::sleep(Duration::from_millis(25));
            let status = checked_status(connected, previous_uptime, heap_floor)?;
            samples += 1;
            println!(
                "patches={} heap={} event_high_water={} event_dropped={} ui_high_water={} ui_dropped={}",
                patch + 1,
                status.free_heap,
                status.event_queue_high_water,
                status.dropped_events,
                status.ui_queue_high_water,
                status.dropped_ui_commands
            );
        }
    }
    Ok(samples)
}

fn run_idle(
    connected: &ConnectedSession,
    seconds: u64,
    previous_uptime: &mut u64,
    heap_floor: &mut u32,
) -> Result<(), String> {
    let keepalives_before = connected.session.diagnostics().keepalives_sent;
    if seconds != 0 {
        println!("idle_start seconds={seconds}");
        thread::sleep(Duration::from_secs(seconds));
    }
    let status = checked_status(connected, previous_uptime, heap_floor)?;
    let keepalives_after = connected.session.diagnostics().keepalives_sent;
    if seconds >= 10 && keepalives_after == keepalives_before {
        return Err("idle session sent no keepalive heartbeats".into());
    }
    println!(
        "idle_done online={} keepalives={} heap={}",
        status.online,
        keepalives_after.saturating_sub(keepalives_before),
        status.free_heap
    );
    Ok(())
}

fn run_soak(
    connected: &ConnectedSession,
    seconds: u64,
    first_iteration: u64,
    previous_uptime: &mut u64,
    heap_floor: &mut u32,
    received_events: &mut u64,
) -> Result<u32, String> {
    let started = Instant::now();
    let deadline = started + Duration::from_secs(seconds);
    let mut iteration = first_iteration;
    let mut next_status = Duration::from_mins(1);
    let mut samples = 0_u32;
    while Instant::now() < deadline {
        patch_fields(connected, iteration)
            .map_err(|error| format!("soak patch {iteration} failed: {error}"))?;
        if iteration.is_multiple_of(20) {
            activate_for_iteration(connected, iteration)
                .map_err(|error| format!("soak screen activation failed: {error}"))?;
        }
        drain_events(connected, received_events);
        iteration += 1;
        if started.elapsed() >= next_status {
            let status = checked_status(connected, previous_uptime, heap_floor)?;
            samples += 1;
            println!(
                "soak_elapsed={}s heap={} valid={} malformed={} event_high_water={} event_dropped={} ui_high_water={} ui_dropped={}",
                started.elapsed().as_secs(),
                status.free_heap,
                status.valid_frames,
                status.malformed_frames,
                status.event_queue_high_water,
                status.dropped_events,
                status.ui_queue_high_water,
                status.dropped_ui_commands
            );
            next_status += Duration::from_mins(1);
        }
        thread::sleep(Duration::from_millis(250));
    }
    Ok(samples)
}

fn run() -> Result<(), String> {
    let options = parse_options()?;
    let connected = connect_session(options.port.as_deref()).map_err(|error| error.to_string())?;
    let initial = connected.initial_status.clone();
    let mut previous_uptime = initial.uptime_ms;
    let mut heap_floor = initial.free_heap;
    let mut status_samples = 0_u32;
    let mut received_events = 0_u64;

    status_samples += run_config_swaps(
        &connected,
        options.config_swaps,
        &mut previous_uptime,
        &mut heap_floor,
    )?;

    thread::sleep(Duration::from_millis(100));
    let stress_baseline = checked_status(&connected, &mut previous_uptime, &mut heap_floor)?;
    status_samples += run_patches(
        &connected,
        options.patches,
        &mut previous_uptime,
        &mut heap_floor,
        &mut received_events,
    )?;

    run_idle(
        &connected,
        options.idle_seconds,
        &mut previous_uptime,
        &mut heap_floor,
    )?;
    drain_events(&connected, &mut received_events);

    if options.soak_seconds != 0 {
        status_samples += run_soak(
            &connected,
            options.soak_seconds,
            u64::from(options.patches),
            &mut previous_uptime,
            &mut heap_floor,
            &mut received_events,
        )?;
    }

    thread::sleep(Duration::from_millis(100));
    let final_status = checked_status(&connected, &mut previous_uptime, &mut heap_floor)?;
    let heap_regression = stress_baseline
        .free_heap
        .saturating_sub(final_status.free_heap);
    if heap_regression > MAX_HEAP_REGRESSION_BYTES {
        return Err(format!(
            "heap regressed by {heap_regression} bytes (limit {MAX_HEAP_REGRESSION_BYTES})"
        ));
    }
    let diagnostics = connected.session.diagnostics();
    println!(
        "PASS port={} configs={} patches={} idle={}s soak={}s uptime_ms={} heap_initial={} heap_baseline={} heap_final={} heap_floor={} samples={} valid={} malformed={} crc={} overflow={} responses_dropped={} rx_drops={} events_received={} events_dropped={} event_high_water={} ui_dropped={} ui_high_water={} keepalives={} reconnects={} host_event_drops={} event_gaps={}",
        connected.port_name,
        options.config_swaps,
        options.patches,
        options.idle_seconds,
        options.soak_seconds,
        final_status.uptime_ms,
        initial.free_heap,
        stress_baseline.free_heap,
        final_status.free_heap,
        heap_floor,
        status_samples,
        final_status.valid_frames,
        final_status.malformed_frames,
        final_status.crc_errors,
        final_status.overflow_frames,
        final_status.dropped_responses,
        final_status.rx_dropped_bytes,
        received_events,
        final_status.dropped_events,
        final_status.event_queue_high_water,
        final_status.dropped_ui_commands,
        final_status.ui_queue_high_water,
        diagnostics.keepalives_sent,
        diagnostics.reconnects,
        diagnostics.locally_dropped_events,
        diagnostics.detected_event_gaps
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("M2 stress failed: {error}");
        process::exit(1);
    }
}
