use std::env;
use std::fmt;
use std::process::{self, Command as ProcessCommand};
use std::time::{SystemTime, UNIX_EPOCH};

use device::{DeviceError, connect};
use protocol::{
    Field, FieldValue, MAX_FIELD_COUNT, MAX_FIELD_KEY_LEN, MAX_FIELD_TEXT_LEN, MAX_WIDGET_ID_LEN,
    PushData, StatusResponse, TimeSync,
};

mod m2;

const USAGE: &str = "\
Usage:
  deskmate-cli status [--port PATH] [--json]
  deskmate-cli time-sync [--offset-minutes N] [--port PATH] [--json]
  deskmate-cli push-data --widget ID --revision N [--field KEY=VALUE]... [--port PATH] [--json]

Field values infer booleans and integers; use s:, i:, or b: to force a type.
Examples: --field summary=s:Clear --field temp=i:23 --field ok=b:true";

#[derive(Debug)]
pub(crate) enum AppError {
    Usage(String),
    Config(String),
    Provider(String),
    Device(DeviceError),
    Host(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(error)
            | Self::Config(error)
            | Self::Provider(error)
            | Self::Host(error) => f.write_str(error),
            Self::Device(error) => error.fmt(f),
        }
    }
}

impl From<DeviceError> for AppError {
    fn from(value: DeviceError) -> Self {
        Self::Device(value)
    }
}

#[derive(Debug)]
enum CliCommand {
    Status,
    TimeSync {
        offset_minutes: Option<i16>,
    },
    PushData {
        widget_id: String,
        revision: u32,
        fields: Vec<Field>,
    },
}

#[derive(Debug)]
struct Options {
    port: Option<String>,
    json: bool,
    command: CliCommand,
}

#[derive(Debug, Default)]
struct CommandOptions {
    offset_minutes: Option<i16>,
    widget_id: Option<String>,
    revision: Option<u32>,
    fields: Vec<Field>,
}

fn next_value(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<String, AppError> {
    arguments
        .next()
        .ok_or_else(|| AppError::Usage(format!("{option} requires a value")))
}

fn parse_field(raw: &str) -> Result<Field, AppError> {
    let (key, raw_value) = raw
        .split_once('=')
        .ok_or_else(|| AppError::Usage("--field must be KEY=VALUE".into()))?;
    if key.is_empty() || key.len() > MAX_FIELD_KEY_LEN {
        return Err(AppError::Usage(format!(
            "field key must be 1..={MAX_FIELD_KEY_LEN} UTF-8 bytes"
        )));
    }
    let value = if let Some(text) = raw_value.strip_prefix("s:") {
        if text.len() > MAX_FIELD_TEXT_LEN {
            return Err(AppError::Usage(format!(
                "field text must be at most {MAX_FIELD_TEXT_LEN} UTF-8 bytes"
            )));
        }
        FieldValue::Text(text.to_owned())
    } else if let Some(integer) = raw_value.strip_prefix("i:") {
        FieldValue::Integer(
            integer
                .parse()
                .map_err(|_| AppError::Usage(format!("invalid integer field: {raw}")))?,
        )
    } else if let Some(boolean) = raw_value.strip_prefix("b:") {
        FieldValue::Boolean(
            boolean
                .parse()
                .map_err(|_| AppError::Usage(format!("invalid boolean field: {raw}")))?,
        )
    } else if let Ok(boolean) = raw_value.parse::<bool>() {
        FieldValue::Boolean(boolean)
    } else if let Ok(integer) = raw_value.parse::<i64>() {
        FieldValue::Integer(integer)
    } else {
        if raw_value.len() > MAX_FIELD_TEXT_LEN {
            return Err(AppError::Usage(format!(
                "field text must be at most {MAX_FIELD_TEXT_LEN} UTF-8 bytes"
            )));
        }
        FieldValue::Text(raw_value.to_owned())
    };
    Ok(Field {
        key: key.to_owned(),
        value,
    })
}

fn build_command(command_name: &str, options: CommandOptions) -> Result<CliCommand, AppError> {
    Ok(match command_name {
        "status" => {
            if options.offset_minutes.is_some()
                || options.widget_id.is_some()
                || options.revision.is_some()
                || !options.fields.is_empty()
            {
                return Err(AppError::Usage(
                    "status does not accept time-sync or push-data options".into(),
                ));
            }
            CliCommand::Status
        }
        "time-sync" => {
            if options.widget_id.is_some()
                || options.revision.is_some()
                || !options.fields.is_empty()
            {
                return Err(AppError::Usage(
                    "time-sync does not accept push-data options".into(),
                ));
            }
            if options
                .offset_minutes
                .is_some_and(|offset| !(-840..=840).contains(&offset))
            {
                return Err(AppError::Usage(
                    "--offset-minutes must be in -840..=840".into(),
                ));
            }
            CliCommand::TimeSync {
                offset_minutes: options.offset_minutes,
            }
        }
        "push-data" => {
            if options.offset_minutes.is_some() {
                return Err(AppError::Usage(
                    "push-data does not accept --offset-minutes".into(),
                ));
            }
            let widget_id = options
                .widget_id
                .ok_or_else(|| AppError::Usage("push-data requires --widget".into()))?;
            if widget_id.is_empty() || widget_id.len() > MAX_WIDGET_ID_LEN {
                return Err(AppError::Usage(format!(
                    "widget ID must be 1..={MAX_WIDGET_ID_LEN} UTF-8 bytes"
                )));
            }
            let revision = options
                .revision
                .filter(|value| *value != 0)
                .ok_or_else(|| AppError::Usage("push-data requires nonzero --revision".into()))?;
            if options.fields.len() > MAX_FIELD_COUNT {
                return Err(AppError::Usage(format!(
                    "push-data accepts at most {MAX_FIELD_COUNT} fields"
                )));
            }
            for (index, field) in options.fields.iter().enumerate() {
                if options.fields[..index]
                    .iter()
                    .any(|previous| previous.key == field.key)
                {
                    return Err(AppError::Usage(format!(
                        "duplicate field key: {}",
                        field.key
                    )));
                }
            }
            CliCommand::PushData {
                widget_id,
                revision,
                fields: options.fields,
            }
        }
        _ => return Err(AppError::Usage(format!("unknown command: {command_name}"))),
    })
}

fn parse_options() -> Result<Options, AppError> {
    let mut arguments = env::args().skip(1);
    let command_name = arguments
        .next()
        .ok_or_else(|| AppError::Usage("missing command".into()))?;
    let mut port = None;
    let mut json = false;
    let mut command_options = CommandOptions::default();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--port" => port = Some(next_value(&mut arguments, "--port")?),
            "--json" => json = true,
            "--offset-minutes" => {
                let value = next_value(&mut arguments, "--offset-minutes")?;
                command_options.offset_minutes = Some(value.parse().map_err(|_| {
                    AppError::Usage("--offset-minutes must be a signed integer".into())
                })?);
            }
            "--widget" => {
                command_options.widget_id = Some(next_value(&mut arguments, "--widget")?);
            }
            "--revision" => {
                let value = next_value(&mut arguments, "--revision")?;
                command_options.revision = Some(
                    value
                        .parse()
                        .map_err(|_| AppError::Usage("--revision must be a nonzero u32".into()))?,
                );
            }
            "--field" => {
                let value = next_value(&mut arguments, "--field")?;
                command_options.fields.push(parse_field(&value)?);
            }
            _ => return Err(AppError::Usage(format!("unknown option: {argument}"))),
        }
    }
    let command = build_command(&command_name, command_options)?;
    Ok(Options {
        port,
        json,
        command,
    })
}

fn system_utc_offset_minutes() -> Result<i16, AppError> {
    let output = ProcessCommand::new("date")
        .arg("+%z")
        .output()
        .map_err(|error| AppError::Host(format!("cannot determine UTC offset: {error}")))?;
    if !output.status.success() {
        return Err(AppError::Host(
            "cannot determine UTC offset; pass --offset-minutes".into(),
        ));
    }
    let raw = String::from_utf8(output.stdout)
        .map_err(|_| AppError::Host("date returned a non-UTF-8 UTC offset".into()))?;
    let raw = raw.trim();
    if raw.len() != 5 || !matches!(raw.as_bytes()[0], b'+' | b'-') {
        return Err(AppError::Host(
            "cannot parse host UTC offset; pass --offset-minutes".into(),
        ));
    }
    let hours: i16 = raw[1..3]
        .parse()
        .map_err(|_| AppError::Host("invalid host UTC offset hours".into()))?;
    let minutes: i16 = raw[3..5]
        .parse()
        .map_err(|_| AppError::Host("invalid host UTC offset minutes".into()))?;
    if minutes >= 60 {
        return Err(AppError::Host("invalid host UTC offset".into()));
    }
    let total = hours * 60 + minutes;
    Ok(if raw.starts_with('-') { -total } else { total })
}

fn unix_seconds() -> Result<i64, AppError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| AppError::Host(format!("system clock precedes Unix epoch: {error}")))?;
    i64::try_from(duration.as_secs())
        .map_err(|_| AppError::Host("system time does not fit protocol seconds".into()))
}

fn json_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(output, "\\u{:04x}", u32::from(character));
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

fn print_status(status: &StatusResponse, port_name: &str, json: bool) {
    if json {
        println!(
            "{{\"port\":{},\"protocol_version\":{},\"max_protocol_version\":{},\"capabilities\":{},\"firmware_version\":{},\"uptime_ms\":{},\"free_heap\":{},\"display_width\":{},\"display_height\":{},\"brightness\":{},\"rotation\":{},\"online\":{},\"latest_revision\":{},\"config_revision\":{},\"latest_interrupt_token\":{},\"valid_frames\":{},\"malformed_frames\":{},\"crc_errors\":{},\"overflow_frames\":{},\"dropped_responses\":{},\"rx_dropped_bytes\":{},\"dropped_events\":{},\"event_queue_high_water\":{},\"dropped_ui_commands\":{},\"ui_queue_high_water\":{}}}",
            json_string(port_name),
            status.protocol_version,
            status.max_protocol_version,
            status.capabilities,
            json_string(&status.firmware_version),
            status.uptime_ms,
            status.free_heap,
            status.display_width,
            status.display_height,
            status.brightness,
            status.rotation,
            status.online,
            status.latest_revision,
            status.config_revision,
            status.latest_interrupt_token,
            status.valid_frames,
            status.malformed_frames,
            status.crc_errors,
            status.overflow_frames,
            status.dropped_responses,
            status.rx_dropped_bytes,
            status.dropped_events,
            status.event_queue_high_water,
            status.dropped_ui_commands,
            status.ui_queue_high_water
        );
    } else {
        println!("Deskmate on {port_name}");
        println!(
            "firmware {} / protocol v{} (max v{}, capabilities 0x{:016x})",
            status.firmware_version,
            status.protocol_version,
            status.max_protocol_version,
            status.capabilities
        );
        println!(
            "display {}x{}, brightness {}, rotation {}°",
            status.display_width, status.display_height, status.brightness, status.rotation
        );
        println!(
            "uptime {} ms, free heap {} bytes, link {}, latest revision {}",
            status.uptime_ms,
            status.free_heap,
            if status.online {
                "online"
            } else {
                "standalone"
            },
            status.latest_revision
        );
        println!(
            "frames valid={} malformed={} crc={} overflow={} dropped_responses={} rx_drops={}",
            status.valid_frames,
            status.malformed_frames,
            status.crc_errors,
            status.overflow_frames,
            status.dropped_responses,
            status.rx_dropped_bytes
        );
        println!(
            "config revision={}, latest interrupt token={}, events dropped={} high_water={}, UI commands dropped={} high_water={}",
            status.config_revision,
            status.latest_interrupt_token,
            status.dropped_events,
            status.event_queue_high_water,
            status.dropped_ui_commands,
            status.ui_queue_high_water
        );
    }
}

fn run() -> Result<(), AppError> {
    if m2::run_if_requested()? {
        return Ok(());
    }
    let options = parse_options()?;
    let mut connected = connect(options.port.as_deref())?;
    match options.command {
        CliCommand::Status => {
            print_status(
                &connected.initial_status,
                &connected.port_name,
                options.json,
            );
        }
        CliCommand::TimeSync { offset_minutes } => {
            let offset_minutes = match offset_minutes {
                Some(offset) => offset,
                None => system_utc_offset_minutes()?,
            };
            let seconds = unix_seconds()?;
            connected.client.time_sync(TimeSync {
                unix_seconds: seconds,
                utc_offset_minutes: offset_minutes,
            })?;
            if options.json {
                println!(
                    "{{\"ok\":true,\"unix_seconds\":{seconds},\"utc_offset_minutes\":{offset_minutes}}}"
                );
            } else {
                println!("time synchronized: unix={seconds}, UTC offset={offset_minutes} minutes");
            }
        }
        CliCommand::PushData {
            widget_id,
            revision,
            fields,
        } => {
            let ack = connected.client.push_data(PushData {
                widget_id: widget_id.clone(),
                revision,
                fields,
            })?;
            let accepted_revision = ack.revision.ok_or(DeviceError::UnexpectedMessage)?;
            if options.json {
                println!(
                    "{{\"ok\":true,\"widget_id\":{},\"revision\":{accepted_revision}}}",
                    json_string(&widget_id)
                );
            } else {
                println!("pushed {widget_id} revision {accepted_revision}");
            }
        }
    }
    Ok(())
}

fn exit_code(error: &AppError) -> i32 {
    match error {
        AppError::Usage(_) => 2,
        AppError::Config(_) => 3,
        AppError::Provider(_) => 4,
        AppError::Device(DeviceError::NoDevice) => 10,
        AppError::Device(DeviceError::Timeout) => 11,
        AppError::Device(DeviceError::VersionMismatch(_)) => 12,
        AppError::Device(DeviceError::Rejected(_)) => 13,
        AppError::Device(
            DeviceError::MalformedResponse(_)
            | DeviceError::UnexpectedRequestId { .. }
            | DeviceError::UnexpectedMessage,
        ) => 14,
        AppError::Host(_) | AppError::Device(_) => 15,
    }
}

fn main() {
    if matches!(
        env::args().nth(1).as_deref(),
        Some("help" | "--help" | "-h")
    ) {
        println!("{USAGE}\n\n{}", m2::M2_USAGE);
        return;
    }
    let json = env::args().any(|argument| argument == "--json");
    if let Err(error) = run() {
        if json {
            eprintln!("{{\"error\":{}}}", json_string(&error.to_string()));
        } else if matches!(error, AppError::Usage(_)) {
            if !error.to_string().is_empty() {
                eprintln!("error: {error}\n");
            }
            eprintln!("{USAGE}\n\n{}", m2::M2_USAGE);
        } else {
            eprintln!("error: {error}");
        }
        process::exit(exit_code(&error));
    }
}
