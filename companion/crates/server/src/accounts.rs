use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::data_cards::DataCardSpecs;
use crate::identity::AccountId;
use crate::image_sources::ImageSourceStore;
use crate::store::DeviceConfigStores;

#[doc(hidden)]
pub struct AccountSpace {
    pub account_id: AccountId,
    pub root: PathBuf,
    pub(crate) image_sources: Arc<ImageSourceStore>,
    pub(crate) data_cards: Mutex<DataCardSpecs>,
    pub(crate) configs: DeviceConfigStores,
}

impl AccountSpace {
    pub(crate) fn open(config_root: &std::path::Path, account_id: AccountId) -> Self {
        let root = config_root.join("accounts").join(&account_id.0);
        app_core::secure_file::create_directory(&root)
            .expect("failed to create the account data directory");
        let image_sources = ImageSourceStore::new(root.clone())
            .expect("failed to load the account's image-source store");
        Self {
            account_id,
            data_cards: Mutex::new(DataCardSpecs::new(root.join("data-cards.json"))),
            configs: DeviceConfigStores::new(root.join("devices")),
            image_sources: Arc::new(image_sources),
            root,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;

    use crate::ServerState;
    use crate::identity::DeviceState;

    #[test]
    fn account_space_lives_under_its_own_folder_and_is_cached() {
        let state = ServerState::in_memory();
        let owner = state
            .identity()
            .create_account("a@example.com", true, true, Utc::now())
            .unwrap();
        let space = state.account_space(&owner.id);
        assert!(space.root.ends_with(format!("accounts/{}", owner.id.0)));
        assert!(Arc::ptr_eq(&space, &state.account_space(&owner.id)));
        assert_eq!(
            space.configs.for_device("dev-0001").store.path(),
            space.root.join("devices/dev-0001.json")
        );
    }

    #[test]
    fn space_for_device_follows_ownership() {
        let state = ServerState::in_memory();
        let account = state
            .identity()
            .create_account("a@example.com", true, true, Utc::now())
            .unwrap();
        assert!(state.space_for_device("dev-0001").is_none());
        state
            .identity()
            .assign_device("dev-0001", &account.id, DeviceState::Active, Utc::now())
            .unwrap();
        assert_eq!(
            state.space_for_device("dev-0001").unwrap().account_id,
            account.id
        );
    }
}
