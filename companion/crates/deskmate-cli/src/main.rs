use std::env;
use std::fmt;
use std::process::{self, Command as ProcessCommand};
use std::time::{SystemTime, UNIX_EPOCH};

use device::{DeviceError, connect, connect_session};
use protocol::{
    Field, FieldValue, MAX_DEVICE_ID_LEN, MAX_DEVICE_TOKEN_LEN, MAX_FIELD_COUNT, MAX_FIELD_KEY_LEN,
    MAX_FIELD_TEXT_LEN, MAX_PSK_LEN, MAX_SERVER_URL_LEN, MAX_SSID_LEN, MAX_WIDGET_ID_LEN,
    NetworkConfig, OtaState, PushData, StatusResponse, Tier, TimeSync, WifiState,
};

const USAGE: &str = "\
Usage:
  deskmate-cli status [--port PATH] [--json]
  deskmate-cli time-sync [--offset-minutes N] [--port PATH] [--json]
  deskmate-cli push-data --widget ID --revision N [--field KEY=VALUE]... [--port PATH] [--json]
  deskmate-cli provision [--ssid SSID] [--psk PSK] [--server-url URL] \\
      [--device-id ID] [--token TOKEN] [--offset-minutes N] [--tier local|networked] \\
      [--port PATH] [--json]
  deskmate-cli factory-reset [--port PATH] [--json]

Field values infer booleans and integers; use s:, i:, or b: to force a type.
Examples: --field summary=s:Clear --field temp=i:23 --field ok=b:true

provision persists over the cable and takes effect on the device's next boot,
not live.";

#[derive(Debug)]
pub(crate) enum AppError {
    Usage(String),
    Device(DeviceError),
    Host(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(error) | Self::Host(error) => f.write_str(error),
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
    Provision {
        config: NetworkConfig,
    },
    FactoryReset,
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
    ssid: Option<String>,
    psk: Option<String>,
    server_url: Option<String>,
    device_id: Option<String>,
    token: Option<String>,
    tier: Option<Tier>,
}

impl CommandOptions {
    fn has_provisioning_options(&self) -> bool {
        self.ssid.is_some()
            || self.psk.is_some()
            || self.server_url.is_some()
            || self.device_id.is_some()
            || self.token.is_some()
            || self.tier.is_some()
    }
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

fn parse_tier(value: &str) -> Result<Tier, AppError> {
    match value {
        "local" => Ok(Tier::Local),
        "networked" => Ok(Tier::Networked),
        other => Err(AppError::Usage(format!(
            "--tier must be local or networked, got: {other}"
        ))),
    }
}

fn build_command(command_name: &str, options: CommandOptions) -> Result<CliCommand, AppError> {
    Ok(match command_name {
        "status" => {
            if options.offset_minutes.is_some()
                || options.widget_id.is_some()
                || options.revision.is_some()
                || !options.fields.is_empty()
                || options.has_provisioning_options()
            {
                return Err(AppError::Usage(
                    "status does not accept time-sync, push-data or provision options".into(),
                ));
            }
            CliCommand::Status
        }
        "time-sync" => {
            if options.widget_id.is_some()
                || options.revision.is_some()
                || !options.fields.is_empty()
                || options.has_provisioning_options()
            {
                return Err(AppError::Usage(
                    "time-sync does not accept push-data or provision options".into(),
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
        "push-data" => build_push_data(options)?,
        "provision" => build_provision(options)?,
        "factory-reset" => {
            if options.offset_minutes.is_some()
                || options.widget_id.is_some()
                || options.revision.is_some()
                || !options.fields.is_empty()
                || options.has_provisioning_options()
            {
                return Err(AppError::Usage(
                    "factory-reset does not accept any command options".into(),
                ));
            }
            CliCommand::FactoryReset
        }
        _ => return Err(AppError::Usage(format!("unknown command: {command_name}"))),
    })
}

fn build_push_data(options: CommandOptions) -> Result<CliCommand, AppError> {
    if options.offset_minutes.is_some() || options.has_provisioning_options() {
        return Err(AppError::Usage(
            "push-data does not accept --offset-minutes or provision options".into(),
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
    Ok(CliCommand::PushData {
        widget_id,
        revision,
        fields: options.fields,
    })
}

fn checked_provision_text(
    value: Option<String>,
    max_len: usize,
    option: &str,
) -> Result<String, AppError> {
    let value = value.unwrap_or_default();
    if value.len() > max_len {
        return Err(AppError::Usage(format!(
            "{option} must be at most {max_len} UTF-8 bytes"
        )));
    }
    Ok(value)
}

fn build_provision(options: CommandOptions) -> Result<CliCommand, AppError> {
    if options.widget_id.is_some() || options.revision.is_some() || !options.fields.is_empty() {
        return Err(AppError::Usage(
            "provision does not accept push-data options".into(),
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
    let ssid = checked_provision_text(options.ssid, MAX_SSID_LEN, "--ssid")?;
    let psk = checked_provision_text(options.psk, MAX_PSK_LEN, "--psk")?;
    let server_url =
        checked_provision_text(options.server_url, MAX_SERVER_URL_LEN, "--server-url")?;
    let device_id = checked_provision_text(options.device_id, MAX_DEVICE_ID_LEN, "--device-id")?;
    let token = checked_provision_text(options.token, MAX_DEVICE_TOKEN_LEN, "--token")?;
    let tier = options.tier.unwrap_or(Tier::Local);
    let utc_offset_minutes = match options.offset_minutes {
        Some(offset) => offset,
        None => system_utc_offset_minutes()?,
    };
    Ok(CliCommand::Provision {
        config: NetworkConfig {
            ssid,
            psk,
            server_url,
            device_id,
            token,
            utc_offset_minutes,
            tier,
        },
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
            "--ssid" => command_options.ssid = Some(next_value(&mut arguments, "--ssid")?),
            "--psk" => command_options.psk = Some(next_value(&mut arguments, "--psk")?),
            "--server-url" => {
                command_options.server_url = Some(next_value(&mut arguments, "--server-url")?);
            }
            "--device-id" => {
                command_options.device_id = Some(next_value(&mut arguments, "--device-id")?);
            }
            "--token" => command_options.token = Some(next_value(&mut arguments, "--token")?),
            "--tier" => {
                let value = next_value(&mut arguments, "--tier")?;
                command_options.tier = Some(parse_tier(&value)?);
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
            "{{\"port\":{},\"protocol_version\":{},\"max_protocol_version\":{},\"capabilities\":{},\"firmware_version\":{},\"uptime_ms\":{},\"free_heap\":{},\"display_width\":{},\"display_height\":{},\"brightness\":{},\"rotation\":{},\"online\":{},\"latest_revision\":{},\"config_revision\":{},\"latest_interrupt_token\":{},\"valid_frames\":{},\"malformed_frames\":{},\"crc_errors\":{},\"overflow_frames\":{},\"dropped_responses\":{},\"rx_dropped_bytes\":{},\"dropped_events\":{},\"event_queue_high_water\":{},\"dropped_ui_commands\":{},\"ui_queue_high_water\":{},\"tier\":{},\"wifi_state\":{},\"wifi_rssi\":{},\"ip\":{},\"ota_state\":{}}}",
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
            status.ui_queue_high_water,
            json_string(tier_name(status.tier)),
            json_string(wifi_state_name(status.wifi_state)),
            status.wifi_rssi,
            json_string(&status.ip),
            json_string(ota_state_name(status.ota_state))
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
            "display {}x{}, brightness {}, rotation {}°, tier {}",
            status.display_width,
            status.display_height,
            status.brightness,
            status.rotation,
            tier_name(status.tier)
        );
        println!(
            "wifi {} (rssi {} dBm, ip {}), ota {}",
            wifi_state_name(status.wifi_state),
            status.wifi_rssi,
            status.ip,
            ota_state_name(status.ota_state)
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
    let options = parse_options()?;
    let port = options.port.as_deref();
    let json = options.json;
    match options.command {
        CliCommand::Status => {
            let connected = connect(port)?;
            print_status(&connected.initial_status, &connected.port_name, json);
        }
        CliCommand::TimeSync { offset_minutes } => {
            let mut connected = connect(port)?;
            let offset_minutes = match offset_minutes {
                Some(offset) => offset,
                None => system_utc_offset_minutes()?,
            };
            let seconds = unix_seconds()?;
            connected.client.time_sync(TimeSync {
                unix_seconds: seconds,
                utc_offset_minutes: offset_minutes,
            })?;
            if json {
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
            let mut connected = connect(port)?;
            let ack = connected.client.push_data(PushData {
                widget_id: widget_id.clone(),
                revision,
                fields,
            })?;
            let accepted_revision = ack.revision.ok_or(DeviceError::UnexpectedMessage)?;
            if json {
                println!(
                    "{{\"ok\":true,\"widget_id\":{},\"revision\":{accepted_revision}}}",
                    json_string(&widget_id)
                );
            } else {
                println!("pushed {widget_id} revision {accepted_revision}");
            }
        }
        CliCommand::Provision { config } => {
            // A dedicated session (not the lighter DeviceClient used above)
            // because DeviceSession::provision is where this request lives;
            // a one-shot session for a single command mirrors the M2
            // commands in m2.rs (e.g. apply-config).
            let connected = connect_session(port)?;
            connected.session.provision(&config)?;
            if json {
                println!(
                    "{{\"ok\":true,\"ssid\":{},\"server_url\":{},\"device_id\":{},\"tier\":{}}}",
                    json_string(&config.ssid),
                    json_string(&config.server_url),
                    json_string(&config.device_id),
                    json_string(tier_name(config.tier))
                );
            } else {
                println!(
                    "provisioned: ssid={} server_url={} device_id={} tier={} (takes effect on next boot)",
                    config.ssid,
                    config.server_url,
                    config.device_id,
                    tier_name(config.tier)
                );
            }
        }
        CliCommand::FactoryReset => {
            let connected = connect_session(port)?;
            connected.session.factory_reset()?;
            if json {
                println!("{{\"ok\":true}}");
            } else {
                println!("factory reset: network config erased (takes effect on next boot)");
            }
        }
    }
    Ok(())
}

fn tier_name(tier: Tier) -> &'static str {
    match tier {
        Tier::Local => "local",
        Tier::Networked => "networked",
    }
}

fn wifi_state_name(state: WifiState) -> &'static str {
    match state {
        WifiState::Down => "down",
        WifiState::Connecting => "connecting",
        WifiState::Connected => "connected",
        WifiState::Failed => "failed",
    }
}

fn ota_state_name(state: OtaState) -> &'static str {
    match state {
        OtaState::Idle => "idle",
        OtaState::Checking => "checking",
        OtaState::Downloading => "downloading",
        OtaState::PendingVerify => "pending_verify",
        OtaState::Failed => "failed",
    }
}

fn exit_code(error: &AppError) -> i32 {
    match error {
        AppError::Usage(_) => 2,
        AppError::Device(DeviceError::NoDevice) => 10,
        AppError::Device(DeviceError::Timeout) => 11,
        AppError::Device(DeviceError::VersionMismatch(_)) => 12,
        AppError::Device(DeviceError::Rejected(_)) => 13,
        AppError::Device(DeviceError::MalformedResponse(_) | DeviceError::UnexpectedMessage) => 14,
        AppError::Host(_) | AppError::Device(_) => 15,
    }
}

fn main() {
    if matches!(
        env::args().nth(1).as_deref(),
        Some("help" | "--help" | "-h")
    ) {
        println!("{USAGE}");
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
            eprintln!("{USAGE}");
        } else {
            eprintln!("error: {error}");
        }
        process::exit(exit_code(&error));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_option_parses_local_and_networked() {
        assert_eq!(parse_tier("local").unwrap(), Tier::Local);
        assert_eq!(parse_tier("networked").unwrap(), Tier::Networked);
    }

    #[test]
    fn tier_option_rejects_an_invalid_value() {
        assert!(matches!(parse_tier("bogus"), Err(AppError::Usage(_))));
    }

    #[test]
    fn wifi_state_name_maps_every_variant() {
        assert_eq!(wifi_state_name(WifiState::Down), "down");
        assert_eq!(wifi_state_name(WifiState::Connecting), "connecting");
        assert_eq!(wifi_state_name(WifiState::Connected), "connected");
        assert_eq!(wifi_state_name(WifiState::Failed), "failed");
    }

    #[test]
    fn ota_state_name_maps_every_variant() {
        assert_eq!(ota_state_name(OtaState::Idle), "idle");
        assert_eq!(ota_state_name(OtaState::Checking), "checking");
        assert_eq!(ota_state_name(OtaState::Downloading), "downloading");
        assert_eq!(ota_state_name(OtaState::PendingVerify), "pending_verify");
        assert_eq!(ota_state_name(OtaState::Failed), "failed");
    }

    #[test]
    fn provision_rejects_an_over_length_token() {
        let options = CommandOptions {
            token: Some("t".repeat(MAX_DEVICE_TOKEN_LEN + 1)),
            offset_minutes: Some(0),
            ..CommandOptions::default()
        };
        assert!(matches!(build_provision(options), Err(AppError::Usage(_))));
    }

    #[test]
    fn provision_accepts_a_minimal_local_config() {
        let options = CommandOptions {
            offset_minutes: Some(0),
            ..CommandOptions::default()
        };
        let command = build_provision(options).unwrap();
        let CliCommand::Provision { config } = command else {
            panic!("expected CliCommand::Provision");
        };
        assert_eq!(config.tier, Tier::Local);
        assert_eq!(config.ssid, "");
        assert_eq!(config.utc_offset_minutes, 0);
    }

    #[test]
    fn provision_rejects_push_data_options() {
        let options = CommandOptions {
            widget_id: Some("clock".into()),
            offset_minutes: Some(0),
            ..CommandOptions::default()
        };
        assert!(matches!(build_provision(options), Err(AppError::Usage(_))));
    }
}
