use std::collections::VecDeque;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use device::{ConnectedSession, DeviceError, connect_session};
use engine::interrupts::InterruptArbiter;
use engine::pomodoro::Pomodoro;
use protocol::{
    ActivateScreen, ApplyConfig, ErrorCode, EventAction, EventKind, Field, FieldValue,
    InterruptPolicy, Message, PushData, ScreenConfig, SizeClass, TapAction, TemplateKind,
    TriggerInterrupt, WidgetConfig, encode_message,
};
use serde::Deserialize;

use crate::{AppError, json_string};

pub const M2_USAGE: &str = "\
M2 commands:
  deskmate-cli inspect-config --config PATH [--json]
  deskmate-cli apply-config --config PATH [--port PATH] [--json]
  deskmate-cli select-screen --screen ID [--port PATH] [--json]
  deskmate-cli push-clock --widget ID [--title TEXT] [--show-seconds BOOL] [--port PATH] [--json]
  deskmate-cli pomodoro --widget ID [--duration-seconds N] [--label TEXT] [--port PATH] [--json]
  deskmate-cli trigger-interrupt --widget ID --token N --reason TEXT [--port PATH] [--json]
  deskmate-cli events [--count N] [--port PATH] [--json]";

const M2_COMMANDS: &[&str] = &[
    "inspect-config",
    "apply-config",
    "select-screen",
    "push-clock",
    "pomodoro",
    "trigger-interrupt",
    "events",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutFile {
    widgets: Vec<LayoutWidget>,
    screens: Vec<LayoutScreen>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutWidget {
    id: String,
    template: String,
    size: String,
    tap_action: String,
    interrupts: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutScreen {
    id: String,
    widget: String,
}

#[derive(Debug, Default)]
struct CommonOptions {
    port: Option<String>,
    json: bool,
}

struct Arguments {
    values: VecDeque<String>,
    common: CommonOptions,
}

impl Arguments {
    fn new(values: impl IntoIterator<Item = String>) -> Self {
        Self {
            values: values.into_iter().collect(),
            common: CommonOptions::default(),
        }
    }

    fn next(&mut self) -> Option<String> {
        self.values.pop_front()
    }

    fn value(&mut self, option: &str) -> Result<String, AppError> {
        self.next()
            .ok_or_else(|| AppError::Usage(format!("{option} requires a value")))
    }

    fn common(&mut self, option: &str) -> Result<bool, AppError> {
        match option {
            "--port" => {
                self.common.port = Some(self.value("--port")?);
                Ok(true)
            }
            "--json" => {
                self.common.json = true;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

pub fn run_if_requested() -> Result<bool, AppError> {
    let mut raw = env::args().skip(1);
    let Some(command) = raw.next() else {
        return Ok(false);
    };
    if !M2_COMMANDS.contains(&command.as_str()) {
        return Ok(false);
    }
    let mut arguments = Arguments::new(raw);
    match command.as_str() {
        "inspect-config" => inspect_config(&mut arguments)?,
        "apply-config" => apply_config(&mut arguments)?,
        "select-screen" => select_screen(&mut arguments)?,
        "push-clock" => push_clock(&mut arguments)?,
        "pomodoro" => run_pomodoro(&mut arguments)?,
        "trigger-interrupt" => trigger_interrupt(&mut arguments)?,
        "events" => print_events(&mut arguments)?,
        _ => unreachable!(),
    }
    Ok(true)
}

fn inspect_config(arguments: &mut Arguments) -> Result<(), AppError> {
    let path = parse_config_path(arguments)?;
    let config = load_config(&path)?;
    if arguments.common.json {
        println!(
            "{{\"ok\":true,\"widgets\":{},\"screens\":{},\"path\":{}}}",
            config.widgets.len(),
            config.screens.len(),
            json_string(&path.to_string_lossy())
        );
    } else {
        println!(
            "valid M2 layout: {} widgets, {} ordered screens ({})",
            config.widgets.len(),
            config.screens.len(),
            path.display()
        );
        for (index, screen) in config.screens.iter().enumerate() {
            println!(
                "  {}. {} -> {}",
                index + 1,
                screen.screen_id,
                screen.widget_id
            );
        }
    }
    Ok(())
}

fn apply_config(arguments: &mut Arguments) -> Result<(), AppError> {
    let path = parse_config_path(arguments)?;
    let config = load_config(&path)?;
    let connected = connect_session(arguments.common.port.as_deref())?;
    let ack =
        connected
            .session
            .apply_next_config(config.rotation, config.widgets, config.screens)?;
    let revision = ack.revision.ok_or(DeviceError::UnexpectedMessage)?;
    print_ok(
        arguments.common.json,
        &format!("applied config revision {revision}"),
        &format!("\"revision\":{revision}"),
    );
    Ok(())
}

fn select_screen(arguments: &mut Arguments) -> Result<(), AppError> {
    let mut screen = None;
    while let Some(option) = arguments.next() {
        if arguments.common(&option)? {
            continue;
        }
        match option.as_str() {
            "--screen" => screen = Some(arguments.value("--screen")?),
            _ => return unknown_option(&option),
        }
    }
    let screen = screen.ok_or_else(|| AppError::Usage("select-screen requires --screen".into()))?;
    let activation = ActivateScreen {
        screen_id: screen.clone(),
    };
    validate_request(&Message::ActivateScreen(activation.clone()))?;
    let connected = connect_session(arguments.common.port.as_deref())?;
    connected.session.activate_screen(activation)?;
    print_ok(
        arguments.common.json,
        &format!("selected screen {screen}"),
        &format!("\"screen_id\":{}", json_string(&screen)),
    );
    Ok(())
}

fn push_clock(arguments: &mut Arguments) -> Result<(), AppError> {
    let mut widget = None;
    let mut title = String::new();
    let mut show_seconds = true;
    while let Some(option) = arguments.next() {
        if arguments.common(&option)? {
            continue;
        }
        match option.as_str() {
            "--widget" => widget = Some(arguments.value("--widget")?),
            "--title" => title = arguments.value("--title")?,
            "--show-seconds" => {
                show_seconds = parse_bool(&arguments.value("--show-seconds")?)?;
            }
            _ => return unknown_option(&option),
        }
    }
    let widget = widget.ok_or_else(|| AppError::Usage("push-clock requires --widget".into()))?;
    let fields = vec![
        text_field("title", title),
        bool_field("show_seconds", show_seconds),
        bool_field("stale", false),
        text_field("error", String::new()),
    ];
    validate_push(&widget, &fields)?;
    let connected = connect_session(arguments.common.port.as_deref())?;
    let ack = connected.session.push_fields(widget.clone(), fields)?;
    let revision = ack.revision.ok_or(DeviceError::UnexpectedMessage)?;
    print_push(arguments.common.json, &widget, revision);
    Ok(())
}

fn trigger_interrupt(arguments: &mut Arguments) -> Result<(), AppError> {
    let mut widget = None;
    let mut token = None;
    let mut reason = None;
    while let Some(option) = arguments.next() {
        if arguments.common(&option)? {
            continue;
        }
        match option.as_str() {
            "--widget" => widget = Some(arguments.value("--widget")?),
            "--token" => token = Some(parse_nonzero_u32(&arguments.value("--token")?, "--token")?),
            "--reason" => reason = Some(arguments.value("--reason")?),
            _ => return unknown_option(&option),
        }
    }
    let interrupt = TriggerInterrupt {
        widget_id: widget
            .ok_or_else(|| AppError::Usage("trigger-interrupt requires --widget".into()))?,
        token: token.ok_or_else(|| AppError::Usage("trigger-interrupt requires --token".into()))?,
        reason: reason
            .ok_or_else(|| AppError::Usage("trigger-interrupt requires --reason".into()))?,
    };
    validate_request(&Message::TriggerInterrupt(interrupt.clone()))?;
    let connected = connect_session(arguments.common.port.as_deref())?;
    connected.session.trigger_interrupt(interrupt.clone())?;
    print_ok(
        arguments.common.json,
        &format!(
            "triggered {} interrupt token {}",
            interrupt.widget_id, interrupt.token
        ),
        &format!(
            "\"widget_id\":{},\"token\":{}",
            json_string(&interrupt.widget_id),
            interrupt.token
        ),
    );
    Ok(())
}

fn print_events(arguments: &mut Arguments) -> Result<(), AppError> {
    let mut count = 0_u64;
    while let Some(option) = arguments.next() {
        if arguments.common(&option)? {
            continue;
        }
        match option.as_str() {
            "--count" => {
                count = arguments
                    .value("--count")?
                    .parse()
                    .map_err(|_| AppError::Usage("--count must be a nonnegative integer".into()))?;
            }
            _ => return unknown_option(&option),
        }
    }
    let mut connected = connect_session(arguments.common.port.as_deref())?;
    let running = shutdown_flag()?;
    let mut received = 0_u64;
    while running.load(Ordering::Relaxed) {
        match connected.session.recv_event_timeout(Duration::from_secs(1)) {
            Ok(Some(event)) => {
                print_event(&event, arguments.common.json);
                received += 1;
                if count != 0 && received >= count {
                    return Ok(());
                }
            }
            Ok(None) => match connected.session.heartbeat() {
                Ok(_) => {}
                Err(DeviceError::Transport(_)) => reconnect_until_available(
                    &mut connected,
                    arguments.common.port.as_deref(),
                    arguments.common.json,
                    Some(&running),
                )?,
                Err(error) => return Err(error.into()),
            },
            Err(DeviceError::Transport(_)) => reconnect_until_available(
                &mut connected,
                arguments.common.port.as_deref(),
                arguments.common.json,
                Some(&running),
            )?,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn run_pomodoro(arguments: &mut Arguments) -> Result<(), AppError> {
    let (widget, label, duration_seconds) = parse_pomodoro_options(arguments)?;
    let mut timer = Pomodoro::new(label, duration_seconds)
        .map_err(|error| AppError::Config(error.to_string()))?;
    let initial = timer.update(Instant::now());
    validate_push(&widget, &initial.fields)?;
    let connected = connect_session(arguments.common.port.as_deref())?;
    connected
        .session
        .push_fields(widget.clone(), initial.fields)?;
    run_pomodoro_loop(
        connected,
        timer,
        &widget,
        arguments.common.port.as_deref(),
        arguments.common.json,
    )
}

fn run_pomodoro_loop(
    mut connected: ConnectedSession,
    mut timer: Pomodoro,
    widget: &str,
    explicit_port: Option<&str>,
    json: bool,
) -> Result<(), AppError> {
    let mut interrupts = InterruptArbiter::default();
    let mut retry_interrupt = None;
    let mut next_push = Instant::now();
    let running = shutdown_flag()?;
    if !json {
        println!("pomodoro ready; tap the device to start/pause, Ctrl-C to stop");
    }
    while running.load(Ordering::Relaxed) {
        let now = Instant::now();
        match connected
            .session
            .recv_event_timeout(Duration::from_millis(100))
        {
            Ok(Some(event)) => {
                print_event(&event, json);
                handle_pomodoro_event(&event, &mut timer, &mut interrupts, now);
            }
            Ok(None) => {}
            Err(DeviceError::Transport(_)) => {
                reconnect_until_available(&mut connected, explicit_port, json, Some(&running))?;
                continue;
            }
            Err(error) => return Err(error.into()),
        }
        if now >= next_push {
            let update = timer.update(now);
            if update.completion_interrupt {
                retry_interrupt = Some(
                    interrupts
                        .schedule(widget, "Timer finished")
                        .map_err(|error| AppError::Host(error.to_string()))?,
                );
            }
            match connected.session.push_fields(widget, update.fields) {
                Ok(_) => {}
                Err(DeviceError::Transport(_)) => {
                    reconnect_until_available(&mut connected, explicit_port, json, Some(&running))?;
                    continue;
                }
                Err(error) => return Err(error.into()),
            }
            next_push = now + Duration::from_secs(1);
        }
        send_pending_interrupt(
            &mut connected,
            explicit_port,
            json,
            &running,
            &mut interrupts,
            &mut retry_interrupt,
        )?;
    }
    Ok(())
}

fn parse_pomodoro_options(arguments: &mut Arguments) -> Result<(String, String, u32), AppError> {
    let mut widget = None;
    let mut label = "Pomodoro".to_owned();
    let mut duration_seconds = 25 * 60;
    while let Some(option) = arguments.next() {
        if arguments.common(&option)? {
            continue;
        }
        match option.as_str() {
            "--widget" => widget = Some(arguments.value("--widget")?),
            "--label" => label = arguments.value("--label")?,
            "--duration-seconds" => {
                duration_seconds = arguments
                    .value("--duration-seconds")?
                    .parse()
                    .map_err(|_| AppError::Config("invalid pomodoro duration".into()))?;
            }
            _ => return unknown_option(&option),
        }
    }
    let widget = widget.ok_or_else(|| AppError::Usage("pomodoro requires --widget".into()))?;
    Ok((widget, label, duration_seconds))
}

fn handle_pomodoro_event(
    event: &device::ReceivedEvent,
    timer: &mut Pomodoro,
    interrupts: &mut InterruptArbiter,
    now: Instant,
) {
    match (event.event.kind, event.event.action) {
        (EventKind::Tap, EventAction::StartPause) => {
            timer.toggle(now);
        }
        (EventKind::Tap, EventAction::Reset) => {
            timer.reset(now);
        }
        (EventKind::InterruptDismissed, EventAction::DismissInterrupt) => {
            if let Some(token) = event.event.interrupt_token {
                let _ = interrupts.dismiss(token);
            }
        }
        _ => {}
    }
}

fn send_pending_interrupt(
    connected: &mut ConnectedSession,
    explicit_port: Option<&str>,
    json: bool,
    running: &Arc<AtomicBool>,
    interrupts: &mut InterruptArbiter,
    retry_interrupt: &mut Option<TriggerInterrupt>,
) -> Result<(), AppError> {
    let Some(interrupt) = retry_interrupt.clone() else {
        return Ok(());
    };
    match connected.session.trigger_interrupt(interrupt.clone()) {
        Ok(_) => {
            interrupts
                .acknowledge(interrupt.token)
                .map_err(|error| AppError::Host(error.to_string()))?;
            *retry_interrupt = None;
        }
        Err(DeviceError::Rejected(error)) if error.code == ErrorCode::Busy => {
            *retry_interrupt = Some(
                interrupts
                    .mark_busy_for_retry(interrupt.token)
                    .map_err(|error| AppError::Host(error.to_string()))?,
            );
            thread::sleep(Duration::from_millis(250));
        }
        Err(DeviceError::Transport(_)) => {
            reconnect_until_available(connected, explicit_port, json, Some(running))?;
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn reconnect_until_available(
    connected: &mut ConnectedSession,
    explicit_port: Option<&str>,
    json: bool,
    running: Option<&AtomicBool>,
) -> Result<(), AppError> {
    loop {
        if running.is_some_and(|flag| !flag.load(Ordering::Relaxed)) {
            return Ok(());
        }
        match connected.reconnect(explicit_port) {
            Ok(()) => {
                if json {
                    println!(
                        "{{\"event\":\"reconnected\",\"port\":{}}}",
                        json_string(&connected.port_name)
                    );
                } else {
                    println!("reconnected on {}", connected.port_name);
                }
                return Ok(());
            }
            Err(DeviceError::NoDevice | DeviceError::Transport(_)) => {
                thread::sleep(Duration::from_secs(1));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn shutdown_flag() -> Result<Arc<AtomicBool>, AppError> {
    let running = Arc::new(AtomicBool::new(true));
    let handler_flag = Arc::clone(&running);
    ctrlc::set_handler(move || handler_flag.store(false, Ordering::Relaxed))
        .map_err(|error| AppError::Host(format!("cannot install Ctrl-C handler: {error}")))?;
    Ok(running)
}

fn parse_config_path(arguments: &mut Arguments) -> Result<PathBuf, AppError> {
    let mut path = None;
    while let Some(option) = arguments.next() {
        if arguments.common(&option)? {
            continue;
        }
        match option.as_str() {
            "--config" => path = Some(PathBuf::from(arguments.value("--config")?)),
            _ => return unknown_option(&option),
        }
    }
    path.ok_or_else(|| AppError::Usage("command requires --config".into()))
}

fn load_config(path: &PathBuf) -> Result<ApplyConfig, AppError> {
    let bytes = fs::read(path)
        .map_err(|error| AppError::Config(format!("cannot read {}: {error}", path.display())))?;
    if bytes.len() > 64 * 1024 {
        return Err(AppError::Config("config file exceeds 64 KiB".into()));
    }
    let layout: LayoutFile = serde_json::from_slice(&bytes)
        .map_err(|error| AppError::Config(format!("invalid config JSON: {error}")))?;
    let widgets = layout
        .widgets
        .into_iter()
        .map(|widget| {
            Ok(WidgetConfig {
                widget_id: widget.id,
                template: match widget.template.as_str() {
                    "digital-clock" => TemplateKind::DigitalClock,
                    "progress-ring" => TemplateKind::ProgressRing,
                    "row-list" => TemplateKind::RowList,
                    other => {
                        return Err(AppError::Config(format!("unknown template: {other}")));
                    }
                },
                size_class: match widget.size.as_str() {
                    "full" => SizeClass::Full,
                    "standard" => SizeClass::Standard,
                    "tile" => SizeClass::Tile,
                    other => return Err(AppError::Config(format!("unknown size: {other}"))),
                },
                tap_action: match widget.tap_action.as_str() {
                    "none" => TapAction::None,
                    "start-pause" => TapAction::StartPause,
                    "reset" => TapAction::Reset,
                    other => {
                        return Err(AppError::Config(format!("unknown tap action: {other}")));
                    }
                },
                interrupt_policy: if widget.interrupts {
                    InterruptPolicy::Enabled
                } else {
                    InterruptPolicy::Disabled
                },
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let screens = layout
        .screens
        .into_iter()
        .map(|screen| ScreenConfig {
            screen_id: screen.id,
            widget_id: screen.widget,
        })
        .collect();
    let config = ApplyConfig {
        revision: 1,
        rotation: 90,
        widgets,
        screens,
    };
    validate_request(&Message::ApplyConfig(config.clone()))?;
    Ok(config)
}

fn validate_push(widget_id: &str, fields: &[Field]) -> Result<(), AppError> {
    validate_request(&Message::PushData(PushData {
        widget_id: widget_id.into(),
        revision: 1,
        fields: fields.to_vec(),
    }))
}

fn validate_request(message: &Message) -> Result<(), AppError> {
    encode_message(1, message)
        .map(|_| ())
        .map_err(|error| AppError::Config(format!("invalid request: {error}")))
}

fn print_event(event: &device::ReceivedEvent, json: bool) {
    if json {
        println!(
            "{{\"event\":\"device\",\"sequence\":{},\"missed_before\":{},\"kind\":{},\"action\":{},\"widget_id\":{},\"screen_id\":{},\"interrupt_token\":{}}}",
            event.event.sequence,
            event.missed_before,
            json_string(event_kind(event.event.kind)),
            json_string(event_action(event.event.action)),
            json_string(&event.event.widget_id),
            json_string(&event.event.screen_id),
            event
                .event
                .interrupt_token
                .map_or_else(|| "null".into(), |token| token.to_string())
        );
    } else {
        println!(
            "event #{} {} {} widget={} screen={}{}{}",
            event.event.sequence,
            event_kind(event.event.kind),
            event_action(event.event.action),
            event.event.widget_id,
            event.event.screen_id,
            event
                .event
                .interrupt_token
                .map_or_else(String::new, |token| format!(" token={token}")),
            if event.missed_before == 0 {
                String::new()
            } else {
                format!(" missed_before={}", event.missed_before)
            }
        );
    }
}

fn event_kind(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Tap => "tap",
        EventKind::Navigation => "navigation",
        EventKind::InterruptDismissed => "interrupt-dismissed",
    }
}

fn event_action(action: EventAction) -> &'static str {
    match action {
        EventAction::StartPause => "start-pause",
        EventAction::Reset => "reset",
        EventAction::NavigatePrevious => "navigate-previous",
        EventAction::NavigateNext => "navigate-next",
        EventAction::DismissInterrupt => "dismiss-interrupt",
    }
}

fn print_ok(json: bool, human: &str, json_fields: &str) {
    if json {
        println!("{{\"ok\":true,{json_fields}}}");
    } else {
        println!("{human}");
    }
}

fn print_push(json: bool, widget: &str, revision: u32) {
    print_ok(
        json,
        &format!("pushed {widget} revision {revision}"),
        &format!(
            "\"widget_id\":{},\"revision\":{revision}",
            json_string(widget)
        ),
    );
}

fn text_field(key: &str, value: String) -> Field {
    Field {
        key: key.into(),
        value: FieldValue::Text(value),
    }
}

fn bool_field(key: &str, value: bool) -> Field {
    Field {
        key: key.into(),
        value: FieldValue::Boolean(value),
    }
}

fn parse_bool(raw: &str) -> Result<bool, AppError> {
    raw.parse()
        .map_err(|_| AppError::Usage(format!("invalid boolean: {raw}")))
}

fn parse_nonzero_u32(raw: &str, option: &str) -> Result<u32, AppError> {
    raw.parse::<u32>()
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| AppError::Usage(format!("{option} must be a nonzero u32")))
}

fn unknown_option<T>(option: &str) -> Result<T, AppError> {
    Err(AppError::Usage(format!("unknown option: {option}")))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn checked_sample_layout_exercises_all_templates_and_legacy_size_classes() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/m2-carousel.json");
        let config = load_config(&path).unwrap();
        assert_eq!(config.widgets.len(), 2);
        assert_eq!(config.screens.len(), 2);
        assert!(config.widgets.iter().any(|widget| {
            widget.template == TemplateKind::DigitalClock && widget.size_class == SizeClass::Full
        }));
        assert!(config.widgets.iter().any(|widget| {
            widget.template == TemplateKind::ProgressRing
                && widget.size_class == SizeClass::Standard
        }));
    }

    #[test]
    fn invalid_layout_is_rejected_before_any_device_connection() {
        let path = std::env::temp_dir().join(format!(
            "deskmate-invalid-layout-{}.json",
            std::process::id()
        ));
        fs::write(
            &path,
            r#"{"widgets":[{"id":"clock","template":"digital-clock","size":"tile","tap_action":"none","interrupts":false}],"screens":[{"id":"main","widget":"clock"}]}"#,
        )
        .unwrap();
        let result = load_config(&path);
        fs::remove_file(path).unwrap();
        assert!(matches!(result, Err(AppError::Config(_))));
    }
}
