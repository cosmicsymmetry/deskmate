//! Per-device configuration storage. The schema version tracked here is
//! whatever `app_core::CURRENT_SCHEMA_VERSION` currently is -- see that
//! constant's documentation rather than pinning a duplicate number here.
//!
//! The server only chooses a path and keeps one [`app_core::ConfigStore`]
//! alive per provisioned device. Parsing, migration, validation, atomic
//! replacement, and genuine-last-good semantics stay in `app-core`, so every
//! server entry point uses the same storage rules.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use app_core::{ConfigOrigin, ConfigStore};
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
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ConfigFallbackReason {
    Recovery,
    ValidationFailed,
}

impl DeviceConfig {
    pub fn record_load(&self, origin: ConfigOrigin, fallback_reason: Option<ConfigFallbackReason>) {
        *self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = DeviceConfigStatus {
            origin: Some(origin),
            using_fallback: fallback_reason.is_some(),
            fallback_reason,
        };
    }

    pub fn record_current(&self) {
        self.record_load(ConfigOrigin::Current, None);
    }

    pub fn status(&self) -> DeviceConfigStatus {
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
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
