use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use protocol::{
    Ack, ActivateScreen, ApplyConfig, CURRENT_CAPABILITIES, DeviceEvent, ErrorCode, ErrorResponse,
    EventAction, EventKind, Field, FieldValue, Frame, HeartbeatAck, InterruptPolicy,
    MAX_CONFIG_SCREENS, MAX_CONFIG_WIDGETS, MAX_PAYLOAD_SIZE, MAX_PROTOCOL_VERSION, MAX_WIRE_FRAME,
    Message, PushData, ScreenConfig, SizeClass, StatusResponse, TapAction, TemplateKind, TimeSync,
    TriggerInterrupt, WidgetConfig, encode_message,
};

fn minimum_config() -> ApplyConfig {
    ApplyConfig {
        revision: 1,
        rotation: 90,
        widgets: vec![WidgetConfig {
            widget_id: "clock".into(),
            template: TemplateKind::DigitalClock,
            size_class: SizeClass::Full,
            tap_action: TapAction::None,
            interrupt_policy: InterruptPolicy::Disabled,
        }],
        screens: vec![ScreenConfig {
            screen_id: "home".into(),
            widget_id: "clock".into(),
        }],
    }
}

fn bounded_id(prefix: &str, index: usize) -> String {
    let start = format!("{prefix}-{index}-");
    format!("{start}{}", "x".repeat(32 - start.len()))
}

fn maximum_config() -> ApplyConfig {
    let widgets = (0..MAX_CONFIG_WIDGETS)
        .map(|index| {
            let template = match index % 3 {
                0 => TemplateKind::DigitalClock,
                1 => TemplateKind::ProgressRing,
                _ => TemplateKind::RowList,
            };
            WidgetConfig {
                widget_id: bounded_id("widget", index),
                template,
                size_class: if index % 2 == 0 {
                    SizeClass::Full
                } else {
                    SizeClass::Standard
                },
                tap_action: if template == TemplateKind::ProgressRing {
                    TapAction::StartPause
                } else {
                    TapAction::None
                },
                interrupt_policy: InterruptPolicy::Enabled,
            }
        })
        .collect::<Vec<_>>();
    let screens = (0..MAX_CONFIG_SCREENS)
        .map(|index| ScreenConfig {
            screen_id: bounded_id("screen", index),
            widget_id: widgets[index].widget_id.clone(),
        })
        .collect();
    ApplyConfig {
        revision: u32::MAX,
        rotation: 270,
        widgets,
        screens,
    }
}

fn cbor_argument(bytes: &mut Vec<u8>, major: u8, value: u64) {
    let prefix = major << 5;
    match value {
        0..=23 => bytes.push(prefix | u8::try_from(value).unwrap()),
        24..=0xff => {
            bytes.push(prefix | 0x18);
            bytes.push(u8::try_from(value).unwrap());
        }
        0x100..=0xffff => {
            bytes.push(prefix | 0x19);
            bytes.extend_from_slice(&u16::try_from(value).unwrap().to_be_bytes());
        }
        _ => {
            bytes.push(prefix | 0x1a);
            bytes.extend_from_slice(&u32::try_from(value).unwrap().to_be_bytes());
        }
    }
}

fn cbor_unsigned(bytes: &mut Vec<u8>, value: u64) {
    cbor_argument(bytes, 0, value);
}

fn cbor_text(bytes: &mut Vec<u8>, value: &str) {
    cbor_argument(bytes, 3, u64::try_from(value.len()).unwrap());
    bytes.extend_from_slice(value.as_bytes());
}

fn raw_config_payload(widgets: &[(&str, u8, u8, u8, u8)], screens: &[(&str, &str)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    cbor_argument(&mut bytes, 5, 3);
    cbor_unsigned(&mut bytes, 0);
    cbor_unsigned(&mut bytes, 1);
    cbor_unsigned(&mut bytes, 1);
    cbor_argument(&mut bytes, 4, u64::try_from(widgets.len()).unwrap());
    for &(widget_id, template, size, action, policy) in widgets {
        cbor_argument(&mut bytes, 5, 5);
        cbor_unsigned(&mut bytes, 0);
        cbor_text(&mut bytes, widget_id);
        cbor_unsigned(&mut bytes, 1);
        cbor_unsigned(&mut bytes, u64::from(template));
        cbor_unsigned(&mut bytes, 2);
        cbor_unsigned(&mut bytes, u64::from(size));
        cbor_unsigned(&mut bytes, 3);
        cbor_unsigned(&mut bytes, u64::from(action));
        cbor_unsigned(&mut bytes, 4);
        cbor_unsigned(&mut bytes, u64::from(policy));
    }
    cbor_unsigned(&mut bytes, 2);
    cbor_argument(&mut bytes, 4, u64::try_from(screens.len()).unwrap());
    for &(screen_id, widget_id) in screens {
        cbor_argument(&mut bytes, 5, 2);
        cbor_unsigned(&mut bytes, 0);
        cbor_text(&mut bytes, screen_id);
        cbor_unsigned(&mut bytes, 1);
        cbor_text(&mut bytes, widget_id);
    }
    bytes
}

fn raw_duplicate_field_payload() -> Vec<u8> {
    let mut bytes = Vec::new();
    cbor_argument(&mut bytes, 5, 3);
    cbor_unsigned(&mut bytes, 0);
    cbor_text(&mut bytes, "clock");
    cbor_unsigned(&mut bytes, 1);
    cbor_unsigned(&mut bytes, 9);
    cbor_unsigned(&mut bytes, 2);
    cbor_argument(&mut bytes, 5, 2);
    cbor_text(&mut bytes, "stale");
    bytes.push(0xf4);
    cbor_text(&mut bytes, "stale");
    bytes.push(0xf5);
    bytes
}

fn write_raw_frame(
    output: &std::path::Path,
    name: &str,
    message_type: u8,
    request_id: u32,
    payload: Vec<u8>,
) -> Result<(), Box<dyn std::error::Error>> {
    let wire = protocol::encode_frame(&Frame::new(message_type, request_id, payload))?;
    fs::write(output.join(name), wire)?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn fixture_messages() -> Vec<(&'static str, u32, Message)> {
    vec![
        ("status_request.bin", 1, Message::StatusRequest),
        (
            "status_response.bin",
            1,
            Message::StatusResponse(StatusResponse {
                protocol_version: 1,
                max_protocol_version: MAX_PROTOCOL_VERSION,
                capabilities: CURRENT_CAPABILITIES,
                firmware_version: "deskmate-m1".into(),
                uptime_ms: 123_456,
                free_heap: 654_321,
                display_width: 368,
                display_height: 448,
                brightness: 200,
                rotation: 0,
                online: true,
                latest_revision: 7,
                valid_frames: 10,
                malformed_frames: 2,
                crc_errors: 1,
                overflow_frames: 1,
                dropped_responses: 0,
                rx_dropped_bytes: 0,
                dropped_events: 3,
                event_queue_high_water: 7,
                dropped_ui_commands: 2,
                ui_queue_high_water: 6,
                config_revision: 4,
                latest_interrupt_token: 5,
            }),
        ),
        (
            "time_sync.bin",
            2,
            Message::TimeSync(TimeSync {
                unix_seconds: 1_775_217_600,
                utc_offset_minutes: 240,
            }),
        ),
        (
            "ack_time.bin",
            2,
            Message::Ack(Ack {
                acknowledged_type: 3,
                revision: None,
            }),
        ),
        (
            "push_data.bin",
            3,
            Message::PushData(PushData {
                widget_id: "weather".into(),
                revision: 7,
                fields: vec![
                    Field {
                        key: "ok".into(),
                        value: FieldValue::Boolean(true),
                    },
                    Field {
                        key: "temp".into(),
                        value: FieldValue::Integer(23),
                    },
                    Field {
                        key: "summary".into(),
                        value: FieldValue::Text("Clear".into()),
                    },
                ],
            }),
        ),
        (
            "ack_push.bin",
            3,
            Message::Ack(Ack {
                acknowledged_type: 5,
                revision: Some(7),
            }),
        ),
        ("heartbeat.bin", 4, Message::Heartbeat),
        (
            "heartbeat_ack.bin",
            4,
            Message::HeartbeatAck(HeartbeatAck { uptime_ms: 123_456 }),
        ),
        (
            "error.bin",
            5,
            Message::Error(ErrorResponse {
                code: ErrorCode::StaleRevision,
                diagnostic: "stale revision".into(),
            }),
        ),
        (
            "apply_config_min.bin",
            10,
            Message::ApplyConfig(minimum_config()),
        ),
        (
            "ack_config.bin",
            10,
            Message::Ack(Ack {
                acknowledged_type: 9,
                revision: Some(1),
            }),
        ),
        (
            "apply_config_max.bin",
            11,
            Message::ApplyConfig(maximum_config()),
        ),
        (
            "activate_screen.bin",
            12,
            Message::ActivateScreen(ActivateScreen {
                screen_id: "home".into(),
            }),
        ),
        (
            "ack_activate.bin",
            12,
            Message::Ack(Ack {
                acknowledged_type: 10,
                revision: None,
            }),
        ),
        (
            "trigger_interrupt.bin",
            13,
            Message::TriggerInterrupt(TriggerInterrupt {
                widget_id: "timer".into(),
                token: 42,
                reason: "Pomodoro complete".into(),
            }),
        ),
        (
            "ack_interrupt.bin",
            13,
            Message::Ack(Ack {
                acknowledged_type: 11,
                revision: None,
            }),
        ),
        (
            "device_event_tap.bin",
            0,
            Message::DeviceEvent(DeviceEvent {
                sequence: 1,
                kind: EventKind::Tap,
                widget_id: "timer".into(),
                screen_id: "focus".into(),
                action: EventAction::StartPause,
                interrupt_token: None,
            }),
        ),
        (
            "device_event_previous.bin",
            0,
            Message::DeviceEvent(DeviceEvent {
                sequence: 2,
                kind: EventKind::Navigation,
                widget_id: "clock".into(),
                screen_id: "home".into(),
                action: EventAction::NavigatePrevious,
                interrupt_token: None,
            }),
        ),
        (
            "device_event_next.bin",
            0,
            Message::DeviceEvent(DeviceEvent {
                sequence: 3,
                kind: EventKind::Navigation,
                widget_id: "timer".into(),
                screen_id: "focus".into(),
                action: EventAction::NavigateNext,
                interrupt_token: None,
            }),
        ),
        (
            "device_event_dismissed.bin",
            0,
            Message::DeviceEvent(DeviceEvent {
                sequence: 4,
                kind: EventKind::InterruptDismissed,
                widget_id: "timer".into(),
                screen_id: "focus".into(),
                action: EventAction::DismissInterrupt,
                interrupt_token: Some(42),
            }),
        ),
        (
            "error_unknown_widget.bin",
            14,
            Message::Error(ErrorResponse {
                code: ErrorCode::UnknownWidget,
                diagnostic: "unknown widget".into(),
            }),
        ),
        (
            "error_unknown_screen.bin",
            15,
            Message::Error(ErrorResponse {
                code: ErrorCode::UnknownScreen,
                diagnostic: "unknown screen".into(),
            }),
        ),
        (
            "error_unsupported_template.bin",
            16,
            Message::Error(ErrorResponse {
                code: ErrorCode::UnsupportedTemplate,
                diagnostic: "unsupported template".into(),
            }),
        ),
        (
            "error_unsupported_size.bin",
            17,
            Message::Error(ErrorResponse {
                code: ErrorCode::UnsupportedSizeClass,
                diagnostic: "unsupported size class".into(),
            }),
        ),
        (
            "error_config_too_large.bin",
            18,
            Message::Error(ErrorResponse {
                code: ErrorCode::ConfigTooLarge,
                diagnostic: "config too large".into(),
            }),
        ),
        (
            "push_unknown_field.bin",
            19,
            Message::PushData(PushData {
                widget_id: "clock".into(),
                revision: 8,
                fields: vec![Field {
                    key: "future_field".into(),
                    value: FieldValue::Boolean(true),
                }],
            }),
        ),
    ]
}

#[allow(clippy::too_many_lines)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: generate-fixtures OUTPUT_DIRECTORY")?;
    fs::create_dir_all(&output)?;

    let messages = fixture_messages();
    let mut manifest =
        String::from("Deskmate protocol v1 deterministic fixtures\n\nValid frames:\n");
    let mut maximum_config_payload = None;
    for (name, request_id, message) in &messages {
        let wire = encode_message(*request_id, message)?;
        let payload_length = protocol::decode_wire_frame(&wire)?.payload.len();
        if *name == "apply_config_max.bin" {
            maximum_config_payload = Some(payload_length);
        }
        fs::write(output.join(name), &wire)?;
        writeln!(
            manifest,
            "- {name}: request_id={request_id}, type={}, payload={payload_length}, wire={}",
            message.type_id(),
            hex(&wire)
        )?;
    }
    let maximum_config_payload = maximum_config_payload.ok_or("missing maximum config fixture")?;
    assert!(maximum_config_payload <= MAX_PAYLOAD_SIZE);
    writeln!(
        manifest,
        "\nMaximum config payload: {maximum_config_payload}/{MAX_PAYLOAD_SIZE} bytes"
    )?;

    let mut bad_crc = encode_message(1, &Message::StatusRequest)?;
    let last_data = bad_crc.len() - 2;
    bad_crc[last_data] ^= 0x40;
    fs::write(output.join("bad_crc.bin"), &bad_crc)?;

    let mut version_frame = Frame::new(1, 6, vec![0xa0]);
    version_frame.version = 2;
    let unsupported_version = protocol::encode_frame(&version_frame)?;
    fs::write(output.join("unsupported_version.bin"), &unsupported_version)?;

    let unsupported_type = protocol::encode_frame(&Frame::new(99, 7, vec![0xa0]))?;
    fs::write(output.join("unsupported_type.bin"), &unsupported_type)?;
    let invalid_cbor = protocol::encode_frame(&Frame::new(3, 8, vec![0xff]))?;
    fs::write(output.join("invalid_cbor.bin"), &invalid_cbor)?;
    let duplicate_keys = protocol::encode_frame(&Frame::new(
        3,
        9,
        vec![0xa2, 0x00, 0x1a, 0x5e, 0x0b, 0xe1, 0x00, 0x00, 0x00],
    ))?;
    fs::write(output.join("duplicate_keys.bin"), &duplicate_keys)?;
    write_raw_frame(
        &output,
        "duplicate_widget_ids.bin",
        9,
        20,
        raw_config_payload(
            &[("dup", 1, 1, 0, 0), ("dup", 2, 2, 1, 1)],
            &[("home", "dup")],
        ),
    )?;
    write_raw_frame(
        &output,
        "duplicate_screen_ids.bin",
        9,
        21,
        raw_config_payload(
            &[("clock", 1, 1, 0, 0)],
            &[("home", "clock"), ("home", "clock")],
        ),
    )?;
    write_raw_frame(
        &output,
        "missing_widget_reference.bin",
        9,
        22,
        raw_config_payload(&[("clock", 1, 1, 0, 0)], &[("home", "ghost")]),
    )?;
    write_raw_frame(
        &output,
        "unsupported_template_config.bin",
        9,
        23,
        raw_config_payload(&[("clock", 99, 1, 0, 0)], &[("home", "clock")]),
    )?;
    write_raw_frame(
        &output,
        "unsupported_size_config.bin",
        9,
        24,
        raw_config_payload(&[("clock", 1, 3, 0, 0)], &[("home", "clock")]),
    )?;
    let excessive_widgets = vec![("clock", 1, 1, 0, 0); MAX_CONFIG_WIDGETS + 1];
    write_raw_frame(
        &output,
        "config_too_many_widgets.bin",
        9,
        25,
        raw_config_payload(&excessive_widgets, &[("home", "clock")]),
    )?;
    let excessive_screens = vec![("home", "clock"); MAX_CONFIG_SCREENS + 1];
    write_raw_frame(
        &output,
        "config_too_many_screens.bin",
        9,
        26,
        raw_config_payload(&[("clock", 1, 1, 0, 0)], &excessive_screens),
    )?;
    let mut invalid_utf8 = raw_config_payload(&[("clock", 1, 1, 0, 0)], &[("home", "clock")]);
    let clock_position = invalid_utf8
        .windows(5)
        .position(|window| window == b"clock")
        .ok_or("clock text missing from fixture")?;
    invalid_utf8[clock_position] = 0xff;
    write_raw_frame(&output, "invalid_config_utf8.bin", 9, 27, invalid_utf8)?;
    write_raw_frame(
        &output,
        "duplicate_field_names.bin",
        5,
        28,
        raw_duplicate_field_payload(),
    )?;
    write_raw_frame(
        &output,
        "zero_request_config.bin",
        9,
        0,
        raw_config_payload(&[("clock", 1, 1, 0, 0)], &[("home", "clock")]),
    )?;
    let event = encode_message(
        0,
        &Message::DeviceEvent(DeviceEvent {
            sequence: 9,
            kind: EventKind::Tap,
            widget_id: "timer".into(),
            screen_id: "focus".into(),
            action: EventAction::StartPause,
            interrupt_token: None,
        }),
    )?;
    let mut nonzero_event = protocol::decode_wire_frame(&event)?;
    nonzero_event.request_id = 29;
    fs::write(
        output.join("nonzero_request_event.bin"),
        protocol::encode_frame(&nonzero_event)?,
    )?;
    let mut overlong = vec![1_u8; MAX_WIRE_FRAME + 1];
    overlong.push(0);
    fs::write(output.join("overlong.bin"), &overlong)?;
    fs::write(output.join("garbage.bin"), b"garbage\0")?;

    manifest.push_str(
        "\nInvalid inputs:\n\
         - bad_crc.bin: Checksum\n\
         - unsupported_version.bin: Version(2) after valid framing\n\
         - unsupported_type.bin: UnsupportedType(99) after valid framing\n\
         - invalid_cbor.bin: invalid/indefinite CBOR\n\
         - duplicate_keys.bin: duplicate numeric map key\n\
         - duplicate_widget_ids.bin: duplicate widget identifiers\n\
         - duplicate_screen_ids.bin: duplicate screen identifiers\n\
         - missing_widget_reference.bin: screen references an unknown widget\n\
         - unsupported_template_config.bin: unknown template enum\n\
         - unsupported_size_config.bin: reserved tile size in M2\n\
         - config_too_many_widgets.bin: configuration capacity exceeded\n\
         - config_too_many_screens.bin: configuration capacity exceeded\n\
         - invalid_config_utf8.bin: invalid UTF-8 identifier\n\
         - duplicate_field_names.bin: duplicate PushData field name\n\
         - zero_request_config.bin: host request uses request ID zero\n\
         - nonzero_request_event.bin: DeviceEvent uses a nonzero request ID\n\
         - overlong.bin: encoded frame overflow then delimiter\n\
         - garbage.bin: malformed COBS/frame\n",
    );
    fs::write(output.join("manifest.txt"), manifest)?;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(output, "{byte:02x}").unwrap();
    }
    output
}
