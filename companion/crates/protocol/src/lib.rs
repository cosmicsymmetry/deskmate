mod cbor;
mod frame;
mod message;
mod scene;
#[cfg(feature = "test-support")]
pub mod test_support;

pub use frame::{
    Deframer, Frame, FrameError, MAX_DECODED_FRAME, MAX_PAYLOAD_SIZE, MAX_WIRE_FRAME, crc32c,
    decode_wire_frame, encode_frame,
};
pub use message::{
    ASSET_DIGEST_LEN, Ack, ActivateScreen, ApplyConfig, AssetBegin, AssetChunk, AssetCommit,
    AssetKind, AssetRelease, CAPABILITY_ASSET_TRANSFER, CAPABILITY_CONFIG_ROTATION,
    CAPABILITY_CORE_WIDGETS, CAPABILITY_DASHBOARD_LAYOUTS, CAPABILITY_EXTENDED_TEMPLATES,
    CAPABILITY_FIRMWARE_UPDATE, CAPABILITY_HOST_TAP_ACTIONS, CAPABILITY_NETWORKING,
    CAPABILITY_SCENE_RENDER, CURRENT_CAPABILITIES, DeviceEvent, ErrorCode, ErrorResponse,
    EventAction, EventKind, Field, FieldValue, HeartbeatAck, InterruptPolicy, LEGACY_CAPABILITIES,
    LINK_TIMEOUT_MS, MAX_ASSET_CHUNK_BYTES, MAX_ASSET_DIGESTS, MAX_ASSET_TOTAL_LENGTH,
    MAX_CARD_ID_LEN, MAX_CONFIG_SCREENS, MAX_CONFIG_WIDGETS, MAX_DEVICE_ID_LEN,
    MAX_DEVICE_TOKEN_LEN, MAX_DIAGNOSTIC_LEN, MAX_FIELD_COUNT, MAX_FIELD_KEY_LEN,
    MAX_FIELD_TEXT_LEN, MAX_FIRMWARE_VERSION_LEN, MAX_INTERRUPT_REASON_LEN, MAX_IP_LEN,
    MAX_PROTOCOL_VERSION, MAX_PSK_LEN, MAX_SCREEN_ID_LEN, MAX_SERVER_URL_LEN, MAX_SSID_LEN,
    MAX_WIDGET_ID_LEN, Message, MessageError, NetworkConfig, OtaState, PROTOCOL_VERSION, PushData,
    PushScene, ScreenConfig, SizeClass, StatusResponse, TYPE_ACK, TYPE_ACTIVATE_SCREEN,
    TYPE_APPLY_CONFIG, TYPE_ASSET_BEGIN, TYPE_ASSET_CHUNK, TYPE_ASSET_COMMIT, TYPE_ASSET_RELEASE,
    TYPE_DEVICE_EVENT, TYPE_ERROR, TYPE_FACTORY_RESET, TYPE_HEARTBEAT, TYPE_HEARTBEAT_ACK,
    TYPE_NETWORK_CONFIG, TYPE_PUSH_DATA, TYPE_PUSH_SCENE, TYPE_STATUS_REQUEST,
    TYPE_STATUS_RESPONSE, TYPE_TIME_SYNC, TYPE_TRIGGER_INTERRUPT, TapAction, TemplateKind, Tier,
    TimeSync, TriggerInterrupt, WidgetConfig, WifiState, decode_message, encode_message,
    template_kind_from_wire, validate_message,
};
pub use scene::{
    MAX_SCENE_BINDING_LEN, MAX_SCENE_GLYPH_NAME_LEN, MAX_SCENE_LINE_POINTS, MAX_SCENE_NODES,
    MAX_SCENE_SCALE_TICKS, MAX_SCENE_TEXT_LEN, SCENE_CANVAS_HEIGHT, SCENE_CANVAS_WIDTH, Scene,
    SceneAlign, SceneArc, SceneClipRect, SceneFont, SceneFontTier, SceneGlyph, SceneImage,
    SceneLabel, SceneLabelAnchor, SceneLine, SceneNode, SceneRect, SceneRotRect, SceneScale,
    SceneText, SceneValue, binding_is_valid, encode_scene_payload, validate_scene,
};
