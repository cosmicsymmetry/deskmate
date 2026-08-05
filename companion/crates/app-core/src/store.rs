use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};

use crate::{
    AppConfig, AppPreferences, CURRENT_SCHEMA_VERSION, CalendarSource, CarouselSettings,
    DisplayTemplate, RefreshPolicy, ScreenLayout, ScreenSettings, UpdaterSettings, ValidationIssue,
    WidgetInterruptPolicy, WidgetSettings, WidgetSize, WidgetTapAction,
};

pub const MAX_CONFIG_FILE_BYTES: usize = 64 * 1_024;

pub struct ConfigStore {
    path: PathBuf,
    state: Mutex<StoreState>,
}

struct StoreState {
    last_good: AppConfig,
    generation: u64,
}

impl ConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            state: Mutex::new(StoreState {
                last_good: AppConfig::default(),
                generation: 0,
            }),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> LoadOutcome {
        let Ok(mut state) = self.state.lock() else {
            return LoadOutcome {
                config: AppConfig::default(),
                origin: ConfigOrigin::LastGood,
                recovery: Some(StoreError::LockPoisoned),
            };
        };

        let bytes = match read_bounded(&self.path) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                let config = AppConfig::default();
                state.last_good = config.clone();
                return LoadOutcome {
                    config,
                    origin: ConfigOrigin::Defaults,
                    recovery: None,
                };
            }
            Err(error) => return recovered(&state, error),
        };

        match decode_config(&bytes) {
            Ok((config, origin)) => {
                state.last_good = config.clone();
                LoadOutcome {
                    config,
                    origin,
                    recovery: None,
                }
            }
            Err(error) => recovered(&state, error),
        }
    }

    pub fn save(&self, config: &AppConfig) -> Result<SaveReceipt, StoreError> {
        let mut state = self.state.lock().map_err(|_| StoreError::LockPoisoned)?;
        config.validate().map_err(|error| StoreError::Validation {
            issues: error.issues,
        })?;
        let bytes = serde_json::to_vec_pretty(config).map_err(|error| StoreError::InvalidJson {
            message: error.to_string(),
        })?;
        if bytes.len() > MAX_CONFIG_FILE_BYTES {
            return Err(StoreError::TooLarge {
                maximum: MAX_CONFIG_FILE_BYTES,
            });
        }

        let parent = usable_parent(&self.path)?;
        fs::create_dir_all(parent).map_err(|error| io_error("create config directory", &error))?;
        write_and_replace(&self.path, &bytes)?;

        state.generation = state.generation.saturating_add(1);
        state.last_good = config.clone();
        let warning = sync_parent(parent).err().map(|error| StoreWarning::Io {
            operation: "sync config directory".into(),
            message: error.to_string(),
        });
        Ok(SaveReceipt {
            generation: state.generation,
            warning,
        })
    }

    pub fn last_good(&self) -> Result<AppConfig, StoreError> {
        self.state
            .lock()
            .map(|state| state.last_good.clone())
            .map_err(|_| StoreError::LockPoisoned)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigOrigin {
    Defaults,
    Current,
    MigratedV0,
    MigratedV1,
    LastGood,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadOutcome {
    pub config: AppConfig,
    pub origin: ConfigOrigin,
    pub recovery: Option<StoreError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveReceipt {
    pub generation: u64,
    pub warning: Option<StoreWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum StoreError {
    Io { operation: String, message: String },
    InvalidPath { message: String },
    TooLarge { maximum: usize },
    InvalidUtf8,
    InvalidJson { message: String },
    UnsupportedVersion { found: u32, supported: u32 },
    Validation { issues: Vec<ValidationIssue> },
    LockPoisoned,
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { operation, message } => write!(formatter, "{operation}: {message}"),
            Self::InvalidPath { message } => write!(formatter, "invalid config path: {message}"),
            Self::TooLarge { maximum } => {
                write!(formatter, "config exceeds the {maximum}-byte limit")
            }
            Self::InvalidUtf8 => formatter.write_str("config is not valid UTF-8"),
            Self::InvalidJson { message } => write!(formatter, "invalid config JSON: {message}"),
            Self::UnsupportedVersion { found, supported } => write!(
                formatter,
                "config schema version {found} is unsupported; expected {supported}"
            ),
            Self::Validation { issues } => {
                write!(formatter, "config has {} validation issue(s)", issues.len())
            }
            Self::LockPoisoned => formatter.write_str("config store lock is poisoned"),
        }
    }
}

impl std::error::Error for StoreError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum StoreWarning {
    Io { operation: String, message: String },
}

#[derive(Deserialize)]
struct VersionHeader {
    schema_version: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyConfigV0 {
    schema_version: u32,
    timezone: String,
    widgets: Vec<LegacyWidgetSettings>,
    screens: Vec<LegacyScreenSettings>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyConfigV1 {
    schema_version: u32,
    preferences: AppPreferences,
    widgets: Vec<LegacyWidgetSettings>,
    screens: Vec<LegacyScreenSettings>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum LegacyWidgetSettings {
    Clock {
        id: String,
        size: WidgetSize,
        title: String,
        show_seconds: bool,
    },
    Pomodoro {
        id: String,
        size: WidgetSize,
        label: String,
        duration_seconds: u32,
    },
    Calendar {
        id: String,
        size: WidgetSize,
        title: String,
        source: CalendarSource,
        refresh_minutes: u16,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyScreenSettings {
    id: String,
    widget_id: String,
}

fn decode_config(bytes: &[u8]) -> Result<(AppConfig, ConfigOrigin), StoreError> {
    let text = std::str::from_utf8(bytes).map_err(|_| StoreError::InvalidUtf8)?;
    let header: VersionHeader =
        serde_json::from_str(text).map_err(|error| StoreError::InvalidJson {
            message: error.to_string(),
        })?;
    let (config, origin) = match header.schema_version {
        CURRENT_SCHEMA_VERSION => (
            serde_json::from_str(text).map_err(|error| StoreError::InvalidJson {
                message: error.to_string(),
            })?,
            ConfigOrigin::Current,
        ),
        1 => {
            let legacy: LegacyConfigV1 =
                serde_json::from_str(text).map_err(|error| StoreError::InvalidJson {
                    message: error.to_string(),
                })?;
            if legacy.schema_version != 1 {
                return Err(StoreError::UnsupportedVersion {
                    found: legacy.schema_version,
                    supported: CURRENT_SCHEMA_VERSION,
                });
            }
            (
                migrate_legacy(legacy.preferences, legacy.widgets, legacy.screens),
                ConfigOrigin::MigratedV1,
            )
        }
        0 => {
            let legacy: LegacyConfigV0 =
                serde_json::from_str(text).map_err(|error| StoreError::InvalidJson {
                    message: error.to_string(),
                })?;
            if legacy.schema_version != 0 {
                return Err(StoreError::UnsupportedVersion {
                    found: legacy.schema_version,
                    supported: CURRENT_SCHEMA_VERSION,
                });
            }
            (
                migrate_legacy(
                    AppPreferences {
                        timezone: legacy.timezone,
                        ..AppPreferences::default()
                    },
                    legacy.widgets,
                    legacy.screens,
                ),
                ConfigOrigin::MigratedV0,
            )
        }
        found => {
            return Err(StoreError::UnsupportedVersion {
                found,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }
    };
    config.validate().map_err(|error| StoreError::Validation {
        issues: error.issues,
    })?;
    Ok((config, origin))
}

fn migrate_legacy(
    preferences: AppPreferences,
    widgets: Vec<LegacyWidgetSettings>,
    screens: Vec<LegacyScreenSettings>,
) -> AppConfig {
    AppConfig {
        schema_version: CURRENT_SCHEMA_VERSION,
        preferences,
        widgets: widgets
            .into_iter()
            .map(|widget| match widget {
                LegacyWidgetSettings::Clock {
                    id,
                    size,
                    title,
                    show_seconds,
                } => WidgetSettings::Clock {
                    id,
                    size,
                    title,
                    show_seconds,
                    template: DisplayTemplate::DigitalClock,
                    tap_action: WidgetTapAction::None,
                    refresh: RefreshPolicy::DeviceLocal,
                    interrupt_policy: WidgetInterruptPolicy::Disabled,
                },
                LegacyWidgetSettings::Pomodoro {
                    id,
                    size,
                    label,
                    duration_seconds,
                } => WidgetSettings::Pomodoro {
                    id,
                    size,
                    label,
                    duration_seconds,
                    template: DisplayTemplate::ProgressRing,
                    tap_action: WidgetTapAction::StartPause,
                    refresh: RefreshPolicy::DeviceLocal,
                    interrupt_policy: WidgetInterruptPolicy::Enabled,
                },
                LegacyWidgetSettings::Calendar {
                    id,
                    size,
                    title,
                    source,
                    refresh_minutes,
                } => WidgetSettings::Calendar {
                    id,
                    size,
                    title,
                    source,
                    template: DisplayTemplate::RowList,
                    tap_action: WidgetTapAction::None,
                    refresh: RefreshPolicy::Interval {
                        minutes: refresh_minutes,
                    },
                    interrupt_policy: WidgetInterruptPolicy::Disabled,
                },
            })
            .collect(),
        screens: screens
            .into_iter()
            .map(|screen| ScreenSettings {
                id: screen.id,
                layout: ScreenLayout::Single {
                    widget_id: screen.widget_id,
                },
            })
            .collect(),
        assets: Vec::new(),
        carousel: CarouselSettings::default(),
        updater: UpdaterSettings::default(),
    }
}

fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("open config", &error)),
    };
    let mut bytes = Vec::new();
    file.take((MAX_CONFIG_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read config", &error))?;
    if bytes.len() > MAX_CONFIG_FILE_BYTES {
        Err(StoreError::TooLarge {
            maximum: MAX_CONFIG_FILE_BYTES,
        })
    } else {
        Ok(Some(bytes))
    }
}

fn recovered(state: &StoreState, error: StoreError) -> LoadOutcome {
    LoadOutcome {
        config: state.last_good.clone(),
        origin: ConfigOrigin::LastGood,
        recovery: Some(error),
    }
}

fn usable_parent(path: &Path) -> Result<&Path, StoreError> {
    let parent = path.parent().ok_or_else(|| StoreError::InvalidPath {
        message: format!("{} has no parent directory", path.display()),
    })?;
    if parent.as_os_str().is_empty() {
        Ok(Path::new("."))
    } else {
        Ok(parent)
    }
}

fn write_and_replace(target: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let mut file = AtomicWriteFile::options()
        .open(target)
        .map_err(|error| io_error("create temporary config", &error))?;
    file.write_all(bytes)
        .map_err(|error| io_error("write temporary config", &error))?;
    file.write_all(b"\n")
        .map_err(|error| io_error("finish temporary config", &error))?;
    file.commit()
        .map_err(|error| io_error("sync and replace config", &error))
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> io::Result<()> {
    Ok(())
}

fn io_error(operation: &str, error: &io::Error) -> StoreError {
    StoreError::Io {
        operation: operation.into(),
        message: error.to_string(),
    }
}
