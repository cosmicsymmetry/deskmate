use std::env;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use device::{DeviceClient, DeviceError, Transport, connect};
use protocol::{
    Deframer, ErrorCode, Frame, Message, PushData, StatusResponse, TYPE_PUSH_DATA, TYPE_TIME_SYNC,
    decode_message, encode_frame, encode_message,
};

const BAD_CRC: &[u8] = include_bytes!("../../../../protocol/fixtures/v1/bad_crc.bin");
const GARBAGE: &[u8] = include_bytes!("../../../../protocol/fixtures/v1/garbage.bin");
const OVERLONG: &[u8] = include_bytes!("../../../../protocol/fixtures/v1/overlong.bin");

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
            _ => return Err(format!("unknown option: {argument}")),
        }
    }
    Ok(port)
}

fn write_all(transport: &mut impl Transport, bytes: &[u8]) -> Result<(), String> {
    let mut written = 0;
    while written < bytes.len() {
        let count = transport
            .write(&bytes[written..])
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Err("device disconnected during write".into());
        }
        written += count;
    }
    Ok(())
}

fn read_frames(
    transport: &mut impl Transport,
    expected: usize,
    timeout: Duration,
) -> Result<Vec<Frame>, String> {
    let deadline = Instant::now() + timeout;
    let mut deframer = Deframer::new();
    let mut frames = Vec::with_capacity(expected);
    let mut chunk = [0_u8; 512];
    while frames.len() < expected {
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for response {}/{}",
                frames.len() + 1,
                expected
            ));
        }
        let count = transport
            .read(&mut chunk)
            .map_err(|error| error.to_string())?;
        for result in deframer.push(&chunk[..count]) {
            frames.push(result.map_err(|error| format!("malformed response: {error}"))?);
        }
    }
    Ok(frames)
}

fn require_status(frame: &Frame, request_id: u32) -> Result<(), String> {
    if frame.request_id != request_id {
        return Err(format!(
            "request ID mismatch: expected {request_id}, received {}",
            frame.request_id
        ));
    }
    match decode_message(frame).map_err(|error| error.to_string())? {
        Message::StatusResponse(_) => Ok(()),
        message => Err(format!("expected status response, received {message:?}")),
    }
}

fn require_error(frame: &Frame, request_id: u32, code: ErrorCode) -> Result<(), String> {
    if frame.request_id != request_id {
        return Err(format!(
            "request ID mismatch: expected {request_id}, received {}",
            frame.request_id
        ));
    }
    match decode_message(frame).map_err(|error| error.to_string())? {
        Message::Error(error) if error.code == code => Ok(()),
        message => Err(format!("expected {code:?} error, received {message:?}")),
    }
}

fn verify_counter_deltas(before: &StatusResponse, after: &StatusResponse) -> Result<(), String> {
    let expected_valid = before.valid_frames.saturating_add(8);
    if after.valid_frames < expected_valid {
        return Err(format!(
            "valid-frame counter did not include the split, coalesced, and recovery requests: before={}, after={}",
            before.valid_frames, after.valid_frames
        ));
    }
    if after.crc_errors <= before.crc_errors {
        return Err("CRC error counter did not advance".into());
    }
    if after.malformed_frames <= before.malformed_frames {
        return Err("malformed-frame counter did not advance".into());
    }
    if after.overflow_frames <= before.overflow_frames {
        return Err("overflow-frame counter did not advance".into());
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let port = parse_port()?;
    let connected = connect(port.as_deref()).map_err(|error| error.to_string())?;
    let port_name = connected.port_name;
    let before = connected.initial_status;
    let mut transport = connected.client.into_transport();

    let split = encode_message(100, &Message::StatusRequest).map_err(|error| error.to_string())?;
    let split_at = split.len() / 2;
    write_all(&mut transport, &split[..split_at])?;
    thread::sleep(Duration::from_millis(20));
    write_all(&mut transport, &split[split_at..])?;
    let split_response = read_frames(&mut transport, 1, Duration::from_secs(2))?;
    require_status(&split_response[0], 100)?;

    let mut coalesced =
        encode_message(101, &Message::StatusRequest).map_err(|error| error.to_string())?;
    coalesced
        .extend(encode_message(102, &Message::StatusRequest).map_err(|error| error.to_string())?);
    write_all(&mut transport, &coalesced)?;
    let coalesced_responses = read_frames(&mut transport, 2, Duration::from_secs(2))?;
    require_status(&coalesced_responses[0], 101)?;
    require_status(&coalesced_responses[1], 102)?;

    write_all(&mut transport, BAD_CRC)?;
    write_all(&mut transport, GARBAGE)?;
    write_all(&mut transport, OVERLONG)?;

    // Canonical {0: "test", 1: 0, 2: {}}: structurally valid CBOR with an
    // invalid zero PushData revision.
    let invalid_push = encode_frame(&Frame::new(
        TYPE_PUSH_DATA,
        103,
        vec![
            0xa3, 0x00, 0x64, b't', b'e', b's', b't', 0x01, 0x00, 0x02, 0xa0,
        ],
    ))
    .map_err(|error| error.to_string())?;
    write_all(&mut transport, &invalid_push)?;
    let invalid_push_response = read_frames(&mut transport, 1, Duration::from_secs(2))?;
    require_error(&invalid_push_response[0], 103, ErrorCode::InvalidPayload)?;

    // Canonical {0: 1577836799, 1: 0}: one second before the v1 time floor.
    let invalid_time = encode_frame(&Frame::new(
        TYPE_TIME_SYNC,
        104,
        vec![0xa2, 0x00, 0x1a, 0x5e, 0x0b, 0xe0, 0xff, 0x01, 0x00],
    ))
    .map_err(|error| error.to_string())?;
    write_all(&mut transport, &invalid_time)?;
    let invalid_time_response = read_frames(&mut transport, 1, Duration::from_secs(2))?;
    require_error(&invalid_time_response[0], 104, ErrorCode::InvalidTime)?;

    let mut client = DeviceClient::new(transport);
    let revision = before
        .latest_revision
        .checked_add(1)
        .ok_or_else(|| "cannot run push check at maximum retained revision".to_owned())?;
    let push = PushData {
        widget_id: "acceptance".into(),
        revision,
        fields: Vec::new(),
    };
    let ack = client
        .push_data(push.clone())
        .map_err(|error| error.to_string())?;
    if ack.revision != Some(revision) {
        return Err(format!(
            "push acknowledgement revision mismatch: {:?}",
            ack.revision
        ));
    }
    match client.push_data(push) {
        Err(DeviceError::Rejected(error)) if error.code == ErrorCode::StaleRevision => {}
        Err(error) => return Err(format!("unexpected stale-push result: {error}")),
        Ok(_) => return Err("device accepted a stale push revision".into()),
    }
    let after = client.status().map_err(|error| error.to_string())?;
    verify_counter_deltas(&before, &after)?;
    if after.latest_revision != revision {
        return Err(format!(
            "retained revision mismatch: expected {revision}, received {}",
            after.latest_revision
        ));
    }

    println!(
        "PASS port={port_name} uptime_ms={} heap={} valid={}->{} malformed={}->{} crc={}->{} overflow={}->{} dropped={} rx_drops={}",
        after.uptime_ms,
        after.free_heap,
        before.valid_frames,
        after.valid_frames,
        before.malformed_frames,
        after.malformed_frames,
        before.crc_errors,
        after.crc_errors,
        before.overflow_frames,
        after.overflow_frames,
        after.dropped_responses,
        after.rx_dropped_bytes
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("hardware acceptance failed: {error}");
        process::exit(1);
    }
}
