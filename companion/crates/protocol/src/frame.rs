use core::fmt;

pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_DECODED_FRAME: usize = 2048;
pub const MAX_PAYLOAD_SIZE: usize = MAX_DECODED_FRAME - 10 - 4;
pub const MAX_WIRE_FRAME: usize = 2058;
const MAX_ENCODED_WITHOUT_DELIMITER: usize = MAX_WIRE_FRAME - 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub version: u8,
    pub message_type: u8,
    pub flags: u16,
    pub request_id: u32,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(message_type: u8, request_id: u32, payload: Vec<u8>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            message_type,
            flags: 0,
            request_id,
            payload,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    MissingDelimiter,
    EmbeddedDelimiter,
    Overlong,
    Cobs,
    TooShort,
    Length,
    Checksum,
    InvalidFlags,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for FrameError {}

pub fn crc32c(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0x82f6_3b78 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

pub(crate) fn cobs_encode(decoded: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(decoded.len() + decoded.len() / 254 + 1);
    encoded.push(0);
    let mut code_index = 0;
    let mut code = 1_u8;
    for &byte in decoded {
        if byte == 0 {
            encoded[code_index] = code;
            code_index = encoded.len();
            encoded.push(0);
            code = 1;
        } else {
            encoded.push(byte);
            code = code.wrapping_add(1);
            if code == 0xff {
                encoded[code_index] = code;
                code_index = encoded.len();
                encoded.push(0);
                code = 1;
            }
        }
    }
    encoded[code_index] = code;
    encoded
}

fn cobs_decode(encoded: &[u8]) -> Result<Vec<u8>, FrameError> {
    if encoded.is_empty() {
        return Err(FrameError::Cobs);
    }
    let mut decoded = Vec::with_capacity(encoded.len());
    let mut pos = 0;
    while pos < encoded.len() {
        let code = encoded[pos];
        if code == 0 {
            return Err(FrameError::Cobs);
        }
        pos += 1;
        let count = usize::from(code - 1);
        let end = pos.checked_add(count).ok_or(FrameError::Cobs)?;
        if end > encoded.len() {
            return Err(FrameError::Cobs);
        }
        decoded.extend_from_slice(&encoded[pos..end]);
        pos = end;
        if code != 0xff && pos < encoded.len() {
            decoded.push(0);
        }
        if decoded.len() > MAX_DECODED_FRAME {
            return Err(FrameError::Overlong);
        }
    }
    Ok(decoded)
}

pub fn encode_frame(frame: &Frame) -> Result<Vec<u8>, FrameError> {
    if frame.flags != 0 {
        return Err(FrameError::InvalidFlags);
    }
    if frame.payload.len() > MAX_PAYLOAD_SIZE {
        return Err(FrameError::Overlong);
    }
    let payload_len = u16::try_from(frame.payload.len()).map_err(|_| FrameError::Overlong)?;
    let mut decoded = Vec::with_capacity(10 + frame.payload.len() + 4);
    decoded.push(frame.version);
    decoded.push(frame.message_type);
    decoded.extend_from_slice(&frame.flags.to_le_bytes());
    decoded.extend_from_slice(&frame.request_id.to_le_bytes());
    decoded.extend_from_slice(&payload_len.to_le_bytes());
    decoded.extend_from_slice(&frame.payload);
    decoded.extend_from_slice(&crc32c(&decoded).to_le_bytes());
    let mut wire = cobs_encode(&decoded);
    wire.push(0);
    debug_assert!(wire.len() <= MAX_WIRE_FRAME);
    Ok(wire)
}

pub fn decode_wire_frame(wire: &[u8]) -> Result<Frame, FrameError> {
    if wire.last() != Some(&0) {
        return Err(FrameError::MissingDelimiter);
    }
    if wire.len() > MAX_WIRE_FRAME {
        return Err(FrameError::Overlong);
    }
    let encoded = &wire[..wire.len() - 1];
    if encoded.contains(&0) {
        return Err(FrameError::EmbeddedDelimiter);
    }
    let decoded = cobs_decode(encoded)?;
    if decoded.len() < 14 {
        return Err(FrameError::TooShort);
    }
    let payload_len = usize::from(u16::from_le_bytes([decoded[8], decoded[9]]));
    if decoded.len() != 10 + payload_len + 4 {
        return Err(FrameError::Length);
    }
    let crc_offset = decoded.len() - 4;
    let expected = u32::from_le_bytes(decoded[crc_offset..].try_into().unwrap());
    if crc32c(&decoded[..crc_offset]) != expected {
        return Err(FrameError::Checksum);
    }
    let flags = u16::from_le_bytes([decoded[2], decoded[3]]);
    if flags != 0 {
        return Err(FrameError::InvalidFlags);
    }
    let request_id = u32::from_le_bytes(decoded[4..8].try_into().unwrap());
    Ok(Frame {
        version: decoded[0],
        message_type: decoded[1],
        flags,
        request_id,
        payload: decoded[10..crc_offset].to_vec(),
    })
}

#[derive(Debug, Default)]
pub struct Deframer {
    encoded: Vec<u8>,
    discarding: bool,
}

impl Deframer {
    pub fn new() -> Self {
        Self {
            encoded: Vec::with_capacity(MAX_ENCODED_WITHOUT_DELIMITER),
            discarding: false,
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<Result<Frame, FrameError>> {
        let mut frames = Vec::new();
        for &byte in chunk {
            if self.discarding {
                if byte == 0 {
                    self.discarding = false;
                    frames.push(Err(FrameError::Overlong));
                }
                continue;
            }
            if byte == 0 {
                if !self.encoded.is_empty() {
                    self.encoded.push(0);
                    frames.push(decode_wire_frame(&self.encoded));
                    self.encoded.clear();
                }
            } else if self.encoded.len() == MAX_ENCODED_WITHOUT_DELIMITER {
                self.encoded.clear();
                self.discarding = true;
            } else {
                self.encoded.push(byte);
            }
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_known_vector() {
        assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    }

    #[test]
    fn embedded_zeroes_round_trip() {
        let frame = Frame::new(3, 0x1234_5678, vec![0, 1, 0, 2, 0]);
        assert_eq!(
            decode_wire_frame(&encode_frame(&frame).unwrap()).unwrap(),
            frame
        );
    }

    #[test]
    fn fragmented_and_coalesced_frames() {
        let a = encode_frame(&Frame::new(1, 1, vec![0xa0])).unwrap();
        let b = encode_frame(&Frame::new(6, 2, vec![0xa0])).unwrap();
        let mut decoder = Deframer::new();
        assert!(decoder.push(&a[..2]).is_empty());
        let mut rest = a[2..].to_vec();
        rest.extend_from_slice(&b);
        let frames = decoder.push(&rest);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].as_ref().unwrap().request_id, 1);
        assert_eq!(frames[1].as_ref().unwrap().request_id, 2);
    }

    #[test]
    fn overflow_resynchronizes() {
        let valid = encode_frame(&Frame::new(1, 9, vec![0xa0])).unwrap();
        let mut bytes = vec![1; MAX_WIRE_FRAME + 10];
        bytes.push(0);
        bytes.extend_from_slice(&valid);
        let frames = Deframer::new().push(&bytes);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], Err(FrameError::Overlong));
        assert_eq!(frames[1].as_ref().unwrap().request_id, 9);
    }

    #[test]
    fn bad_crc_is_rejected() {
        let mut wire = encode_frame(&Frame::new(1, 1, vec![0xa0])).unwrap();
        let last_data = wire.len() - 2;
        wire[last_data] ^= 0x40;
        assert_eq!(decode_wire_frame(&wire), Err(FrameError::Checksum));
    }
}
