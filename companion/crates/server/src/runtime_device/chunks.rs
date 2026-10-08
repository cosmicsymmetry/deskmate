//! One bounded, ordered asset transfer owned by the socket actor.
use std::collections::VecDeque;
use std::sync::mpsc::SyncSender;

use device::{DeviceError, TransportError, session_state::require_ack};
use protocol::{ASSET_CHUNK_WINDOW, ASSET_DIGEST_LEN, AssetChunk, MAX_ASSET_CHUNK_BYTES, Message};
use tokio::time::Instant;

use super::{AssetTransfer, REQUEST_TIMEOUT, record_chunk};

pub(super) type ChunkResult = Result<Option<AssetTransfer>, (u32, DeviceError)>;

struct PendingChunk {
    request_id: u32,
    offset: u32,
    bytes: usize,
    sent: Instant,
    acknowledged: bool,
}

pub(super) struct ChunkTransfer {
    digest: [u8; ASSET_DIGEST_LEN],
    wire: Vec<u8>,
    offset: usize,
    pending: VecDeque<PendingChunk>,
    response: SyncSender<ChunkResult>,
    accounting: Option<AssetTransfer>,
    finished: bool,
}

impl ChunkTransfer {
    pub(super) fn new(
        digest: [u8; ASSET_DIGEST_LEN],
        wire: Vec<u8>,
        response: SyncSender<ChunkResult>,
        accounting: Option<AssetTransfer>,
    ) -> Self {
        Self {
            digest,
            wire,
            offset: 0,
            pending: VecDeque::new(),
            response,
            accounting,
            finished: false,
        }
    }

    pub(super) fn can_send(&self) -> bool {
        !self.finished && self.offset < self.wire.len() && self.pending.len() < ASSET_CHUNK_WINDOW
    }

    pub(super) fn next(&mut self, request_id: u32) -> Message {
        let end = (self.offset + MAX_ASSET_CHUNK_BYTES).min(self.wire.len());
        let chunk = AssetChunk {
            digest: self.digest,
            offset: u32::try_from(self.offset)
                .expect("asset wire length was validated before AssetBegin"),
            data: self.wire[self.offset..end].to_vec(),
        };
        self.pending.push_back(PendingChunk {
            request_id,
            offset: chunk.offset,
            bytes: chunk.data.len(),
            sent: Instant::now(),
            acknowledged: false,
        });
        self.offset = end;
        Message::AssetChunk(chunk)
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.pending
            .iter()
            .find(|chunk| !chunk.acknowledged)
            .map(|chunk| chunk.sent + REQUEST_TIMEOUT)
    }

    pub(super) fn finished(&self) -> bool {
        self.finished
    }

    pub(super) fn receive(&mut self, request_id: u32, message: &Message, device_id: &str) -> bool {
        let Some(index) = self
            .pending
            .iter()
            .position(|chunk| chunk.request_id == request_id && !chunk.acknowledged)
        else {
            return false;
        };
        let result = match message {
            Message::Error(error) => Err(DeviceError::Rejected(error.clone())),
            message => require_ack(message, protocol::TYPE_ASSET_CHUNK, None),
        };
        if let Err(error) = result {
            self.fail(index, error, device_id);
            return true;
        }
        let chunk = &mut self.pending[index];
        record_chunk(
            device_id,
            &mut self.accounting,
            chunk.offset,
            chunk.bytes,
            Ok(()),
            chunk.sent.elapsed(),
        );
        chunk.acknowledged = true;
        // Match by ID, but release window slots in send order. A later Ack
        // cannot hide a missing earlier chunk or extend its deadline.
        while self.pending.front().is_some_and(|chunk| chunk.acknowledged) {
            self.pending.pop_front();
        }
        if self.offset == self.wire.len() && self.pending.is_empty() {
            self.finished = true;
            let _ = self.response.send(Ok(self.accounting.take()));
        }
        true
    }

    pub(super) fn expire(&mut self, device_id: &str, generation: u64) {
        if let Some(index) = self
            .pending
            .iter()
            .position(|chunk| !chunk.acknowledged && chunk.sent + REQUEST_TIMEOUT <= Instant::now())
        {
            tracing::info!(target: "server::tap_latency", device_id, generation,
                request_id = self.pending[index].request_id,
                unix_us = chrono::Utc::now().timestamp_micros(), "device request timed out");
            self.fail(index, DeviceError::Timeout, device_id);
        }
    }

    pub(super) fn send_failed(&mut self, error: DeviceError, device_id: &str) {
        self.fail(self.pending.len() - 1, error, device_id);
    }

    fn fail(&mut self, index: usize, error: DeviceError, device_id: &str) {
        let chunk = &self.pending[index];
        let offset = chunk.offset;
        record_chunk(
            device_id,
            &mut self.accounting,
            offset,
            chunk.bytes,
            Err(&error),
            chunk.sent.elapsed(),
        );
        self.finished = true;
        let _ = self.response.send(Err((offset, error)));
        self.pending.clear();
    }

    pub(super) fn disconnected(&mut self, device_id: &str) {
        if !self.finished && !self.pending.is_empty() {
            self.fail(
                0,
                DeviceError::Transport(TransportError::Disconnected),
                device_id,
            );
        }
    }
}
