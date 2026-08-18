#![allow(clippy::struct_excessive_bools)]

use core::fmt;

use crate::cbor::{CborError, Decoder, Encoder, deterministic_key_before};
use crate::frame::{Frame, FrameError, MAX_PAYLOAD_SIZE, encode_frame};

pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_PROTOCOL_VERSION: u8 = 1;
pub const CAPABILITY_CORE_WIDGETS: u64 = 1 << 0;
pub const CAPABILITY_CONFIG_ROTATION: u64 = 1 << 1;
pub const CAPABILITY_DASHBOARD_LAYOUTS: u64 = 1 << 2;
pub const CAPABILITY_EXTENDED_TEMPLATES: u64 = 1 << 3;
pub const CAPABILITY_HOST_TAP_ACTIONS: u64 = 1 << 4;
pub const CAPABILITY_ASSET_TRANSFER: u64 = 1 << 5;
pub const CAPABILITY_FIRMWARE_UPDATE: u64 = 1 << 6;
pub const CAPABILITY_NETWORKING: u64 = 1 << 7;
pub const LEGACY_CAPABILITIES: u64 = CAPABILITY_CORE_WIDGETS;
pub const CURRENT_CAPABILITIES: u64 =
    CAPABILITY_CORE_WIDGETS | CAPABILITY_CONFIG_ROTATION | CAPABILITY_EXTENDED_TEMPLATES;
pub const LINK_TIMEOUT_MS: u64 = 10_000;
pub const MAX_WIDGET_ID_LEN: usize = 32;
pub const MAX_SCREEN_ID_LEN: usize = 32;
pub const MAX_CONFIG_WIDGETS: usize = 8;
pub const MAX_CONFIG_SCREENS: usize = 8;
pub const MAX_FIELD_COUNT: usize = 16;
pub const MAX_FIELD_KEY_LEN: usize = 32;
pub const MAX_FIELD_TEXT_LEN: usize = 128;
pub const MAX_DIAGNOSTIC_LEN: usize = 96;
pub const MAX_INTERRUPT_REASON_LEN: usize = 96;
pub const MAX_FIRMWARE_VERSION_LEN: usize = 32;
pub const MIN_UNIX_SECONDS: i64 = 1_577_836_800;
pub const MAX_UNIX_SECONDS: i64 = 4_102_444_800;
pub const MIN_UTC_OFFSET_MINUTES: i16 = -840;
pub const MAX_UTC_OFFSET_MINUTES: i16 = 840;
pub const MAX_SSID_LEN: usize = 32;
pub const MAX_PSK_LEN: usize = 64;
pub const MAX_SERVER_URL_LEN: usize = 128;
pub const MAX_DEVICE_TOKEN_LEN: usize = 128;
pub const MAX_DEVICE_ID_LEN: usize = 32;
pub const MAX_IP_LEN: usize = 15;

pub const TYPE_STATUS_REQUEST: u8 = 1;
pub const TYPE_STATUS_RESPONSE: u8 = 2;
pub const TYPE_TIME_SYNC: u8 = 3;
pub const TYPE_ACK: u8 = 4;
pub const TYPE_PUSH_DATA: u8 = 5;
pub const TYPE_HEARTBEAT: u8 = 6;
pub const TYPE_HEARTBEAT_ACK: u8 = 7;
pub const TYPE_ERROR: u8 = 8;
pub const TYPE_APPLY_CONFIG: u8 = 9;
pub const TYPE_ACTIVATE_SCREEN: u8 = 10;
pub const TYPE_TRIGGER_INTERRUPT: u8 = 11;
pub const TYPE_DEVICE_EVENT: u8 = 12;
pub const TYPE_NETWORK_CONFIG: u8 = 13;
pub const TYPE_FACTORY_RESET: u8 = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TemplateKind {
    DigitalClock = 1,
    ProgressRing = 2,
    RowList = 3,
    AnalogClock = 4,
    BigNumberLabel = 5,
    IconBadgeText = 6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SizeClass {
    Full = 1,
    Standard = 2,
    Tile = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TapAction {
    None = 0,
    StartPause = 1,
    Reset = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InterruptPolicy {
    Disabled = 0,
    Enabled = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EventKind {
    Tap = 1,
    Navigation = 2,
    InterruptDismissed = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EventAction {
    None = 0,
    StartPause = 1,
    Reset = 2,
    NavigatePrevious = 3,
    NavigateNext = 4,
    DismissInterrupt = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Tier {
    Local = 0,
    Networked = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WifiState {
    Down = 0,
    Connecting = 1,
    Connected = 2,
    Failed = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OtaState {
    Idle = 0,
    Checking = 1,
    Downloading = 2,
    PendingVerify = 3,
    Failed = 4,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkConfig {
    pub ssid: String,
    pub psk: String,
    pub server_url: String,
    pub device_id: String,
    pub token: String,
    pub utc_offset_minutes: i16,
    pub tier: Tier,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WidgetConfig {
    pub widget_id: String,
    pub template: TemplateKind,
    pub size_class: SizeClass,
    pub tap_action: TapAction,
    pub interrupt_policy: InterruptPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenConfig {
    pub screen_id: String,
    pub widget_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyConfig {
    pub revision: u32,
    pub rotation: u16,
    pub widgets: Vec<WidgetConfig>,
    pub screens: Vec<ScreenConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivateScreen {
    pub screen_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerInterrupt {
    pub widget_id: String,
    pub token: u32,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceEvent {
    pub sequence: u64,
    pub kind: EventKind,
    pub widget_id: String,
    pub screen_id: String,
    pub action: EventAction,
    pub interrupt_token: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    Text(String),
    Integer(i64),
    Boolean(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub key: String,
    pub value: FieldValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushData {
    pub widget_id: String,
    pub revision: u32,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSync {
    pub unix_seconds: i64,
    pub utc_offset_minutes: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ack {
    pub acknowledged_type: u8,
    pub revision: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusResponse {
    pub protocol_version: u8,
    pub max_protocol_version: u8,
    pub capabilities: u64,
    pub firmware_version: String,
    pub uptime_ms: u64,
    pub free_heap: u32,
    pub display_width: u16,
    pub display_height: u16,
    pub brightness: u8,
    pub rotation: u16,
    pub online: bool,
    pub latest_revision: u32,
    pub valid_frames: u32,
    pub malformed_frames: u32,
    pub crc_errors: u32,
    pub overflow_frames: u32,
    pub dropped_responses: u32,
    pub rx_dropped_bytes: u32,
    pub dropped_events: u32,
    pub event_queue_high_water: u32,
    pub dropped_ui_commands: u32,
    pub ui_queue_high_water: u32,
    pub config_revision: u32,
    pub latest_interrupt_token: u32,
    pub tier: Tier,
    pub wifi_state: WifiState,
    pub wifi_rssi: i8,
    pub ip: String,
    pub ota_state: OtaState,
    pub last_network_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeartbeatAck {
    pub uptime_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ErrorCode {
    MalformedFrame = 1,
    BadChecksum = 2,
    VersionMismatch = 3,
    UnsupportedMessage = 4,
    InvalidPayload = 5,
    InvalidTime = 6,
    StaleRevision = 7,
    Busy = 8,
    Internal = 9,
    UnknownWidget = 10,
    UnknownScreen = 11,
    UnsupportedTemplate = 12,
    UnsupportedSizeClass = 13,
    ConfigTooLarge = 14,
    WrongTier = 15,
}

impl TryFrom<u16> for ErrorCode {
    type Error = MessageError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::MalformedFrame),
            2 => Ok(Self::BadChecksum),
            3 => Ok(Self::VersionMismatch),
            4 => Ok(Self::UnsupportedMessage),
            5 => Ok(Self::InvalidPayload),
            6 => Ok(Self::InvalidTime),
            7 => Ok(Self::StaleRevision),
            8 => Ok(Self::Busy),
            9 => Ok(Self::Internal),
            10 => Ok(Self::UnknownWidget),
            11 => Ok(Self::UnknownScreen),
            12 => Ok(Self::UnsupportedTemplate),
            13 => Ok(Self::UnsupportedSizeClass),
            14 => Ok(Self::ConfigTooLarge),
            15 => Ok(Self::WrongTier),
            _ => Err(MessageError::InvalidValue("error code")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorResponse {
    pub code: ErrorCode,
    pub diagnostic: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    StatusRequest,
    StatusResponse(StatusResponse),
    TimeSync(TimeSync),
    Ack(Ack),
    PushData(PushData),
    Heartbeat,
    HeartbeatAck(HeartbeatAck),
    Error(ErrorResponse),
    ApplyConfig(ApplyConfig),
    ActivateScreen(ActivateScreen),
    TriggerInterrupt(TriggerInterrupt),
    DeviceEvent(DeviceEvent),
    NetworkConfig(NetworkConfig),
    FactoryReset,
}

impl Message {
    #[must_use]
    pub const fn type_id(&self) -> u8 {
        match self {
            Self::StatusRequest => TYPE_STATUS_REQUEST,
            Self::StatusResponse(_) => TYPE_STATUS_RESPONSE,
            Self::TimeSync(_) => TYPE_TIME_SYNC,
            Self::Ack(_) => TYPE_ACK,
            Self::PushData(_) => TYPE_PUSH_DATA,
            Self::Heartbeat => TYPE_HEARTBEAT,
            Self::HeartbeatAck(_) => TYPE_HEARTBEAT_ACK,
            Self::Error(_) => TYPE_ERROR,
            Self::ApplyConfig(_) => TYPE_APPLY_CONFIG,
            Self::ActivateScreen(_) => TYPE_ACTIVATE_SCREEN,
            Self::TriggerInterrupt(_) => TYPE_TRIGGER_INTERRUPT,
            Self::DeviceEvent(_) => TYPE_DEVICE_EVENT,
            Self::NetworkConfig(_) => TYPE_NETWORK_CONFIG,
            Self::FactoryReset => TYPE_FACTORY_RESET,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageError {
    Frame(FrameError),
    Cbor(CborError),
    Version(u8),
    UnsupportedType(u8),
    InvalidValue(&'static str),
    MissingField(u64),
    DuplicateOrUnsortedKey,
    PayloadTooLarge,
    InvalidRequestId,
    UnsupportedTemplate(u8),
    UnsupportedSizeClass(u8),
    ConfigTooLarge,
    UnknownWidgetReference,
}

impl fmt::Display for MessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for MessageError {}

impl From<CborError> for MessageError {
    fn from(value: CborError) -> Self {
        Self::Cbor(value)
    }
}

impl From<FrameError> for MessageError {
    fn from(value: FrameError) -> Self {
        Self::Frame(value)
    }
}

fn checked_text(
    value: &str,
    min: usize,
    max: usize,
    name: &'static str,
) -> Result<(), MessageError> {
    if (min..=max).contains(&value.len()) {
        Ok(())
    } else {
        Err(MessageError::InvalidValue(name))
    }
}

/// Decodes a wire-format template kind byte into a [`TemplateKind`].
///
/// # Errors
///
/// Returns [`MessageError::UnsupportedTemplate`] for any value outside the
/// known template range; unknown kinds are rejected, never clamped.
pub fn template_kind_from_wire(value: u8) -> Result<TemplateKind, MessageError> {
    match value {
        1 => Ok(TemplateKind::DigitalClock),
        2 => Ok(TemplateKind::ProgressRing),
        3 => Ok(TemplateKind::RowList),
        4 => Ok(TemplateKind::AnalogClock),
        5 => Ok(TemplateKind::BigNumberLabel),
        6 => Ok(TemplateKind::IconBadgeText),
        other => Err(MessageError::UnsupportedTemplate(other)),
    }
}

fn size_class(value: u8) -> Result<SizeClass, MessageError> {
    match value {
        1 => Ok(SizeClass::Full),
        2 => Ok(SizeClass::Standard),
        3 => Ok(SizeClass::Tile),
        other => Err(MessageError::UnsupportedSizeClass(other)),
    }
}

fn tap_action(value: u8) -> Result<TapAction, MessageError> {
    match value {
        0 => Ok(TapAction::None),
        1 => Ok(TapAction::StartPause),
        2 => Ok(TapAction::Reset),
        _ => Err(MessageError::InvalidValue("tap action")),
    }
}

fn interrupt_policy(value: u8) -> Result<InterruptPolicy, MessageError> {
    match value {
        0 => Ok(InterruptPolicy::Disabled),
        1 => Ok(InterruptPolicy::Enabled),
        _ => Err(MessageError::InvalidValue("interrupt policy")),
    }
}

fn tier_from_wire(value: u8) -> Result<Tier, MessageError> {
    match value {
        0 => Ok(Tier::Local),
        1 => Ok(Tier::Networked),
        _ => Err(MessageError::InvalidValue("tier")),
    }
}

fn wifi_state_from_wire(value: u8) -> Result<WifiState, MessageError> {
    match value {
        0 => Ok(WifiState::Down),
        1 => Ok(WifiState::Connecting),
        2 => Ok(WifiState::Connected),
        3 => Ok(WifiState::Failed),
        _ => Err(MessageError::InvalidValue("wifi state")),
    }
}

fn ota_state_from_wire(value: u8) -> Result<OtaState, MessageError> {
    match value {
        0 => Ok(OtaState::Idle),
        1 => Ok(OtaState::Checking),
        2 => Ok(OtaState::Downloading),
        3 => Ok(OtaState::PendingVerify),
        4 => Ok(OtaState::Failed),
        _ => Err(MessageError::InvalidValue("ota state")),
    }
}

fn event_kind(value: u8) -> Result<EventKind, MessageError> {
    match value {
        1 => Ok(EventKind::Tap),
        2 => Ok(EventKind::Navigation),
        3 => Ok(EventKind::InterruptDismissed),
        _ => Err(MessageError::InvalidValue("event kind")),
    }
}

fn event_action(value: u8) -> Result<EventAction, MessageError> {
    match value {
        0 => Ok(EventAction::None),
        1 => Ok(EventAction::StartPause),
        2 => Ok(EventAction::Reset),
        3 => Ok(EventAction::NavigatePrevious),
        4 => Ok(EventAction::NavigateNext),
        5 => Ok(EventAction::DismissInterrupt),
        _ => Err(MessageError::InvalidValue("event action")),
    }
}

fn validate_network_config(config: &NetworkConfig) -> Result<(), MessageError> {
    checked_text(&config.ssid, 0, MAX_SSID_LEN, "ssid")?;
    checked_text(&config.psk, 0, MAX_PSK_LEN, "psk")?;
    checked_text(&config.server_url, 0, MAX_SERVER_URL_LEN, "server_url")?;
    checked_text(&config.device_id, 0, MAX_DEVICE_ID_LEN, "device_id")?;
    checked_text(&config.token, 0, MAX_DEVICE_TOKEN_LEN, "token")?;
    if !(MIN_UTC_OFFSET_MINUTES..=MAX_UTC_OFFSET_MINUTES).contains(&config.utc_offset_minutes) {
        return Err(MessageError::InvalidValue("UTC offset"));
    }
    Ok(())
}

fn validate_apply_config(config: &ApplyConfig) -> Result<(), MessageError> {
    if config.revision == 0 {
        return Err(MessageError::InvalidValue("config revision"));
    }
    if config.widgets.is_empty() || config.screens.is_empty() {
        return Err(MessageError::InvalidValue("config count"));
    }
    if config.widgets.len() > MAX_CONFIG_WIDGETS || config.screens.len() > MAX_CONFIG_SCREENS {
        return Err(MessageError::ConfigTooLarge);
    }
    if !matches!(config.rotation, 90 | 270) {
        return Err(MessageError::InvalidValue("config rotation"));
    }
    for (index, widget) in config.widgets.iter().enumerate() {
        checked_text(&widget.widget_id, 1, MAX_WIDGET_ID_LEN, "widget_id")?;
        if config.widgets[..index]
            .iter()
            .any(|other| other.widget_id == widget.widget_id)
        {
            return Err(MessageError::DuplicateOrUnsortedKey);
        }
        if widget.size_class == SizeClass::Tile {
            return Err(MessageError::UnsupportedSizeClass(SizeClass::Tile as u8));
        }
        if widget.template != TemplateKind::ProgressRing && widget.tap_action != TapAction::None {
            return Err(MessageError::InvalidValue("template tap action"));
        }
    }
    for (index, screen) in config.screens.iter().enumerate() {
        checked_text(&screen.screen_id, 1, MAX_SCREEN_ID_LEN, "screen_id")?;
        checked_text(&screen.widget_id, 1, MAX_WIDGET_ID_LEN, "widget_id")?;
        if config.screens[..index]
            .iter()
            .any(|other| other.screen_id == screen.screen_id)
        {
            return Err(MessageError::DuplicateOrUnsortedKey);
        }
        if !config
            .widgets
            .iter()
            .any(|widget| widget.widget_id == screen.widget_id)
        {
            return Err(MessageError::UnknownWidgetReference);
        }
    }
    Ok(())
}

fn validate_device_event(event: &DeviceEvent) -> Result<(), MessageError> {
    if event.sequence == 0 {
        return Err(MessageError::InvalidValue("event sequence"));
    }
    checked_text(&event.widget_id, 1, MAX_WIDGET_ID_LEN, "widget_id")?;
    checked_text(&event.screen_id, 1, MAX_SCREEN_ID_LEN, "screen_id")?;
    let valid = match event.kind {
        EventKind::Tap => {
            matches!(event.action, EventAction::StartPause | EventAction::Reset)
                && event.interrupt_token.is_none()
        }
        EventKind::Navigation => {
            matches!(
                event.action,
                EventAction::NavigatePrevious | EventAction::NavigateNext
            ) && event.interrupt_token.is_none()
        }
        EventKind::InterruptDismissed => {
            event.action == EventAction::DismissInterrupt
                && event.interrupt_token.is_some_and(|token| token != 0)
        }
    };
    if valid {
        Ok(())
    } else {
        Err(MessageError::InvalidValue("event kind/action/token"))
    }
}

fn validate_push(push: &PushData) -> Result<(), MessageError> {
    checked_text(&push.widget_id, 1, MAX_WIDGET_ID_LEN, "widget_id")?;
    if push.revision == 0 {
        return Err(MessageError::InvalidValue("revision"));
    }
    if push.fields.len() > MAX_FIELD_COUNT {
        return Err(MessageError::InvalidValue("field count"));
    }
    let mut keys: Vec<&str> = push.fields.iter().map(|field| field.key.as_str()).collect();
    for field in &push.fields {
        checked_text(&field.key, 1, MAX_FIELD_KEY_LEN, "field key")?;
        if let FieldValue::Text(text) = &field.value {
            checked_text(text, 0, MAX_FIELD_TEXT_LEN, "field text")?;
        }
    }
    keys.sort_unstable();
    if keys.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(MessageError::DuplicateOrUnsortedKey);
    }
    Ok(())
}

fn validate_message(message: &Message) -> Result<(), MessageError> {
    match message {
        Message::StatusResponse(status) => {
            if status.protocol_version != PROTOCOL_VERSION {
                return Err(MessageError::InvalidValue("status protocol version"));
            }
            if status.max_protocol_version < status.protocol_version {
                return Err(MessageError::InvalidValue("maximum protocol version"));
            }
            checked_text(
                &status.firmware_version,
                1,
                MAX_FIRMWARE_VERSION_LEN,
                "firmware version",
            )?;
            if !matches!(status.rotation, 0 | 90 | 180 | 270) {
                return Err(MessageError::InvalidValue("rotation"));
            }
            checked_text(&status.ip, 0, MAX_IP_LEN, "ip")?;
            if let Some(diagnostic) = &status.last_network_error {
                checked_text(diagnostic, 0, MAX_DIAGNOSTIC_LEN, "last network error")?;
            }
            Ok(())
        }
        Message::TimeSync(sync) => {
            if !(MIN_UNIX_SECONDS..=MAX_UNIX_SECONDS).contains(&sync.unix_seconds) {
                return Err(MessageError::InvalidValue("unix seconds"));
            }
            if !(MIN_UTC_OFFSET_MINUTES..=MAX_UTC_OFFSET_MINUTES).contains(&sync.utc_offset_minutes)
            {
                return Err(MessageError::InvalidValue("UTC offset"));
            }
            Ok(())
        }
        Message::Ack(ack) => {
            let revision_required =
                matches!(ack.acknowledged_type, TYPE_PUSH_DATA | TYPE_APPLY_CONFIG);
            if !matches!(
                ack.acknowledged_type,
                TYPE_TIME_SYNC
                    | TYPE_PUSH_DATA
                    | TYPE_APPLY_CONFIG
                    | TYPE_ACTIVATE_SCREEN
                    | TYPE_TRIGGER_INTERRUPT
            ) {
                return Err(MessageError::InvalidValue("acknowledged type"));
            }
            if revision_required != ack.revision.is_some() {
                return Err(MessageError::InvalidValue("ack revision"));
            }
            if ack.revision == Some(0) {
                return Err(MessageError::InvalidValue("ack revision"));
            }
            Ok(())
        }
        Message::PushData(push) => validate_push(push),
        Message::Error(error) => {
            checked_text(&error.diagnostic, 0, MAX_DIAGNOSTIC_LEN, "diagnostic")
        }
        Message::ApplyConfig(config) => validate_apply_config(config),
        Message::ActivateScreen(activate) => {
            checked_text(&activate.screen_id, 1, MAX_SCREEN_ID_LEN, "screen_id")
        }
        Message::TriggerInterrupt(interrupt) => {
            checked_text(&interrupt.widget_id, 1, MAX_WIDGET_ID_LEN, "widget_id")?;
            checked_text(
                &interrupt.reason,
                0,
                MAX_INTERRUPT_REASON_LEN,
                "interrupt reason",
            )?;
            if interrupt.token == 0 {
                Err(MessageError::InvalidValue("interrupt token"))
            } else {
                Ok(())
            }
        }
        Message::DeviceEvent(event) => validate_device_event(event),
        Message::NetworkConfig(config) => validate_network_config(config),
        _ => Ok(()),
    }
}

fn encode_field_value(encoder: &mut Encoder, value: &FieldValue) {
    match value {
        FieldValue::Text(text) => encoder.text(text),
        FieldValue::Integer(integer) => encoder.signed(*integer),
        FieldValue::Boolean(boolean) => encoder.boolean(*boolean),
    }
}

fn encoded_text_key(key: &str) -> Vec<u8> {
    let mut encoder = Encoder::new();
    encoder.text(key);
    encoder.into_bytes()
}

fn encode_config_payload(encoder: &mut Encoder, config: &ApplyConfig) {
    encoder.map(4);
    encoder.unsigned(0);
    encoder.unsigned(u64::from(config.revision));
    encoder.unsigned(1);
    encoder.array(config.widgets.len());
    for widget in &config.widgets {
        encoder.map(5);
        encoder.unsigned(0);
        encoder.text(&widget.widget_id);
        encoder.unsigned(1);
        encoder.unsigned(u64::from(widget.template as u8));
        encoder.unsigned(2);
        encoder.unsigned(u64::from(widget.size_class as u8));
        encoder.unsigned(3);
        encoder.unsigned(u64::from(widget.tap_action as u8));
        encoder.unsigned(4);
        encoder.unsigned(u64::from(widget.interrupt_policy as u8));
    }
    encoder.unsigned(2);
    encoder.array(config.screens.len());
    for screen in &config.screens {
        encoder.map(2);
        encoder.unsigned(0);
        encoder.text(&screen.screen_id);
        encoder.unsigned(1);
        encoder.text(&screen.widget_id);
    }
    encoder.unsigned(3);
    encoder.unsigned(u64::from(config.rotation));
}

fn encode_device_event_payload(encoder: &mut Encoder, event: &DeviceEvent) {
    encoder.map(if event.interrupt_token.is_some() {
        6
    } else {
        5
    });
    encoder.unsigned(0);
    encoder.unsigned(event.sequence);
    encoder.unsigned(1);
    encoder.unsigned(u64::from(event.kind as u8));
    encoder.unsigned(2);
    encoder.text(&event.widget_id);
    encoder.unsigned(3);
    encoder.text(&event.screen_id);
    encoder.unsigned(4);
    encoder.unsigned(u64::from(event.action as u8));
    if let Some(token) = event.interrupt_token {
        encoder.unsigned(5);
        encoder.unsigned(u64::from(token));
    }
}

fn encode_status_payload(encoder: &mut Encoder, status: &StatusResponse) {
    let entry_count = 24 + 5 + usize::from(status.last_network_error.is_some());
    encoder.map(entry_count);
    encoder.unsigned(0);
    encoder.unsigned(u64::from(status.protocol_version));
    encoder.unsigned(1);
    encoder.text(&status.firmware_version);
    let unsigned_fields = [
        status.uptime_ms,
        u64::from(status.free_heap),
        u64::from(status.display_width),
        u64::from(status.display_height),
        u64::from(status.brightness),
        u64::from(status.rotation),
        u64::from(u8::from(status.online)),
        u64::from(status.latest_revision),
        u64::from(status.valid_frames),
        u64::from(status.malformed_frames),
        u64::from(status.crc_errors),
        u64::from(status.overflow_frames),
        u64::from(status.dropped_responses),
        u64::from(status.rx_dropped_bytes),
        u64::from(status.dropped_events),
        u64::from(status.event_queue_high_water),
        u64::from(status.dropped_ui_commands),
        u64::from(status.ui_queue_high_water),
        u64::from(status.config_revision),
        u64::from(status.latest_interrupt_token),
        u64::from(status.max_protocol_version),
        status.capabilities,
    ];
    for (offset, value) in unsigned_fields.into_iter().enumerate() {
        encoder.unsigned(u64::try_from(offset + 2).unwrap());
        encoder.unsigned(value);
    }
    encoder.unsigned(24);
    encoder.unsigned(u64::from(status.tier as u8));
    encoder.unsigned(25);
    encoder.unsigned(u64::from(status.wifi_state as u8));
    encoder.unsigned(26);
    encoder.signed(i64::from(status.wifi_rssi));
    encoder.unsigned(27);
    encoder.text(&status.ip);
    encoder.unsigned(28);
    encoder.unsigned(u64::from(status.ota_state as u8));
    if let Some(diagnostic) = &status.last_network_error {
        encoder.unsigned(29);
        encoder.text(diagnostic);
    }
}

fn encode_network_config_payload(encoder: &mut Encoder, config: &NetworkConfig) {
    encoder.map(7);
    encoder.unsigned(1);
    encoder.text(&config.ssid);
    encoder.unsigned(2);
    encoder.text(&config.psk);
    encoder.unsigned(3);
    encoder.text(&config.server_url);
    encoder.unsigned(4);
    encoder.text(&config.device_id);
    encoder.unsigned(5);
    encoder.text(&config.token);
    encoder.unsigned(6);
    encoder.signed(i64::from(config.utc_offset_minutes));
    encoder.unsigned(7);
    encoder.unsigned(u64::from(config.tier as u8));
}

fn encode_payload(message: &Message) -> Result<Vec<u8>, MessageError> {
    validate_message(message)?;
    let mut encoder = Encoder::new();
    match message {
        Message::StatusRequest | Message::Heartbeat | Message::FactoryReset => encoder.map(0),
        Message::TimeSync(sync) => {
            encoder.map(2);
            encoder.unsigned(0);
            encoder.signed(sync.unix_seconds);
            encoder.unsigned(1);
            encoder.signed(i64::from(sync.utc_offset_minutes));
        }
        Message::PushData(push) => {
            encoder.map(3);
            encoder.unsigned(0);
            encoder.text(&push.widget_id);
            encoder.unsigned(1);
            encoder.unsigned(u64::from(push.revision));
            encoder.unsigned(2);
            encoder.map(push.fields.len());
            let mut fields: Vec<(&Field, Vec<u8>)> = push
                .fields
                .iter()
                .map(|field| (field, encoded_text_key(&field.key)))
                .collect();
            fields.sort_by(|a, b| a.1.len().cmp(&b.1.len()).then_with(|| a.1.cmp(&b.1)));
            for (field, _) in fields {
                encoder.text(&field.key);
                encode_field_value(&mut encoder, &field.value);
            }
        }
        Message::ApplyConfig(config) => encode_config_payload(&mut encoder, config),
        Message::ActivateScreen(activate) => {
            encoder.map(1);
            encoder.unsigned(0);
            encoder.text(&activate.screen_id);
        }
        Message::TriggerInterrupt(interrupt) => {
            encoder.map(3);
            encoder.unsigned(0);
            encoder.text(&interrupt.widget_id);
            encoder.unsigned(1);
            encoder.unsigned(u64::from(interrupt.token));
            encoder.unsigned(2);
            encoder.text(&interrupt.reason);
        }
        Message::DeviceEvent(event) => encode_device_event_payload(&mut encoder, event),
        Message::Ack(ack) => {
            encoder.map(if ack.revision.is_some() { 2 } else { 1 });
            encoder.unsigned(0);
            encoder.unsigned(u64::from(ack.acknowledged_type));
            if let Some(revision) = ack.revision {
                encoder.unsigned(1);
                encoder.unsigned(u64::from(revision));
            }
        }
        Message::HeartbeatAck(ack) => {
            encoder.map(1);
            encoder.unsigned(0);
            encoder.unsigned(ack.uptime_ms);
        }
        Message::Error(error) => {
            encoder.map(2);
            encoder.unsigned(0);
            encoder.unsigned(u64::from(error.code as u16));
            encoder.unsigned(1);
            encoder.text(&error.diagnostic);
        }
        Message::StatusResponse(status) => encode_status_payload(&mut encoder, status),
        Message::NetworkConfig(config) => encode_network_config_payload(&mut encoder, config),
    }
    let payload = encoder.into_bytes();
    if payload.len() > MAX_PAYLOAD_SIZE {
        Err(MessageError::PayloadTooLarge)
    } else {
        Ok(payload)
    }
}

pub fn encode_message(request_id: u32, message: &Message) -> Result<Vec<u8>, MessageError> {
    if (request_id == 0) != matches!(message, Message::DeviceEvent(_)) {
        return Err(MessageError::InvalidRequestId);
    }
    let payload = encode_payload(message)?;
    Ok(encode_frame(&Frame::new(
        message.type_id(),
        request_id,
        payload,
    ))?)
}

fn next_numeric_key(
    decoder: &mut Decoder<'_>,
    previous: &mut Option<u64>,
) -> Result<u64, MessageError> {
    let key = decoder.unsigned()?;
    if previous.is_some_and(|last| key <= last) {
        return Err(MessageError::DuplicateOrUnsortedKey);
    }
    *previous = Some(key);
    Ok(key)
}

fn read_u8(decoder: &mut Decoder<'_>, name: &'static str) -> Result<u8, MessageError> {
    u8::try_from(decoder.unsigned()?).map_err(|_| MessageError::InvalidValue(name))
}

fn read_u16(decoder: &mut Decoder<'_>, name: &'static str) -> Result<u16, MessageError> {
    u16::try_from(decoder.unsigned()?).map_err(|_| MessageError::InvalidValue(name))
}

fn read_u32(decoder: &mut Decoder<'_>, name: &'static str) -> Result<u32, MessageError> {
    u32::try_from(decoder.unsigned()?).map_err(|_| MessageError::InvalidValue(name))
}

fn require_empty_map(payload: &[u8]) -> Result<(), MessageError> {
    let mut decoder = Decoder::new(payload);
    if decoder.map_len()? != 0 {
        return Err(MessageError::InvalidValue("empty map"));
    }
    decoder.finish()?;
    Ok(())
}

fn decode_time_sync(payload: &[u8]) -> Result<TimeSync, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut seconds = None;
    let mut offset = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => seconds = Some(decoder.signed()?),
            1 => {
                offset = Some(
                    i16::try_from(decoder.signed()?)
                        .map_err(|_| MessageError::InvalidValue("UTC offset"))?,
                );
            }
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    let value = TimeSync {
        unix_seconds: seconds.ok_or(MessageError::MissingField(0))?,
        utc_offset_minutes: offset.ok_or(MessageError::MissingField(1))?,
    };
    validate_message(&Message::TimeSync(value))?;
    Ok(value)
}

fn decode_fields(decoder: &mut Decoder<'_>) -> Result<Vec<Field>, MessageError> {
    let count = decoder.map_len()?;
    if count > MAX_FIELD_COUNT {
        return Err(MessageError::InvalidValue("field count"));
    }
    let mut fields = Vec::with_capacity(count);
    let mut previous_raw: Option<Vec<u8>> = None;
    for _ in 0..count {
        let start = decoder.position();
        let key = decoder.text()?.to_owned();
        let end = decoder.position();
        let raw = decoder.slice(start, end).to_vec();
        if previous_raw
            .as_ref()
            .is_some_and(|previous| !deterministic_key_before(previous, &raw))
        {
            return Err(MessageError::DuplicateOrUnsortedKey);
        }
        previous_raw = Some(raw);
        checked_text(&key, 1, MAX_FIELD_KEY_LEN, "field key")?;
        let value = match decoder.peek_major()? {
            0 | 1 => FieldValue::Integer(decoder.signed()?),
            3 => {
                let text = decoder.text()?.to_owned();
                checked_text(&text, 0, MAX_FIELD_TEXT_LEN, "field text")?;
                FieldValue::Text(text)
            }
            7 => FieldValue::Boolean(decoder.boolean()?),
            _ => return Err(MessageError::InvalidValue("field value")),
        };
        fields.push(Field { key, value });
    }
    Ok(fields)
}

fn decode_push(payload: &[u8]) -> Result<PushData, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut widget_id = None;
    let mut revision = None;
    let mut fields = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => widget_id = Some(decoder.text()?.to_owned()),
            1 => revision = Some(read_u32(&mut decoder, "revision")?),
            2 => fields = Some(decode_fields(&mut decoder)?),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    let value = PushData {
        widget_id: widget_id.ok_or(MessageError::MissingField(0))?,
        revision: revision.ok_or(MessageError::MissingField(1))?,
        fields: fields.ok_or(MessageError::MissingField(2))?,
    };
    validate_push(&value)?;
    Ok(value)
}

fn decode_widgets(decoder: &mut Decoder<'_>) -> Result<Vec<WidgetConfig>, MessageError> {
    let count = decoder.array_len()?;
    if count > MAX_CONFIG_WIDGETS {
        return Err(MessageError::ConfigTooLarge);
    }
    let mut widgets = Vec::with_capacity(count);
    for _ in 0..count {
        let len = decoder.map_len()?;
        let mut previous = None;
        let mut widget_id = None;
        let mut template = None;
        let mut size = None;
        let mut action = None;
        let mut policy = None;
        for _ in 0..len {
            match next_numeric_key(decoder, &mut previous)? {
                0 => widget_id = Some(decoder.text()?.to_owned()),
                1 => template = Some(template_kind_from_wire(read_u8(decoder, "template")?)?),
                2 => size = Some(size_class(read_u8(decoder, "size class")?)?),
                3 => action = Some(tap_action(read_u8(decoder, "tap action")?)?),
                4 => {
                    policy = Some(interrupt_policy(read_u8(decoder, "interrupt policy")?)?);
                }
                _ => decoder.skip()?,
            }
        }
        widgets.push(WidgetConfig {
            widget_id: widget_id.ok_or(MessageError::MissingField(0))?,
            template: template.ok_or(MessageError::MissingField(1))?,
            size_class: size.ok_or(MessageError::MissingField(2))?,
            tap_action: action.ok_or(MessageError::MissingField(3))?,
            interrupt_policy: policy.ok_or(MessageError::MissingField(4))?,
        });
    }
    Ok(widgets)
}

fn decode_screens(decoder: &mut Decoder<'_>) -> Result<Vec<ScreenConfig>, MessageError> {
    let count = decoder.array_len()?;
    if count > MAX_CONFIG_SCREENS {
        return Err(MessageError::ConfigTooLarge);
    }
    let mut screens = Vec::with_capacity(count);
    for _ in 0..count {
        let len = decoder.map_len()?;
        let mut previous = None;
        let mut screen_id = None;
        let mut widget_id = None;
        for _ in 0..len {
            match next_numeric_key(decoder, &mut previous)? {
                0 => screen_id = Some(decoder.text()?.to_owned()),
                1 => widget_id = Some(decoder.text()?.to_owned()),
                _ => decoder.skip()?,
            }
        }
        screens.push(ScreenConfig {
            screen_id: screen_id.ok_or(MessageError::MissingField(0))?,
            widget_id: widget_id.ok_or(MessageError::MissingField(1))?,
        });
    }
    Ok(screens)
}

fn decode_apply_config(payload: &[u8]) -> Result<ApplyConfig, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut revision = None;
    let mut widgets = None;
    let mut screens = None;
    let mut rotation = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => revision = Some(read_u32(&mut decoder, "config revision")?),
            1 => widgets = Some(decode_widgets(&mut decoder)?),
            2 => screens = Some(decode_screens(&mut decoder)?),
            3 => rotation = Some(read_u16(&mut decoder, "config rotation")?),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    let config = ApplyConfig {
        revision: revision.ok_or(MessageError::MissingField(0))?,
        rotation: rotation.unwrap_or(90),
        widgets: widgets.ok_or(MessageError::MissingField(1))?,
        screens: screens.ok_or(MessageError::MissingField(2))?,
    };
    validate_apply_config(&config)?;
    Ok(config)
}

fn decode_network_config(payload: &[u8]) -> Result<NetworkConfig, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut ssid = None;
    let mut psk = None;
    let mut server_url = None;
    let mut device_id = None;
    let mut token = None;
    let mut utc_offset_minutes = None;
    let mut tier = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            1 => ssid = Some(decoder.text()?.to_owned()),
            2 => psk = Some(decoder.text()?.to_owned()),
            3 => server_url = Some(decoder.text()?.to_owned()),
            4 => device_id = Some(decoder.text()?.to_owned()),
            5 => token = Some(decoder.text()?.to_owned()),
            6 => {
                utc_offset_minutes = Some(
                    i16::try_from(decoder.signed()?)
                        .map_err(|_| MessageError::InvalidValue("UTC offset"))?,
                );
            }
            7 => tier = Some(tier_from_wire(read_u8(&mut decoder, "tier")?)?),
            // Unlike the other additive message types, network config is a
            // closed, security-sensitive schema: an unrecognized key is
            // rejected rather than skipped.
            _ => return Err(MessageError::InvalidValue("network config key")),
        }
    }
    decoder.finish()?;
    let config = NetworkConfig {
        ssid: ssid.ok_or(MessageError::MissingField(1))?,
        psk: psk.ok_or(MessageError::MissingField(2))?,
        server_url: server_url.ok_or(MessageError::MissingField(3))?,
        device_id: device_id.ok_or(MessageError::MissingField(4))?,
        token: token.ok_or(MessageError::MissingField(5))?,
        utc_offset_minutes: utc_offset_minutes.ok_or(MessageError::MissingField(6))?,
        tier: tier.ok_or(MessageError::MissingField(7))?,
    };
    validate_network_config(&config)?;
    Ok(config)
}

fn decode_activate_screen(payload: &[u8]) -> Result<ActivateScreen, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut screen_id = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => screen_id = Some(decoder.text()?.to_owned()),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    let activate = ActivateScreen {
        screen_id: screen_id.ok_or(MessageError::MissingField(0))?,
    };
    validate_message(&Message::ActivateScreen(activate.clone()))?;
    Ok(activate)
}

fn decode_trigger_interrupt(payload: &[u8]) -> Result<TriggerInterrupt, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut widget_id = None;
    let mut token = None;
    let mut reason = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => widget_id = Some(decoder.text()?.to_owned()),
            1 => token = Some(read_u32(&mut decoder, "interrupt token")?),
            2 => reason = Some(decoder.text()?.to_owned()),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    let interrupt = TriggerInterrupt {
        widget_id: widget_id.ok_or(MessageError::MissingField(0))?,
        token: token.ok_or(MessageError::MissingField(1))?,
        reason: reason.ok_or(MessageError::MissingField(2))?,
    };
    validate_message(&Message::TriggerInterrupt(interrupt.clone()))?;
    Ok(interrupt)
}

fn decode_device_event(payload: &[u8]) -> Result<DeviceEvent, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut sequence = None;
    let mut kind = None;
    let mut widget_id = None;
    let mut screen_id = None;
    let mut action = None;
    let mut interrupt_token = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => sequence = Some(decoder.unsigned()?),
            1 => kind = Some(event_kind(read_u8(&mut decoder, "event kind")?)?),
            2 => widget_id = Some(decoder.text()?.to_owned()),
            3 => screen_id = Some(decoder.text()?.to_owned()),
            4 => action = Some(event_action(read_u8(&mut decoder, "event action")?)?),
            5 => interrupt_token = Some(read_u32(&mut decoder, "interrupt token")?),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    let event = DeviceEvent {
        sequence: sequence.ok_or(MessageError::MissingField(0))?,
        kind: kind.ok_or(MessageError::MissingField(1))?,
        widget_id: widget_id.ok_or(MessageError::MissingField(2))?,
        screen_id: screen_id.ok_or(MessageError::MissingField(3))?,
        action: action.ok_or(MessageError::MissingField(4))?,
        interrupt_token,
    };
    validate_device_event(&event)?;
    Ok(event)
}

fn decode_ack(payload: &[u8]) -> Result<Ack, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut acknowledged_type = None;
    let mut revision = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => acknowledged_type = Some(read_u8(&mut decoder, "acknowledged type")?),
            1 => revision = Some(read_u32(&mut decoder, "revision")?),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    let value = Ack {
        acknowledged_type: acknowledged_type.ok_or(MessageError::MissingField(0))?,
        revision,
    };
    validate_message(&Message::Ack(value))?;
    Ok(value)
}

fn decode_heartbeat_ack(payload: &[u8]) -> Result<HeartbeatAck, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut uptime_ms = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => uptime_ms = Some(decoder.unsigned()?),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    Ok(HeartbeatAck {
        uptime_ms: uptime_ms.ok_or(MessageError::MissingField(0))?,
    })
}

fn decode_error(payload: &[u8]) -> Result<ErrorResponse, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut code = None;
    let mut diagnostic = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => code = Some(ErrorCode::try_from(read_u16(&mut decoder, "error code")?)?),
            1 => diagnostic = Some(decoder.text()?.to_owned()),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    let value = ErrorResponse {
        code: code.ok_or(MessageError::MissingField(0))?,
        diagnostic: diagnostic.ok_or(MessageError::MissingField(1))?,
    };
    validate_message(&Message::Error(value.clone()))?;
    Ok(value)
}

#[allow(clippy::too_many_lines)]
fn decode_status(payload: &[u8]) -> Result<StatusResponse, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut protocol_version = None;
    let mut firmware_version = None;
    let mut uptime_ms = None;
    let mut free_heap = None;
    let mut display_width = None;
    let mut display_height = None;
    let mut brightness = None;
    let mut rotation = None;
    let mut online = None;
    let mut latest_revision = None;
    let mut valid_frames = None;
    let mut malformed_frames = None;
    let mut crc_errors = None;
    let mut overflow_frames = None;
    let mut dropped_responses = None;
    let mut rx_dropped_bytes = None;
    let mut dropped_events = None;
    let mut event_queue_high_water = None;
    let mut dropped_ui_commands = None;
    let mut ui_queue_high_water = None;
    let mut config_revision = None;
    let mut latest_interrupt_token = None;
    let mut max_protocol_version = None;
    let mut capabilities = None;
    let mut tier = None;
    let mut wifi_state = None;
    let mut wifi_rssi = None;
    let mut ip = None;
    let mut ota_state = None;
    let mut last_network_error = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => protocol_version = Some(read_u8(&mut decoder, "protocol version")?),
            1 => firmware_version = Some(decoder.text()?.to_owned()),
            2 => uptime_ms = Some(decoder.unsigned()?),
            3 => free_heap = Some(read_u32(&mut decoder, "free heap")?),
            4 => display_width = Some(read_u16(&mut decoder, "display width")?),
            5 => display_height = Some(read_u16(&mut decoder, "display height")?),
            6 => brightness = Some(read_u8(&mut decoder, "brightness")?),
            7 => rotation = Some(read_u16(&mut decoder, "rotation")?),
            8 => {
                online = Some(match read_u8(&mut decoder, "link state")? {
                    0 => false,
                    1 => true,
                    _ => return Err(MessageError::InvalidValue("link state")),
                });
            }
            9 => latest_revision = Some(read_u32(&mut decoder, "latest revision")?),
            10 => valid_frames = Some(read_u32(&mut decoder, "valid frames")?),
            11 => malformed_frames = Some(read_u32(&mut decoder, "malformed frames")?),
            12 => crc_errors = Some(read_u32(&mut decoder, "CRC errors")?),
            13 => overflow_frames = Some(read_u32(&mut decoder, "overflow frames")?),
            14 => dropped_responses = Some(read_u32(&mut decoder, "dropped responses")?),
            15 => rx_dropped_bytes = Some(read_u32(&mut decoder, "RX drops")?),
            16 => dropped_events = Some(read_u32(&mut decoder, "dropped events")?),
            17 => {
                event_queue_high_water = Some(read_u32(&mut decoder, "event queue high water")?);
            }
            18 => {
                dropped_ui_commands = Some(read_u32(&mut decoder, "dropped UI commands")?);
            }
            19 => ui_queue_high_water = Some(read_u32(&mut decoder, "UI queue high water")?),
            20 => config_revision = Some(read_u32(&mut decoder, "config revision")?),
            21 => {
                latest_interrupt_token = Some(read_u32(&mut decoder, "latest interrupt token")?);
            }
            22 => {
                max_protocol_version = Some(read_u8(&mut decoder, "maximum protocol version")?);
            }
            23 => capabilities = Some(decoder.unsigned()?),
            24 => tier = Some(tier_from_wire(read_u8(&mut decoder, "tier")?)?),
            25 => wifi_state = Some(wifi_state_from_wire(read_u8(&mut decoder, "wifi state")?)?),
            26 => {
                wifi_rssi = Some(
                    i8::try_from(decoder.signed()?)
                        .map_err(|_| MessageError::InvalidValue("wifi rssi"))?,
                );
            }
            27 => ip = Some(decoder.text()?.to_owned()),
            28 => ota_state = Some(ota_state_from_wire(read_u8(&mut decoder, "ota state")?)?),
            29 => last_network_error = Some(decoder.text()?.to_owned()),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    let value = StatusResponse {
        protocol_version: protocol_version.ok_or(MessageError::MissingField(0))?,
        max_protocol_version: max_protocol_version.unwrap_or(PROTOCOL_VERSION),
        capabilities: capabilities.unwrap_or(LEGACY_CAPABILITIES),
        firmware_version: firmware_version.ok_or(MessageError::MissingField(1))?,
        uptime_ms: uptime_ms.ok_or(MessageError::MissingField(2))?,
        free_heap: free_heap.ok_or(MessageError::MissingField(3))?,
        display_width: display_width.ok_or(MessageError::MissingField(4))?,
        display_height: display_height.ok_or(MessageError::MissingField(5))?,
        brightness: brightness.ok_or(MessageError::MissingField(6))?,
        rotation: rotation.ok_or(MessageError::MissingField(7))?,
        online: online.ok_or(MessageError::MissingField(8))?,
        latest_revision: latest_revision.ok_or(MessageError::MissingField(9))?,
        valid_frames: valid_frames.ok_or(MessageError::MissingField(10))?,
        malformed_frames: malformed_frames.ok_or(MessageError::MissingField(11))?,
        crc_errors: crc_errors.ok_or(MessageError::MissingField(12))?,
        overflow_frames: overflow_frames.ok_or(MessageError::MissingField(13))?,
        dropped_responses: dropped_responses.ok_or(MessageError::MissingField(14))?,
        rx_dropped_bytes: rx_dropped_bytes.ok_or(MessageError::MissingField(15))?,
        dropped_events: dropped_events.unwrap_or(0),
        event_queue_high_water: event_queue_high_water.unwrap_or(0),
        dropped_ui_commands: dropped_ui_commands.unwrap_or(0),
        ui_queue_high_water: ui_queue_high_water.unwrap_or(0),
        config_revision: config_revision.unwrap_or(0),
        latest_interrupt_token: latest_interrupt_token.unwrap_or(0),
        tier: tier.unwrap_or(Tier::Local),
        wifi_state: wifi_state.unwrap_or(WifiState::Down),
        wifi_rssi: wifi_rssi.unwrap_or(0),
        ip: ip.unwrap_or_default(),
        ota_state: ota_state.unwrap_or(OtaState::Idle),
        last_network_error,
    };
    validate_message(&Message::StatusResponse(value.clone()))?;
    Ok(value)
}

pub fn decode_message(frame: &Frame) -> Result<Message, MessageError> {
    if frame.version != PROTOCOL_VERSION {
        return Err(MessageError::Version(frame.version));
    }
    if frame.flags != 0 {
        return Err(MessageError::Frame(FrameError::InvalidFlags));
    }
    if (frame.request_id == 0) != (frame.message_type == TYPE_DEVICE_EVENT) {
        return Err(MessageError::InvalidRequestId);
    }
    let message = match frame.message_type {
        TYPE_STATUS_REQUEST => {
            require_empty_map(&frame.payload)?;
            Message::StatusRequest
        }
        TYPE_STATUS_RESPONSE => Message::StatusResponse(decode_status(&frame.payload)?),
        TYPE_TIME_SYNC => Message::TimeSync(decode_time_sync(&frame.payload)?),
        TYPE_ACK => Message::Ack(decode_ack(&frame.payload)?),
        TYPE_PUSH_DATA => Message::PushData(decode_push(&frame.payload)?),
        TYPE_HEARTBEAT => {
            require_empty_map(&frame.payload)?;
            Message::Heartbeat
        }
        TYPE_HEARTBEAT_ACK => Message::HeartbeatAck(decode_heartbeat_ack(&frame.payload)?),
        TYPE_ERROR => Message::Error(decode_error(&frame.payload)?),
        TYPE_APPLY_CONFIG => Message::ApplyConfig(decode_apply_config(&frame.payload)?),
        TYPE_ACTIVATE_SCREEN => Message::ActivateScreen(decode_activate_screen(&frame.payload)?),
        TYPE_TRIGGER_INTERRUPT => {
            Message::TriggerInterrupt(decode_trigger_interrupt(&frame.payload)?)
        }
        TYPE_DEVICE_EVENT => Message::DeviceEvent(decode_device_event(&frame.payload)?),
        TYPE_NETWORK_CONFIG => Message::NetworkConfig(decode_network_config(&frame.payload)?),
        TYPE_FACTORY_RESET => {
            require_empty_map(&frame.payload)?;
            Message::FactoryReset
        }
        other => return Err(MessageError::UnsupportedType(other)),
    };
    Ok(message)
}

#[cfg(test)]
mod tests {
    use crate::frame::decode_wire_frame;

    use super::*;

    fn status() -> StatusResponse {
        StatusResponse {
            protocol_version: 1,
            max_protocol_version: MAX_PROTOCOL_VERSION,
            capabilities: CURRENT_CAPABILITIES,
            firmware_version: "m1-test".into(),
            uptime_ms: 123_456,
            free_heap: 654_321,
            display_width: 368,
            display_height: 448,
            brightness: 200,
            rotation: 0,
            online: true,
            latest_revision: 7,
            valid_frames: 10,
            malformed_frames: 2,
            crc_errors: 1,
            overflow_frames: 1,
            dropped_responses: 0,
            rx_dropped_bytes: 0,
            dropped_events: 0,
            event_queue_high_water: 0,
            dropped_ui_commands: 0,
            ui_queue_high_water: 0,
            config_revision: 3,
            latest_interrupt_token: 9,
            tier: Tier::Local,
            wifi_state: WifiState::Down,
            wifi_rssi: 0,
            ip: String::new(),
            ota_state: OtaState::Idle,
            last_network_error: None,
        }
    }

    fn round_trip(message: &Message) {
        let request_id = if matches!(message, Message::DeviceEvent(_)) {
            0
        } else {
            42
        };
        let wire = encode_message(request_id, message).unwrap();
        let frame = decode_wire_frame(&wire).unwrap();
        assert_eq!(&decode_message(&frame).unwrap(), message);
    }

    #[test]
    fn every_message_round_trips() {
        round_trip(&Message::StatusRequest);
        round_trip(&Message::StatusResponse(status()));
        round_trip(&Message::TimeSync(TimeSync {
            unix_seconds: 1_775_217_600,
            utc_offset_minutes: 240,
        }));
        round_trip(&Message::Ack(Ack {
            acknowledged_type: TYPE_TIME_SYNC,
            revision: None,
        }));
        round_trip(&Message::PushData(PushData {
            widget_id: "weather".into(),
            revision: 7,
            fields: vec![
                Field {
                    key: "ok".into(),
                    value: FieldValue::Boolean(true),
                },
                Field {
                    key: "temp".into(),
                    value: FieldValue::Integer(23),
                },
                Field {
                    key: "summary".into(),
                    value: FieldValue::Text("Clear".into()),
                },
            ],
        }));
        round_trip(&Message::Heartbeat);
        round_trip(&Message::HeartbeatAck(HeartbeatAck { uptime_ms: 99 }));
        round_trip(&Message::Error(ErrorResponse {
            code: ErrorCode::StaleRevision,
            diagnostic: "stale revision".into(),
        }));
        round_trip(&Message::ApplyConfig(ApplyConfig {
            revision: 3,
            rotation: 270,
            widgets: vec![WidgetConfig {
                widget_id: "timer".into(),
                template: TemplateKind::ProgressRing,
                size_class: SizeClass::Standard,
                tap_action: TapAction::StartPause,
                interrupt_policy: InterruptPolicy::Enabled,
            }],
            screens: vec![ScreenConfig {
                screen_id: "focus".into(),
                widget_id: "timer".into(),
            }],
        }));
        round_trip(&Message::ActivateScreen(ActivateScreen {
            screen_id: "focus".into(),
        }));
        round_trip(&Message::TriggerInterrupt(TriggerInterrupt {
            widget_id: "timer".into(),
            token: 4,
            reason: "done".into(),
        }));
        round_trip(&Message::DeviceEvent(DeviceEvent {
            sequence: 5,
            kind: EventKind::InterruptDismissed,
            widget_id: "timer".into(),
            screen_id: "focus".into(),
            action: EventAction::DismissInterrupt,
            interrupt_token: Some(4),
        }));
    }

    #[test]
    fn network_config_round_trips() {
        let message = Message::NetworkConfig(NetworkConfig {
            ssid: "home-network".into(),
            psk: "correct horse battery staple".into(),
            server_url: "wss://deskmate.example.com/v1/device/link".into(),
            device_id: "dev-0001".into(),
            token: "t".repeat(MAX_DEVICE_TOKEN_LEN),
            utc_offset_minutes: 240,
            tier: Tier::Networked,
        });
        round_trip(&message);
    }

    #[test]
    fn network_config_rejects_oversized_ssid() {
        let message = Message::NetworkConfig(NetworkConfig {
            ssid: "s".repeat(MAX_SSID_LEN + 1),
            psk: String::new(),
            server_url: "wss://example.com/l".into(),
            device_id: "dev-0001".into(),
            token: "token".into(),
            utc_offset_minutes: 0,
            tier: Tier::Networked,
        });
        assert!(encode_message(8, &message).is_err());
    }

    #[test]
    fn factory_reset_round_trips() {
        round_trip(&Message::FactoryReset);
    }

    #[test]
    fn cardinal_display_rotations_are_valid() {
        for rotation in [0, 90, 180, 270] {
            let mut value = status();
            value.rotation = rotation;
            round_trip(&Message::StatusResponse(value));
        }

        let mut value = status();
        value.rotation = 45;
        assert!(encode_message(1, &Message::StatusResponse(value)).is_err());
    }

    #[test]
    fn status_response_network_fields_round_trip() {
        let mut status = status();
        status.tier = Tier::Networked;
        status.wifi_state = WifiState::Connected;
        status.wifi_rssi = -58;
        status.ip = "192.168.1.42".into();
        status.ota_state = OtaState::Idle;
        let wire = encode_message(3, &Message::StatusResponse(status.clone())).expect("encode");
        let frame = decode_wire_frame(&wire).expect("decode frame");
        let Message::StatusResponse(decoded) = decode_message(&frame).expect("decode") else {
            panic!("wrong variant");
        };
        assert_eq!(decoded, status);
    }

    #[test]
    fn legacy_status_without_additive_m3_or_m4_fields_remains_compatible() {
        let payload = encode_payload(&Message::StatusResponse(status())).unwrap();
        // The additive networking fields (M4-and-later Task 1) keep the entry
        // count at or above 24, so the map header stays in its two-byte
        // extended form regardless of the exact count.
        assert_eq!(payload[0], 0xb8, "status should use the extended map form");
        let original_count = payload[1];
        // Keys 21 (latest_interrupt_token), 22 (max_protocol_version), and 23
        // (capabilities) are still encoded contiguously and in this order
        // because keys are canonical; locate and drop them regardless of
        // what now follows them on the wire.
        let pattern = [0x15, 0x09, 0x16, 0x01, 0x17, 0x0b];
        let offset = payload
            .windows(pattern.len())
            .position(|window| window == pattern)
            .expect("legacy-optional status fields must be contiguous");
        let mut truncated = payload;
        truncated.drain(offset..offset + pattern.len());
        truncated[1] = original_count - 3;

        let decoded = decode_message(&Frame::new(TYPE_STATUS_RESPONSE, 1, truncated)).unwrap();
        let Message::StatusResponse(decoded) = decoded else {
            panic!("decoded message should remain a status response");
        };
        assert_eq!(decoded.latest_interrupt_token, 0);
        assert_eq!(decoded.max_protocol_version, PROTOCOL_VERSION);
        assert_eq!(decoded.capabilities, LEGACY_CAPABILITIES);
    }

    #[test]
    fn capability_handshake_is_bounded_and_forward_compatible() {
        let mut value = status();
        value.capabilities |= 1 << 63;
        round_trip(&Message::StatusResponse(value));

        let mut invalid = status();
        invalid.max_protocol_version = 0;
        assert!(encode_message(1, &Message::StatusResponse(invalid)).is_err());
    }

    #[test]
    fn released_m3_status_reader_can_skip_the_current_handshake_extension() {
        let payload = encode_payload(&Message::StatusResponse(status())).unwrap();
        let mut decoder = Decoder::new(&payload);
        let len = decoder.map_len().unwrap();
        assert_eq!(len, 29);
        let mut previous = None;
        let mut released_fields = 0_u32;
        for _ in 0..len {
            let key = next_numeric_key(&mut decoder, &mut previous).unwrap();
            if key <= 20 {
                released_fields |= 1_u32 << key;
            }
            decoder.skip().unwrap();
        }
        decoder.finish().unwrap();
        assert_eq!(released_fields, (1_u32 << 21) - 1);
    }

    #[test]
    fn config_rotation_is_landscape_only_and_legacy_payloads_default_to_90() {
        let config = ApplyConfig {
            revision: 1,
            rotation: 270,
            widgets: vec![WidgetConfig {
                widget_id: "clock".into(),
                template: TemplateKind::DigitalClock,
                size_class: SizeClass::Full,
                tap_action: TapAction::None,
                interrupt_policy: InterruptPolicy::Disabled,
            }],
            screens: vec![ScreenConfig {
                screen_id: "home".into(),
                widget_id: "clock".into(),
            }],
        };
        let mut payload = encode_payload(&Message::ApplyConfig(config.clone())).unwrap();
        assert_eq!(payload[0], 0xa4, "config should be a four-entry CBOR map");
        assert_eq!(
            payload.split_off(payload.len() - 4),
            [0x03, 0x19, 0x01, 0x0e]
        );
        payload[0] = 0xa3;
        let decoded = decode_message(&Frame::new(TYPE_APPLY_CONFIG, 1, payload)).unwrap();
        let Message::ApplyConfig(decoded) = decoded else {
            panic!("decoded message should remain an apply-config request");
        };
        assert_eq!(decoded.rotation, 90);

        let mut invalid = config;
        invalid.rotation = 180;
        assert!(encode_message(1, &Message::ApplyConfig(invalid)).is_err());
    }

    #[test]
    fn invalid_time_and_bounds_are_rejected() {
        assert!(
            encode_message(
                1,
                &Message::TimeSync(TimeSync {
                    unix_seconds: 0,
                    utc_offset_minutes: 0
                })
            )
            .is_err()
        );
        assert!(
            encode_message(
                1,
                &Message::PushData(PushData {
                    widget_id: "x".repeat(MAX_WIDGET_ID_LEN + 1),
                    revision: 1,
                    fields: vec![]
                })
            )
            .is_err()
        );
    }

    #[test]
    fn duplicate_and_unsorted_map_keys_are_rejected() {
        let frame = Frame::new(
            TYPE_TIME_SYNC,
            1,
            vec![0xa2, 0x00, 0x1a, 0x5e, 0x0b, 0xe1, 0x00, 0x00, 0x00],
        );
        assert_eq!(
            decode_message(&frame),
            Err(MessageError::DuplicateOrUnsortedKey)
        );
    }

    #[test]
    fn version_and_unknown_type_are_explicit() {
        let mut frame = Frame::new(TYPE_STATUS_REQUEST, 1, vec![0xa0]);
        frame.version = 2;
        assert_eq!(decode_message(&frame), Err(MessageError::Version(2)));
        frame.version = 1;
        frame.message_type = 99;
        assert_eq!(
            decode_message(&frame),
            Err(MessageError::UnsupportedType(99))
        );
    }
}
