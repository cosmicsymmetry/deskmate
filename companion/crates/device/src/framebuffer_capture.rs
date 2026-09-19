//! Dev-build-only RGB565 framebuffer capture over the raw USB transport.
//!
//! The 0x7E request and 0x7F response chunks deliberately live here rather
//! than in `protocol`: they are diagnostics compiled out of release firmware,
//! not part of the frozen wire contract. Keeping the reassembler in one place
//! also means every hardware harness validates the same request ids, totals,
//! chunk CRCs, sequential offsets, and bounds before trusting a device frame.

use std::fmt;
use std::time::{Duration, Instant};

use protocol::{Deframer, Frame, TYPE_ERROR, crc32c, decode_message, encode_frame};

use crate::{DeviceClient, Transport};

/// Width of the logical RGB565 snapshot returned by dev firmware.
pub const FRAME_WIDTH: usize = 448;
/// Height of the logical RGB565 snapshot returned by dev firmware.
pub const FRAME_HEIGHT: usize = 368;
/// Bytes in one complete little-endian RGB565 snapshot.
pub const FRAME_BYTES: usize = FRAME_WIDTH * FRAME_HEIGHT * 2;

/// Dev-only message ids outside the release range. They stay out of `protocol`
/// because release hosts must not treat diagnostics as part of protocol v2.
const CAPTURE_REQUEST_TYPE: u8 = 0x7E;
const CAPTURE_CHUNK_TYPE: u8 = 0x7F;
const CHUNK_HEADER_LEN: usize = 12;
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureError {
    Timeout { bytes_received: usize },
    Invalid(String),
}

impl CaptureError {
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout { .. })
    }
}

impl fmt::Display for CaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout { bytes_received } => write!(
                formatter,
                "timed out after {bytes_received}/{FRAME_BYTES} bytes -- is the device a \
                 DESKMATE_DEV_DIAG=1 build?"
            ),
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for CaptureError {}

fn invalid(message: impl Into<String>) -> CaptureError {
    CaptureError::Invalid(message.into())
}

fn write_all(transport: &mut impl Transport, bytes: &[u8]) -> Result<(), CaptureError> {
    let mut written = 0;
    while written < bytes.len() {
        let count = transport
            .write(&bytes[written..])
            .map_err(|error| invalid(error.to_string()))?;
        if count == 0 {
            return Err(invalid("device disconnected during write"));
        }
        written += count;
    }
    Ok(())
}

/// Sends one 0x7E capture request and reassembles its 0x7F chunk stream.
///
/// This temporarily takes ownership of the raw transport because
/// [`DeviceClient`] only understands release messages. The returned client is
/// fresh, matching the previous single-harness implementation, so callers can
/// resume ordinary requests whether capture succeeded or failed.
pub fn capture_framebuffer<T: Transport>(
    client: DeviceClient<T>,
    request_id: u32,
) -> (DeviceClient<T>, Result<Vec<u8>, CaptureError>) {
    let mut transport = client.into_transport();
    let result = (|| -> Result<Vec<u8>, CaptureError> {
        let wire = encode_frame(&Frame::new(CAPTURE_REQUEST_TYPE, request_id, Vec::new()))
            .map_err(|error| invalid(error.to_string()))?;
        write_all(&mut transport, &wire)?;

        let mut deframer = Deframer::new();
        let mut read_buffer = [0u8; 512];
        let deadline = Instant::now() + CAPTURE_TIMEOUT;
        let mut frame_buffer = vec![0u8; FRAME_BYTES];
        let mut bytes_received = 0usize;

        while bytes_received < FRAME_BYTES {
            if Instant::now() >= deadline {
                return Err(CaptureError::Timeout { bytes_received });
            }
            let count = transport
                .read(&mut read_buffer)
                .map_err(|error| invalid(error.to_string()))?;
            if count == 0 {
                continue;
            }
            for result in deframer.push(&read_buffer[..count]) {
                let frame = result
                    .map_err(|error| invalid(format!("malformed capture response: {error}")))?;
                if frame.request_id != request_id {
                    return Err(invalid(format!(
                        "capture response request ID mismatch: expected {request_id}, received {}",
                        frame.request_id
                    )));
                }
                if frame.message_type == TYPE_ERROR {
                    let message =
                        decode_message(&frame).map_err(|error| invalid(error.to_string()))?;
                    return Err(invalid(format!(
                        "device rejected capture request: {message:?}"
                    )));
                }
                if frame.message_type != CAPTURE_CHUNK_TYPE {
                    return Err(invalid(format!(
                        "unexpected response message type 0x{:02x}",
                        frame.message_type
                    )));
                }
                if frame.payload.len() < CHUNK_HEADER_LEN {
                    return Err(invalid("capture chunk shorter than its header"));
                }
                let offset = u32::from_le_bytes(frame.payload[0..4].try_into().unwrap()) as usize;
                let chunk_total = u32::from_le_bytes(frame.payload[4..8].try_into().unwrap());
                let chunk_crc = u32::from_le_bytes(frame.payload[8..12].try_into().unwrap());
                let data = &frame.payload[CHUNK_HEADER_LEN..];
                if crc32c(data) != chunk_crc {
                    return Err(invalid(format!("chunk at offset {offset} failed its CRC")));
                }
                if chunk_total as usize != FRAME_BYTES {
                    return Err(invalid(format!(
                        "capture total {chunk_total} does not match the expected \
                         {FRAME_BYTES}-byte frame"
                    )));
                }
                if offset != bytes_received {
                    return Err(invalid(format!(
                        "chunk at offset {offset} is not the next expected byte {bytes_received}"
                    )));
                }
                let end = offset.saturating_add(data.len());
                if end > frame_buffer.len() {
                    return Err(invalid(format!(
                        "chunk at offset {offset} (length {}) overruns the \
                         {FRAME_BYTES}-byte frame",
                        data.len()
                    )));
                }
                frame_buffer[offset..end].copy_from_slice(data);
                bytes_received = end;
                if bytes_received >= FRAME_BYTES {
                    break;
                }
            }
        }
        Ok(frame_buffer)
    })();
    (DeviceClient::new(transport), result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TransportError;

    #[derive(Default)]
    struct FakeTransport {
        reads: Vec<u8>,
        read_at: usize,
        written: Vec<u8>,
        maximum_read: usize,
        maximum_write: usize,
    }

    impl Transport for FakeTransport {
        fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError> {
            let count = bytes.len().min(self.maximum_write);
            self.written.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        fn read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportError> {
            let available = self.reads.len().saturating_sub(self.read_at);
            let count = available.min(bytes.len()).min(self.maximum_read);
            bytes[..count].copy_from_slice(&self.reads[self.read_at..self.read_at + count]);
            self.read_at += count;
            Ok(count)
        }
    }

    fn capture_responses(request_id: u32, frame: &[u8]) -> Vec<u8> {
        frame_chunks("capture_responses", &capture_chunks(request_id, frame))
    }

    fn frame_chunks(name: &str, chunks: &[Frame]) -> Vec<u8> {
        chunks
            .iter()
            .flat_map(|chunk| {
                encode_frame(chunk)
                    .unwrap_or_else(|error| panic!("{name}: frame encoding failed: {error:?}"))
            })
            .collect()
    }

    fn capture_chunks(request_id: u32, frame: &[u8]) -> Vec<Frame> {
        const DATA_PER_CHUNK: usize = 1000;
        let total = u32::try_from(frame.len()).unwrap();
        let mut chunks = Vec::new();
        for (offset, data) in frame.chunks(DATA_PER_CHUNK).enumerate() {
            let offset = u32::try_from(offset * DATA_PER_CHUNK).unwrap();
            let mut payload = Vec::with_capacity(CHUNK_HEADER_LEN + data.len());
            payload.extend_from_slice(&offset.to_le_bytes());
            payload.extend_from_slice(&total.to_le_bytes());
            payload.extend_from_slice(&crc32c(data).to_le_bytes());
            payload.extend_from_slice(data);
            chunks.push(Frame::new(CAPTURE_CHUNK_TYPE, request_id, payload));
        }
        chunks
    }

    #[test]
    fn out_of_order_chunks_are_rejected() {
        let request_id = 41;
        let original = capture_chunks(request_id, &vec![1; FRAME_BYTES]);
        let mut repeated = original.clone();
        repeated.insert(1, repeated[0].clone());
        let mut skipped = original;
        skipped.remove(1);

        for (name, chunks) in [
            ("a_repeated_chunk_is_rejected", repeated),
            ("a_skipped_chunk_is_rejected", skipped),
        ] {
            let transport = FakeTransport {
                reads: frame_chunks(name, &chunks),
                maximum_read: 73,
                maximum_write: 3,
                ..FakeTransport::default()
            };
            let (_, captured) = capture_framebuffer(DeviceClient::new(transport), request_id);
            assert!(
                matches!(&captured, Err(CaptureError::Invalid(_))),
                "{name}: an out-of-order chunk must produce Invalid, got {captured:?}"
            );
            assert!(
                !captured.as_ref().unwrap_err().is_timeout(),
                "{name}: an out-of-order chunk must not be classified as a timeout"
            );
        }
    }

    #[test]
    fn partial_io_reassembles_the_complete_crc_checked_frame() {
        let request_id = 41;
        let frame = (0..FRAME_BYTES)
            .map(|index| u8::try_from(index % 251).unwrap())
            .collect::<Vec<_>>();
        let transport = FakeTransport {
            reads: capture_responses(request_id, &frame),
            maximum_read: 73,
            maximum_write: 3,
            ..FakeTransport::default()
        };
        let expected_request =
            encode_frame(&Frame::new(CAPTURE_REQUEST_TYPE, request_id, Vec::new())).unwrap();

        let (client, captured) = capture_framebuffer(DeviceClient::new(transport), request_id);
        assert_eq!(captured.unwrap(), frame);
        assert_eq!(client.into_transport().written, expected_request);
    }
}
