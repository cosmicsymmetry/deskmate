//! Shared constructors for intentionally invalid protocol fixtures.
//!
//! These helpers are public only so integration tests and hardware-check
//! examples in other workspace crates exercise the same malformed bytes.

use crate::PROTOCOL_VERSION;
use crate::frame::{MAX_PAYLOAD_SIZE, cobs_encode, crc32c};
use crate::message::TYPE_STATUS_REQUEST;

/// Build a checksummed frame whose declared payload is one byte over the
/// protocol maximum. It cannot be constructed through the validated API.
pub fn oversized_declared_payload(request_id: u32) -> Vec<u8> {
    let declared = u16::try_from(MAX_PAYLOAD_SIZE + 1).expect("payload bound fits u16");
    let mut decoded = vec![PROTOCOL_VERSION, TYPE_STATUS_REQUEST, 0, 0];
    decoded.extend_from_slice(&request_id.to_le_bytes());
    decoded.extend_from_slice(&declared.to_le_bytes());
    decoded.extend_from_slice(&crc32c(&decoded).to_le_bytes());
    let mut wire = cobs_encode(&decoded);
    wire.push(0);
    wire
}
