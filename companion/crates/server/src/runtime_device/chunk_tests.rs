//! Drive the real actor with an in-memory WebSocket message stream. No listener
//! or alternate request/correlation implementation is involved.
use std::pin::Pin;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, mpsc};
use std::task::{Context, Poll};
use std::time::Duration;

use app_core::RuntimeDevice;
use axum::extract::ws::Message as WsMessage;
use device::DeviceError;
use futures_util::{Sink, Stream};
use protocol::{ASSET_CHUNK_WINDOW, MAX_ASSET_CHUNK_BYTES, Message};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::{AssetTransfer, ChunkTransfer, DeviceCommand, SocketPeer, WebSocketRuntimeDevice};

struct FakeSocket {
    incoming: UnboundedReceiver<WsMessage>,
    outgoing: UnboundedSender<WsMessage>,
}

impl Stream for FakeSocket {
    type Item = Result<WsMessage, std::io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.incoming.poll_recv(cx).map(|message| message.map(Ok))
    }
}

impl Sink<WsMessage> for FakeSocket {
    type Error = std::io::Error;
    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
    fn start_send(self: Pin<&mut Self>, message: WsMessage) -> Result<(), Self::Error> {
        self.outgoing
            .send(message)
            .map_err(|_| std::io::ErrorKind::BrokenPipe.into())
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

struct FakeDevice {
    incoming: UnboundedSender<WsMessage>,
    outgoing: UnboundedReceiver<WsMessage>,
    actor: tokio::task::JoinHandle<()>,
}

impl FakeDevice {
    fn new(peer: SocketPeer) -> Self {
        let (incoming, rx) = unbounded_channel();
        let (tx, outgoing) = unbounded_channel();
        let actor = tokio::spawn(peer.run_socket(
            FakeSocket {
                incoming: rx,
                outgoing: tx,
            },
            "pipeline-test",
            Arc::new(AtomicU64::new(0)),
        ));
        Self {
            incoming,
            outgoing,
            actor,
        }
    }

    async fn request(&mut self) -> (u32, Message) {
        loop {
            let message = tokio::time::timeout(Duration::from_secs(1), self.outgoing.recv())
                .await
                .unwrap()
                .unwrap();
            match message {
                WsMessage::Binary(wire) => {
                    let frame = protocol::decode_wire_frame(&wire).unwrap();
                    return (frame.request_id, protocol::decode_message(&frame).unwrap());
                }
                WsMessage::Ping(payload) => {
                    self.incoming.send(WsMessage::Pong(payload)).unwrap();
                }
                other => panic!("unexpected message: {other:?}"),
            }
        }
    }

    fn reply(&self, id: u32, message: &Message) {
        self.incoming
            .send(WsMessage::Binary(
                protocol::encode_message(id, message).unwrap().into(),
            ))
            .unwrap();
    }

    fn ack(&self, id: u32) {
        self.reply(
            id,
            &Message::Ack(protocol::Ack {
                acknowledged_type: protocol::TYPE_ASSET_CHUNK,
                revision: None,
                already_present: None,
            }),
        );
    }

    async fn no_request(&mut self) {
        // Yield to the actor without advancing paused time toward a deadline.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(
            self.outgoing.try_recv().is_err(),
            "window sent an extra request"
        );
    }
}

impl Drop for FakeDevice {
    fn drop(&mut self) {
        self.actor.abort();
    }
}

fn start_batch(
    bytes: Vec<u8>,
) -> (
    WebSocketRuntimeDevice,
    FakeDevice,
    mpsc::Receiver<super::chunks::ChunkResult>,
) {
    let (device, connector) = WebSocketRuntimeDevice::channel("pipeline-test".into());
    let peer = connector.attach();
    let (response, result) = mpsc::sync_channel(1);
    device
        .transport
        .current()
        .unwrap()
        .1
        .send(DeviceCommand::Chunks(ChunkTransfer::new(
            [7; 32],
            bytes,
            response,
            Some(AssetTransfer::opened()),
        )))
        .unwrap_or_else(|_| panic!("actor gone"));
    (device, FakeDevice::new(peer), result)
}

fn data() -> Vec<u8> {
    (0..(MAX_ASSET_CHUNK_BYTES * 19 + 17))
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect()
}

async fn take_window(fake: &mut FakeDevice) -> Vec<u32> {
    let mut ids = Vec::new();
    for _ in 0..ASSET_CHUNK_WINDOW {
        let (id, message) = fake.request().await;
        assert!(matches!(message, Message::AssetChunk(_)));
        ids.push(id);
    }
    fake.no_request().await;
    ids
}

#[tokio::test(start_paused = true)]
async fn chunk_window_is_eight_with_ordered_offsets_and_event_delivery() {
    let bytes = data();
    let (mut device, mut fake, result) = start_batch(bytes.clone());
    let mut outstanding = std::collections::VecDeque::new();
    let mut received = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for index in 0..bytes.len().div_ceil(MAX_ASSET_CHUNK_BYTES) {
        let (id, message) = fake.request().await;
        assert!(ids.insert(id));
        let Message::AssetChunk(chunk) = message else {
            panic!("expected chunk");
        };
        assert_eq!(chunk.digest, [7; 32]);
        assert_eq!(chunk.offset as usize, index * MAX_ASSET_CHUNK_BYTES);
        received.extend(chunk.data);
        outstanding.push_back(id);
        if outstanding.len() == ASSET_CHUNK_WINDOW {
            fake.no_request().await;
            // A tap must be delivered even before any chunk has been acked.
            if index == ASSET_CHUNK_WINDOW - 1 {
                fake.reply(
                    0,
                    &Message::DeviceEvent(protocol::DeviceEvent {
                        view_index: None,
                        sequence: 1,
                        kind: protocol::EventKind::Tap,
                        card_id: "picture".into(),
                        action: protocol::EventAction::StartPause,
                        interrupt_token: None,
                    }),
                );
                for _ in 0..10 {
                    tokio::task::yield_now().await;
                }
                assert_eq!(device.try_recv_event().unwrap().event.card_id, "picture");
            }
            fake.ack(outstanding.pop_front().unwrap());
        }
    }
    assert!(
        result.try_recv().is_err(),
        "must wait for every Ack before commit"
    );
    for id in outstanding {
        fake.ack(id);
    }
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    let accounting = result.try_recv().unwrap().unwrap().unwrap();
    assert_eq!(received, bytes);
    assert_eq!(usize::try_from(accounting.bytes).unwrap(), bytes.len());
    assert_eq!(
        accounting.chunks as usize,
        bytes.len().div_ceil(MAX_ASSET_CHUNK_BYTES)
    );
    fake.no_request().await;
}

#[tokio::test(start_paused = true)]
async fn chunk_window_correlates_out_of_order_acks_without_releasing_earlier_slots() {
    let (_device, mut fake, result) = start_batch(data());
    let ids = take_window(&mut fake).await;
    fake.ack(ids[3]);
    fake.ack(ids[3]); // Duplicate cannot count twice.
    fake.ack(999); // Unmatched cannot consume a slot.
    fake.no_request().await;
    fake.ack(ids[0]);
    let (_, message) = fake.request().await;
    assert!(
        matches!(message, Message::AssetChunk(chunk) if chunk.offset as usize == 8 * MAX_ASSET_CHUNK_BYTES)
    );
    fake.no_request().await;
    tokio::time::advance(Duration::from_secs(2)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(
        matches!(result.try_recv().unwrap(), Err((offset, DeviceError::Timeout)) if offset as usize == MAX_ASSET_CHUNK_BYTES)
    );
}

#[tokio::test(start_paused = true)]
async fn chunk_window_abandons_on_mid_window_error_and_ignores_late_acks() {
    let (device, mut fake, result) = start_batch(data());
    let ids = take_window(&mut fake).await;
    fake.reply(
        ids[3],
        &Message::Error(protocol::ErrorResponse {
            code: protocol::ErrorCode::Busy,
            diagnostic: "injected".into(),
        }),
    );
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(
        matches!(result.try_recv().unwrap(), Err((offset, DeviceError::Rejected(_))) if offset as usize == 3 * MAX_ASSET_CHUNK_BYTES)
    );
    for id in ids {
        fake.ack(id);
    }
    fake.no_request().await;
    let (response, rx) = mpsc::sync_channel(1);
    device
        .transport
        .current()
        .unwrap()
        .1
        .send(DeviceCommand::Request(super::DeviceRequest {
            message: Message::StatusRequest,
            response,
        }))
        .unwrap_or_else(|_| panic!("actor gone"));
    let (id, message) = fake.request().await;
    assert_eq!(message, Message::StatusRequest);
    fake.reply(
        id,
        &Message::StatusResponse(protocol::test_support::sample_status_response()),
    );
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(rx.try_recv().unwrap().is_ok());
}

#[tokio::test(start_paused = true)]
async fn chunk_window_timeout_is_measured_from_each_send() {
    let (_device, mut fake, result) = start_batch(data());
    let ids = take_window(&mut fake).await;
    tokio::time::advance(Duration::from_millis(1500)).await;
    fake.ack(ids[0]);
    let (_, message) = fake.request().await;
    assert!(
        matches!(message, Message::AssetChunk(chunk) if chunk.offset as usize == 8 * MAX_ASSET_CHUNK_BYTES)
    );
    // The next original chunk has 500ms left, not a new 2000ms budget.
    tokio::time::advance(Duration::from_millis(500)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(
        matches!(result.try_recv().unwrap(), Err((offset, DeviceError::Timeout)) if offset as usize == MAX_ASSET_CHUNK_BYTES)
    );
    fake.no_request().await;
}

#[tokio::test]
async fn chunk_window_is_one_without_capability_and_eight_with_it() {
    for pipelined in [false, true] {
        let (mut device, connector) = WebSocketRuntimeDevice::channel("pipeline-test".into());
        let peer = connector.attach();
        device.connected_generation = Some(peer.generation);
        device.capabilities = if pipelined {
            protocol::CAPABILITY_PIPELINED_ASSET_CHUNKS
        } else {
            0
        };
        device.asset_transfer = Some(AssetTransfer::opened());
        let mut fake = FakeDevice::new(peer);
        let caller = tokio::task::spawn_blocking(move || {
            device.send_asset_chunks([7; 32], &data()).unwrap();
            device.asset_transfer.unwrap()
        });
        let count = data().len().div_ceil(MAX_ASSET_CHUNK_BYTES);
        let window = if pipelined { ASSET_CHUNK_WINDOW } else { 1 };
        let mut outstanding = std::collections::VecDeque::new();
        for index in 0..count {
            let (id, message) = fake.request().await;
            assert!(
                matches!(message, Message::AssetChunk(chunk) if chunk.offset as usize == index * MAX_ASSET_CHUNK_BYTES)
            );
            outstanding.push_back(id);
            if outstanding.len() == window {
                assert!(
                    tokio::time::timeout(Duration::from_millis(10), fake.outgoing.recv())
                        .await
                        .is_err()
                );
                fake.ack(outstanding.pop_front().unwrap());
            }
        }
        for id in outstanding {
            fake.ack(id);
        }
        assert_eq!(
            usize::try_from(caller.await.unwrap().bytes).unwrap(),
            data().len()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn local_tap_indexes_crossing_a_scene_ack_fall_back_without_losing_events() {
    let (mut device, connector) = WebSocketRuntimeDevice::channel("tap-boundary".into());
    let mut fake = FakeDevice::new(connector.attach());
    let scene = |revision| protocol::PushScene {
        card_id: "picture".into(),
        revision,
        scene: protocol::Scene {
            revision,
            background: 0,
            nodes: vec![],
        },
        tap_views: vec![protocol::Scene {
            revision,
            background: 1,
            nodes: vec![],
        }],
        tap_wrap: true,
    };
    let first = scene(1);
    let connected = tokio::task::spawn_blocking(move || {
        device.connect().unwrap();
        device.push_scene(first).unwrap();
        device
    });
    let (id, message) = fake.request().await;
    assert!(matches!(message, Message::StatusRequest));
    fake.reply(
        id,
        &Message::StatusResponse(protocol::test_support::sample_status_response()),
    );
    let (id, message) = fake.request().await;
    assert!(matches!(message, Message::PushScene(_)));
    let ack = |revision| {
        Message::Ack(protocol::Ack {
            acknowledged_type: protocol::TYPE_PUSH_SCENE,
            revision: Some(revision),
            already_present: None,
        })
    };
    fake.reply(id, &ack(1));
    let mut device = connected.await.unwrap();
    let tap = |sequence| {
        Message::DeviceEvent(protocol::DeviceEvent {
            sequence,
            card_id: "picture".into(),
            kind: protocol::EventKind::Tap,
            action: protocol::EventAction::StartPause,
            interrupt_token: None,
            view_index: Some(1),
        })
    };
    fake.reply(0, &tap(1));
    let next = scene(2);
    let replaced = tokio::task::spawn_blocking(move || {
        device.push_scene(next).unwrap();
        device
    });
    let (id, message) = fake.request().await;
    assert!(matches!(message, Message::PushScene(_)));
    fake.reply(0, &tap(2));
    fake.reply(id, &ack(2));
    fake.reply(0, &tap(3));
    let mut device = replaced.await.unwrap();
    let events = tokio::time::timeout(Duration::from_secs(2), async {
        let mut events = Vec::new();
        while events.len() < 3 {
            if let Some(event) = device.try_recv_event() {
                events.push(event.event);
            } else {
                tokio::task::yield_now().await;
            }
        }
        events
    })
    .await
    .unwrap();
    assert_eq!(
        events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event.view_index)
            .collect::<Vec<_>>(),
        [None, None, Some(1)]
    );
}
