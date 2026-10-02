use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::secure_file::{self, BoundedReadError, FileIoError};
use crate::{AppConfig, CURRENT_SCHEMA_VERSION, ValidationIssue};

pub const MAX_CONFIG_FILE_BYTES: usize = 64 * 1_024;
pub const SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE: &str =
    "Your saved settings failed validation and were not applied";

pub struct ConfigStore {
    path: PathBuf,
    state: Mutex<StoreState>,
}

struct StoreState {
    last_good: Option<AppConfig>,
    generation: u64,
}

impl ConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            state: Mutex::new(StoreState {
                last_good: None,
                generation: 0,
            }),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> LoadOutcome {
        let Ok(mut state) = self.state.lock() else {
            return LoadOutcome::Recovered {
                config: AppConfig::default(),
                origin: ConfigOrigin::Defaults,
                error: StoreError::LockPoisoned,
            };
        };

        let bytes = match read_bounded(&self.path) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                let config = AppConfig::default();
                return LoadOutcome::Loaded {
                    config,
                    origin: ConfigOrigin::Defaults,
                };
            }
            Err(error) => return recovered(&state, error),
        };

        match decode_config(&bytes) {
            Ok((config, origin)) => {
                state.last_good = Some(config.clone());
                LoadOutcome::Loaded { config, origin }
            }
            Err(StoreError::Validation { issues }) => validation_failed(&state, issues),
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
        // Account for the trailing newline appended by write_and_replace.
        if bytes.len() >= MAX_CONFIG_FILE_BYTES {
            return Err(StoreError::TooLarge {
                maximum: MAX_CONFIG_FILE_BYTES,
            });
        }

        let parent =
            secure_file::usable_parent(&self.path).ok_or_else(|| StoreError::InvalidPath {
                message: format!("{} has no parent directory", self.path.display()),
            })?;
        secure_file::create_directory(parent)
            .map_err(|error| secure_io_error("config directory", error))?;
        secure_file::write_and_replace(&self.path, &bytes)
            .map_err(|error| secure_io_error("config", error))?;

        state.generation = state.generation.saturating_add(1);
        state.last_good = Some(config.clone());
        let warning = secure_file::sync_parent(parent).err().map(|error| {
            let (operation, message) = error.into_strings("config directory");
            StoreWarning::Io { operation, message }
        });
        Ok(SaveReceipt {
            generation: state.generation,
            warning,
        })
    }

    pub fn last_good(&self) -> Result<AppConfig, StoreError> {
        self.state
            .lock()
            .map(|state| state.last_good.clone().unwrap_or_default())
            .map_err(|_| StoreError::LockPoisoned)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigOrigin {
    Defaults,
    Current,
    Migrated,
    LastGood,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum LoadOutcome {
    Loaded {
        config: AppConfig,
        origin: ConfigOrigin,
    },
    Recovered {
        config: AppConfig,
        origin: ConfigOrigin,
        error: StoreError,
    },
    ValidationFailed {
        config: AppConfig,
        origin: ConfigOrigin,
        issues: Vec<ValidationIssue>,
    },
}

impl LoadOutcome {
    pub fn config(&self) -> &AppConfig {
        match self {
            Self::Loaded { config, .. }
            | Self::Recovered { config, .. }
            | Self::ValidationFailed { config, .. } => config,
        }
    }

    pub const fn origin(&self) -> ConfigOrigin {
        match self {
            Self::Loaded { origin, .. }
            | Self::Recovered { origin, .. }
            | Self::ValidationFailed { origin, .. } => *origin,
        }
    }

    pub fn recovery(&self) -> Option<StoreError> {
        match self {
            Self::Loaded { .. } => None,
            Self::Recovered { error, .. } => Some(error.clone()),
            Self::ValidationFailed { issues, .. } => Some(StoreError::Validation {
                issues: issues.clone(),
            }),
        }
    }
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
            Self::Validation { .. } => {
                formatter.write_str(SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE)
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

fn parse_json<T: DeserializeOwned>(text: &str) -> Result<T, StoreError> {
    serde_json::from_str(text).map_err(|error| StoreError::InvalidJson {
        message: error.to_string(),
    })
}

fn decode_config(bytes: &[u8]) -> Result<(AppConfig, ConfigOrigin), StoreError> {
    let text = std::str::from_utf8(bytes).map_err(|_| StoreError::InvalidUtf8)?;
    let header: VersionHeader = parse_json(text)?;
    if header.schema_version != 10 && header.schema_version != CURRENT_SCHEMA_VERSION {
        return Err(StoreError::UnsupportedVersion {
            found: header.schema_version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }
    let (config, origin) = if header.schema_version == 10 {
        (
            parse_json::<ConfigV10>(text)?.migrate(),
            ConfigOrigin::Migrated,
        )
    } else {
        (parse_json::<AppConfig>(text)?, ConfigOrigin::Current)
    };
    config.validate().map_err(|error| StoreError::Validation {
        issues: error.issues,
    })?;
    Ok((config, origin))
}

// Keep v10's closed shape: accepting a v11 brightness key in a v10 document
// would silently reinterpret an unsupported extension during migration.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreferencesV10 {
    timezone: String,
    autostart: bool,
    paused: bool,
    #[serde(default)]
    orientation: crate::DisplayOrientation,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigV10 {
    schema_version: u32,
    preferences: PreferencesV10,
    cards: Vec<crate::CardSettings>,
    #[serde(default)]
    image_sources: Vec<crate::config::ImageSource>,
    assets: Vec<crate::AssetSettings>,
    advance: crate::CarouselAdvance,
    updater: crate::UpdaterSettings,
}

impl ConfigV10 {
    fn migrate(self) -> AppConfig {
        debug_assert_eq!(self.schema_version, 10);
        AppConfig {
            schema_version: CURRENT_SCHEMA_VERSION,
            preferences: crate::AppPreferences {
                timezone: self.preferences.timezone,
                autostart: self.preferences.autostart,
                paused: self.preferences.paused,
                orientation: self.preferences.orientation,
                ..crate::AppPreferences::default()
            },
            cards: self.cards,
            image_sources: self.image_sources,
            assets: self.assets,
            advance: self.advance,
            updater: self.updater,
        }
    }
}

fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
    secure_file::read_bounded(path, MAX_CONFIG_FILE_BYTES).map_err(|error| match error {
        BoundedReadError::Io(error) => secure_io_error("config", error),
        BoundedReadError::TooLarge { maximum } => StoreError::TooLarge { maximum },
    })
}

fn recovered(state: &StoreState, error: StoreError) -> LoadOutcome {
    let (config, origin) = fallback(state);
    LoadOutcome::Recovered {
        config,
        origin,
        error,
    }
}

fn validation_failed(state: &StoreState, issues: Vec<ValidationIssue>) -> LoadOutcome {
    let (config, origin) = fallback(state);
    LoadOutcome::ValidationFailed {
        config,
        origin,
        issues,
    }
}

fn fallback(state: &StoreState) -> (AppConfig, ConfigOrigin) {
    match &state.last_good {
        Some(config) => (config.clone(), ConfigOrigin::LastGood),
        None => (AppConfig::default(), ConfigOrigin::Defaults),
    }
}

fn secure_io_error(subject: &str, error: FileIoError) -> StoreError {
    let (operation, message) = error.into_strings(subject);
    StoreError::Io { operation, message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use std::{fs, io};

    #[test]
    fn failed_mode_repair_does_not_discard_valid_config() {
        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "deskmate-mode-repair-failure-{}-{serial}.json",
            std::process::id()
        ));
        fs::write(
            &path,
            serde_json::to_vec_pretty(&AppConfig::default()).unwrap(),
        )
        .unwrap();

        let bytes =
            secure_file::read_bounded_with_mode_repair(&path, MAX_CONFIG_FILE_BYTES, |_| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected chmod failure",
                ))
            })
            .unwrap()
            .unwrap();
        let (config, origin) = decode_config(&bytes).unwrap();

        assert_eq!(origin, ConfigOrigin::Current);
        assert_eq!(config, AppConfig::default());
        fs::remove_file(path).unwrap();
    }
}
