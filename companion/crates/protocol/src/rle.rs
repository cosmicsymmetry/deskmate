//! Run-length encoding for little-endian RGB565 pixel streams.
//!
//! The wire format is a sequence of `(count: u16 LE, pixel: u16 LE)` runs.
//! It deliberately does not include the 12-byte LVGL image header: callers
//! keep that header raw and apply this codec only to the pixel bytes.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rle565Error {
    OddPixelLength,
    TruncatedStream,
    ZeroCount,
    DecodedOverrun,
    DecodedLengthMismatch,
}

/// Encode a little-endian RGB565 pixel stream without deciding whether the
/// result is worth sending. The caller owns the raw-vs-RLE size comparison.
pub fn encode_rle565(pixel_bytes: &[u8]) -> Result<Vec<u8>, Rle565Error> {
    if !pixel_bytes.len().is_multiple_of(2) {
        return Err(Rle565Error::OddPixelLength);
    }

    let pixels = pixel_bytes.as_chunks::<2>().0;
    let mut encoded = Vec::new();
    let mut index = 0usize;
    while index < pixels.len() {
        let pixel = pixels[index];
        let mut count = 1u16;
        while index + usize::from(count) < pixels.len()
            && pixels[index + usize::from(count)] == pixel
            && count < u16::MAX
        {
            count += 1;
        }
        encoded.extend_from_slice(&count.to_le_bytes());
        encoded.extend_from_slice(&pixel);
        index += usize::from(count);
    }
    Ok(encoded)
}

/// Decode exactly `decoded_length` bytes of little-endian RGB565 pixels.
pub fn decode_rle565(encoded: &[u8], decoded_length: usize) -> Result<Vec<u8>, Rle565Error> {
    if !decoded_length.is_multiple_of(2) {
        return Err(Rle565Error::OddPixelLength);
    }
    if !encoded.len().is_multiple_of(4) {
        return Err(Rle565Error::TruncatedStream);
    }

    let mut decoded = Vec::with_capacity(decoded_length);
    for run in encoded.as_chunks::<4>().0 {
        let count = u16::from_le_bytes([run[0], run[1]]);
        if count == 0 {
            return Err(Rle565Error::ZeroCount);
        }
        let run_bytes = usize::from(count)
            .checked_mul(2)
            .ok_or(Rle565Error::DecodedOverrun)?;
        if run_bytes > decoded_length - decoded.len() {
            return Err(Rle565Error::DecodedOverrun);
        }
        for _ in 0..count {
            decoded.extend_from_slice(&run[2..4]);
        }
    }
    if decoded.len() != decoded_length {
        return Err(Rle565Error::DecodedLengthMismatch);
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_round_trip_across_run_boundaries() {
        let mut pixels = Vec::new();
        for &(count, pixel) in &[
            (1usize, 0x0000u16),
            (2, 0xffff),
            (u16::MAX as usize, 0x1234),
            (3, 0x1234),
            (257, 0xabcd),
        ] {
            for _ in 0..count {
                pixels.extend_from_slice(&pixel.to_le_bytes());
            }
        }
        let encoded = encode_rle565(&pixels).unwrap();
        assert_eq!(decode_rle565(&encoded, pixels.len()).unwrap(), pixels);
    }

    #[test]
    fn deterministic_property_round_trips_many_pixel_streams() {
        let mut state = 0x6d2b_79f5u32;
        for pixel_count in 0..512usize {
            let mut pixels = Vec::with_capacity(pixel_count * 2);
            let mut previous = 0u16;
            for index in 0..pixel_count {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let pixel = if index % 7 == 0 {
                    previous
                } else {
                    u16::try_from(state & 0xffff).unwrap()
                };
                pixels.extend_from_slice(&pixel.to_le_bytes());
                previous = pixel;
            }
            let encoded = encode_rle565(&pixels).unwrap();
            assert_eq!(decode_rle565(&encoded, pixels.len()).unwrap(), pixels);
        }
    }

    #[test]
    fn hostile_streams_are_rejected_by_class() {
        assert_eq!(
            decode_rle565(&[1, 0, 0], 2),
            Err(Rle565Error::TruncatedStream)
        );
        assert_eq!(
            decode_rle565(&[0, 0, 0x34, 0x12], 2),
            Err(Rle565Error::ZeroCount)
        );
        assert_eq!(
            decode_rle565(&[2, 0, 0x34, 0x12], 2),
            Err(Rle565Error::DecodedOverrun)
        );
        assert_eq!(
            decode_rle565(&[1, 0, 0x34, 0x12], 4),
            Err(Rle565Error::DecodedLengthMismatch)
        );
    }

    #[test]
    fn encoder_rejects_a_partial_pixel() {
        assert_eq!(encode_rle565(&[0]), Err(Rle565Error::OddPixelLength));
    }
}
