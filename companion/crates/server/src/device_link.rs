//! `GET /v1/device/link` -- the networked-tier device's persistent link.
//!
//! This task's handler does the bare minimum to make the contract real: it
//! authenticates the upgrade, decodes and logs incoming protocol message
//! types, and provides a deliberately small status-request echo. Task 9
//! replaces that echo with the `app-core` ownership runtime; nothing here
//! should grow logic that runtime will need to own instead.
//!
//! What *does* belong here, because Task 9 would otherwise inherit the gap
//! rather than add it: bounding how many links and how much per-link memory
//! this process will hold for an anonymous internet caller once Task 8 puts
//! this behind a public tunnel. A half-open socket is routine for a `WiFi`
//! device behind NAT, and without a cap it pins a tokio task -- and, from
//! Task 9 on, an ownership slot -- forever.

use std::time::{Duration, Instant};

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::{MissedTickBehavior, interval, timeout};

use crate::ServerState;
use crate::auth::AuthenticatedDevice;
use crate::registry::DeviceId;

use protocol::{Message as ProtocolMessage, OtaState, StatusResponse, Tier, WifiState};

/// Caps concurrent device links. V2 is single-tenant and realistically has
/// one device, but this endpoint sits on a public tunnel from Task 8
/// onward, so an anonymous caller with a stolen or brute-forced token (or
/// simply many slow, half-open connect attempts) must not be able to pin an
/// unbounded number of sockets open.
///
/// Provisional: sized by judgment for a single-device V2 deployment, not
/// derived from a real one. Revisit against actual numbers in V3. It is
/// `<= MAX_CONCURRENT_REQUESTS` (`lib.rs`) by construction of the values
/// below, and an upgrade request releases its request-level permit at the
/// 101 response, so the two pools cannot deadlock each other.
pub(crate) const MAX_CONCURRENT_LINKS: usize = 32;

/// How long the link may go without receiving anything -- including a pong
/// answering our own keepalive ping -- before the server gives up on it.
/// Mirrors the protocol's own `LINK_TIMEOUT_MS`: the device's declared
/// tolerance for host silence, reused here so both ends agree on what "the
/// link is dead" means.
const IDLE_TIMEOUT: Duration = Duration::from_millis(protocol::LINK_TIMEOUT_MS);

/// How often the server checks link liveness and, if the link is still
/// alive, proactively pings it -- so a dead peer (NAT rebind, sleeping
/// device, severed `WiFi`) is detected well inside `IDLE_TIMEOUT` rather
/// than right at its edge.
///
/// This must stay strictly less than `IDLE_TIMEOUT` for the liveness check
/// below to ever fire before a much coarser fallback (bare TCP
/// retransmission timeout, on the order of minutes) does instead.
///
/// Provisional: sized by judgment (`PING_SEND_TIMEOUT` < `PING_INTERVAL` <
/// `IDLE_TIMEOUT` holds), not tuned against a real deployment. Revisit in
/// V3.
const PING_INTERVAL: Duration = Duration::from_secs(3);

/// Status is deliberately independent of the WebSocket keepalive: it proves
/// that protocol frames make a server-to-device-to-server round trip before
/// Task 9 introduces the runtime that will own real requests.
const STATUS_INTERVAL: Duration = Duration::from_secs(5);

/// How long a single keepalive ping's `send` may take. A peer that has
/// stopped reading (TCP zero-window) would otherwise let `send` block
/// indefinitely -- and because the ping is one arm of a `select!`, a stuck
/// `send` parks the *other* arm (reading, where `last_activity` gets
/// updated) too, defeating the idle-timeout check below. Bounding the send
/// itself is what keeps the loop able to make progress either way.
///
/// Provisional, like `PING_INTERVAL` above: a judgment call for V2, not a
/// tuned value. Revisit in V3.
const PING_SEND_TIMEOUT: Duration = Duration::from_secs(2);

/// The largest single WebSocket message this link will accept: one maximum
/// COBS-framed protocol frame, delimiter included. A message may also contain
/// multiple smaller complete frames; their combined size is still bounded by
/// this same limit. Without the bound axum defaults to a 64 MiB message limit,
/// so every authenticated link could otherwise make the server buffer 64 MiB
/// for a message this protocol can never legally send. `max_frame_size` is
/// capped identically: tungstenite checks a single frame against it before
/// the message-size check ever applies.
const MAX_WS_MESSAGE_SIZE: usize = protocol::MAX_WIRE_FRAME;

fn allocate_request_id(next_request_id: &mut u32) -> u32 {
    if *next_request_id == 0 {
        *next_request_id = 1;
    }
    let request_id = *next_request_id;
    *next_request_id = next_request_id.wrapping_add(1);
    if *next_request_id == 0 {
        *next_request_id = 1;
    }
    request_id
}

pub async fn handler(
    State(state): State<ServerState>,
    AuthenticatedDevice { device_id }: AuthenticatedDevice,
    ws: WebSocketUpgrade,
) -> Response {
    let Ok(permit) = state.link_slots().try_acquire_owned() else {
        tracing::warn!(
            device_id = %device_id,
            "device link refused: at MAX_CONCURRENT_LINKS capacity"
        );
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };

    ws.max_message_size(MAX_WS_MESSAGE_SIZE)
        .max_frame_size(MAX_WS_MESSAGE_SIZE)
        .on_upgrade(move |socket| run(socket, device_id, permit))
}

fn echo_status() -> StatusResponse {
    StatusResponse {
        protocol_version: protocol::PROTOCOL_VERSION,
        max_protocol_version: protocol::MAX_PROTOCOL_VERSION,
        capabilities: 0,
        firmware_version: env!("CARGO_PKG_VERSION").to_owned(),
        uptime_ms: 0,
        free_heap: 0,
        display_width: 0,
        display_height: 0,
        brightness: 0,
        rotation: 0,
        online: true,
        latest_revision: 0,
        valid_frames: 0,
        malformed_frames: 0,
        crc_errors: 0,
        overflow_frames: 0,
        dropped_responses: 0,
        rx_dropped_bytes: 0,
        dropped_events: 0,
        event_queue_high_water: 0,
        dropped_ui_commands: 0,
        ui_queue_high_water: 0,
        config_revision: 0,
        latest_interrupt_token: 0,
        tier: Tier::Local,
        wifi_state: WifiState::Down,
        wifi_rssi: 0,
        ip: String::new(),
        ota_state: OtaState::Idle,
        last_network_error: None,
    }
}

async fn handle_binary(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    bytes: &[u8],
    device_id: &DeviceId,
) -> bool {
    if bytes.is_empty() {
        tracing::warn!(device_id = %device_id, "device link received an empty binary message");
        return false;
    }

    for wire in bytes.split_inclusive(|byte| *byte == 0) {
        let frame = match protocol::decode_wire_frame(wire) {
            Ok(frame) => frame,
            Err(error) => {
                tracing::warn!(device_id = %device_id, %error, "device link frame decode failed");
                return false;
            }
        };
        let message = match protocol::decode_message(&frame) {
            Ok(message) => message,
            Err(error) => {
                tracing::warn!(device_id = %device_id, %error, "device link message decode failed");
                return false;
            }
        };
        tracing::info!(
            device_id = %device_id,
            message_type = frame.message_type,
            "device protocol message received"
        );

        if let ProtocolMessage::StatusResponse(status) = &message {
            tracing::info!(
                device_id = %device_id,
                request_id = frame.request_id,
                firmware_version = %status.firmware_version,
                uptime_ms = status.uptime_ms,
                free_heap = status.free_heap,
                tier = ?status.tier,
                wifi_state = ?status.wifi_state,
                wifi_rssi = status.wifi_rssi,
                ip = %status.ip,
                ota_state = ?status.ota_state,
                "device status response"
            );
        }

        if matches!(message, ProtocolMessage::StatusRequest) {
            let response = ProtocolMessage::StatusResponse(echo_status());
            let response_wire = match protocol::encode_message(frame.request_id, &response) {
                Ok(wire) => wire,
                Err(error) => {
                    tracing::error!(device_id = %device_id, %error, "status echo encode failed");
                    return false;
                }
            };
            let send = sender.send(Message::Binary(response_wire.into()));
            match timeout(PING_SEND_TIMEOUT, send).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(device_id = %device_id, %error, "status echo send failed");
                    return false;
                }
                Err(_) => {
                    tracing::warn!(device_id = %device_id, "status echo send stalled");
                    return false;
                }
            }
        }
    }
    true
}

/// Holds the link open with a keepalive/idle-timeout loop and the minimal
/// protocol echo described in the module docs.
///
/// Liveness is tracked with a plain `last_activity: Instant` rather than a
/// `tokio::time::timeout` wrapped around each read: a per-iteration timeout
/// re-arms from zero on *every* loop pass, including every time the ping
/// arm wins the `select!` -- with `PING_INTERVAL` shorter than
/// `IDLE_TIMEOUT`, such a deadline is torn down and rebuilt before it can
/// ever elapse, so it would never fire. Checking `last_activity.elapsed()`
/// on a separate, unconditional tick has no such blind spot.
async fn run(socket: WebSocket, device_id: DeviceId, _permit: OwnedSemaphorePermit) {
    tracing::info!(device_id = %device_id, "device link established");

    let (mut sender, mut receiver) = socket.split();
    let mut last_activity = Instant::now();

    let mut ping_tick = interval(PING_INTERVAL);
    ping_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ping_tick.tick().await; // the first tick fires immediately; skip it

    let mut status_tick = interval(STATUS_INTERVAL);
    status_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    status_tick.tick().await; // the first tick fires immediately; skip it
    let mut next_request_id = 1_u32;

    loop {
        tokio::select! {
            next_message = receiver.next() => {
                match next_message {
                    None | Some(Err(_) | Ok(Message::Close(_))) => break,
                    Some(Ok(Message::Text(_))) => {
                        // The wire protocol is binary-only end to end; a
                        // text frame means the peer isn't speaking it.
                        tracing::warn!(
                            device_id = %device_id,
                            "device link received a text frame, closing"
                        );
                        break;
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        last_activity = Instant::now();
                        if !handle_binary(&mut sender, &bytes, &device_id).await {
                            let close = sender.send(Message::Close(None));
                            let _ = timeout(PING_SEND_TIMEOUT, close).await;
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
                        // A `Pong` answering our own keepalive counts as
                        // activity too, which is the point: it's how a
                        // healthy link's deadline gets pushed out.
                        last_activity = Instant::now();
                    }
                }
            }
            _ = ping_tick.tick() => {
                if last_activity.elapsed() > IDLE_TIMEOUT {
                    tracing::warn!(device_id = %device_id, "device link idle timeout, closing");
                    break;
                }
                let ping = sender.send(Message::Ping(Vec::new().into()));
                if timeout(PING_SEND_TIMEOUT, ping).await.is_err() {
                    tracing::warn!(
                        device_id = %device_id,
                        "device link ping send stalled, closing"
                    );
                    break;
                }
            }
            _ = status_tick.tick() => {
                let request_id = allocate_request_id(&mut next_request_id);
                let wire = match protocol::encode_message(
                    request_id,
                    &ProtocolMessage::StatusRequest,
                ) {
                    Ok(wire) => wire,
                    Err(error) => {
                        tracing::error!(
                            device_id = %device_id,
                            %error,
                            "periodic status request encode failed"
                        );
                        break;
                    }
                };
                let send = sender.send(Message::Binary(wire.into()));
                match timeout(PING_SEND_TIMEOUT, send).await {
                    Ok(Ok(())) => {
                        tracing::debug!(
                            device_id = %device_id,
                            request_id,
                            "periodic status request sent"
                        );
                    }
                    Ok(Err(error)) => {
                        tracing::warn!(
                            device_id = %device_id,
                            %error,
                            "periodic status request send failed"
                        );
                        break;
                    }
                    Err(_) => {
                        tracing::warn!(
                            device_id = %device_id,
                            "periodic status request send stalled"
                        );
                        break;
                    }
                }
            }
        }
    }

    tracing::info!(device_id = %device_id, "device link closed");
}

#[cfg(test)]
mod tests {
    use super::allocate_request_id;

    #[test]
    fn request_ids_are_nonzero_and_wrap_to_one() {
        let mut next = 0_u32;
        assert_eq!(allocate_request_id(&mut next), 1);
        assert_eq!(next, 2);

        next = u32::MAX;
        assert_eq!(allocate_request_id(&mut next), u32::MAX);
        assert_eq!(next, 1);
        assert_eq!(allocate_request_id(&mut next), 1);
    }
}
