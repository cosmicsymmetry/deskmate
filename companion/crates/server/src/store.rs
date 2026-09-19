//! Per-device configuration storage. The schema version tracked here is
//! whatever `app_core::CURRENT_SCHEMA_VERSION` currently is -- see that
//! constant's documentation rather than pinning a duplicate number here.
//!
//! The server only chooses a path and keeps one [`app_core::ConfigStore`]
//! alive per provisioned device. Parsing, version refusal, validation, atomic
//! replacement, and genuine-last-good semantics stay in `app-core`, so every
//! server entry point uses the same storage rules.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use app_core::{
    AppConfig, AppSnapshot, ConfigOrigin, ConfigStore, PersistenceState,
    SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE, SaveReceipt, StoreError,
};
use serde::Serialize;

use crate::registry::DeviceId;

pub(crate) struct DeviceConfig {
    pub store: ConfigStore,
    /// Serializes a socket's load/start transition with admin writes. This
    /// closes the race where a PUT could save after the socket loaded but
    /// before its runtime became visible, leaving the new config persisted
    /// yet unapplied until the next reconnect.
    pub update: tokio::sync::Mutex<()>,
    status: Mutex<DeviceConfigStatus>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct DeviceConfigStatus {
    origin: Option<ConfigOrigin>,
    using_fallback: bool,
    fallback_reason: Option<ConfigFallbackReason>,
    #[serde(skip)]
    load_error: Option<StoreError>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ConfigFallbackReason {
    Recovery,
    ValidationFailed,
}

impl DeviceConfig {
    pub fn compile_and_save(&self, config: &AppConfig) -> Result<SaveReceipt, StoreError> {
        config.compile(1).map_err(|error| StoreError::Validation {
            issues: error.issues,
        })?;
        self.store.save(config)
    }

    pub fn record_load(
        &self,
        origin: ConfigOrigin,
        fallback_reason: Option<ConfigFallbackReason>,
        load_error: Option<StoreError>,
    ) {
        *self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = DeviceConfigStatus {
            origin: Some(origin),
            using_fallback: fallback_reason.is_some(),
            fallback_reason,
            load_error,
        };
    }

    pub fn record_current(&self) {
        self.record_load(ConfigOrigin::Current, None, None);
    }

    pub fn status(&self) -> DeviceConfigStatus {
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn apply_load_diagnostic(&self, snapshot: &mut AppSnapshot) {
        if !matches!(snapshot.persistence, PersistenceState::Clean) {
            return;
        }
        let load_error = self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .load_error
            .clone();
        snapshot.persistence = match load_error {
            Some(StoreError::Validation { issues }) => PersistenceState::ValidationFailed {
                message: SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE.into(),
                issues,
            },
            Some(error) => PersistenceState::RecoverableError {
                message: error.to_string(),
            },
            None => return,
        };
    }
}

pub(crate) struct DeviceConfigStores {
    root: PathBuf,
    stores: Mutex<HashMap<DeviceId, Arc<DeviceConfig>>>,
}

impl DeviceConfigStores {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            stores: Mutex::new(HashMap::new()),
        }
    }

    pub fn for_device(&self, device_id: &str) -> Arc<DeviceConfig> {
        let mut stores = self
            .stores
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(stores.entry(device_id.to_owned()).or_insert_with(|| {
            Arc::new(DeviceConfig {
                store: ConfigStore::new(self.root.join(format!("{device_id}.json"))),
                update: tokio::sync::Mutex::new(()),
                status: Mutex::new(DeviceConfigStatus::default()),
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_diagnostic_overlays_only_clean_snapshots_and_successful_save_clears_it() {
        let root = tempfile::tempdir().expect("config root");
        let stores = DeviceConfigStores::new(root.path().to_path_buf());
        let device = stores.for_device("dev-0001");
        device.record_load(
            ConfigOrigin::Defaults,
            Some(ConfigFallbackReason::Recovery),
            Some(StoreError::UnsupportedVersion {
                found: 11,
                supported: 10,
            }),
        );

        let mut clean = app_core::initial_snapshot(
            &AppConfig::default(),
            app_core::RuntimeDiagnostics::default(),
        );
        device.apply_load_diagnostic(&mut clean);
        assert_eq!(
            clean.persistence,
            PersistenceState::RecoverableError {
                message: "config schema version 11 is unsupported; expected 10".into(),
            }
        );

        let status = serde_json::to_value(device.status()).expect("serialize status");
        assert!(status.get("load_error").is_none());
        assert_eq!(status["fallback_reason"], "recovery");

        let mut saving = clean;
        saving.persistence = PersistenceState::Saving;
        device.apply_load_diagnostic(&mut saving);
        assert_eq!(saving.persistence, PersistenceState::Saving);

        device.record_current();
        let mut cleared = app_core::initial_snapshot(
            &AppConfig::default(),
            app_core::RuntimeDiagnostics::default(),
        );
        device.apply_load_diagnostic(&mut cleared);
        assert_eq!(cleared.persistence, PersistenceState::Clean);
    }
}
