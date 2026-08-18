//! Per-device schema-v4 configuration storage.
//!
//! The server only chooses a path and keeps one [`app_core::ConfigStore`]
//! alive per provisioned device. Parsing, migration, validation, atomic
//! replacement, and genuine-last-good semantics stay in `app-core`, shared
//! with the Mac app.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use app_core::ConfigStore;

use crate::registry::DeviceId;

pub(crate) struct DeviceConfig {
    pub store: ConfigStore,
    /// Serializes a socket's load/start transition with admin writes. This
    /// closes the race where a PUT could save after the socket loaded but
    /// before its runtime became visible, leaving the new config persisted
    /// yet unapplied until the next reconnect.
    pub update: tokio::sync::Mutex<()>,
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
            })
        }))
    }
}
