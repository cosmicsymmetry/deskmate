//! Shared protocol test fixtures: one valid status and malformed frames.
//!
//! These helpers are public only so integration tests and hardware-check
//! examples in other workspace crates exercise the same fixture values.

use crate::frame::{MAX_PAYLOAD_SIZE, cobs_encode, crc32c};
use crate::message::TYPE_STATUS_REQUEST;
use crate::{
    CURRENT_CAPABILITIES, MAX_PROTOCOL_VERSION, OtaState, PROTOCOL_VERSION, StatusResponse, Tier,
    WifiState,
};

pub fn sample_status_response() -> StatusResponse {
    StatusResponse {
        protocol_version: PROTOCOL_VERSION,
        max_protocol_version: MAX_PROTOCOL_VERSION,
        capabilities: CURRENT_CAPABILITIES,
        firmware_version: "test-device".into(),
        uptime_ms: 100,
        free_heap: 100_000,
        display_width: 448,
        display_height: 368,
        brightness: 200,
        rotation: 90,
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
        last_ota_error: None,
    }
}

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
