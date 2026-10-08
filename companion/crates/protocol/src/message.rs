#![allow(clippy::struct_excessive_bools)]

use core::fmt;

use crate::PROTOCOL_VERSION;
use crate::cbor::{CborError, Decoder, Encoder};
use crate::frame::{Frame, FrameError, MAX_PAYLOAD_SIZE, encode_frame};
use crate::scene::{Scene, decode_scene, encode_scene, validate_scene};

pub const MAX_PROTOCOL_VERSION: u8 = 2;

/// Protocol v1's bits 0-4 described a device that rendered templates itself:
/// core widgets, config rotation, dashboard layouts, extended templates, host
/// tap actions. v2 has no templates, so none of them said anything a host could
/// act on, and they are retired rather than re-used. **Bit numbers are not
/// compacted.** A bit's meaning is its identity; reusing a retired number would
/// make a v1 capability word decode as a plausible v2 one instead of an
/// obviously wrong one.
/// The invariant below reserves these numbers so they cannot be re-issued.
const RETIRED_V1_CAPABILITY_BITS: u64 = 0b1_1111;
pub const CAPABILITY_ASSET_TRANSFER: u64 = 1 << 5;
pub const CAPABILITY_FIRMWARE_UPDATE: u64 = 1 << 6;
pub const CAPABILITY_NETWORKING: u64 = 1 << 7;
/// Gates `PushScene` (type 19). A host that does not see this bit in the
/// device's `StatusResponse` must not send one -- the same contract
/// `NetworkConfig` and `FactoryReset` have under bit 7. Defining a bit is not
/// switching it on: bit 7 sat defined-but-dark for most of V2 and the constant
/// read 75 instead of 203, so [`CURRENT_CAPABILITIES`] is pinned by a test in
/// both languages.
pub const CAPABILITY_SCENE_RENDER: u64 = 1 << 8;
/// Accepts and resolves `AssetBegin { volatile: true }` image assets. This is
/// separate from bit 5 because deployed asset-transfer builds reject that
/// tier explicitly.
pub const CAPABILITY_VOLATILE_ASSETS: u64 = 1 << 9;
/// Honours `AssetBegin`'s `encoding` and `decoded_length` on the **durable**
/// tier, not only the volatile one.
///
/// Bit 9 cannot carry this promise, for exactly the reason bit 5 could not
/// carry bit 9's: a deployed bit-9 build's durable `AssetBegin` branch ignores
/// `encoding` entirely and writes the wire bytes to flash verbatim, so sending
/// it an RLE565 durable asset stores compressed bytes as if they were pixels.
/// A host must see THIS bit before compressing a durable transfer.
///
/// Picture cards are why it exists. A 448x368 frame is 329,740 raw bytes, which
/// is 172 sequential chunk round trips at `MAX_ASSET_CHUNK_BYTES`, and the
/// tunnel does not survive that. The same frame RLE565-encoded is a handful.
pub const CAPABILITY_DURABLE_ASSET_ENCODING: u64 = 1 << 10;
pub const CAPABILITY_DISPLAY_BRIGHTNESS: u64 = 1 << 11;
pub const CAPABILITY_PIPELINED_ASSET_CHUNKS: u64 = 1 << 12;
pub const ASSET_CHUNK_WINDOW: usize = 8;
pub const MIN_DISPLAY_BRIGHTNESS: u8 = 26;
pub const CURRENT_CAPABILITIES: u64 = CAPABILITY_ASSET_TRANSFER
    | CAPABILITY_FIRMWARE_UPDATE
    | CAPABILITY_NETWORKING
    | CAPABILITY_SCENE_RENDER
    | CAPABILITY_VOLATILE_ASSETS
    | CAPABILITY_DURABLE_ASSET_ENCODING
    | CAPABILITY_DISPLAY_BRIGHTNESS
    | CAPABILITY_PIPELINED_ASSET_CHUNKS;
const _: () = assert!(CURRENT_CAPABILITIES & RETIRED_V1_CAPABILITY_BITS == 0);
/// A card id is an identifier the host chose, not free text.
///
/// Protocol v1 had three of these -- one for a widget id, one for a screen id
/// and one for a card id -- all 32, because a widget, a screen and a card were
/// three names for the thing config schema v10 settled as one card.
pub const MAX_CARD_ID_LEN: usize = 32;
pub const MAX_CONFIG_CARDS: usize = 8;
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
/// 1920, not the 2034-byte envelope cap: the CBOR map header, the 32-byte
/// digest with its bstr header, the offset key/value, and the data bstr
/// header cost roughly 46 bytes; this leaves deliberate margin. Must match
/// firmware's `PROTOCOL_MAX_ASSET_CHUNK_BYTES` exactly.
pub const MAX_ASSET_CHUNK_BYTES: usize = 1920;
pub const MAX_ASSET_DIGESTS: usize = 32;
pub const ASSET_DIGEST_LEN: usize = 32;
pub const MAX_ASSET_TOTAL_LENGTH: u32 = 1_048_576;
pub const ASSET_ENCODING_RAW: u8 = 0;
pub const ASSET_ENCODING_RLE565: u8 = 1;
pub const VOLATILE_IMAGE_DECODED_LENGTH: u32 = 329_740;

pub const TYPE_STATUS_REQUEST: u8 = 1;
pub const TYPE_STATUS_RESPONSE: u8 = 2;
pub const TYPE_TIME_SYNC: u8 = 3;
pub const TYPE_ACK: u8 = 4;
pub const TYPE_PUSH_TIMER: u8 = 5;
pub const TYPE_HEARTBEAT: u8 = 6;
pub const TYPE_HEARTBEAT_ACK: u8 = 7;
pub const TYPE_ERROR: u8 = 8;
pub const TYPE_APPLY_CONFIG: u8 = 9;
pub const TYPE_ACTIVATE_CARD: u8 = 10;
pub const TYPE_TRIGGER_INTERRUPT: u8 = 11;
pub const TYPE_DEVICE_EVENT: u8 = 12;
pub const TYPE_NETWORK_CONFIG: u8 = 13;
pub const TYPE_FACTORY_RESET: u8 = 14;
pub const TYPE_ASSET_BEGIN: u8 = 15;
pub const TYPE_ASSET_CHUNK: u8 = 16;
pub const TYPE_ASSET_COMMIT: u8 = 17;
pub const TYPE_ASSET_RELEASE: u8 = 18;
pub const TYPE_PUSH_SCENE: u8 = 19;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TapAction {
    None = 0,
    StartPause = 1,
    Reset = 2,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AssetKind {
    Font = 1,
    IconFont = 2,
    Image = 3,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetBegin {
    pub digest: [u8; ASSET_DIGEST_LEN],
    pub kind: AssetKind,
    pub total_length: u32,
    pub volatile: bool,
    pub encoding: u8,
    pub decoded_length: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetChunk {
    pub digest: [u8; ASSET_DIGEST_LEN],
    pub offset: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetCommit {
    pub digest: [u8; ASSET_DIGEST_LEN],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetRelease {
    pub digests: Vec<[u8; ASSET_DIGEST_LEN]>,
}

/// One card's whole display list, replacing whatever that card drew before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushScene {
    pub card_id: String,
    /// Required and nonzero, like `PushTimer`'s and `ApplyConfig`'s: a scene
    /// the host cannot pin a revision to is one it cannot tell apart from the
    /// scene already on the panel.
    pub revision: u32,
    pub scene: Scene,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One card in the loop. Protocol v2 carries only what the device decides for
/// itself: which card this is, and what a tap on it does. The host-rendered scene
/// carries the face's visual content.
pub struct CardConfig {
    pub card_id: String,
    pub tap_action: TapAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyConfig {
    pub revision: u32,
    /// Raw panel level, 26..=255. Omitted unless bit 11 is advertised.
    pub brightness: Option<u8>,
    pub rotation: u16,
    /// The loop, in order. Protocol v2 dropped the parallel `screens` array:
    /// since config schema v10 a screen id is always the card id, so it
    /// restated this list.
    pub cards: Vec<CardConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivateCard {
    pub card_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerInterrupt {
    pub card_id: String,
    pub token: u32,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Something the person at the panel did, reported upward.
///
/// Protocol v1 carried two identifiers here, `widget_id` (key 2) and
/// `screen_id` (key 3), and every producer and consumer set them to the same
/// card. v2 states the one card id (key 2); key 3 is retired rather than reused.
pub struct DeviceEvent {
    pub sequence: u64,
    pub kind: EventKind,
    pub card_id: String,
    pub action: EventAction,
    pub interrupt_token: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// The timer a card's `timer.*` scene bindings resolve against.
///
/// Replaces protocol v1's `PushData`, which carried an arbitrary field bag the
/// device validated against a per-template registry. Its only surviving consumer
/// read three keys out of it by name -- `duration_seconds`, `remaining_seconds`
/// and `running` -- so v2 states them. Nothing has emitted a `field.*` binding
/// since config schema v9 removed manifest plugins, so the rest of the bag had
/// no reader at all.
pub struct PushTimer {
    pub card_id: String,
    pub revision: u32,
    pub total_ms: u32,
    pub remaining_ms: u32,
    pub running: bool,
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
    /// `Some` iff `acknowledged_type == TYPE_ASSET_BEGIN`; both directions
    /// enforce this symmetrically on encode and decode.
    pub already_present: Option<bool>,
}

/// The device's own accounting of its **durable** asset store -- the 6 MB flash
/// partition, not the volatile PSRAM tier.
///
/// `firmware/main/link/protocol_task.c` fills key 31 from
/// `asset_store_stats(asset_flash_store(), ..)`, and `free_bytes` is
/// `blob_region_size - high_water` (`firmware/main/core/asset_store.c`), so
/// these three numbers say nothing about PSRAM occupancy. What they are good for
/// is watching `used_bytes` stop moving once picture frames become volatile: the
/// durable tier has never held anything but picture frames, because every font
/// the host emits is baked and the config declares no assets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetStoreStats {
    pub used_bytes: u32,
    pub free_bytes: u32,
    pub asset_count: u32,
}

/// The volatile frame pool, and the PSRAM heap its frames are allocated from --
/// key 32.
///
/// This is the tier a picture frame actually lives in, and the only place a host
/// can learn whether the pool has room. `free_heap` cannot answer that:
/// `protocol_task.c` fills it from `esp_get_free_heap_size()`, a total across
/// every capability, so it counts internal SRAM the pool cannot use.
/// `psram_low_water_bytes` is the minimum ever seen rather than the figure now,
/// because the question a pool of 330 KB allocations raises is about the worst
/// moment, not the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolatileAssetStats {
    /// Committed frames only. An incoming allocation is live heap, but no scene
    /// can name it yet, so it is not occupancy.
    pub committed_count: u32,
    /// `VOLATILE_ASSET_SLOT_COUNT` as the running image was built with it. The
    /// host reads it rather than assuming, because the capacity is not a wire
    /// constant and a fleet mid-rollout has both values in it.
    pub slot_capacity: u32,
    pub used_bytes: u32,
    pub psram_free_bytes: u32,
    pub psram_low_water_bytes: u32,
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
    pub last_ota_error: Option<String>,
    /// Absent until the flash store has formatted; the device omits key 31
    /// rather than encoding it empty.
    pub asset_store: Option<AssetStoreStats>,
    /// Absent on any image built before the frame pool existed. A reader must
    /// not read that absence as a pool with no room.
    pub volatile_assets: Option<VolatileAssetStats>,
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
    UnknownCard = 10,
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
            10 => Ok(Self::UnknownCard),
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
    PushTimer(PushTimer),
    Heartbeat,
    HeartbeatAck(HeartbeatAck),
    Error(ErrorResponse),
    ApplyConfig(ApplyConfig),
    ActivateCard(ActivateCard),
    TriggerInterrupt(TriggerInterrupt),
    DeviceEvent(DeviceEvent),
    NetworkConfig(NetworkConfig),
    FactoryReset,
    AssetBegin(AssetBegin),
    AssetChunk(AssetChunk),
    AssetCommit(AssetCommit),
    AssetRelease(AssetRelease),
    PushScene(PushScene),
}

impl Message {
    #[must_use]
    pub const fn type_id(&self) -> u8 {
        match self {
            Self::StatusRequest => TYPE_STATUS_REQUEST,
            Self::StatusResponse(_) => TYPE_STATUS_RESPONSE,
            Self::TimeSync(_) => TYPE_TIME_SYNC,
            Self::Ack(_) => TYPE_ACK,
            Self::PushTimer(_) => TYPE_PUSH_TIMER,
            Self::Heartbeat => TYPE_HEARTBEAT,
            Self::HeartbeatAck(_) => TYPE_HEARTBEAT_ACK,
            Self::Error(_) => TYPE_ERROR,
            Self::ApplyConfig(_) => TYPE_APPLY_CONFIG,
            Self::ActivateCard(_) => TYPE_ACTIVATE_CARD,
            Self::TriggerInterrupt(_) => TYPE_TRIGGER_INTERRUPT,
            Self::DeviceEvent(_) => TYPE_DEVICE_EVENT,
            Self::NetworkConfig(_) => TYPE_NETWORK_CONFIG,
            Self::FactoryReset => TYPE_FACTORY_RESET,
            Self::AssetBegin(_) => TYPE_ASSET_BEGIN,
            Self::AssetChunk(_) => TYPE_ASSET_CHUNK,
            Self::AssetCommit(_) => TYPE_ASSET_COMMIT,
            Self::AssetRelease(_) => TYPE_ASSET_RELEASE,
            Self::PushScene(_) => TYPE_PUSH_SCENE,
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
    ConfigTooLarge,
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

fn tap_action(value: u8) -> Result<TapAction, MessageError> {
    match value {
        0 => Ok(TapAction::None),
        1 => Ok(TapAction::StartPause),
        2 => Ok(TapAction::Reset),
        _ => Err(MessageError::InvalidValue("tap action")),
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

fn asset_kind_from_wire(value: u8) -> Result<AssetKind, MessageError> {
    match value {
        1 => Ok(AssetKind::Font),
        2 => Ok(AssetKind::IconFont),
        3 => Ok(AssetKind::Image),
        _ => Err(MessageError::InvalidValue("asset kind")),
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
        1 => Ok(EventAction::StartPause),
        2 => Ok(EventAction::Reset),
        3 => Ok(EventAction::NavigatePrevious),
        4 => Ok(EventAction::NavigateNext),
        5 => Ok(EventAction::DismissInterrupt),
        _ => Err(MessageError::InvalidValue("event action")),
    }
}

/// Returns the sole valid response type for a host request type.
///
/// Response-only, unsolicited, and unknown type IDs return `None`.
pub const fn expected_response_type(request_type: u8) -> Option<u8> {
    match request_type {
        TYPE_STATUS_REQUEST => Some(TYPE_STATUS_RESPONSE),
        TYPE_TIME_SYNC
        | TYPE_PUSH_TIMER
        | TYPE_APPLY_CONFIG
        | TYPE_ACTIVATE_CARD
        | TYPE_TRIGGER_INTERRUPT
        | TYPE_NETWORK_CONFIG
        | TYPE_FACTORY_RESET
        | TYPE_ASSET_BEGIN
        | TYPE_ASSET_CHUNK
        | TYPE_ASSET_COMMIT
        | TYPE_ASSET_RELEASE
        | TYPE_PUSH_SCENE => Some(TYPE_ACK),
        TYPE_HEARTBEAT => Some(TYPE_HEARTBEAT_ACK),
        _ => None,
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

fn validate_asset_begin(begin: &AssetBegin) -> Result<(), MessageError> {
    if begin.total_length == 0 || begin.total_length > MAX_ASSET_TOTAL_LENGTH {
        return Err(MessageError::InvalidValue("asset total length"));
    }
    match begin.encoding {
        ASSET_ENCODING_RAW => {
            if begin.decoded_length.is_some() {
                return Err(MessageError::InvalidValue("decoded length on raw asset"));
            }
        }
        ASSET_ENCODING_RLE565 => {
            // Encoding was volatile-only when this rule was written, because
            // the durable tier could not decode. Bit 10 changed that, and a
            // durable picture frame is exactly the case it was blocking.
            let decoded_length = begin
                .decoded_length
                .ok_or(MessageError::InvalidValue("missing decoded length"))?;
            if decoded_length == 0 || decoded_length > MAX_ASSET_TOTAL_LENGTH {
                return Err(MessageError::InvalidValue("asset decoded length"));
            }
            // A VOLATILE image is a full-canvas frame by construction -- it
            // lands in a fixed-size PSRAM slot -- so its decoded length is
            // pinned. A durable image is an ordinary stored asset and may be
            // any bounded size, so pinning it there would reject every scene
            // image that is not a whole screen.
            if begin.volatile
                && begin.kind == AssetKind::Image
                && decoded_length != VOLATILE_IMAGE_DECODED_LENGTH
            {
                return Err(MessageError::InvalidValue("volatile image decoded length"));
            }
            if begin.total_length >= decoded_length {
                return Err(MessageError::InvalidValue("expanding RLE asset"));
            }
        }
        _ => return Err(MessageError::InvalidValue("asset encoding")),
    }
    Ok(())
}

fn validate_asset_chunk(chunk: &AssetChunk) -> Result<(), MessageError> {
    if chunk.data.len() > MAX_ASSET_CHUNK_BYTES {
        return Err(MessageError::InvalidValue("asset chunk data too large"));
    }
    Ok(())
}

fn validate_asset_release(release: &AssetRelease) -> Result<(), MessageError> {
    if release.digests.len() > MAX_ASSET_DIGESTS {
        return Err(MessageError::InvalidValue("too many asset digests"));
    }
    Ok(())
}

fn validate_apply_config(config: &ApplyConfig) -> Result<(), MessageError> {
    if config.revision == 0 {
        return Err(MessageError::InvalidValue("config revision"));
    }
    if config.cards.is_empty() {
        return Err(MessageError::InvalidValue("config count"));
    }
    if config.cards.len() > MAX_CONFIG_CARDS {
        return Err(MessageError::ConfigTooLarge);
    }
    if !matches!(config.rotation, 90 | 270) {
        return Err(MessageError::InvalidValue("config rotation"));
    }
    if config
        .brightness
        .is_some_and(|level| level < MIN_DISPLAY_BRIGHTNESS)
    {
        return Err(MessageError::InvalidValue("config brightness"));
    }
    for (index, card) in config.cards.iter().enumerate() {
        checked_text(&card.card_id, 1, MAX_CARD_ID_LEN, "card_id")?;
        if config.cards[..index]
            .iter()
            .any(|other| other.card_id == card.card_id)
        {
            return Err(MessageError::DuplicateOrUnsortedKey);
        }
    }
    Ok(())
}

fn validate_device_event(event: &DeviceEvent) -> Result<(), MessageError> {
    if event.sequence == 0 {
        return Err(MessageError::InvalidValue("event sequence"));
    }
    checked_text(&event.card_id, 1, MAX_CARD_ID_LEN, "card_id")?;
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

fn validate_push(push: &PushTimer) -> Result<(), MessageError> {
    checked_text(&push.card_id, 1, MAX_CARD_ID_LEN, "card_id")?;
    if push.revision == 0 {
        return Err(MessageError::InvalidValue("revision"));
    }
    // A timer that has run past its own total is not a state the host can mean,
    // and on the device it would drive a `timer.permille` binding out of range.
    if push.remaining_ms > push.total_ms {
        return Err(MessageError::InvalidValue("remaining_ms"));
    }
    Ok(())
}

/// Checks a message against the wire contract before a host sends it, without
/// requiring an encode attempt and a later failure to interpret. Also serves
/// as the single exit check of [`decode_message`].
///
/// # Errors
///
/// Returns [`MessageError`] when the message violates the wire contract.
// One arm per message type; it grows by a few lines whenever the protocol
// gains one, and splitting it would put a type's bounds somewhere other than
// with every other type's.
#[allow(clippy::too_many_lines)]
pub fn validate_message(message: &Message) -> Result<(), MessageError> {
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
            if let Some(diagnostic) = &status.last_ota_error {
                checked_text(diagnostic, 0, MAX_DIAGNOSTIC_LEN, "last ota error")?;
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
            let revision_required = matches!(
                ack.acknowledged_type,
                TYPE_PUSH_TIMER | TYPE_APPLY_CONFIG | TYPE_PUSH_SCENE
            );
            let already_present_required = ack.acknowledged_type == TYPE_ASSET_BEGIN;
            if expected_response_type(ack.acknowledged_type) != Some(TYPE_ACK) {
                return Err(MessageError::InvalidValue("acknowledged type"));
            }
            if revision_required != ack.revision.is_some() {
                return Err(MessageError::InvalidValue("ack revision"));
            }
            if ack.revision == Some(0) {
                return Err(MessageError::InvalidValue("ack revision"));
            }
            if already_present_required != ack.already_present.is_some() {
                return Err(MessageError::InvalidValue("ack already present"));
            }
            Ok(())
        }
        Message::PushTimer(push) => validate_push(push),
        Message::Error(error) => {
            checked_text(&error.diagnostic, 0, MAX_DIAGNOSTIC_LEN, "diagnostic")
        }
        Message::ApplyConfig(config) => validate_apply_config(config),
        Message::ActivateCard(activate) => {
            checked_text(&activate.card_id, 1, MAX_CARD_ID_LEN, "card_id")
        }
        Message::TriggerInterrupt(interrupt) => {
            checked_text(&interrupt.card_id, 1, MAX_CARD_ID_LEN, "card_id")?;
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
        Message::AssetBegin(begin) => validate_asset_begin(begin),
        Message::AssetChunk(chunk) => validate_asset_chunk(chunk),
        Message::AssetRelease(release) => validate_asset_release(release),
        Message::PushScene(push) => {
            checked_text(&push.card_id, 1, MAX_CARD_ID_LEN, "card id")?;
            if push.revision == 0 {
                return Err(MessageError::InvalidValue("scene revision"));
            }
            validate_scene(&push.scene)
        }
        _ => Ok(()),
    }
}

fn encode_config_payload(encoder: &mut Encoder, config: &ApplyConfig) {
    encoder.map(3 + usize::from(config.brightness.is_some()));
    encoder.unsigned(0);
    encoder.unsigned(u64::from(config.revision));
    encoder.unsigned(1);
    encoder.array(config.cards.len());
    for card in &config.cards {
        encoder.map(2);
        encoder.unsigned(0);
        encoder.text(&card.card_id);
        encoder.unsigned(1);
        encoder.unsigned(u64::from(card.tap_action as u8));
    }
    encoder.unsigned(3);
    encoder.unsigned(u64::from(config.rotation));
    if let Some(level) = config.brightness {
        encoder.unsigned(4);
        encoder.unsigned(u64::from(level));
    }
}

fn encode_device_event_payload(encoder: &mut Encoder, event: &DeviceEvent) {
    encoder.map(if event.interrupt_token.is_some() {
        5
    } else {
        4
    });
    encoder.unsigned(0);
    encoder.unsigned(event.sequence);
    encoder.unsigned(1);
    encoder.unsigned(u64::from(event.kind as u8));
    encoder.unsigned(2);
    encoder.text(&event.card_id);
    encoder.unsigned(4);
    encoder.unsigned(u64::from(event.action as u8));
    if let Some(token) = event.interrupt_token {
        encoder.unsigned(5);
        encoder.unsigned(u64::from(token));
    }
}

fn encode_status_payload(encoder: &mut Encoder, status: &StatusResponse) {
    let entry_count = 24
        + 5
        + usize::from(status.last_network_error.is_some())
        + usize::from(status.last_ota_error.is_some())
        + usize::from(status.asset_store.is_some())
        + usize::from(status.volatile_assets.is_some());
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
    if let Some(diagnostic) = &status.last_ota_error {
        encoder.unsigned(30);
        encoder.text(diagnostic);
    }
    if let Some(stats) = &status.asset_store {
        encoder.unsigned(31);
        encoder.map(3);
        encoder.unsigned(0);
        encoder.unsigned(u64::from(stats.used_bytes));
        encoder.unsigned(1);
        encoder.unsigned(u64::from(stats.free_bytes));
        encoder.unsigned(2);
        encoder.unsigned(u64::from(stats.asset_count));
    }
    if let Some(pool) = &status.volatile_assets {
        encoder.unsigned(32);
        encoder.map(5);
        encoder.unsigned(0);
        encoder.unsigned(u64::from(pool.committed_count));
        encoder.unsigned(1);
        encoder.unsigned(u64::from(pool.slot_capacity));
        encoder.unsigned(2);
        encoder.unsigned(u64::from(pool.used_bytes));
        encoder.unsigned(3);
        encoder.unsigned(u64::from(pool.psram_free_bytes));
        encoder.unsigned(4);
        encoder.unsigned(u64::from(pool.psram_low_water_bytes));
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

#[allow(clippy::too_many_lines)]
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
        Message::PushTimer(push) => {
            encoder.map(5);
            encoder.unsigned(0);
            encoder.text(&push.card_id);
            encoder.unsigned(1);
            encoder.unsigned(u64::from(push.revision));
            encoder.unsigned(2);
            encoder.unsigned(u64::from(push.total_ms));
            encoder.unsigned(3);
            encoder.unsigned(u64::from(push.remaining_ms));
            encoder.unsigned(4);
            encoder.boolean(push.running);
        }
        Message::ApplyConfig(config) => encode_config_payload(&mut encoder, config),
        Message::ActivateCard(activate) => {
            encoder.map(1);
            encoder.unsigned(0);
            encoder.text(&activate.card_id);
        }
        Message::TriggerInterrupt(interrupt) => {
            encoder.map(3);
            encoder.unsigned(0);
            encoder.text(&interrupt.card_id);
            encoder.unsigned(1);
            encoder.unsigned(u64::from(interrupt.token));
            encoder.unsigned(2);
            encoder.text(&interrupt.reason);
        }
        Message::DeviceEvent(event) => encode_device_event_payload(&mut encoder, event),
        Message::Ack(ack) => {
            let field_count = 1
                + usize::from(ack.revision.is_some())
                + usize::from(ack.already_present.is_some());
            encoder.map(field_count);
            encoder.unsigned(0);
            encoder.unsigned(u64::from(ack.acknowledged_type));
            if let Some(revision) = ack.revision {
                encoder.unsigned(1);
                encoder.unsigned(u64::from(revision));
            }
            if let Some(already_present) = ack.already_present {
                encoder.unsigned(2);
                encoder.boolean(already_present);
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
        Message::AssetBegin(begin) => {
            let optional_count = usize::from(begin.encoding != ASSET_ENCODING_RAW)
                + usize::from(begin.decoded_length.is_some());
            encoder.map(4 + optional_count);
            encoder.unsigned(0);
            encoder.bytes(&begin.digest);
            encoder.unsigned(1);
            encoder.unsigned(u64::from(begin.kind as u8));
            encoder.unsigned(2);
            encoder.unsigned(u64::from(begin.total_length));
            // Key 3 is ALWAYS emitted, false included, although this side's
            // decoder tolerates its absence. Every deployed firmware decoder
            // requires it (`REQUIRED_BIT(3)` until the encoding keys landed),
            // so omitting the false case would break durable asset sync
            // against the fleet the moment the server redeployed. The
            // canonical emission rule yields to wire history here; keys 4/5
            // follow the canonical omit-when-default rule (docs/protocol/v2.md).
            encoder.unsigned(3);
            encoder.boolean(begin.volatile);
            if begin.encoding != ASSET_ENCODING_RAW {
                encoder.unsigned(4);
                encoder.unsigned(u64::from(begin.encoding));
            }
            if let Some(decoded_length) = begin.decoded_length {
                encoder.unsigned(5);
                encoder.unsigned(u64::from(decoded_length));
            }
        }
        Message::AssetChunk(chunk) => {
            encoder.map(3);
            encoder.unsigned(0);
            encoder.bytes(&chunk.digest);
            encoder.unsigned(1);
            encoder.unsigned(u64::from(chunk.offset));
            encoder.unsigned(2);
            encoder.bytes(&chunk.data);
        }
        Message::AssetCommit(commit) => {
            encoder.map(1);
            encoder.unsigned(0);
            encoder.bytes(&commit.digest);
        }
        Message::AssetRelease(release) => {
            encoder.map(1);
            encoder.unsigned(0);
            encoder.array(release.digests.len());
            for digest in &release.digests {
                encoder.bytes(digest);
            }
        }
        Message::PushScene(push) => encode_push_scene_payload(&mut encoder, push),
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

pub(super) fn next_numeric_key(
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

pub(super) fn read_u32(decoder: &mut Decoder<'_>, name: &'static str) -> Result<u32, MessageError> {
    u32::try_from(decoder.unsigned()?).map_err(|_| MessageError::InvalidValue(name))
}

fn read_digest(decoder: &mut Decoder<'_>) -> Result<[u8; ASSET_DIGEST_LEN], MessageError> {
    let raw = decoder.bytes()?;
    <[u8; ASSET_DIGEST_LEN]>::try_from(raw).map_err(|_| MessageError::InvalidValue("asset digest"))
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
    Ok(TimeSync {
        unix_seconds: seconds.ok_or(MessageError::MissingField(0))?,
        utc_offset_minutes: offset.ok_or(MessageError::MissingField(1))?,
    })
}

fn decode_push(payload: &[u8]) -> Result<PushTimer, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut card_id = None;
    let mut revision = None;
    let mut total_ms = None;
    let mut remaining_ms = None;
    let mut running = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => card_id = Some(decoder.text()?.to_owned()),
            1 => revision = Some(read_u32(&mut decoder, "revision")?),
            2 => total_ms = Some(read_u32(&mut decoder, "total_ms")?),
            3 => remaining_ms = Some(read_u32(&mut decoder, "remaining_ms")?),
            4 => running = Some(decoder.boolean()?),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    Ok(PushTimer {
        card_id: card_id.ok_or(MessageError::MissingField(0))?,
        revision: revision.ok_or(MessageError::MissingField(1))?,
        total_ms: total_ms.ok_or(MessageError::MissingField(2))?,
        remaining_ms: remaining_ms.ok_or(MessageError::MissingField(3))?,
        running: running.ok_or(MessageError::MissingField(4))?,
    })
}

fn decode_cards(decoder: &mut Decoder<'_>) -> Result<Vec<CardConfig>, MessageError> {
    let count = decoder.array_len()?;
    if count > MAX_CONFIG_CARDS {
        return Err(MessageError::ConfigTooLarge);
    }
    let mut cards = Vec::with_capacity(count);
    for _ in 0..count {
        let len = decoder.map_len()?;
        let mut previous = None;
        let mut card_id = None;
        let mut action = None;
        for _ in 0..len {
            match next_numeric_key(decoder, &mut previous)? {
                0 => card_id = Some(decoder.text()?.to_owned()),
                1 => action = Some(tap_action(read_u8(decoder, "tap action")?)?),
                _ => decoder.skip()?,
            }
        }
        cards.push(CardConfig {
            card_id: card_id.ok_or(MessageError::MissingField(0))?,
            tap_action: action.ok_or(MessageError::MissingField(1))?,
        });
    }
    Ok(cards)
}

fn decode_apply_config(payload: &[u8]) -> Result<ApplyConfig, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut revision = None;
    let mut cards = None;
    let mut rotation = None;
    let mut brightness = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => revision = Some(read_u32(&mut decoder, "config revision")?),
            1 => cards = Some(decode_cards(&mut decoder)?),
            3 => rotation = Some(read_u16(&mut decoder, "config rotation")?),
            4 => brightness = Some(read_u8(&mut decoder, "config brightness")?),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    Ok(ApplyConfig {
        revision: revision.ok_or(MessageError::MissingField(0))?,
        rotation: rotation.unwrap_or(90),
        brightness,
        cards: cards.ok_or(MessageError::MissingField(1))?,
    })
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
    Ok(NetworkConfig {
        ssid: ssid.ok_or(MessageError::MissingField(1))?,
        psk: psk.ok_or(MessageError::MissingField(2))?,
        server_url: server_url.ok_or(MessageError::MissingField(3))?,
        device_id: device_id.ok_or(MessageError::MissingField(4))?,
        token: token.ok_or(MessageError::MissingField(5))?,
        utc_offset_minutes: utc_offset_minutes.ok_or(MessageError::MissingField(6))?,
        tier: tier.ok_or(MessageError::MissingField(7))?,
    })
}

fn decode_activate_card(payload: &[u8]) -> Result<ActivateCard, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut card_id = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => card_id = Some(decoder.text()?.to_owned()),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    Ok(ActivateCard {
        card_id: card_id.ok_or(MessageError::MissingField(0))?,
    })
}

fn decode_trigger_interrupt(payload: &[u8]) -> Result<TriggerInterrupt, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut card_id = None;
    let mut token = None;
    let mut reason = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => card_id = Some(decoder.text()?.to_owned()),
            1 => token = Some(read_u32(&mut decoder, "interrupt token")?),
            2 => reason = Some(decoder.text()?.to_owned()),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    Ok(TriggerInterrupt {
        card_id: card_id.ok_or(MessageError::MissingField(0))?,
        token: token.ok_or(MessageError::MissingField(1))?,
        reason: reason.ok_or(MessageError::MissingField(2))?,
    })
}

fn decode_device_event(payload: &[u8]) -> Result<DeviceEvent, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut sequence = None;
    let mut kind = None;
    let mut card_id = None;
    let mut action = None;
    let mut interrupt_token = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => sequence = Some(decoder.unsigned()?),
            1 => kind = Some(event_kind(read_u8(&mut decoder, "event kind")?)?),
            2 => card_id = Some(decoder.text()?.to_owned()),
            4 => action = Some(event_action(read_u8(&mut decoder, "event action")?)?),
            5 => interrupt_token = Some(read_u32(&mut decoder, "interrupt token")?),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    Ok(DeviceEvent {
        sequence: sequence.ok_or(MessageError::MissingField(0))?,
        kind: kind.ok_or(MessageError::MissingField(1))?,
        card_id: card_id.ok_or(MessageError::MissingField(2))?,
        action: action.ok_or(MessageError::MissingField(4))?,
        interrupt_token,
    })
}

fn decode_ack(payload: &[u8]) -> Result<Ack, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut acknowledged_type = None;
    let mut revision = None;
    let mut already_present = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => acknowledged_type = Some(read_u8(&mut decoder, "acknowledged type")?),
            1 => revision = Some(read_u32(&mut decoder, "revision")?),
            2 => already_present = Some(decoder.boolean()?),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    Ok(Ack {
        acknowledged_type: acknowledged_type.ok_or(MessageError::MissingField(0))?,
        revision,
        already_present,
    })
}

fn encode_push_scene_payload(encoder: &mut Encoder, push: &PushScene) {
    encoder.map(3);
    encoder.unsigned(0);
    encoder.text(&push.card_id);
    encoder.unsigned(1);
    encoder.unsigned(u64::from(push.revision));
    encoder.unsigned(2);
    encode_scene(encoder, &push.scene);
}

fn decode_push_scene(payload: &[u8]) -> Result<PushScene, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut card_id = None;
    let mut revision = None;
    let mut scene = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => card_id = Some(decoder.text()?.to_owned()),
            1 => revision = Some(read_u32(&mut decoder, "scene revision")?),
            2 => scene = Some(decode_scene(&mut decoder)?),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    Ok(PushScene {
        card_id: card_id.ok_or(MessageError::MissingField(0))?,
        revision: revision.ok_or(MessageError::MissingField(1))?,
        scene: scene.ok_or(MessageError::MissingField(2))?,
    })
}

fn decode_asset_begin(payload: &[u8]) -> Result<AssetBegin, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut digest = None;
    let mut kind = None;
    let mut total_length = None;
    let mut volatile = None;
    let mut encoding = None;
    let mut decoded_length = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => digest = Some(read_digest(&mut decoder)?),
            1 => kind = Some(asset_kind_from_wire(read_u8(&mut decoder, "asset kind")?)?),
            2 => total_length = Some(read_u32(&mut decoder, "asset total length")?),
            3 => volatile = Some(decoder.boolean()?),
            4 => encoding = Some(read_u8(&mut decoder, "asset encoding")?),
            5 => decoded_length = Some(read_u32(&mut decoder, "asset decoded length")?),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    Ok(AssetBegin {
        digest: digest.ok_or(MessageError::MissingField(0))?,
        kind: kind.ok_or(MessageError::MissingField(1))?,
        total_length: total_length.ok_or(MessageError::MissingField(2))?,
        volatile: volatile.unwrap_or(false),
        encoding: encoding.unwrap_or(ASSET_ENCODING_RAW),
        decoded_length,
    })
}

fn decode_asset_chunk(payload: &[u8]) -> Result<AssetChunk, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut digest = None;
    let mut offset = None;
    let mut data = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => digest = Some(read_digest(&mut decoder)?),
            1 => offset = Some(read_u32(&mut decoder, "asset chunk offset")?),
            2 => data = Some(decoder.bytes()?.to_vec()),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    Ok(AssetChunk {
        digest: digest.ok_or(MessageError::MissingField(0))?,
        offset: offset.ok_or(MessageError::MissingField(1))?,
        data: data.ok_or(MessageError::MissingField(2))?,
    })
}

fn decode_asset_commit(payload: &[u8]) -> Result<AssetCommit, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut digest = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => digest = Some(read_digest(&mut decoder)?),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    Ok(AssetCommit {
        digest: digest.ok_or(MessageError::MissingField(0))?,
    })
}

fn decode_asset_digests(
    decoder: &mut Decoder<'_>,
) -> Result<Vec<[u8; ASSET_DIGEST_LEN]>, MessageError> {
    let count = decoder.array_len()?;
    if count > MAX_ASSET_DIGESTS {
        return Err(MessageError::InvalidValue("too many asset digests"));
    }
    let mut digests = Vec::with_capacity(count);
    for _ in 0..count {
        digests.push(read_digest(decoder)?);
    }
    Ok(digests)
}

fn decode_asset_release(payload: &[u8]) -> Result<AssetRelease, MessageError> {
    let mut decoder = Decoder::new(payload);
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut digests = None;
    for _ in 0..len {
        match next_numeric_key(&mut decoder, &mut previous)? {
            0 => digests = Some(decode_asset_digests(&mut decoder)?),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    Ok(AssetRelease {
        digests: digests.ok_or(MessageError::MissingField(0))?,
    })
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
    Ok(ErrorResponse {
        code: code.ok_or(MessageError::MissingField(0))?,
        diagnostic: diagnostic.ok_or(MessageError::MissingField(1))?,
    })
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
    let mut last_ota_error = None;
    let mut asset_store = None;
    let mut volatile_assets = None;
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
            30 => last_ota_error = Some(decoder.text()?.to_owned()),
            31 => asset_store = Some(decode_asset_store_stats(&mut decoder)?),
            32 => volatile_assets = Some(decode_volatile_asset_stats(&mut decoder)?),
            _ => decoder.skip()?,
        }
    }
    decoder.finish()?;
    Ok(StatusResponse {
        protocol_version: protocol_version.ok_or(MessageError::MissingField(0))?,
        max_protocol_version: max_protocol_version.unwrap_or(PROTOCOL_VERSION),
        // A v2 device always states its capabilities. An absent word used to
        // mean "the oldest v1 build", and no such build speaks v2.
        capabilities: capabilities.unwrap_or(0),
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
        last_ota_error,
        asset_store,
        volatile_assets,
    })
}

/// Key 32's nested map, all five sub-keys required, for the same reason key 31's
/// three are: the device encodes the whole map or omits the key, so a partial one
/// is a malformed frame. Defaulting a gap to zero would be worse here than for
/// key 31 -- a zero `slot_capacity` reads as a device with no pool at all, and a
/// zero `psram_free_bytes` as one with no memory left.
fn decode_volatile_asset_stats(
    decoder: &mut Decoder<'_>,
) -> Result<VolatileAssetStats, MessageError> {
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut committed_count = None;
    let mut slot_capacity = None;
    let mut used_bytes = None;
    let mut psram_free_bytes = None;
    let mut psram_low_water_bytes = None;
    for _ in 0..len {
        match next_numeric_key(decoder, &mut previous)? {
            0 => committed_count = Some(read_u32(decoder, "volatile committed count")?),
            1 => slot_capacity = Some(read_u32(decoder, "volatile slot capacity")?),
            2 => used_bytes = Some(read_u32(decoder, "volatile used bytes")?),
            3 => psram_free_bytes = Some(read_u32(decoder, "PSRAM free bytes")?),
            4 => psram_low_water_bytes = Some(read_u32(decoder, "PSRAM low water bytes")?),
            _ => decoder.skip()?,
        }
    }
    Ok(VolatileAssetStats {
        committed_count: committed_count.ok_or(MessageError::MissingField(32))?,
        slot_capacity: slot_capacity.ok_or(MessageError::MissingField(32))?,
        used_bytes: used_bytes.ok_or(MessageError::MissingField(32))?,
        psram_free_bytes: psram_free_bytes.ok_or(MessageError::MissingField(32))?,
        psram_low_water_bytes: psram_low_water_bytes.ok_or(MessageError::MissingField(32))?,
    })
}

/// Key 31's nested map, all three sub-keys required.
///
/// The device either omits the key or encodes every field of it
/// (`firmware/main/core/protocol_message.c` refuses its own partial map with
/// `ERR_MISSING_FIELD`), so a partial map is a malformed frame rather than a
/// device with less to say. `MissingField` names the outer key, which is what a
/// reader of the diagnostic has to go looking for.
fn decode_asset_store_stats(decoder: &mut Decoder<'_>) -> Result<AssetStoreStats, MessageError> {
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut used_bytes = None;
    let mut free_bytes = None;
    let mut asset_count = None;
    for _ in 0..len {
        match next_numeric_key(decoder, &mut previous)? {
            0 => used_bytes = Some(read_u32(decoder, "asset store used bytes")?),
            1 => free_bytes = Some(read_u32(decoder, "asset store free bytes")?),
            2 => asset_count = Some(read_u32(decoder, "asset store count")?),
            _ => decoder.skip()?,
        }
    }
    Ok(AssetStoreStats {
        used_bytes: used_bytes.ok_or(MessageError::MissingField(31))?,
        free_bytes: free_bytes.ok_or(MessageError::MissingField(31))?,
        asset_count: asset_count.ok_or(MessageError::MissingField(31))?,
    })
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
        TYPE_PUSH_TIMER => Message::PushTimer(decode_push(&frame.payload)?),
        TYPE_HEARTBEAT => {
            require_empty_map(&frame.payload)?;
            Message::Heartbeat
        }
        TYPE_HEARTBEAT_ACK => Message::HeartbeatAck(decode_heartbeat_ack(&frame.payload)?),
        TYPE_ERROR => Message::Error(decode_error(&frame.payload)?),
        TYPE_APPLY_CONFIG => Message::ApplyConfig(decode_apply_config(&frame.payload)?),
        TYPE_ACTIVATE_CARD => Message::ActivateCard(decode_activate_card(&frame.payload)?),
        TYPE_TRIGGER_INTERRUPT => {
            Message::TriggerInterrupt(decode_trigger_interrupt(&frame.payload)?)
        }
        TYPE_DEVICE_EVENT => Message::DeviceEvent(decode_device_event(&frame.payload)?),
        TYPE_NETWORK_CONFIG => Message::NetworkConfig(decode_network_config(&frame.payload)?),
        TYPE_FACTORY_RESET => {
            require_empty_map(&frame.payload)?;
            Message::FactoryReset
        }
        TYPE_ASSET_BEGIN => Message::AssetBegin(decode_asset_begin(&frame.payload)?),
        TYPE_ASSET_CHUNK => Message::AssetChunk(decode_asset_chunk(&frame.payload)?),
        TYPE_ASSET_COMMIT => Message::AssetCommit(decode_asset_commit(&frame.payload)?),
        TYPE_ASSET_RELEASE => Message::AssetRelease(decode_asset_release(&frame.payload)?),
        TYPE_PUSH_SCENE => Message::PushScene(decode_push_scene(&frame.payload)?),
        other => return Err(MessageError::UnsupportedType(other)),
    };
    validate_message(&message)?;
    Ok(message)
}

#[cfg(test)]
mod tests {
    use crate::frame::decode_wire_frame;
    use crate::scene::{
        MAX_SCENE_NODES, SceneFont, SceneFontTier, SceneNode, SceneRect, SceneText, SceneValue,
    };

    use super::*;

    fn status() -> StatusResponse {
        StatusResponse {
            firmware_version: "m1-test".into(),
            uptime_ms: 123_456,
            free_heap: 654_321,
            display_width: 368,
            display_height: 448,
            rotation: 0,
            latest_revision: 7,
            valid_frames: 10,
            malformed_frames: 2,
            crc_errors: 1,
            overflow_frames: 1,
            config_revision: 3,
            latest_interrupt_token: 9,
            ..crate::test_support::sample_status_response()
        }
    }

    fn decode_wire(wire: &[u8]) -> Result<Message, MessageError> {
        decode_message(&decode_wire_frame(wire)?)
    }

    fn round_trip(name: &str, request_id: u32, message: &Message) {
        let wire = encode_message(request_id, message)
            .unwrap_or_else(|error| panic!("{name}: encode failed: {error:?}"));
        let frame = decode_wire_frame(&wire)
            .unwrap_or_else(|error| panic!("{name}: frame decode failed: {error:?}"));
        let decoded = decode_message(&frame)
            .unwrap_or_else(|error| panic!("{name}: message decode failed: {error:?}"));
        assert_eq!(&decoded, message, "{name}");
    }

    macro_rules! round_trip_cases {
        ($($name:literal, $request_id:expr, $message:expr;)+) => {
            $(round_trip($name, $request_id, &$message);)+
        };
    }

    #[test]
    fn every_message_round_trips() {
        round_trip_cases! {
            "status_request", 42, Message::StatusRequest;
            "status_response", 42, Message::StatusResponse(status());
            "time_sync", 42, Message::TimeSync(TimeSync {
                unix_seconds: 1_775_217_600,
                utc_offset_minutes: 240,
            });
            "ack", 42, Message::Ack(Ack {
                acknowledged_type: TYPE_TIME_SYNC,
                revision: None,
                already_present: None,
            });
            "push_timer", 42, Message::PushTimer(PushTimer {
                card_id: "focus".into(),
                revision: 7,
                total_ms: 1_500_000,
                remaining_ms: 900_000,
                running: true,
            });
            "heartbeat", 42, Message::Heartbeat;
            "heartbeat_ack", 42, Message::HeartbeatAck(HeartbeatAck { uptime_ms: 99 });
            "error", 42, Message::Error(ErrorResponse {
                code: ErrorCode::StaleRevision,
                diagnostic: "stale revision".into(),
            });
            "apply_config", 42, Message::ApplyConfig(ApplyConfig {
                brightness: None,
                revision: 3,
                rotation: 270,
                cards: vec![CardConfig {
                    card_id: "timer".into(),
                    tap_action: TapAction::StartPause,
                }],
            });
            "activate_card", 42, Message::ActivateCard(ActivateCard {
                card_id: "focus".into(),
            });
            "trigger_interrupt", 42, Message::TriggerInterrupt(TriggerInterrupt {
                card_id: "timer".into(),
                token: 4,
                reason: "done".into(),
            });
            "device_event", 0, Message::DeviceEvent(DeviceEvent {
                sequence: 5,
                kind: EventKind::InterruptDismissed,
                card_id: "timer".into(),
                action: EventAction::DismissInterrupt,
                interrupt_token: Some(4),
            });
            "network_config_round_trips", 42, Message::NetworkConfig(NetworkConfig {
                ssid: "home-network".into(),
                psk: "correct horse battery staple".into(),
                server_url: "wss://deskmate.example.com/v1/device/link".into(),
                device_id: "dev-0001".into(),
                token: "t".repeat(MAX_DEVICE_TOKEN_LEN),
                utc_offset_minutes: 240,
                tier: Tier::Networked,
            });
            "factory_reset_round_trips", 42, Message::FactoryReset;
            "push_scene_roundtrips", 45, Message::PushScene(PushScene {
                card_id: "clock".into(),
                revision: 7,
                scene: sample_scene(),
            });
            "rle_asset_begin_roundtrips_with_wire_and_decoded_lengths", 8,
                Message::AssetBegin(AssetBegin {
                    digest: [0x6b; 32],
                    kind: AssetKind::Image,
                    total_length: 10_032,
                    volatile: true,
                    encoding: ASSET_ENCODING_RLE565,
                    decoded_length: Some(VOLATILE_IMAGE_DECODED_LENGTH),
                });
            "asset_chunk_roundtrips", 8, Message::AssetChunk(AssetChunk {
                digest: [0x11; 32],
                offset: 512,
                data: vec![0xab; 128],
            });
            "asset_commit_roundtrips", 9, Message::AssetCommit(AssetCommit {
                digest: [0x22; 32],
            });
            "asset_release_roundtrips", 10, Message::AssetRelease(AssetRelease {
                digests: vec![[0x33; 32], [0x44; 32]],
            });
            "ack_already_present_roundtrips_on_asset_begin", 11, Message::Ack(Ack {
                acknowledged_type: TYPE_ASSET_BEGIN,
                revision: None,
                already_present: Some(true),
            });
        }
    }

    #[test]
    fn device_event_with_zero_sequence_is_rejected_on_decode() {
        let mut encoder = Encoder::new();
        encoder.map(4);
        encoder.unsigned(0);
        encoder.unsigned(0);
        encoder.unsigned(1);
        encoder.unsigned(EventKind::Tap as u64);
        encoder.unsigned(2);
        encoder.text("timer");
        encoder.unsigned(4);
        encoder.unsigned(EventAction::StartPause as u64);
        assert_eq!(
            decode_message(&Frame::new(TYPE_DEVICE_EVENT, 0, encoder.into_bytes())),
            Err(MessageError::InvalidValue("event sequence"))
        );
    }

    #[test]
    fn push_timer_ack_without_revision_is_rejected_on_decode() {
        let mut encoder = Encoder::new();
        encoder.map(1);
        encoder.unsigned(0);
        encoder.unsigned(u64::from(TYPE_PUSH_TIMER));
        assert_eq!(
            decode_message(&Frame::new(TYPE_ACK, 1, encoder.into_bytes())),
            Err(MessageError::InvalidValue("ack revision"))
        );
    }

    #[test]
    fn activate_card_with_empty_id_is_rejected_on_decode() {
        let mut encoder = Encoder::new();
        encoder.map(1);
        encoder.unsigned(0);
        encoder.text("");
        assert_eq!(
            decode_message(&Frame::new(TYPE_ACTIVATE_CARD, 1, encoder.into_bytes())),
            Err(MessageError::InvalidValue("card_id"))
        );
    }

    #[test]
    fn event_action_zero_is_reserved_and_invalid() {
        let mut encoder = Encoder::new();
        encoder.map(5);
        encoder.unsigned(0);
        encoder.unsigned(1);
        encoder.unsigned(1);
        encoder.unsigned(EventKind::Tap as u64);
        encoder.unsigned(2);
        encoder.text("timer");
        encoder.unsigned(3);
        encoder.text("focus");
        encoder.unsigned(4);
        encoder.unsigned(0);
        let frame = Frame::new(TYPE_DEVICE_EVENT, 0, encoder.into_bytes());

        assert_eq!(
            decode_message(&frame),
            Err(MessageError::InvalidValue("event action"))
        );
    }

    #[test]
    fn expected_response_table_covers_every_request_type() {
        assert_eq!(
            expected_response_type(TYPE_STATUS_REQUEST),
            Some(TYPE_STATUS_RESPONSE)
        );
        assert_eq!(
            expected_response_type(TYPE_HEARTBEAT),
            Some(TYPE_HEARTBEAT_ACK)
        );
        for request_type in [
            TYPE_TIME_SYNC,
            TYPE_PUSH_TIMER,
            TYPE_APPLY_CONFIG,
            TYPE_ACTIVATE_CARD,
            TYPE_TRIGGER_INTERRUPT,
            TYPE_NETWORK_CONFIG,
            TYPE_FACTORY_RESET,
            TYPE_ASSET_BEGIN,
            TYPE_ASSET_CHUNK,
            TYPE_ASSET_COMMIT,
            TYPE_ASSET_RELEASE,
            TYPE_PUSH_SCENE,
        ] {
            assert_eq!(expected_response_type(request_type), Some(TYPE_ACK));
        }
        for response_or_unsolicited_type in [
            TYPE_STATUS_RESPONSE,
            TYPE_ACK,
            TYPE_HEARTBEAT_ACK,
            TYPE_ERROR,
            TYPE_DEVICE_EVENT,
            0,
            u8::MAX,
        ] {
            assert_eq!(expected_response_type(response_or_unsolicited_type), None);
        }
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
    fn cardinal_display_rotations_are_valid() {
        for rotation in [0, 90, 180, 270] {
            let mut value = status();
            value.rotation = rotation;
            round_trip(
                &format!("rotation_{rotation}"),
                42,
                &Message::StatusResponse(value),
            );
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
        status.last_network_error = Some("dns resolution timed out".into());
        status.last_ota_error = Some("download: ESP_ERR_NO_MEM".into());
        let wire = encode_message(3, &Message::StatusResponse(status.clone())).expect("encode");
        let frame = decode_wire_frame(&wire).expect("decode frame");
        let Message::StatusResponse(decoded) = decode_message(&frame).expect("decode") else {
            panic!("wrong variant");
        };
        assert_eq!(decoded, status);
    }

    #[test]
    fn status_response_ota_error_is_optional_and_bounded_on_decode() {
        let payload = encode_payload(&Message::StatusResponse(status())).unwrap();
        let Message::StatusResponse(decoded) =
            decode_message(&Frame::new(TYPE_STATUS_RESPONSE, 1, payload.clone())).unwrap()
        else {
            panic!("decoded message should remain a status response");
        };
        assert_eq!(decoded.last_ota_error, None);

        let mut boundary = status();
        boundary.last_ota_error = Some("x".repeat(MAX_DIAGNOSTIC_LEN));
        round_trip(
            "maximum_length_ota_error",
            42,
            &Message::StatusResponse(boundary),
        );

        let mut oversized_value = status();
        oversized_value.last_ota_error = Some("x".repeat(MAX_DIAGNOSTIC_LEN + 1));
        assert!(encode_message(1, &Message::StatusResponse(oversized_value)).is_err());

        assert_eq!(payload[0], 0xb8, "status should use the extended map form");
        let mut oversized_payload = payload;
        oversized_payload[1] += 1;
        oversized_payload.extend_from_slice(&[0x18, 30, 0x78, 97]);
        oversized_payload.extend(std::iter::repeat_n(b'x', MAX_DIAGNOSTIC_LEN + 1));
        assert_eq!(
            decode_message(&Frame::new(TYPE_STATUS_RESPONSE, 1, oversized_payload)),
            Err(MessageError::InvalidValue("last ota error"))
        );
    }

    #[test]
    fn status_without_optional_handshake_fields_remains_compatible() {
        let payload = encode_payload(&Message::StatusResponse(status())).unwrap();
        // The current status fields keep the entry count at or above 24, so the
        // map header stays in its two-byte extended form regardless of the
        // exact count.
        assert_eq!(payload[0], 0xb8, "status should use the extended map form");
        let original_count = payload[1];
        // Keys 21 (latest_interrupt_token), 22 (max_protocol_version), and 23
        // (capabilities) are still encoded contiguously and in this order
        // because keys are canonical; locate and drop them regardless of
        // what now follows them on the wire. The tail is
        // CURRENT_CAPABILITIES, which protocol v2 re-based: bits 0-4 described
        // a device that rendered templates and are retired, leaving
        // 8160 (0x1fe0). It moves whenever a capability bit changes.
        let pattern = [0x15, 0x09, 0x16, 0x02, 0x17, 0x19, 0x1f, 0xe0];
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
        // A v2 device always states its capabilities; an absent word is zero.
        assert_eq!(decoded.capabilities, 0);
    }

    #[test]
    fn capability_handshake_is_bounded_and_forward_compatible() {
        let mut value = status();
        value.capabilities |= 1 << 63;
        round_trip(
            "unknown_capability_bit",
            42,
            &Message::StatusResponse(value),
        );

        let mut invalid = status();
        invalid.max_protocol_version = 0;
        assert!(encode_message(1, &Message::StatusResponse(invalid)).is_err());
    }

    #[test]
    fn prefix_status_reader_can_skip_the_current_handshake_extension() {
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
    fn asset_store_stats_are_optional_whole_and_forward_compatible() {
        // The realistic shape: four picture frames in the 6 MB blob region.
        let mut present = status();
        present.asset_store = Some(AssetStoreStats {
            used_bytes: 141_312,
            free_bytes: 6_149_120,
            asset_count: 4,
        });
        round_trip(
            "status_asset_store_stats",
            42,
            &Message::StatusResponse(present),
        );

        // Absent is the ordinary case for a store that never formatted, and it
        // is what every golden frame in `tests/fixtures.rs` was captured as.
        assert!(status().asset_store.is_none());
        let absent = encode_payload(&Message::StatusResponse(status())).unwrap();
        assert!(decode_status(&absent).unwrap().asset_store.is_none());

        // Single-byte values so the nested map's bytes can be named exactly.
        let mut small = status();
        small.asset_store = Some(AssetStoreStats {
            used_bytes: 4,
            free_bytes: 9,
            asset_count: 2,
        });
        let mut trunk = encode_payload(&Message::StatusResponse(small)).unwrap();
        let mut decoder = Decoder::new(&trunk);
        assert_eq!(
            decoder.map_len().unwrap(),
            30,
            "key 31 should add exactly one outer entry"
        );
        let stats = trunk.split_off(trunk.len() - 9);
        assert_eq!(
            stats,
            [0x18, 0x1f, 0xa3, 0x00, 0x04, 0x01, 0x09, 0x02, 0x02]
        );

        // The device encodes all three sub-keys or omits the key, and refuses
        // its own partial map with a missing-field error. A host that filled any
        // one gap with a zero would report a full store as an empty one, so each
        // omission is named: one case alone only pins the last field read.
        for (omitted, pairs) in [
            ("used bytes", [0x01, 0x09, 0x02, 0x02]),
            ("free bytes", [0x00, 0x04, 0x02, 0x02]),
            ("asset count", [0x00, 0x04, 0x01, 0x09]),
        ] {
            let mut partial = trunk.clone();
            partial.extend_from_slice(&[0x18, 0x1f, 0xa2]);
            partial.extend_from_slice(&pairs);
            assert_eq!(
                decode_status(&partial),
                Err(MessageError::MissingField(31)),
                "a map without its {omitted} should be refused"
            );
        }

        // A sub-key this host has never heard of is skipped, not refused.
        let mut extended = trunk.clone();
        extended.extend_from_slice(&[
            0x18, 0x1f, 0xa4, 0x00, 0x04, 0x01, 0x09, 0x02, 0x02, 0x03, 0x07,
        ]);
        assert_eq!(
            decode_status(&extended).unwrap().asset_store,
            Some(AssetStoreStats {
                used_bytes: 4,
                free_bytes: 9,
                asset_count: 2,
            })
        );

        // The nested map obeys the same key ordering as every other map here.
        let mut unsorted = trunk;
        unsorted.extend_from_slice(&[0x18, 0x1f, 0xa3, 0x01, 0x09, 0x00, 0x04, 0x02, 0x02]);
        assert_eq!(
            decode_status(&unsorted),
            Err(MessageError::DuplicateOrUnsortedKey)
        );
    }

    #[test]
    fn volatile_pool_stats_are_optional_whole_and_forward_compatible() {
        // Four frames of a sixteen-slot pool, with the heap figures a board
        // holding them would report.
        let mut present = status();
        present.volatile_assets = Some(VolatileAssetStats {
            committed_count: 4,
            slot_capacity: 16,
            used_bytes: 1_318_960,
            psram_free_bytes: 7_012_352,
            psram_low_water_bytes: 6_803_456,
        });
        round_trip(
            "status_volatile_pool_stats",
            42,
            &Message::StatusResponse(present),
        );

        // Absent is every image built before the pool existed, including the one
        // the fleet is running right now.
        assert!(status().volatile_assets.is_none());
        let absent = encode_payload(&Message::StatusResponse(status())).unwrap();
        assert!(decode_status(&absent).unwrap().volatile_assets.is_none());

        // Keys 31 and 32 are independent, and both present is what a device on
        // the new image actually sends.
        let mut both = status();
        both.asset_store = Some(AssetStoreStats {
            used_bytes: 4,
            free_bytes: 9,
            asset_count: 2,
        });
        both.volatile_assets = Some(VolatileAssetStats {
            committed_count: 1,
            slot_capacity: 2,
            used_bytes: 3,
            psram_free_bytes: 5,
            psram_low_water_bytes: 6,
        });
        let mut trunk = encode_payload(&Message::StatusResponse(both)).unwrap();
        let mut decoder = Decoder::new(&trunk);
        assert_eq!(
            decoder.map_len().unwrap(),
            31,
            "keys 31 and 32 should add one outer entry each"
        );
        let pool = trunk.split_off(trunk.len() - 13);
        assert_eq!(
            pool,
            [
                0x18, 0x20, 0xa5, 0x00, 0x01, 0x01, 0x02, 0x02, 0x03, 0x03, 0x05, 0x04, 0x06
            ]
        );

        // Each omission separately: one case pins only the last field read, and
        // a zero here reads as a device with no pool or no memory left.
        for (omitted, pairs) in [
            (
                "committed count",
                vec![0x01, 0x02, 0x02, 0x03, 0x03, 0x05, 0x04, 0x06],
            ),
            (
                "slot capacity",
                vec![0x00, 0x01, 0x02, 0x03, 0x03, 0x05, 0x04, 0x06],
            ),
            (
                "used bytes",
                vec![0x00, 0x01, 0x01, 0x02, 0x03, 0x05, 0x04, 0x06],
            ),
            (
                "PSRAM free",
                vec![0x00, 0x01, 0x01, 0x02, 0x02, 0x03, 0x04, 0x06],
            ),
            (
                "PSRAM low water",
                vec![0x00, 0x01, 0x01, 0x02, 0x02, 0x03, 0x03, 0x05],
            ),
        ] {
            let mut partial = trunk.clone();
            partial.extend_from_slice(&[0x18, 0x20, 0xa4]);
            partial.extend_from_slice(&pairs);
            assert_eq!(
                decode_status(&partial),
                Err(MessageError::MissingField(32)),
                "a pool map without its {omitted} should be refused"
            );
        }

        // A sub-key this host has never heard of is skipped, not refused.
        let mut extended = trunk;
        extended.extend_from_slice(&[
            0x18, 0x20, 0xa6, 0x00, 0x01, 0x01, 0x02, 0x02, 0x03, 0x03, 0x05, 0x04, 0x06, 0x05,
            0x07,
        ]);
        assert_eq!(
            decode_status(&extended).unwrap().volatile_assets,
            Some(VolatileAssetStats {
                committed_count: 1,
                slot_capacity: 2,
                used_bytes: 3,
                psram_free_bytes: 5,
                psram_low_water_bytes: 6,
            })
        );
    }

    #[test]
    fn config_rotation_is_landscape_only_and_legacy_payloads_default_to_90() {
        let config = ApplyConfig {
            brightness: None,
            revision: 1,
            rotation: 270,
            cards: vec![CardConfig {
                card_id: "clock".into(),
                tap_action: TapAction::None,
            }],
        };
        let mut payload = encode_payload(&Message::ApplyConfig(config.clone())).unwrap();
        assert_eq!(payload[0], 0xa3, "config should be a three-entry CBOR map");
        assert_eq!(
            payload.split_off(payload.len() - 4),
            [0x03, 0x19, 0x01, 0x0e]
        );
        payload[0] = 0xa2;
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
                &Message::PushTimer(PushTimer {
                    card_id: "x".repeat(MAX_CARD_ID_LEN + 1),
                    revision: 1,
                    total_ms: 0,
                    remaining_ms: 0,
                    running: false,
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
        // v1 is a real version this build deliberately does not speak.
        frame.version = 1;
        assert_eq!(decode_message(&frame), Err(MessageError::Version(1)));
        frame.version = PROTOCOL_VERSION;
        frame.message_type = 99;
        assert_eq!(
            decode_message(&frame),
            Err(MessageError::UnsupportedType(99))
        );
    }

    /// Encodes a `PushScene` payload WITHOUT validating it, so the decode
    /// direction can be handed something a conforming host would never send.
    fn encode_push_scene_payload_unchecked(push: &PushScene) -> Vec<u8> {
        let mut encoder = Encoder::new();
        encode_push_scene_payload(&mut encoder, push);
        encoder.into_bytes()
    }

    fn sample_scene() -> Scene {
        Scene {
            revision: 9,
            background: 0x0000_0000,
            nodes: vec![
                SceneNode::Rect(SceneRect {
                    x: 4,
                    y: 5,
                    w: 10,
                    h: 11,
                    ..SceneRect::default()
                }),
                SceneNode::Text(SceneText {
                    x: 16,
                    baseline_y: 200,
                    w: 400,
                    font: SceneFont::Baked(SceneFontTier::Body),
                    value: SceneValue::Binding("time:HH:mm".into()),
                    ..SceneText::default()
                }),
            ],
        }
    }

    #[test]
    fn push_scene_rejects_a_scene_over_the_node_cap() {
        // Every node is individually valid and inside the canvas, and the
        // envelope around them is one the message layer accepts, so the count
        // is the only thing that can refuse this.
        let node = SceneNode::Rect(SceneRect {
            x: 4,
            y: 5,
            w: 10,
            h: 11,
            ..SceneRect::default()
        });
        let at_cap = PushScene {
            card_id: "clock".into(),
            revision: 3,
            scene: Scene {
                revision: 9,
                background: 0,
                nodes: vec![node.clone(); MAX_SCENE_NODES],
            },
        };
        let wire = encode_message(45, &Message::PushScene(at_cap.clone())).unwrap();
        assert!(decode_wire(&wire).is_ok());

        let mut over_cap = at_cap;
        over_cap.scene.nodes.push(node);
        // The encoder refuses to emit it at all ...
        assert_eq!(
            encode_message(45, &Message::PushScene(over_cap.clone())),
            Err(MessageError::InvalidValue("scene node count"))
        );
        // ... and a hand-built frame carrying one is refused on decode, which
        // is the direction that matters for an untrusted peer.
        let payload = encode_push_scene_payload_unchecked(&over_cap);
        let frame = Frame::new(TYPE_PUSH_SCENE, 45, payload);
        assert_eq!(
            decode_message(&frame),
            Err(MessageError::InvalidValue("scene node count"))
        );
    }

    /// The mirror of C's `test_push_scene_skips_an_unknown_key_after_the_scene`.
    ///
    /// `docs/protocol/v2.md` promises unknown integer keys are skipped so a
    /// later revision can add a field. Every other `PushScene` test and both
    /// fixtures end the payload map at key 2, so nothing else reads the
    /// decoder's position AFTER the nested scene -- on the firmware side that
    /// is what makes `cbor_value_leave_container()` untested, and on this side
    /// it is what makes the `_ => decoder.skip()?` arm untested for anything
    /// following a scene.
    #[test]
    fn push_scene_skips_an_unknown_key_after_the_scene() {
        let push = PushScene {
            card_id: "clock".into(),
            revision: 6,
            scene: sample_scene(),
        };
        // Hand-built rather than encoded: a conforming encoder never emits an
        // unknown key, which is exactly why this case needs one.
        let mut encoder = Encoder::new();
        encoder.map(4);
        encoder.unsigned(0);
        encoder.text(&push.card_id);
        encoder.unsigned(1);
        encoder.unsigned(u64::from(push.revision));
        encoder.unsigned(2);
        crate::scene::encode_scene(&mut encoder, &push.scene);
        encoder.unsigned(3);
        encoder.unsigned(42);

        let frame = Frame::new(TYPE_PUSH_SCENE, 45, encoder.into_bytes());
        // Not merely accepted -- everything before the skipped key survives.
        assert_eq!(decode_message(&frame), Ok(Message::PushScene(push)));
    }

    #[test]
    fn push_scene_rejects_an_off_canvas_node() {
        let message = Message::PushScene(PushScene {
            card_id: "clock".into(),
            revision: 3,
            scene: Scene {
                revision: 9,
                background: 0,
                nodes: vec![SceneNode::Rect(SceneRect {
                    x: 0,
                    y: 0,
                    w: 4096,
                    h: 10,
                    ..SceneRect::default()
                })],
            },
        });
        assert!(encode_message(45, &message).is_err());
    }

    #[test]
    fn push_scene_rejects_a_zero_revision() {
        let message = Message::PushScene(PushScene {
            card_id: "clock".into(),
            revision: 0,
            scene: sample_scene(),
        });
        assert!(encode_message(45, &message).is_err());
    }

    #[test]
    fn ack_for_push_scene_requires_a_revision() {
        let with = Message::Ack(Ack {
            acknowledged_type: TYPE_PUSH_SCENE,
            revision: Some(5),
            already_present: None,
        });
        let wire = encode_message(45, &with).unwrap();
        assert_eq!(decode_wire(&wire).unwrap(), with);

        let without = Message::Ack(Ack {
            acknowledged_type: TYPE_PUSH_SCENE,
            revision: None,
            already_present: None,
        });
        assert!(encode_message(45, &without).is_err());
    }

    #[test]
    fn asset_begin_roundtrips() {
        let message = Message::AssetBegin(AssetBegin {
            digest: [0x5a; 32],
            kind: AssetKind::Font,
            total_length: 4096,
            volatile: false,
            encoding: ASSET_ENCODING_RAW,
            decoded_length: None,
        });
        let frame = encode_message(7, &message).expect("encode");
        let decoded = decode_wire(&frame).expect("decode");
        assert_eq!(decoded, message);
        // Four keys: digest, kind, total_length, and volatile. Key 3 is
        // always emitted -- false included -- because every deployed decoder
        // requires it; only the encoding keys (4/5) follow the omit-default
        // rule, and a raw begin therefore carries neither.
        let wire_frame = crate::decode_wire_frame(&frame).unwrap();
        assert_eq!(wire_frame.payload[0], 0xa4, "raw begin: 4 keys, no 4/5");
    }

    #[test]
    fn asset_chunk_rejects_oversize_data() {
        let message = Message::AssetChunk(AssetChunk {
            digest: [0x01; 32],
            offset: 0,
            data: vec![0u8; MAX_ASSET_CHUNK_BYTES + 1],
        });
        assert!(matches!(
            encode_message(8, &message),
            Err(MessageError::InvalidValue("asset chunk data too large"))
        ));
    }

    #[test]
    fn asset_release_rejects_too_many_digests() {
        let message = Message::AssetRelease(AssetRelease {
            digests: vec![[0u8; 32]; MAX_ASSET_DIGESTS + 1],
        });
        assert!(matches!(
            encode_message(9, &message),
            Err(MessageError::InvalidValue("too many asset digests"))
        ));
    }

    #[test]
    fn asset_begin_rejects_out_of_range_total_length() {
        let mut too_small = Message::AssetBegin(AssetBegin {
            digest: [0x5a; 32],
            kind: AssetKind::Image,
            total_length: 0,
            volatile: true,
            encoding: ASSET_ENCODING_RAW,
            decoded_length: None,
        });
        assert!(encode_message(7, &too_small).is_err());
        if let Message::AssetBegin(begin) = &mut too_small {
            begin.total_length = 1_048_577;
        }
        assert!(encode_message(7, &too_small).is_err());
    }

    #[test]
    fn an_encoded_durable_asset_is_accepted_now_that_the_durable_tier_decodes() {
        // This was rejected outright until bit 10, and rejecting it is what
        // made a picture card's frame cross the wire as 172 raw chunks.
        let begin = AssetBegin {
            digest: [0x5a; 32],
            kind: AssetKind::Image,
            total_length: 10_032,
            volatile: false,
            encoding: ASSET_ENCODING_RLE565,
            decoded_length: Some(VOLATILE_IMAGE_DECODED_LENGTH),
        };
        assert!(validate_message(&Message::AssetBegin(begin)).is_ok());
        // And it has to survive encoding, which is where the device-facing
        // failure actually surfaced.
        assert!(encode_message(1, &Message::AssetBegin(begin)).is_ok());
    }

    #[test]
    fn a_durable_encoded_image_may_be_any_bounded_size_but_a_volatile_one_may_not() {
        // A volatile image lands in a fixed-size PSRAM slot, so its decoded
        // length is pinned. A durable image is an ordinary stored asset, and
        // pinning it would reject every scene image that is not a whole
        // screen.
        let partial = AssetBegin {
            digest: [0x5a; 32],
            kind: AssetKind::Image,
            total_length: 1_000,
            volatile: false,
            encoding: ASSET_ENCODING_RLE565,
            decoded_length: Some(4_096),
        };
        assert!(validate_message(&Message::AssetBegin(partial)).is_ok());
        assert!(
            validate_message(&Message::AssetBegin(AssetBegin {
                volatile: true,
                ..partial
            }))
            .is_err()
        );
    }

    #[test]
    fn asset_begin_rejects_invalid_encoding_relationships() {
        let base = AssetBegin {
            digest: [0x5a; 32],
            kind: AssetKind::Image,
            total_length: 10_032,
            volatile: true,
            encoding: ASSET_ENCODING_RLE565,
            decoded_length: Some(VOLATILE_IMAGE_DECODED_LENGTH),
        };
        for invalid in [
            AssetBegin {
                encoding: 2,
                ..base
            },
            AssetBegin {
                encoding: ASSET_ENCODING_RAW,
                ..base
            },
            AssetBegin {
                decoded_length: None,
                ..base
            },
            AssetBegin {
                decoded_length: Some(0),
                ..base
            },
            AssetBegin {
                decoded_length: Some(MAX_ASSET_TOTAL_LENGTH + 1),
                ..base
            },
            AssetBegin {
                total_length: VOLATILE_IMAGE_DECODED_LENGTH,
                ..base
            },
        ] {
            assert!(validate_message(&Message::AssetBegin(invalid)).is_err());
        }
    }

    #[test]
    fn already_present_is_rejected_on_non_asset_begin_acks() {
        let message = Message::Ack(Ack {
            acknowledged_type: TYPE_ASSET_COMMIT,
            revision: None,
            already_present: Some(true),
        });
        assert!(encode_message(10, &message).is_err());
    }

    #[test]
    fn already_present_is_required_on_asset_begin_acks() {
        let message = Message::Ack(Ack {
            acknowledged_type: TYPE_ASSET_BEGIN,
            revision: None,
            already_present: None,
        });
        assert!(encode_message(12, &message).is_err());
    }
}
