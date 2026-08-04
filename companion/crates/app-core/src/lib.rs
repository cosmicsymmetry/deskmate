pub mod commands;
pub mod config;
pub mod runtime;
mod scheduler;
pub mod state;
pub mod store;

pub use commands::{PomodoroAction, RuntimeError};
pub use config::{
    AppConfig, AppPreferences, CURRENT_SCHEMA_VERSION, CalendarSource, CompiledAppConfig,
    ConfigValidationError, MAX_WIDGET_ID_LEN, ScreenSettings, ValidationCode, ValidationIssue,
    WidgetSettings, WidgetSize,
};
pub use runtime::{
    CalendarRefreshRequest, CalendarRefreshResult, CalendarRefresher, DeviceConnection,
    RuntimeDevice, RuntimeHandle, RuntimeOptions, RuntimeSubscription, SerialRuntimeDevice,
    SystemCalendarRefresher,
};
pub use state::{
    AppSnapshot, ConnectionState, DeviceCounters, DeviceSnapshot, PersistenceState,
    PomodoroSnapshot, PomodoroState, ProviderSnapshot, ProviderState, RuntimeDiagnostics,
    RuntimeState,
};
pub use store::{
    ConfigOrigin, ConfigStore, LoadOutcome, MAX_CONFIG_FILE_BYTES, SaveReceipt, StoreError,
    StoreWarning,
};
