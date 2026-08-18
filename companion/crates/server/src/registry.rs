//! In-memory device registry: mints per-device bearer tokens and authenticates
//! them in constant time. V2 is single-tenant and has no accounts, so this is
//! deliberately not a database -- nothing here survives a server restart.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use rand::RngCore;

/// A device's opaque identifier, e.g. `dev-0001`.
pub type DeviceId = String;

/// A freshly minted device identity: the id the server assigns internally and
/// the bearer token the device presents on every request. The token is
/// returned exactly once, at mint time; the server never logs it again, and
/// `Debug` never prints it either (see the hand-written impl below), so a
/// stray `tracing::info!(?identity)` can't leak a live bearer secret.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub device_id: DeviceId,
    pub token: String,
}

impl fmt::Debug for DeviceIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceIdentity")
            .field("device_id", &self.device_id)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// Token-to-device lookup, held in memory for the life of the process.
pub struct Registry {
    next_sequence: AtomicU64,
    tokens: Mutex<HashMap<String, DeviceId>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_sequence: AtomicU64::new(0),
            tokens: Mutex::new(HashMap::new()),
        }
    }

    /// Mints a new device identity: a monotonic `dev-NNNN` id and a random
    /// 32-byte token rendered as 64 lowercase hex characters.
    pub fn mint(&self) -> DeviceIdentity {
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let device_id = format!("dev-{sequence:04}");
        let token = random_token();
        self.tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(token.clone(), device_id.clone());
        DeviceIdentity { device_id, token }
    }

    /// Authenticates a presented bearer token against every minted token.
    ///
    /// A *match* short-circuits via `Iterator::find`, same as any lookup --
    /// that's fine, since which token matched is exactly the information
    /// being asked for. The property this guards is about *rejection*: a
    /// wrong token is compared against the entire table with
    /// [`constant_time_eq`] rather than `==`, so how many of its leading
    /// bytes happen to match some real token never shows up as a timing
    /// difference. At the device counts V2's single-tenant registry ever
    /// holds, scanning the whole table on a miss is cheap.
    pub fn authenticate(&self, token: &str) -> Option<DeviceId> {
        let token = token.as_bytes();
        // A panic while some other request held the lock must not turn every
        // *subsequent* authentication into a panic too -- that would be a
        // latent, self-inflicted denial of service on an internet-facing
        // endpoint. The registry holds no invariant a panic mid-mutation
        // could leave inconsistent (it's an insert or a scan, nothing more),
        // so recovering the poisoned guard and carrying on is safe.
        let tokens = self
            .tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tokens
            .iter()
            .find(|(candidate, _)| constant_time_eq(candidate.as_bytes(), token))
            .map(|(_, device_id)| device_id.clone())
    }
}

/// Generates a random 32-byte token, rendered as 64 lowercase hex characters.
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            use std::fmt::Write;
            write!(hex, "{byte:02x}").expect("writing to a String cannot fail");
            hex
        })
}

/// Compares two byte strings without short-circuiting on the first mismatch,
/// so the comparison's timing does not leak how many leading bytes agree.
///
/// Lengths are checked up front: our tokens are always the same fixed
/// length, so a length mismatch only tells an attacker their guess had the
/// wrong shape, never anything about the secret's content. Shared by device
/// token authentication ([`Registry::authenticate`]) and, via
/// `ServerState::verify_admin_token`, the single admin token -- the crate
/// has exactly one way to compare a presented secret against a real one.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_assigns_monotonic_ids() {
        let registry = Registry::new();
        let first = registry.mint();
        let second = registry.mint();
        assert_eq!(first.device_id, "dev-0001");
        assert_eq!(second.device_id, "dev-0002");
        assert_ne!(first.token, second.token);
    }

    #[test]
    fn authenticate_finds_a_minted_token() {
        let registry = Registry::new();
        let identity = registry.mint();
        assert_eq!(
            registry.authenticate(&identity.token),
            Some(identity.device_id)
        );
    }

    #[test]
    fn authenticate_rejects_an_unknown_token() {
        let registry = Registry::new();
        registry.mint();
        assert_eq!(registry.authenticate("not-a-real-token"), None);
    }

    #[test]
    fn constant_time_eq_matches_equal_bytes() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn debug_redacts_the_token() {
        let identity = DeviceIdentity {
            device_id: "dev-0001".to_string(),
            token: "super-secret-value".to_string(),
        };
        let rendered = format!("{identity:?}");
        assert!(!rendered.contains("super-secret-value"));
        assert!(rendered.contains("dev-0001"));
    }
}
