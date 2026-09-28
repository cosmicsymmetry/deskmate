//! PKCE (RFC 7636) and CSRF-`state` primitives for the OAuth consent flow.

use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};

pub(crate) struct PkcePair {
    pub(crate) verifier: String,
    pub(crate) challenge: String,
}

/// Generates a fresh PKCE verifier and its S256 challenge.
pub(crate) fn generate_pkce() -> PkcePair {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let verifier = BASE64_URL_SAFE_NO_PAD.encode(bytes);
    let challenge = challenge_for(&verifier);
    PkcePair {
        verifier,
        challenge,
    }
}

/// The S256 `code_challenge` for a verifier: base64url-no-pad(SHA-256(verifier)).
#[must_use]
pub(super) fn challenge_for(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    BASE64_URL_SAFE_NO_PAD.encode(digest)
}

/// A random opaque CSRF `state` value.
#[must_use]
pub(crate) fn generate_state() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    BASE64_URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_is_the_base64url_sha256_of_the_verifier() {
        let pair = generate_pkce();
        assert_eq!(challenge_for(&pair.verifier), pair.challenge);
    }

    #[test]
    fn verifier_meets_rfc7636_length_and_charset() {
        let pair = generate_pkce();
        // 32 bytes base64url-no-pad = 43 chars, inside RFC 7636's 43..=128.
        assert_eq!(pair.verifier.len(), 43);
        assert!(
            pair.verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "verifier must be base64url with no padding"
        );
        assert!(!pair.challenge.contains('='), "challenge must be unpadded");
    }

    #[test]
    fn generated_values_are_distinct_each_call() {
        assert_ne!(generate_state(), generate_state());
        assert_ne!(generate_pkce().verifier, generate_pkce().verifier);
    }

    #[test]
    fn challenge_for_matches_a_known_vector() {
        // RFC 7636 Appendix B verifier/challenge pair.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            challenge_for(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
}
