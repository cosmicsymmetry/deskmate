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

/// The largest single WebSocket message this link will accept: one
/// COBS-framed protocol frame, delimiter included -- exactly
/// `protocol::MAX_WIRE_FRAME`, since that is what one binary WS message
/// carries end to end (see the module doc on framing in `lib.rs`/the design
/// spec). Without this bound axum defaults to a 64 MiB message limit, so
/// every authenticated link could otherwise make the server buffer 64 MiB
/// for a message this protocol can never legally send. `max_frame_size` is
/// capped identically: tungstenite checks a single frame against it
/// *before* the message-size check ever applies, and this protocol never
/// fragments one frame across multiple WS frames, so the two bounds should
/// coincide -- leaving `max_frame_size` at its 16 MiB default would let one
/// link buffer 16 MiB before rejection regardless of the message-size cap.
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
        .max_frame_size(MAX_WS_MESSAGE_SIZE)
        .on_upgrade(move |socket| run(socket, device_id, permit))
}

/// Holds the link open with a keepalive/idle-timeout loop. Does nothing
/// else with the frames it receives -- see the module doc.
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
                    Some(Ok(_binary_ping_or_pong)) => {
                        // A `Pong` answering our own keepalive counts as
                        // activity too, which is the point: it's how a
                        // healthy link's deadline gets pushed out.
                        last_activity = Instant::now();
                        // Task 9 hands inbound binary frames to the
                        // ownership runtime. Until then the link is held
                        // open and nothing is echoed.
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
        }
    }

    tracing::info!(device_id = %device_id, "device link closed");
}
