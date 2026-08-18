//! `GET /v1/device/link` -- the networked-tier device's persistent link.
//!
//! This task's handler does the bare minimum to make the contract real: it
//! authenticates the upgrade, logs which device connected, and then holds
//! the socket open, discarding whatever the device sends. Task 9 replaces
//! the body of [`run`] with the `app-core` ownership runtime; nothing here
//! should grow logic that runtime will need to own instead.
//!
//! What *does* belong here, because Task 9 would otherwise inherit the gap
//! rather than add it: bounding how many links and how much per-link memory
//! this process will hold for an anonymous internet caller once Task 8 puts
//! this behind a public tunnel. A half-open socket is routine for a `WiFi`
//! device behind NAT, and without a cap it pins a tokio task -- and, from
//! Task 9 on, an ownership slot -- forever.

use std::time::Duration;

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

/// Caps concurrent device links. V2 is single-tenant and realistically has
/// one device, but this endpoint sits on a public tunnel from Task 8
/// onward, so an anonymous caller with a stolen or brute-forced token (or
/// simply many slow, half-open connect attempts) must not be able to pin an
/// unbounded number of sockets open.
pub(crate) const MAX_CONCURRENT_LINKS: usize = 32;

/// How long the link may go without receiving anything -- including a pong
/// answering our own keepalive ping -- before the server gives up on it.
/// Mirrors the protocol's own `LINK_TIMEOUT_MS`: the device's declared
/// tolerance for host silence, reused here so both ends agree on what "the
/// link is dead" means.
const IDLE_TIMEOUT: Duration = Duration::from_millis(protocol::LINK_TIMEOUT_MS);

/// How often the server proactively pings an otherwise-quiet link, so a
/// dead peer (NAT rebind, sleeping device, severed `WiFi`) is detected well
/// inside `IDLE_TIMEOUT` rather than right at its edge.
const PING_INTERVAL: Duration = Duration::from_secs(3);

/// The largest single WebSocket message this link will accept: one
/// COBS-framed protocol frame, delimiter included -- exactly
/// `protocol::MAX_WIRE_FRAME`, since that is what one binary WS message
/// carries end to end (see the module doc on framing in `lib.rs`/the design
/// spec). Without this bound axum defaults to a 64 MiB message limit, so
/// every authenticated link could otherwise make the server buffer 64 MiB
/// for a message this protocol can never legally send.
const MAX_WS_MESSAGE_SIZE: usize = protocol::MAX_WIRE_FRAME;

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
        .on_upgrade(move |socket| run(socket, device_id, permit))
}

/// Holds the link open with a keepalive/idle-timeout loop. Does nothing
/// else with the frames it receives -- see the module doc.
async fn run(socket: WebSocket, device_id: DeviceId, _permit: OwnedSemaphorePermit) {
    tracing::info!(device_id = %device_id, "device link established");

    let (mut sender, mut receiver) = socket.split();

    let mut ping_tick = interval(PING_INTERVAL);
    ping_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ping_tick.tick().await; // the first tick fires immediately; skip it

    loop {
        tokio::select! {
            next_message = timeout(IDLE_TIMEOUT, receiver.next()) => {
                match next_message {
                    Err(_elapsed) => {
                        tracing::warn!(device_id = %device_id, "device link idle timeout, closing");
                        break;
                    }
                    Ok(None | Some(Err(_) | Ok(Message::Close(_)))) => break,
                    Ok(Some(Ok(Message::Text(_)))) => {
                        // The wire protocol is binary-only end to end; a
                        // text frame means the peer isn't speaking it.
                        tracing::warn!(
                            device_id = %device_id,
                            "device link received a text frame, closing"
                        );
                        break;
                    }
                    Ok(Some(Ok(_binary_ping_or_pong))) => {
                        // Task 9 hands inbound binary frames to the
                        // ownership runtime. Until then the link is held
                        // open and nothing is echoed.
                    }
                }
            }
            _ = ping_tick.tick() => {
                if sender.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
        }
    }

    tracing::info!(device_id = %device_id, "device link closed");
}
