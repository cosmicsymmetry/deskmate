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
    MigratedV4,
    MigratedV5,
    MigratedV6,
    MigratedV7,
    MigratedV8,
    MigratedV9,
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

fn finish_migration(mut config: AppConfig) -> AppConfig {
    // Retiring card kinds can empty the loop, and `cards` IS the loop since
    // schema v10. An empty one is not merely sparse: it makes the document
    // invalid and leaves the panel with no face, so it gets the exact fallback
    // card `AppConfig::default()` ships.
    if config.cards.is_empty() {
        let fallback = AppConfig::default()
            .cards
            .into_iter()
            .next()
            .expect("the default configuration has one clock card");
        config.cards.push(fallback);
    }
    config
}

fn drop_retired_cards_from_json(text: &str) -> Result<AppConfig, StoreError> {
    let mut value: serde_json::Value = parse_json(text)?;
    let mut retired_ids = std::collections::HashSet::new();
    if let Some(cards) = value
        .get_mut("cards")
        .and_then(serde_json::Value::as_array_mut)
    {
        cards.retain(|card| {
            let retired = card
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| {
                    matches!(
                        kind,
                        "calendar" | "weather" | "json-feed" | "rss" | "plugin"
                    )
                });
            if retired && let Some(id) = card.get("id").and_then(serde_json::Value::as_str) {
                retired_ids.insert(id.to_owned());
            }
            !retired
        });
    }
    if let Some(playlists) = value
        .get_mut("playlists")
        .and_then(serde_json::Value::as_array_mut)
    {
        for playlist in playlists {
            if let Some(entries) = playlist
                .get_mut("entries")
                .and_then(serde_json::Value::as_array_mut)
            {
                // Entries name cards by identity, not by kind. Remove every
                // reference in the same pass as its retired card or a formerly
                // valid document would fail later as a dangling reference.
                entries.retain(|entry| {
                    entry
                        .get("card_id")
                        .and_then(serde_json::Value::as_str)
                        .is_none_or(|id| !retired_ids.contains(id))
                });
            }
        }
    }
    fold_playlists_into_the_card_order(&mut value);
    serde_json::from_value(value).map_err(|error| StoreError::InvalidJson {
        message: error.to_string(),
    })
}

/// Schema v10: `cards` IS the loop, so the active playlist's order becomes the
/// card order and its `advance` becomes the document's.
///
/// Runs on `Value` for the same reason the retired-kind strip above does: the
/// current `AppConfig` has no `playlists` key, so a stored one would fail
/// `deny_unknown_fields` before any Rust-side filter could run, and the owner
/// would be told their settings are unreadable.
///
/// **A card is never dropped.** Cards the active playlist did not name are
/// appended after the ones it did, in their original order, which is what a
/// v4-v9 document's "card library outside the loop" becomes. What IS lost, on
/// purpose, is the grouping: playlist names and every inactive playlist. The
/// current product exposes one ordered loop, so the discarded groupings are
/// not representable in the current configuration model.
fn fold_playlists_into_the_card_order(value: &mut serde_json::Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let playlists = object.remove("playlists");
    let active_id = object
        .remove("active_playlist_id")
        .and_then(|id| id.as_str().map(str::to_owned));
    let Some(active) = playlists
        .and_then(|playlists| match playlists {
            serde_json::Value::Array(playlists) => Some(playlists),
            _ => None,
        })
        .and_then(|playlists| {
            let active_id = active_id?;
            playlists.into_iter().find(|playlist| {
                playlist.get("id").and_then(serde_json::Value::as_str) == Some(active_id.as_str())
            })
        })
    else {
        // No active playlist to fold: leave the card order alone and let
        // validation speak if the document is short of a card.
        object
            .entry("advance")
            .or_insert(serde_json::json!({ "kind": "manual" }));
        return;
    };

    if let Some(advance) = active.get("advance") {
        object.insert("advance".into(), advance.clone());
    } else {
        object.insert("advance".into(), serde_json::json!({ "kind": "manual" }));
    }

    let entries: Vec<(String, serde_json::Value)> = active
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let card_id = entry.get("card_id")?.as_str()?.to_owned();
                    let dwell = entry
                        .get("dwell_seconds")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    Some((card_id, dwell))
                })
                .collect()
        })
        .unwrap_or_default();

    let Some(cards) = object
        .get_mut("cards")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    let mut ordered: Vec<serde_json::Value> = Vec::with_capacity(cards.len());
    for (card_id, dwell) in &entries {
        if let Some(position) = cards.iter().position(|card| {
            card.get("id").and_then(serde_json::Value::as_str) == Some(card_id.as_str())
        }) {
            let mut card = cards.remove(position);
            if let Some(card_object) = card.as_object_mut() {
                card_object.insert("dwell_seconds".into(), dwell.clone());
            }
            ordered.push(card);
        }
    }
    // Whatever the active playlist did not name keeps its original order and
    // joins the end of the loop rather than disappearing.
    for mut card in cards.drain(..) {
        if let Some(card_object) = card.as_object_mut() {
            card_object.insert("dwell_seconds".into(), serde_json::Value::Null);
        }
        ordered.push(card);
    }
    *cards = ordered;
}

fn parse_json<T: DeserializeOwned>(text: &str) -> Result<T, StoreError> {
    serde_json::from_str(text).map_err(|error| StoreError::InvalidJson {
        message: error.to_string(),
    })
}

#[allow(clippy::too_many_lines)]
fn decode_config(bytes: &[u8]) -> Result<(AppConfig, ConfigOrigin), StoreError> {
    let text = std::str::from_utf8(bytes).map_err(|_| StoreError::InvalidUtf8)?;
    let header: VersionHeader = parse_json(text)?;
    let (config, origin) = match header.schema_version {
        CURRENT_SCHEMA_VERSION => (parse_json(text)?, ConfigOrigin::Current),
        version @ (4..=9) => {
            // Every readable legacy version takes the same path:
            // `drop_retired_cards_from_json` strips retired card kinds and their
            // playlist entries, then folds the active playlist into the card
            // order for schema v10. This runs on the raw `Value` because strict
            // current-shape deserialization would refuse the retired keys.
            // `#[serde(default)]` supplies `image_sources` for older documents.
            // v4's old asset variants need no transformation: at v4 the compile
            // step rejected a non-empty `assets` array, so no saved v4 document
            // contained them (see `docs/config/v4.md`).
            let legacy = drop_retired_cards_from_json(text)?;
            let origin = match version {
                4 => ConfigOrigin::MigratedV4,
                5 => ConfigOrigin::MigratedV5,
                6 => ConfigOrigin::MigratedV6,
                7 => ConfigOrigin::MigratedV7,
                8 => ConfigOrigin::MigratedV8,
                9 => ConfigOrigin::MigratedV9,
                _ => unreachable!(),
            };
            (
                finish_migration(AppConfig {
                    schema_version: CURRENT_SCHEMA_VERSION,
                    ..legacy
                }),
                origin,
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
