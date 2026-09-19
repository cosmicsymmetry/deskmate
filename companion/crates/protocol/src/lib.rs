mod cbor;
mod frame;
mod message;
mod rle;
mod scene;
#[cfg(feature = "test-support")]
pub mod test_support;
mod util;

pub const PROTOCOL_VERSION: u8 = 2;

pub use frame::{
    Deframer, Frame, FrameError, MAX_DECODED_FRAME, MAX_PAYLOAD_SIZE, MAX_WIRE_FRAME, crc32c,
    decode_wire_frame, encode_frame,
};
pub use message::CAPABILITY_DURABLE_ASSET_ENCODING;
pub use message::{
    ASSET_DIGEST_LEN, ASSET_ENCODING_RAW, ASSET_ENCODING_RLE565, Ack, ActivateCard, ApplyConfig,
    AssetBegin, AssetChunk, AssetCommit, AssetKind, AssetRelease, CAPABILITY_ASSET_TRANSFER,
    CAPABILITY_FIRMWARE_UPDATE, CAPABILITY_NETWORKING, CAPABILITY_SCENE_RENDER,
    CAPABILITY_VOLATILE_ASSETS, CURRENT_CAPABILITIES, CardConfig, DeviceEvent, ErrorCode,
    ErrorResponse, EventAction, EventKind, HeartbeatAck, MAX_ASSET_CHUNK_BYTES, MAX_ASSET_DIGESTS,
    MAX_ASSET_TOTAL_LENGTH, MAX_CARD_ID_LEN, MAX_CONFIG_CARDS, MAX_DEVICE_ID_LEN,
    MAX_DEVICE_TOKEN_LEN, MAX_DIAGNOSTIC_LEN, MAX_FIRMWARE_VERSION_LEN, MAX_INTERRUPT_REASON_LEN,
    MAX_IP_LEN, MAX_PROTOCOL_VERSION, MAX_PSK_LEN, MAX_SERVER_URL_LEN, MAX_SSID_LEN, Message,
    MessageError, NetworkConfig, OtaState, PushScene, PushTimer, StatusResponse, TYPE_ACK,
    TYPE_ACTIVATE_CARD, TYPE_APPLY_CONFIG, TYPE_ASSET_BEGIN, TYPE_ASSET_CHUNK, TYPE_ASSET_COMMIT,
    TYPE_ASSET_RELEASE, TYPE_DEVICE_EVENT, TYPE_ERROR, TYPE_FACTORY_RESET, TYPE_HEARTBEAT,
    TYPE_HEARTBEAT_ACK, TYPE_NETWORK_CONFIG, TYPE_PUSH_SCENE, TYPE_PUSH_TIMER, TYPE_STATUS_REQUEST,
    TYPE_STATUS_RESPONSE, TYPE_TIME_SYNC, TYPE_TRIGGER_INTERRUPT, TapAction, Tier, TimeSync,
    TriggerInterrupt, VOLATILE_IMAGE_DECODED_LENGTH, WifiState, decode_message, encode_message,
    expected_response_type, validate_message,
};
pub use rle::{Rle565Error, decode_rle565, encode_rle565};
pub use scene::{
    MAX_SCENE_BINDING_LEN, MAX_SCENE_GLYPH_NAME_LEN, MAX_SCENE_LINE_POINTS, MAX_SCENE_NODES,
    MAX_SCENE_SCALE_TICKS, MAX_SCENE_TEXT_LEN, SCENE_CANVAS_HEIGHT, SCENE_CANVAS_WIDTH, Scene,
    SceneAlign, SceneArc, SceneClipRect, SceneFont, SceneFontTier, SceneGlyph, SceneImage,
    SceneLabel, SceneLabelAnchor, SceneLine, SceneNode, SceneRect, SceneRotRect, SceneScale,
    SceneText, SceneValue, binding_is_valid, encode_scene_payload, validate_scene,
};
pub use util::{RequestIdAllocator, digest_hex, truncate_utf8_to_bytes};
