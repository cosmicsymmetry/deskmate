use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::secure_file::{self, BoundedReadError, FileIoError};
use crate::state::DeviceTier;

pub const NETWORK_SETTINGS_FORMAT_VERSION: u32 = 1;
pub const MAX_NETWORK_SETTINGS_FILE_BYTES: usize = 16 * 1_024;

/// The non-secret portion of the desktop's network settings. This is the only
/// settings shape returned by the store and is therefore safe to serialize into IPC.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkSettings {
    pub server_url: String,
    pub device_id: String,
    #[serde(default)]
    pub tier: Option<DeviceTier>,
}

/// A write request. Secret fields are deliberately private and this type implements
/// neither `Debug` nor `Serialize`, so app-core cannot accidentally put their values
/// into diagnostics, snapshots, or events.
pub struct NetworkSettingsUpdate {
    server_url: String,
    device_id: String,
    tier: Option<DeviceTier>,
    admin_token: Option<String>,
}

impl NetworkSettingsUpdate {
    /// `None` retains the last known tier and admin token. `Some("")` explicitly
    /// clears the admin token.
    pub fn new(
        server_url: impl Into<String>,
        device_id: impl Into<String>,
        tier: Option<DeviceTier>,
        admin_token: Option<String>,
    ) -> Self {
        Self {
            server_url: server_url.into(),
            device_id: device_id.into(),
            tier,
            admin_token,
        }
    }
}

pub struct NetworkSettingsStore {
    path: PathBuf,
    state: Mutex<NetworkSettingsStoreState>,
}

#[derive(Default)]
struct NetworkSettingsStoreState {
    last_good: Option<PersistedNetworkSettings>,
    generation: u64,
}

impl NetworkSettingsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            state: Mutex::new(NetworkSettingsStoreState::default()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> NetworkSettingsLoadOutcome {
        let Ok(mut state) = self.state.lock() else {
            return NetworkSettingsLoadOutcome::Recovered {
                settings: NetworkSettings::default(),
                origin: NetworkSettingsOrigin::Defaults,
                error: NetworkSettingsStoreError::LockPoisoned,
            };
        };

        match read_persisted(&self.path) {
            Ok(Some(persisted)) => {
                let settings = persisted.public_settings();
                state.last_good = Some(persisted);
                NetworkSettingsLoadOutcome::Loaded {
                    settings,
                    origin: NetworkSettingsOrigin::Current,
                }
            }
            Ok(None) => NetworkSettingsLoadOutcome::Loaded {
                settings: NetworkSettings::default(),
                origin: NetworkSettingsOrigin::Defaults,
            },
            Err(error) => recovered(&state, error),
        }
    }

    pub fn save(
        &self,
        update: NetworkSettingsUpdate,
    ) -> Result<NetworkSettingsSaveReceipt, NetworkSettingsStoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| NetworkSettingsStoreError::LockPoisoned)?;
        let NetworkSettingsUpdate {
            server_url,
            device_id,
            tier,
            admin_token,
        } = update;

        // A blank write-only control is represented by `None`. Load the private
        // persisted shape only inside the store so the old token can survive without
        // ever crossing the public read boundary.
        let previous = if admin_token.is_none() || tier.is_none() {
            match &state.last_good {
                Some(settings) => settings.clone(),
                None => read_persisted(&self.path)?.unwrap_or_else(PersistedNetworkSettings::empty),
            }
        } else {
            PersistedNetworkSettings::empty()
        };
        let persisted = PersistedNetworkSettings {
            format_version: NETWORK_SETTINGS_FORMAT_VERSION,
            server_url,
            device_id,
            tier: tier.or(previous.tier),
            admin_token: admin_token.unwrap_or(previous.admin_token),
            // Version-1 files may contain a device token written by an older app.
            // There is no desktop consumer for it, so every subsequent save removes
            // it instead of retaining a plaintext secret with no purpose.
            device_token: String::new(),
        };
        let bytes = serde_json::to_vec_pretty(&persisted).map_err(|error| {
            NetworkSettingsStoreError::InvalidJson {
                message: error.to_string(),
            }
        })?;
        if bytes.len() > MAX_NETWORK_SETTINGS_FILE_BYTES {
            return Err(NetworkSettingsStoreError::TooLarge {
                maximum: MAX_NETWORK_SETTINGS_FILE_BYTES,
            });
        }

        let parent = secure_file::usable_parent(&self.path).ok_or_else(|| {
            NetworkSettingsStoreError::InvalidPath {
                message: format!("{} has no parent directory", self.path.display()),
            }
        })?;
        secure_file::create_directory(parent)
            .map_err(|error| secure_io_error("network settings directory", error))?;
        secure_file::write_and_replace(&self.path, &bytes)
            .map_err(|error| secure_io_error("network settings", error))?;

        state.generation = state.generation.saturating_add(1);
        state.last_good = Some(persisted);
        let warning = secure_file::sync_parent(parent).err().map(|error| {
            let (operation, message) = error.into_strings("network settings directory");
            NetworkSettingsStoreWarning::Io { operation, message }
        });
        Ok(NetworkSettingsSaveReceipt {
            generation: state.generation,
            warning,
        })
    }

    /// Lends the stored admin token only for the duration of one operation. The
    /// store never exposes the token through a getter, snapshot, or serializable DTO.
    pub fn with_admin_token<R>(
        &self,
        operation: impl FnOnce(&str) -> R,
    ) -> Result<Option<R>, NetworkSettingsStoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| NetworkSettingsStoreError::LockPoisoned)?;
        if state.last_good.is_none() {
            state.last_good = read_persisted(&self.path)?;
        }
        let token = state
            .last_good
            .as_ref()
            .map(|settings| settings.admin_token.clone())
            .filter(|token| !token.is_empty());
        drop(state);
        Ok(token.map(|token| operation(&token)))
    }

    /// Removes the legacy device token without ever returning it to the caller.
    /// New writes never include this field; this operation scrubs an older v1 file
    /// even when no other settings happen to change after upgrading the app.
    pub fn discard_device_token(&self) -> Result<bool, NetworkSettingsStoreError> {
        let settings = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| NetworkSettingsStoreError::LockPoisoned)?;
            if state.last_good.is_none() {
                state.last_good = read_persisted(&self.path)?;
            }
            let Some(persisted) = state.last_good.as_ref() else {
                return Ok(false);
            };
            if persisted.device_token.is_empty() {
                return Ok(false);
            }
            persisted.public_settings()
        };
        self.save(NetworkSettingsUpdate::new(
            settings.server_url,
            settings.device_id,
            settings.tier,
            None,
        ))?;
        Ok(true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkSettingsOrigin {
    Defaults,
    Current,
    LastGood,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum NetworkSettingsLoadOutcome {
    Loaded {
        settings: NetworkSettings,
        origin: NetworkSettingsOrigin,
    },
    Recovered {
        settings: NetworkSettings,
        origin: NetworkSettingsOrigin,
        error: NetworkSettingsStoreError,
    },
}

impl NetworkSettingsLoadOutcome {
    pub fn settings(&self) -> &NetworkSettings {
        match self {
            Self::Loaded { settings, .. } | Self::Recovered { settings, .. } => settings,
        }
    }

    pub const fn origin(&self) -> NetworkSettingsOrigin {
        match self {
            Self::Loaded { origin, .. } | Self::Recovered { origin, .. } => *origin,
        }
    }

    pub fn recovery(&self) -> Option<NetworkSettingsStoreError> {
        match self {
            Self::Loaded { .. } => None,
            Self::Recovered { error, .. } => Some(error.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkSettingsSaveReceipt {
    pub generation: u64,
    pub warning: Option<NetworkSettingsStoreWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum NetworkSettingsStoreWarning {
    Io { operation: String, message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum NetworkSettingsStoreError {
    Io { operation: String, message: String },
    InvalidPath { message: String },
    TooLarge { maximum: usize },
    InvalidUtf8,
    InvalidJson { message: String },
    UnsupportedVersion { found: u32, supported: u32 },
    LockPoisoned,
}

impl fmt::Display for NetworkSettingsStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { operation, message } => write!(formatter, "{operation}: {message}"),
            Self::InvalidPath { message } => {
                write!(formatter, "invalid network settings path: {message}")
            }
            Self::TooLarge { maximum } => {
                write!(
                    formatter,
                    "network settings exceed the {maximum}-byte limit"
                )
            }
            Self::InvalidUtf8 => formatter.write_str("network settings are not valid UTF-8"),
            Self::InvalidJson { message } => write!(formatter, "invalid settings JSON: {message}"),
            Self::UnsupportedVersion { found, supported } => write!(
                formatter,
                "network settings format version {found} is unsupported; expected {supported}"
            ),
            Self::LockPoisoned => formatter.write_str("network settings store lock is poisoned"),
        }
    }
}

impl std::error::Error for NetworkSettingsStoreError {}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedNetworkSettings {
    format_version: u32,
    server_url: String,
    device_id: String,
    #[serde(default)]
    tier: Option<DeviceTier>,
    admin_token: String,
    #[serde(default, skip_serializing, rename = "device_token")]
    device_token: String,
}

impl PersistedNetworkSettings {
    fn empty() -> Self {
        Self {
            format_version: NETWORK_SETTINGS_FORMAT_VERSION,
            server_url: String::new(),
            device_id: String::new(),
            tier: None,
            admin_token: String::new(),
            device_token: String::new(),
        }
    }

    fn public_settings(&self) -> NetworkSettings {
        NetworkSettings {
            server_url: self.server_url.clone(),
            device_id: self.device_id.clone(),
            tier: self.tier,
        }
    }
}

#[derive(Deserialize)]
struct VersionHeader {
    format_version: u32,
}

fn read_persisted(
    path: &Path,
) -> Result<Option<PersistedNetworkSettings>, NetworkSettingsStoreError> {
    let Some(bytes) = read_bounded(path)? else {
        return Ok(None);
    };
    let text = std::str::from_utf8(&bytes).map_err(|_| NetworkSettingsStoreError::InvalidUtf8)?;
    let header: VersionHeader =
        serde_json::from_str(text).map_err(|error| NetworkSettingsStoreError::InvalidJson {
            message: error.to_string(),
        })?;
    if header.format_version != NETWORK_SETTINGS_FORMAT_VERSION {
        return Err(NetworkSettingsStoreError::UnsupportedVersion {
            found: header.format_version,
            supported: NETWORK_SETTINGS_FORMAT_VERSION,
        });
    }
    let persisted =
        serde_json::from_str(text).map_err(|error| NetworkSettingsStoreError::InvalidJson {
            message: error.to_string(),
        })?;
    Ok(Some(persisted))
}

fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, NetworkSettingsStoreError> {
    secure_file::read_bounded(path, MAX_NETWORK_SETTINGS_FILE_BYTES).map_err(|error| match error {
        BoundedReadError::Io(error) => secure_io_error("network settings", error),
        BoundedReadError::TooLarge { maximum } => NetworkSettingsStoreError::TooLarge { maximum },
    })
}

fn recovered(
    state: &NetworkSettingsStoreState,
    error: NetworkSettingsStoreError,
) -> NetworkSettingsLoadOutcome {
    let (settings, origin) = match &state.last_good {
        Some(persisted) => (persisted.public_settings(), NetworkSettingsOrigin::LastGood),
        None => (NetworkSettings::default(), NetworkSettingsOrigin::Defaults),
    };
    NetworkSettingsLoadOutcome::Recovered {
        settings,
        origin,
        error,
    }
}

fn secure_io_error(subject: &str, error: FileIoError) -> NetworkSettingsStoreError {
    let (operation, message) = error.into_strings(subject);
    NetworkSettingsStoreError::Io { operation, message }
}
