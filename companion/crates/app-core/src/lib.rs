pub mod asset_sync;
pub mod config;
mod interrupts;
mod pomodoro;
pub mod render_negotiation;
pub mod runtime;
mod runtime_command;
pub mod scene_build;
mod scheduler;
pub mod secure_file;
pub mod state;
pub mod store;

pub use asset_sync::DesiredAsset;
pub use config::{
    AlertHold, AppConfig, AppPreferences, AssetKind, AssetSettings, AssetSource,
    CURRENT_SCHEMA_VERSION, CardAlert, CardSettings, CarouselAdvance, CompiledAppConfig,
    ConfigValidationError, DisplayOrientation, DisplayTemplate, IconGlyphMapping,
    MAX_ACTION_URL_LEN, MAX_ALERT_HOLD_SECONDS, MAX_ASSET_BYTES, MAX_ASSET_SOURCE_LEN, MAX_ASSETS,
    MAX_CARD_ID_LEN, MAX_CARD_REFRESH_MINUTES, MAX_CONFIG_CARDS, MAX_DWELL_SECONDS,
    MAX_HOST_ACTION_TARGET_LEN, MAX_ICON_GLYPH_NAME_LEN, MAX_ICON_GLYPHS, MAX_TIMEZONE_LEN,
    MAX_TOTAL_ASSET_BYTES, MAX_WIDGET_TITLE_LEN, MIN_ALERT_HOLD_SECONDS, MIN_CARD_REFRESH_MINUTES,
    MIN_DWELL_SECONDS, RefreshPolicy, UpdateChannel, UpdateCheckPolicy, UpdaterSettings,
    ValidationCode, ValidationIssue, WidgetTapAction, utc_offset_minutes,
};
pub use protocol::{
    MAX_DEVICE_ID_LEN, MAX_DEVICE_TOKEN_LEN, MAX_PSK_LEN, MAX_SERVER_URL_LEN, MAX_SSID_LEN,
};
pub use render_negotiation::{
    DeviceRenderProfile, RenderRequirements, analyze_scene, validate_native_scene,
};
pub use runtime::{
    DeviceConnection, ImageSourceFrame, ImageSourceHost, RuntimeDevice, RuntimeHandle,
    RuntimeOptions, RuntimeSubscription, empty_device, initial_snapshot, preview_card_scene,
};
pub use runtime_command::{PomodoroAction, RuntimeError};
pub use scene_build::{
    AnalogClockCard, ClockCard, ProgressRingCard, build_analog_clock_scene,
    build_digital_clock_scene, build_progress_ring_scene,
};
pub use state::{
    AppSnapshot, CardDataSnapshot, CardError, CardErrorKind, CardField, CardFieldValue,
    ConnectionState, DeviceAssetStore, DeviceCapability, DeviceCounters, DeviceOtaState,
    DeviceSnapshot, DeviceTier, DeviceWifiState, PersistenceState, PomodoroSnapshot, PomodoroState,
    RuntimeDiagnostics, RuntimeState,
};
pub use store::{
    ConfigOrigin, ConfigStore, LoadOutcome, MAX_CONFIG_FILE_BYTES,
    SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE, SaveReceipt, StoreError, StoreWarning,
};
