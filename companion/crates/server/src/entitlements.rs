use app_core::MAX_CONFIG_CARDS;
use app_core::config::MAX_IMAGE_SOURCES;
use serde::Serialize;

use crate::identity::AccountId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Edition {
    SelfHosted,
    Hosted,
}

pub trait Entitlements: Send + Sync + 'static {
    fn max_cards(&self, account: &AccountId) -> usize;
    fn max_image_sources(&self, account: &AccountId) -> usize;
    fn max_panels(&self, account: &AccountId) -> Option<usize>;
    fn feature_enabled(&self, account: &AccountId, feature: &str) -> bool;
}

#[derive(Debug, Default)]
pub struct SelfHosted;

impl Entitlements for SelfHosted {
    fn max_cards(&self, _account: &AccountId) -> usize {
        MAX_CONFIG_CARDS
    }

    fn max_image_sources(&self, _account: &AccountId) -> usize {
        MAX_IMAGE_SOURCES
    }

    fn max_panels(&self, _account: &AccountId) -> Option<usize> {
        None
    }

    fn feature_enabled(&self, _account: &AccountId, _feature: &str) -> bool {
        true
    }
}
