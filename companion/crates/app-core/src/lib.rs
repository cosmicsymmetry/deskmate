pub mod asset_sync;
pub mod commands;
pub mod config;
pub mod network_settings;
pub mod render_negotiation;
pub mod runtime;
pub mod scene_build;
mod scheduler;
mod secure_file;
pub mod state;
pub mod store;

pub use asset_sync::DesiredAsset;
pub use commands::{PomodoroAction, RuntimeError};
pub use config::{
    AlertHold, AppConfig, AppPreferences, AssetKind, AssetSettings, AssetSource,
    CURRENT_SCHEMA_VERSION, CalendarSource, CardAlert, CardSettings, CarouselAdvance,
    CompiledAppConfig, ConfigValidationError, DisplayOrientation, DisplayTemplate,
    ED25519_SIGNATURE_BASE64_LEN, FirmwareArtifactMetadata, IconGlyphMapping, JsonFieldMapping,
    MAX_ALERT_HOLD_SECONDS, MAX_ALERT_LEAD_MINUTES, MAX_ASSET_BYTES, MAX_ASSET_SOURCE_LEN,
    MAX_ASSETS, MAX_CONFIG_CARDS, MAX_DWELL_SECONDS, MAX_HOST_ACTION_TARGET_LEN,
    MAX_ICON_GLYPH_NAME_LEN, MAX_ICON_GLYPHS, MAX_ICS_SOURCE_LEN, MAX_JSON_MAPPINGS,
    MAX_JSON_PATH_LEN, MAX_LOCATION_LEN, MAX_PLAYLIST_ENTRIES, MAX_PLAYLIST_NAME_LEN,
    MAX_PLAYLISTS, MAX_PLUGIN_ID_LEN, MAX_PROVIDER_REDIRECTS, MAX_PROVIDER_RESPONSE_BYTES,
    MAX_PROVIDER_URL_LEN, MAX_RSS_ITEMS, MAX_SIGNING_KEY_ID_LEN, MAX_TIMEZONE_LEN,
    MAX_TOTAL_ASSET_BYTES, MAX_UPDATE_ARTIFACT_BYTES, MAX_UPDATE_MODEL_LEN, MAX_UPDATE_VERSION_LEN,
    MAX_WIDGET_ID_LEN, MAX_WIDGET_TITLE_LEN, MIN_ALERT_HOLD_SECONDS, MIN_ALERT_LEAD_MINUTES,
    MIN_DWELL_SECONDS, MIN_WEATHER_REFRESH_MINUTES, PROVIDER_REQUEST_TIMEOUT_SECONDS, Playlist,
    PlaylistEntry, RefreshPolicy, UpdateChannel, UpdateCheckPolicy, UpdaterSettings,
    ValidationCode, ValidationIssue, WeatherUnits, WidgetTapAction, utc_offset_minutes,
};
pub use network_settings::{
    MAX_NETWORK_SETTINGS_FILE_BYTES, NETWORK_SETTINGS_FORMAT_VERSION, NetworkSettings,
    NetworkSettingsLoadOutcome, NetworkSettingsOrigin, NetworkSettingsSaveReceipt,
    NetworkSettingsStore, NetworkSettingsStoreError, NetworkSettingsStoreWarning,
    NetworkSettingsUpdate,
};
pub use protocol::{NetworkConfig, Tier as ProvisioningTier};
pub use providers::ics::MAX_ICS_BYTES;
pub use render_negotiation::{
    BindingClass, BindingRequirements, DeviceRenderProfile, NativeSource, RenderDecision,
    RenderRequirements, RequirementsError, SceneNodeKind, analyze_raster_only, analyze_scene,
    classify_binding, negotiate,
};
pub use runtime::{
    CalendarRefreshRequest, CalendarRefreshResult, CalendarRefresher, DeviceConnection, PluginHost,
    ProviderRefreshRequest, ProviderRefreshResult, ProviderRefresher, ProviderRequest,
    RuntimeDevice, RuntimeHandle, RuntimeOptions, RuntimeSubscription, SceneCandidate,
    SerialRuntimeDevice, SystemCalendarRefresher, SystemProviderRefresher,
};
pub use scene_build::{
    AnalogClockCard, BakedFontMetrics, BigNumberCard, ClockCard, IconBadgeCard, NumericAdvances,
    ProgressRingCard, RowListCard, SHIPPED_SCENE_SURFACE_COLOR, SceneDataState, TierMetrics,
    build_analog_clock_scene, build_big_number_label_scene, build_digital_clock_scene,
    build_icon_badge_text_scene, build_progress_ring_scene, build_row_list_scene, number_font_tier,
    text_is_numeric, with_scene_data_state,
};
pub use state::{
    AppSnapshot, CardDataSnapshot, CardError, CardErrorKind, CardField, CardFieldValue,
    ConnectionState, DeviceCapability, DeviceCounters, DeviceOtaState, DeviceSnapshot, DeviceTier,
    DeviceWifiState, PersistenceState, PomodoroSnapshot, PomodoroState, ProviderSnapshot,
    ProviderState, RuntimeDiagnostics, RuntimeState,
};
pub use store::{
    ConfigOrigin, ConfigStore, LoadOutcome, MAX_CONFIG_FILE_BYTES,
    SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE, SaveReceipt, StoreError, StoreWarning,
};
