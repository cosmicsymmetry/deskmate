use crate::ASSET_DIGEST_LEN;

/// Allocates request IDs in wire order, wrapping from `u32::MAX` to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestIdAllocator {
    next: u32,
}

impl RequestIdAllocator {
    #[must_use]
    pub const fn new() -> Self {
        Self { next: 1 }
    }

    pub fn allocate(&mut self) -> u32 {
        let request_id = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        request_id
    }
}

impl Default for RequestIdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the longest prefix no longer than `maximum_bytes` that ends on a
/// UTF-8 character boundary.
#[must_use]
pub fn truncate_utf8_to_bytes(value: &str, maximum_bytes: usize) -> &str {
    if value.len() <= maximum_bytes {
        return value;
    }
    let mut end = maximum_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Encodes a protocol asset digest as 64 lowercase hexadecimal characters.
#[must_use]
pub fn digest_hex(digest: &[u8; ASSET_DIGEST_LEN]) -> String {
    const LOWER_HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity(ASSET_DIGEST_LEN * 2);
    for byte in digest {
        encoded.push(char::from(LOWER_HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(LOWER_HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_FIELD_TEXT_LEN;

    #[test]
    fn request_ids_are_nonzero_and_wrap_to_one() {
        let mut allocator = RequestIdAllocator::new();
        assert_eq!(allocator.allocate(), 1);
        allocator.next = u32::MAX;
        assert_eq!(allocator.allocate(), u32::MAX);
        assert_eq!(allocator.allocate(), 1);
    }

    #[test]
    fn truncate_utf8_to_bytes_handles_boundaries() {
        for (value, cap, expected) in [
            ("short", MAX_FIELD_TEXT_LEN, "short"),
            ("exact", 5, "exact"),
            ("x°°°°°°°°", 16, "x°°°°°°°"),
            ("text", 0, ""),
        ] {
            assert_eq!(truncate_utf8_to_bytes(value, cap), expected);
        }
    }

    #[test]
    fn digest_hex_is_fixed_width_and_lowercase() {
        let mut digest = [0_u8; ASSET_DIGEST_LEN];
        digest[..8].copy_from_slice(&[0x00, 0x01, 0x0f, 0x10, 0xab, 0xcd, 0xef, 0xff]);
        assert_eq!(
            digest_hex(&digest),
            format!("00010f10abcdefff{}", "00".repeat(ASSET_DIGEST_LEN - 8))
        );
    }
}
