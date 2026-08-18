use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use atomic_write_file::AtomicWriteFile;
#[cfg(unix)]
use atomic_write_file::unix::OpenOptionsExt as AtomicOpenOptionsExt;
use serde::{Deserialize, Serialize};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt as UnixOpenOptionsExt, PermissionsExt};

pub const NETWORK_SETTINGS_FORMAT_VERSION: u32 = 1;
pub const MAX_NETWORK_SETTINGS_FILE_BYTES: usize = 16 * 1_024;

/// The non-secret portion of the desktop's network settings. This is the only
/// settings shape returned by the store and is therefore safe to serialize into IPC.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkSettings {
    pub server_url: String,
    pub device_id: String,
}

/// A write request. Secret fields are deliberately private and this type implements
/// neither `Debug` nor `Serialize`, so app-core cannot accidentally put their values
/// into diagnostics, snapshots, or events.
pub struct NetworkSettingsUpdate {
    server_url: String,
    device_id: String,
    admin_token: Option<String>,
    device_token: Option<String>,
}

impl NetworkSettingsUpdate {
    /// `None` retains an already-stored token; `Some("")` explicitly clears it.
    pub fn new(
        server_url: impl Into<String>,
        device_id: impl Into<String>,
        admin_token: Option<String>,
        device_token: Option<String>,
    ) -> Self {
        Self {
            server_url: server_url.into(),
            device_id: device_id.into(),
            admin_token,
            device_token,
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
            admin_token,
            device_token,
        } = update;

        // A blank write-only control is represented by `None`. Load the private
        // persisted shape only inside the store so the old token can survive without
        // ever crossing the public read boundary.
        let previous = if admin_token.is_none() || device_token.is_none() {
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
            admin_token: admin_token.unwrap_or(previous.admin_token),
            device_token: device_token.unwrap_or(previous.device_token),
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

        let parent = usable_parent(&self.path)?;
        fs::create_dir_all(parent)
            .map_err(|error| io_error("create network settings directory", &error))?;
        write_and_replace(&self.path, &bytes)?;

        state.generation = state.generation.saturating_add(1);
        state.last_good = Some(persisted);
        let warning = sync_parent(parent)
            .err()
            .map(|error| NetworkSettingsStoreWarning::Io {
                operation: "sync network settings directory".into(),
                message: error.to_string(),
            });
        Ok(NetworkSettingsSaveReceipt {
            generation: state.generation,
            warning,
        })
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
    admin_token: String,
    device_token: String,
}

impl PersistedNetworkSettings {
    fn empty() -> Self {
        Self {
            format_version: NETWORK_SETTINGS_FORMAT_VERSION,
            server_url: String::new(),
            device_id: String::new(),
            admin_token: String::new(),
            device_token: String::new(),
        }
    }

    fn public_settings(&self) -> NetworkSettings {
        NetworkSettings {
            server_url: self.server_url.clone(),
            device_id: self.device_id.clone(),
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
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("open network settings", &error)),
    };
    let _ = secure_open_settings(&file);
    let mut bytes = Vec::new();
    file.take((MAX_NETWORK_SETTINGS_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read network settings", &error))?;
    if bytes.len() > MAX_NETWORK_SETTINGS_FILE_BYTES {
        Err(NetworkSettingsStoreError::TooLarge {
            maximum: MAX_NETWORK_SETTINGS_FILE_BYTES,
        })
    } else {
        Ok(Some(bytes))
    }
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

fn usable_parent(path: &Path) -> Result<&Path, NetworkSettingsStoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| NetworkSettingsStoreError::InvalidPath {
            message: format!("{} has no parent directory", path.display()),
        })?;
    if parent.as_os_str().is_empty() {
        Ok(Path::new("."))
    } else {
        Ok(parent)
    }
}

fn write_and_replace(target: &Path, bytes: &[u8]) -> Result<(), NetworkSettingsStoreError> {
    let mut options = AtomicWriteFile::options();
    secure_atomic_options(&mut options);
    let mut file = options
        .open(target)
        .map_err(|error| io_error("create temporary network settings", &error))?;
    file.write_all(bytes)
        .map_err(|error| io_error("write temporary network settings", &error))?;
    file.write_all(b"\n")
        .map_err(|error| io_error("finish temporary network settings", &error))?;
    file.commit()
        .map_err(|error| io_error("sync and replace network settings", &error))
}

#[cfg(unix)]
fn secure_open_settings(file: &File) -> io::Result<()> {
    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn secure_open_settings(_file: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn secure_atomic_options(options: &mut atomic_write_file::OpenOptions) {
    options.preserve_mode(false);
    options.mode(0o600);
}

#[cfg(not(unix))]
fn secure_atomic_options(_options: &mut atomic_write_file::OpenOptions) {}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> io::Result<()> {
    Ok(())
}

fn io_error(operation: &str, error: &io::Error) -> NetworkSettingsStoreError {
    NetworkSettingsStoreError::Io {
        operation: operation.into(),
        message: error.to_string(),
    }
}
