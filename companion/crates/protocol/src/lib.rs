mod cbor;
mod frame;
mod message;

pub use frame::{
    Deframer, Frame, FrameError, MAX_DECODED_FRAME, MAX_PAYLOAD_SIZE, MAX_WIRE_FRAME, crc32c,
    decode_wire_frame, encode_frame,
};
pub use message::{
    Ack, ActivateScreen, ApplyConfig, DeviceEvent, ErrorCode, ErrorResponse, EventAction,
    EventKind, Field, FieldValue, HeartbeatAck, InterruptPolicy, LINK_TIMEOUT_MS,
    MAX_CONFIG_SCREENS, MAX_CONFIG_WIDGETS, MAX_DIAGNOSTIC_LEN, MAX_FIELD_COUNT, MAX_FIELD_KEY_LEN,
    MAX_FIELD_TEXT_LEN, MAX_FIRMWARE_VERSION_LEN, MAX_INTERRUPT_REASON_LEN, MAX_SCREEN_ID_LEN,
    MAX_WIDGET_ID_LEN, Message, MessageError, PROTOCOL_VERSION, PushData, ScreenConfig, SizeClass,
    StatusResponse, TYPE_ACK, TYPE_ACTIVATE_SCREEN, TYPE_APPLY_CONFIG, TYPE_DEVICE_EVENT,
    TYPE_ERROR, TYPE_HEARTBEAT, TYPE_HEARTBEAT_ACK, TYPE_PUSH_DATA, TYPE_STATUS_REQUEST,
    TYPE_STATUS_RESPONSE, TYPE_TIME_SYNC, TYPE_TRIGGER_INTERRUPT, TapAction, TemplateKind,
    TimeSync, TriggerInterrupt, WidgetConfig, decode_message, encode_message,
};
