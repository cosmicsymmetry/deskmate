//! Shared primitives for bearer credentials persisted as SHA-256 digests.

use rand::RngCore;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DigestDecodeError {
    Length,
    Character,
}

pub(crate) fn token_digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// Generates 32 random bytes as 64 lowercase hexadecimal characters.
pub(crate) fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    protocol::digest_hex(&bytes)
}

pub(crate) fn decode_digest(encoded: &str) -> Result<[u8; 32], DigestDecodeError> {
    if encoded.len() != 64 || !encoded.is_ascii() {
        return Err(DigestDecodeError::Length);
    }
    let mut digest = [0u8; 32];
    for (output, pair) in digest.iter_mut().zip(encoded.as_bytes().as_chunks::<2>().0) {
        *output = (decode_hex_digit(pair[0])? << 4) | decode_hex_digit(pair[1])?;
    }
    Ok(digest)
}

fn decode_hex_digit(byte: u8) -> Result<u8, DigestDecodeError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(DigestDecodeError::Character),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_decoder_accepts_canonical_lowercase_sha256() {
        assert_eq!(decode_digest(&"01".repeat(32)), Ok([0x01; 32]));
    }

    #[test]
    fn digest_decoder_distinguishes_shape_from_character_errors() {
        assert_eq!(decode_digest("00"), Err(DigestDecodeError::Length));
        assert_eq!(
            decode_digest(&"GG".repeat(32)),
            Err(DigestDecodeError::Character)
        );
    }
}
