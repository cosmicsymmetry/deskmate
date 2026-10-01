use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

pub(super) const MAX_FACE_STATE_BYTES: usize = 16 * 1024;
const FACE_STATE_VERSION: u8 = 1;

/// Small, durable state returned by faces, keyed by image source.
pub(super) struct FaceStateStore {
    path: PathBuf,
    sources: Mutex<BTreeMap<String, serde_json::Value>>,
    transitions: Mutex<BTreeMap<String, Arc<Mutex<u64>>>>,
}

#[derive(Deserialize, Serialize)]
struct PersistedFaceState {
    version: u8,
    sources: BTreeMap<String, serde_json::Value>,
}

impl FaceStateStore {
    /// Loads saved state. A missing, unreadable, corrupt, or unknown-version file
    /// starts empty so it can be replaced by the next successful render.
    pub(super) fn load(path: PathBuf) -> Self {
        let sources = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<PersistedFaceState>(&bytes) {
                Ok(saved) if saved.version == FACE_STATE_VERSION => saved.sources,
                Ok(saved) => {
                    tracing::warn!(target: "server::data_cards",
                        path = %path.display(), version = saved.version,
                        "ignoring a face-state file with an unsupported version");
                    BTreeMap::new()
                }
                Err(error) => {
                    tracing::warn!(target: "server::data_cards",
                        path = %path.display(), %error,
                        "ignoring a corrupt face-state file");
                    BTreeMap::new()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => {
                tracing::warn!(target: "server::data_cards",
                    path = %path.display(), %error,
                    "the face-state file could not be read; starting without state");
                BTreeMap::new()
            }
        };
        Self {
            path,
            sources: Mutex::new(sources),
            transitions: Mutex::new(BTreeMap::new()),
        }
    }

    /// Serializes selection/read/commit for one source, never its render. The
    /// generation prevents a render started earlier from overwriting a tap.
    pub(super) fn transition(&self, source_id: &str) -> Arc<Mutex<u64>> {
        self.transitions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(source_id.to_owned())
            .or_default()
            .clone()
    }

    pub(super) fn get(&self, source_id: &str) -> Option<serde_json::Value> {
        self.sources
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(source_id)
            .cloned()
    }

    /// Replaces or clears one source's state. Persistence failures are logged and
    /// leave the last successfully stored value in place; they do not fail a frame.
    pub(super) fn put(&self, source_id: &str, state: Option<serde_json::Value>) {
        if let Some(value) = &state {
            let Ok(encoded) = serde_json::to_vec(value) else {
                tracing::warn!(target: "server::data_cards", source_id,
                    "the face returned state that could not be encoded; keeping the stored state unchanged");
                return;
            };
            if encoded.len() > MAX_FACE_STATE_BYTES {
                tracing::warn!(target: "server::data_cards",
                    source_id, state_bytes = encoded.len(), maximum_bytes = MAX_FACE_STATE_BYTES,
                    "the face returned too much state; keeping the stored state unchanged");
                return;
            }
        }

        self.replace(source_id, state);
    }

    pub(super) fn remove(&self, source_id: &str) {
        self.replace(source_id, None);
    }

    fn replace(&self, source_id: &str, state: Option<serde_json::Value>) {
        let mut sources = self
            .sources
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if sources.get(source_id) == state.as_ref() {
            return;
        }

        let mut updated = sources.clone();
        match state {
            Some(value) => {
                updated.insert(source_id.to_owned(), value);
            }
            None => {
                updated.remove(source_id);
            }
        }
        let saved = PersistedFaceState {
            version: FACE_STATE_VERSION,
            sources: updated.clone(),
        };
        let bytes = match serde_json::to_vec_pretty(&saved) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(target: "server::data_cards", source_id, %error,
                    "the face-state file could not be encoded; keeping the stored state unchanged");
                return;
            }
        };
        if let Err(error) = app_core::secure_file::write_and_replace(&self.path, &bytes) {
            let (operation, message) = error.into_strings("face state");
            tracing::warn!(target: "server::data_cards",
                source_id, path = %self.path.display(), %operation, %message,
                "the face state could not be saved; keeping the stored state unchanged");
            return;
        }
        *sources = updated;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_no_state_rather_than_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FaceStateStore::load(dir.path().join("face-state.json"));
        assert_eq!(store.get("hn"), None);
    }

    #[test]
    fn a_corrupt_file_is_no_state_and_is_overwritten_by_the_next_put() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("face-state.json");
        std::fs::write(&path, b"{not json").expect("write");
        let store = FaceStateStore::load(path.clone());
        assert_eq!(store.get("hn"), None);
        store.put("hn", Some(serde_json::json!({ "page": 1 })));
        let reloaded = FaceStateStore::load(path);
        assert_eq!(reloaded.get("hn"), Some(serde_json::json!({ "page": 1 })));
    }

    #[test]
    fn state_survives_a_reload_and_a_none_clears_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("face-state.json");
        let store = FaceStateStore::load(path.clone());
        store.put("hn", Some(serde_json::json!({ "page": 3 })));
        assert_eq!(
            FaceStateStore::load(path.clone()).get("hn"),
            Some(serde_json::json!({ "page": 3 }))
        );
        store.put("hn", None);
        assert_eq!(FaceStateStore::load(path).get("hn"), None);
    }

    #[test]
    fn state_larger_than_the_cap_is_refused_and_the_previous_state_survives() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("face-state.json");
        let store = FaceStateStore::load(path);
        store.put("hn", Some(serde_json::json!({ "page": 3 })));
        let huge = serde_json::json!({ "junk": "x".repeat(MAX_FACE_STATE_BYTES) });
        store.put("hn", Some(huge));
        assert_eq!(store.get("hn"), Some(serde_json::json!({ "page": 3 })));
    }

    #[test]
    fn removing_a_source_forgets_its_state_and_keeps_its_neighbours() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("face-state.json");
        let store = FaceStateStore::load(path);
        store.put("hn", Some(serde_json::json!({ "page": 3 })));
        store.put("weather", Some(serde_json::json!({ "view": "days" })));
        store.remove("hn");
        assert_eq!(store.get("hn"), None);
        assert_eq!(
            store.get("weather"),
            Some(serde_json::json!({ "view": "days" }))
        );
    }

    #[test]
    fn an_unchanged_state_does_not_rewrite_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("face-state.json");
        let store = FaceStateStore::load(path.clone());
        store.put("hn", Some(serde_json::json!({ "page": 3 })));
        let first = std::fs::metadata(&path)
            .expect("metadata")
            .modified()
            .expect("mtime");
        store.put("hn", Some(serde_json::json!({ "page": 3 })));
        assert_eq!(
            std::fs::metadata(&path)
                .expect("metadata")
                .modified()
                .expect("mtime"),
            first
        );
    }
}
