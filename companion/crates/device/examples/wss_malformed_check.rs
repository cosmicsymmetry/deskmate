//! Hardware-only malformed-input check for the device's WSS transport.
//!
//! This binary is the local WebSocket server the board dials through a TLS
//! tunnel. It is intentionally an example, not an automated test: a physical
//! board must be provisioned with the tunnel's `wss://.../v1/device/link` URL
//! and the same bearer token supplied here.
//!
//! Run from `companion/`:
//! `cargo run -p device --example wss_malformed_check -- --listen 127.0.0.1:8787 --token <token>`

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use protocol::{ErrorCode, Frame, Message, StatusResponse};
use tokio::sync::mpsc;
use tokio::time::timeout;

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const SETTLE_TIME: Duration = Duration::from_secs(1);

struct Arguments {
    listen: SocketAddr,
    token: String,
}

#[derive(Clone)]
struct HarnessState {
    token: Arc<String>,
    result_tx: mpsc::UnboundedSender<Result<(), String>>,
}

fn parse_arguments() -> Result<Arguments, String> {
    let mut listen = "127.0.0.1:8787".parse().unwrap();
    let mut token = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--listen" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| "--listen requires an address".to_owned())?;
                listen = value
                    .parse()
                    .map_err(|error| format!("invalid --listen address: {error}"))?;
            }
            "--token" => {
                token = Some(
                    arguments
                        .next()
                        .ok_or_else(|| "--token requires a value".to_owned())?,
                );
            }
            _ => return Err(format!("unknown option: {argument}")),
        }
    }
    let token = token.ok_or_else(|| "--token is required".to_owned())?;
    if token.is_empty() || token.len() > protocol::MAX_DEVICE_TOKEN_LEN {
        return Err(format!(
            "--token must contain 1..={} bytes",
            protocol::MAX_DEVICE_TOKEN_LEN
        ));
    }
    Ok(Arguments { listen, token })
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (left, right) in left.iter().zip(right) {
        difference |= left ^ right;
    }
    difference == 0
}

fn authorized(headers: &HeaderMap, expected: &str) -> bool {
    let Some(header) = headers.get("Authorization") else {
        return false;
    };
    let Ok(header) = header.to_str() else {
        return false;
    };
    let Some(token) = header.strip_prefix("Bearer ") else {
        return false;
    };
    constant_time_eq(token.as_bytes(), expected.as_bytes())
}

async fn device_link(
    State(state): State<HarnessState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !authorized(&headers, &state.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.max_message_size(protocol::MAX_WIRE_FRAME)
        .max_frame_size(protocol::MAX_WIRE_FRAME)
        .on_upgrade(move |socket| async move {
            let result = check_device(socket).await;
            let _ = state.result_tx.send(result);
        })
}

async fn send(socket: &mut WebSocket, bytes: Vec<u8>) -> Result<(), String> {
    socket
        .send(WsMessage::Binary(bytes.into()))
        .await
        .map_err(|error| format!("WebSocket send failed: {error}"))
}

async fn next_protocol_message(socket: &mut WebSocket) -> Result<(Frame, Message), String> {
    timeout(RESPONSE_TIMEOUT, async {
        loop {
            match socket.recv().await {
                Some(Ok(WsMessage::Binary(wire))) => {
                    let frame = protocol::decode_wire_frame(&wire)
                        .map_err(|error| format!("malformed board frame: {error}"))?;
                    let message = protocol::decode_message(&frame)
                        .map_err(|error| format!("malformed board message: {error}"))?;
                    return Ok((frame, message));
                }
                Some(Ok(WsMessage::Ping(payload))) => {
                    socket
                        .send(WsMessage::Pong(payload))
                        .await
                        .map_err(|error| format!("WebSocket pong failed: {error}"))?;
                }
                Some(Ok(WsMessage::Close(_))) | None => {
                    return Err("board disconnected during malformed-input check".to_owned());
                }
                Some(Ok(WsMessage::Text(_))) => {
                    return Err("board sent an unexpected text message".to_owned());
                }
                Some(Ok(WsMessage::Pong(_))) => {}
                Some(Err(error)) => return Err(format!("WebSocket read failed: {error}")),
            }
        }
    })
    .await
    .map_err(|_| "timed out waiting for a board response".to_owned())?
}

async fn status(socket: &mut WebSocket, request_id: u32) -> Result<StatusResponse, String> {
    send(
        socket,
        protocol::encode_message(request_id, &Message::StatusRequest)
            .map_err(|error| error.to_string())?,
    )
    .await?;
    loop {
        let (frame, message) = next_protocol_message(socket).await?;
        if frame.request_id == request_id {
            return match message {
                Message::StatusResponse(status) => Ok(status),
                other => Err(format!("expected status response, received {other:?}")),
            };
        }
    }
}

async fn check_device(mut socket: WebSocket) -> Result<(), String> {
    let before = status(&mut socket, 1).await?;

    let mut corrupt_delimiter =
        protocol::encode_message(2, &Message::StatusRequest).map_err(|error| error.to_string())?;
    corrupt_delimiter.insert(corrupt_delimiter.len() / 2, 0);
    send(&mut socket, corrupt_delimiter).await?;

    send(
        &mut socket,
        protocol::test_support::oversized_declared_payload(3),
    )
    .await?;

    let unknown_type = protocol::encode_frame(&Frame::new(u8::MAX, 4, vec![0xa0]))
        .map_err(|error| error.to_string())?;
    send(&mut socket, unknown_type).await?;

    let mut truncated =
        protocol::encode_message(5, &Message::StatusRequest).map_err(|error| error.to_string())?;
    if truncated.len() < 3 || truncated.last() != Some(&0) {
        return Err("status fixture was too short or missing its delimiter".to_owned());
    }
    truncated.truncate(truncated.len() - 3);
    // Keep a delimiter so the destructively shortened frame reaches the
    // decoder and the byte stream resynchronises for the next case.
    truncated.push(0);
    send(&mut socket, truncated).await?;

    let mut concatenated =
        protocol::encode_message(10, &Message::StatusRequest).map_err(|error| error.to_string())?;
    concatenated.extend(
        protocol::encode_message(11, &Message::StatusRequest).map_err(|error| error.to_string())?,
    );
    send(&mut socket, concatenated).await?;

    tokio::time::sleep(SETTLE_TIME).await;
    send(
        &mut socket,
        protocol::encode_message(20, &Message::StatusRequest).map_err(|error| error.to_string())?,
    )
    .await?;

    let mut saw_first_concatenated = false;
    let mut saw_second_concatenated = false;
    let mut saw_unknown_type_rejection = false;
    let after = loop {
        let (frame, message) = next_protocol_message(&mut socket).await?;
        match message {
            Message::StatusResponse(_) if matches!(frame.request_id, 2 | 3 | 5) => {
                return Err(format!(
                    "board accepted malformed status request {}",
                    frame.request_id
                ));
            }
            Message::Error(error)
                if frame.request_id == 4 && error.code == ErrorCode::UnsupportedMessage =>
            {
                saw_unknown_type_rejection = true;
            }
            Message::StatusResponse(_) if frame.request_id == 10 => {
                saw_first_concatenated = true;
            }
            Message::StatusResponse(_) if frame.request_id == 11 => {
                saw_second_concatenated = true;
            }
            Message::StatusResponse(status) if frame.request_id == 20 => {
                break status;
            }
            _ => {}
        }
    };

    if !saw_unknown_type_rejection {
        return Err("board did not reject the unknown message type".to_owned());
    }
    if !saw_first_concatenated || !saw_second_concatenated {
        return Err("board did not decode both concatenated frames".to_owned());
    }
    if after.uptime_ms <= before.uptime_ms {
        return Err(format!(
            "board uptime did not advance (before={}, after={}); it may have rebooted",
            before.uptime_ms, after.uptime_ms
        ));
    }

    println!(
        "PASS: WSS framing recovered without reboot (uptime {} -> {} ms)",
        before.uptime_ms, after.uptime_ms
    );
    Ok(())
}

async fn run() -> Result<(), String> {
    let arguments = parse_arguments()?;
    let listener = tokio::net::TcpListener::bind(arguments.listen)
        .await
        .map_err(|error| format!("failed to bind {}: {error}", arguments.listen))?;
    let local_address = listener
        .local_addr()
        .map_err(|error| format!("failed to read listener address: {error}"))?;
    let (result_tx, mut result_rx) = mpsc::unbounded_channel();
    let state = HarnessState {
        token: Arc::new(arguments.token),
        result_tx,
    };
    let application = Router::new()
        .route("/v1/device/link", get(device_link))
        .with_state(state);

    println!("malformed-input server listening on http://{local_address}");
    println!(
        "expose this address with a TLS tunnel, then provision the board with its wss:// hostname and /v1/device/link"
    );
    println!("waiting for the networked-tier board to connect...");

    let server = tokio::spawn(async move {
        axum::serve(listener, application)
            .await
            .map_err(|error| error.to_string())
    });
    tokio::select! {
        result = result_rx.recv() => result.ok_or_else(|| "check task stopped without a result".to_owned())?,
        server_result = server => {
            server_result
                .map_err(|error| format!("server task failed: {error}"))?
                .and_then(|()| Err("server stopped before the board connected".to_owned()))
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("FAIL: {error}");
        std::process::exit(1);
    }
}
