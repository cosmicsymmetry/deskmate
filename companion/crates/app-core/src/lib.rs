pub mod commands;
pub mod config;
pub mod runtime;
mod scheduler;
pub mod state;
pub mod store;

pub use commands::{PomodoroAction, RuntimeError};
pub use config::{
    AlertHold, AppConfig, AppPreferences, AssetKind, AssetSettings, AssetSource,
    CURRENT_SCHEMA_VERSION, CalendarSource, CardAlert, CardSettings, CarouselAdvance,
    CompiledAppConfig, ConfigValidationError, DisplayOrientation, DisplayTemplate,
    ED25519_SIGNATURE_BASE64_LEN, FirmwareArtifactMetadata, GlyphRange, JsonFieldMapping,
    MAX_ALERT_HOLD_SECONDS, MAX_ALERT_LEAD_MINUTES, MAX_ASSET_BYTES, MAX_ASSET_SOURCE_LEN,
    MAX_ASSETS, MAX_CONFIG_CARDS, MAX_DWELL_SECONDS, MAX_FONT_GLYPHS, MAX_GLYPH_RANGES,
    MAX_HOST_ACTION_TARGET_LEN, MAX_ICON_DIMENSION, MAX_ICS_SOURCE_LEN, MAX_JSON_MAPPINGS,
    MAX_JSON_PATH_LEN, MAX_LOCATION_LEN, MAX_PLAYLIST_ENTRIES, MAX_PLAYLIST_NAME_LEN,
    MAX_PLAYLISTS, MAX_PROVIDER_REDIRECTS, MAX_PROVIDER_RESPONSE_BYTES, MAX_PROVIDER_URL_LEN,
    MAX_RSS_ITEMS, MAX_SIGNING_KEY_ID_LEN, MAX_TIMEZONE_LEN, MAX_TOTAL_ASSET_BYTES,
    MAX_UPDATE_ARTIFACT_BYTES, MAX_UPDATE_MODEL_LEN, MAX_UPDATE_VERSION_LEN, MAX_WIDGET_ID_LEN,
    MAX_WIDGET_TITLE_LEN, MIN_ALERT_HOLD_SECONDS, MIN_ALERT_LEAD_MINUTES, MIN_DWELL_SECONDS,
    MIN_WEATHER_REFRESH_MINUTES, PROVIDER_REQUEST_TIMEOUT_SECONDS, Playlist, PlaylistEntry,
    RefreshPolicy, UpdateChannel, UpdateCheckPolicy, UpdaterSettings, ValidationCode,
    ValidationIssue, WeatherUnits, WidgetTapAction, utc_offset_minutes,
};
pub use providers::ics::MAX_ICS_BYTES;
pub use runtime::{
    CalendarRefreshRequest, CalendarRefreshResult, CalendarRefresher, DeviceConnection,
    ProviderRefreshRequest, ProviderRefreshResult, ProviderRefresher, ProviderRequest,
    RuntimeDevice, RuntimeHandle, RuntimeOptions, RuntimeSubscription, SerialRuntimeDevice,
    SystemCalendarRefresher, SystemProviderRefresher,
};
pub use state::{
    AppSnapshot, CardDataSnapshot, CardError, CardField, CardFieldValue, ConnectionState,
    DeviceCapability, DeviceCounters, DeviceSnapshot, PersistenceState, PomodoroSnapshot,
    PomodoroState, ProviderSnapshot, ProviderState, RuntimeDiagnostics, RuntimeState,
};
pub use store::{
    ConfigOrigin, ConfigStore, LoadOutcome, MAX_CONFIG_FILE_BYTES, SaveReceipt, StoreError,
    StoreWarning,
};
