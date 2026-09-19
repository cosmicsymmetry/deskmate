use std::sync::atomic::{AtomicU64, Ordering};

use protocol::Message;

use crate::DeviceError;

pub use crate::replay::ReplayState;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionDiagnostics {
    pub keepalives_sent: u64,
    pub reconnects: u64,
    pub duplicate_or_out_of_order_events: u64,
    pub locally_dropped_events: u64,
    pub detected_event_gaps: u64,
    pub malformed_device_frames: u64,
    pub unexpected_device_frames: u64,
}

#[derive(Default)]
pub struct DiagnosticCounters {
    pub keepalives_sent: AtomicU64,
    pub reconnects: AtomicU64,
    pub duplicate_or_out_of_order_events: AtomicU64,
    pub locally_dropped_events: AtomicU64,
    pub detected_event_gaps: AtomicU64,
    pub malformed_device_frames: AtomicU64,
    pub unexpected_device_frames: AtomicU64,
}

impl DiagnosticCounters {
    pub fn snapshot(&self) -> SessionDiagnostics {
        SessionDiagnostics {
            keepalives_sent: self.keepalives_sent.load(Ordering::Relaxed),
            reconnects: self.reconnects.load(Ordering::Relaxed),
            duplicate_or_out_of_order_events: self
                .duplicate_or_out_of_order_events
                .load(Ordering::Relaxed),
            locally_dropped_events: self.locally_dropped_events.load(Ordering::Relaxed),
            detected_event_gaps: self.detected_event_gaps.load(Ordering::Relaxed),
            malformed_device_frames: self.malformed_device_frames.load(Ordering::Relaxed),
            unexpected_device_frames: self.unexpected_device_frames.load(Ordering::Relaxed),
        }
    }
}

pub fn require_ack(
    response: &Message,
    acknowledged_type: u8,
    revision: Option<u32>,
) -> Result<(), DeviceError> {
    match response {
        Message::Ack(ack)
            if ack.acknowledged_type == acknowledged_type && ack.revision == revision =>
        {
            Ok(())
        }
        _ => Err(DeviceError::UnexpectedMessage),
    }
}
