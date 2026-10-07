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

/// What one account may hold, as one immutable snapshot. Read it at the
/// moment of admission; never cache it across a lock release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountPolicy {
    /// `None` means the edition sets no panel limit.
    pub max_panels: Option<usize>,
    /// Per PANEL, not per account: each panel's config holds its own cards.
    pub max_cards: usize,
    pub max_image_sources: usize,
}

impl AccountPolicy {
    /// The public build's policy: no commercial cap below the structure.
    pub const SELF_HOSTED: Self = Self {
        max_panels: None,
        max_cards: MAX_CONFIG_CARDS,
        max_image_sources: MAX_IMAGE_SOURCES,
    };

    #[must_use]
    pub fn effective_cards(&self) -> usize {
        self.max_cards.min(MAX_CONFIG_CARDS)
    }

    #[must_use]
    pub fn effective_image_sources(&self) -> usize {
        self.max_image_sources.min(MAX_IMAGE_SOURCES)
    }
}

pub trait Entitlements: Send + Sync + 'static {
    fn policy(&self, account: &AccountId) -> AccountPolicy;
}

#[derive(Debug, Default)]
pub struct SelfHosted;

impl Entitlements for SelfHosted {
    fn policy(&self, _account: &AccountId) -> AccountPolicy {
        AccountPolicy::SELF_HOSTED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_hosted_is_exactly_the_structural_ceiling() {
        let account = AccountId("acct-self-host-test".to_owned());
        assert_eq!(
            SelfHosted.policy(&account),
            AccountPolicy {
                max_panels: None,
                max_cards: MAX_CONFIG_CARDS,
                max_image_sources: MAX_IMAGE_SOURCES,
            }
        );
    }

    #[test]
    fn a_policy_above_the_structure_is_clamped_to_it() {
        let generous = AccountPolicy {
            max_panels: Some(100),
            max_cards: 99,
            max_image_sources: 99,
        };
        assert_eq!(generous.effective_cards(), MAX_CONFIG_CARDS);
        assert_eq!(generous.effective_image_sources(), MAX_IMAGE_SOURCES);
        let strict = AccountPolicy {
            max_cards: 2,
            max_image_sources: 4,
            ..generous
        };
        assert_eq!(strict.effective_cards(), 2);
        assert_eq!(strict.effective_image_sources(), 4);
    }
}
