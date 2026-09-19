use std::fmt;
use std::io::{Read, Write};
use std::time::{Duration, Instant};

use protocol::{
    Ack, Deframer, ErrorResponse, FrameError, HeartbeatAck, Message, MessageError, PushTimer,
    RequestIdAllocator, StatusResponse, TYPE_PUSH_TIMER, TYPE_TIME_SYNC, TimeSync, decode_message,
    encode_message, expected_response_type,
};

pub mod framebuffer_capture;
mod replay;
mod session;
pub mod session_state;

pub use replay::ReplayState;
pub use session::{
    ConnectedSession, DEFAULT_EVENT_QUEUE_CAPACITY, DeviceSession, ReceivedEvent, connect_session,
};
pub use session_state::SessionDiagnostics;

const ESPRESSIF_USB_VID: u16 = 0x303a;
const ESP32_S3_SERIAL_JTAG_PID: u16 = 0x1001;
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const SERIAL_READ_TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    Disconnected,
    Io(String),
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected => f.write_str("device disconnected"),
            Self::Io(error) => write!(f, "serial I/O: {error}"),
        }
    }
}

impl std::error::Error for TransportError {}

pub trait Transport {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError>;
    fn read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceError {
    NoDevice,
    Timeout,
    Transport(TransportError),
    MalformedResponse(String),
    UnexpectedMessage,
    VersionMismatch(u8),
    Rejected(ErrorResponse),
    InvalidRequest,
    RevisionExhausted,
    MissingCapabilities { required: u64, available: u64 },
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDevice => f.write_str("no Deskmate device found"),
            Self::Timeout => f.write_str("device request timed out"),
            Self::Transport(error) => error.fmt(f),
            Self::MalformedResponse(error) => write!(f, "malformed device response: {error}"),
            Self::UnexpectedMessage => f.write_str("device returned an unexpected message type"),
            Self::VersionMismatch(version) => {
                write!(f, "unsupported device protocol version {version}")
            }
            Self::Rejected(error) => {
                write!(
                    f,
                    "device rejected request ({:?}): {}",
                    error.code, error.diagnostic
                )
            }
            Self::InvalidRequest => f.write_str("message is not a host request"),
            Self::RevisionExhausted => f.write_str("device revision counter is exhausted"),
            Self::MissingCapabilities {
                required,
                available,
            } => write!(
                f,
                "device capabilities 0x{available:016x} do not satisfy required 0x{required:016x}"
            ),
        }
    }
}

impl std::error::Error for DeviceError {}

impl From<TransportError> for DeviceError {
    fn from(value: TransportError) -> Self {
        Self::Transport(value)
    }
}

fn message_error(error: MessageError) -> DeviceError {
    match error {
        MessageError::Version(version) => DeviceError::VersionMismatch(version),
        other => DeviceError::MalformedResponse(other.to_string()),
    }
}

pub struct DeviceClient<T> {
    transport: T,
    deframer: Deframer,
    request_ids: RequestIdAllocator,
    timeout: Duration,
}

impl<T: Transport> DeviceClient<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            deframer: Deframer::new(),
            request_ids: RequestIdAllocator::new(),
            timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_timeout(transport: T, timeout: Duration) -> Self {
        Self {
            timeout,
            ..Self::new(transport)
        }
    }

    pub fn into_transport(self) -> T {
        self.transport
    }

    pub fn request(&mut self, request: &Message) -> Result<Message, DeviceError> {
        let expected_type =
            expected_response_type(request.type_id()).ok_or(DeviceError::InvalidRequest)?;
        let request_id = self.request_ids.allocate();
        let wire = encode_message(request_id, request).map_err(message_error)?;
        let deadline = Instant::now() + self.timeout;
        let mut written = 0;
        while written < wire.len() {
            if Instant::now() >= deadline {
                return Err(DeviceError::Timeout);
            }
            let count = self.transport.write(&wire[written..])?;
            if count == 0 {
                return Err(DeviceError::Transport(TransportError::Disconnected));
            }
            written += count;
        }

        let mut chunk = [0_u8; 512];
        loop {
            if Instant::now() >= deadline {
                return Err(DeviceError::Timeout);
            }
            let count = self.transport.read(&mut chunk)?;
            if count == 0 {
                continue;
            }
            for framed in self.deframer.push(&chunk[..count]) {
                let frame = framed.map_err(|error: FrameError| {
                    DeviceError::MalformedResponse(error.to_string())
                })?;
                if frame.request_id != request_id {
                    continue;
                }
                let message = decode_message(&frame).map_err(message_error)?;
                if let Message::Error(error) = message {
                    return Err(DeviceError::Rejected(error));
                }
                if message.type_id() != expected_type {
                    return Err(DeviceError::UnexpectedMessage);
                }
                return Ok(message);
            }
        }
    }

    pub fn status(&mut self) -> Result<StatusResponse, DeviceError> {
        match self.request(&Message::StatusRequest)? {
            Message::StatusResponse(status) => Ok(status),
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }

    pub fn time_sync(&mut self, sync: TimeSync) -> Result<Ack, DeviceError> {
        match self.request(&Message::TimeSync(sync))? {
            Message::Ack(ack)
                if ack.acknowledged_type == TYPE_TIME_SYNC && ack.revision.is_none() =>
            {
                Ok(ack)
            }
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }

    pub fn push_timer(&mut self, push: PushTimer) -> Result<Ack, DeviceError> {
        match self.request(&Message::PushTimer(push))? {
            Message::Ack(ack)
                if ack.acknowledged_type == TYPE_PUSH_TIMER && ack.revision.is_some() =>
            {
                Ok(ack)
            }
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }

    pub fn heartbeat(&mut self) -> Result<HeartbeatAck, DeviceError> {
        match self.request(&Message::Heartbeat)? {
            Message::HeartbeatAck(ack) => Ok(ack),
            _ => Err(DeviceError::UnexpectedMessage),
        }
    }
}

pub struct SerialTransport {
    port: Box<dyn serialport::SerialPort>,
}

impl SerialTransport {
    fn open(path: &str) -> Result<Self, DeviceError> {
        let port = serialport::new(path, 115_200)
            .timeout(SERIAL_READ_TIMEOUT)
            .open()
            .map_err(|error| DeviceError::Transport(TransportError::Io(error.to_string())))?;
        // The ESP32-S3 ROM may have emitted startup text before application
        // firmware takes ownership. Drain any kernel-side backlog before the
        // status handshake; runtime application logs are routed to UART0.
        port.clear(serialport::ClearBuffer::Input)
            .map_err(|error| DeviceError::Transport(TransportError::Io(error.to_string())))?;
        Ok(Self { port })
    }
}

impl Transport for SerialTransport {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError> {
        self.port.write(bytes).map_err(|error| {
            if error.kind() == std::io::ErrorKind::BrokenPipe {
                TransportError::Disconnected
            } else {
                TransportError::Io(error.to_string())
            }
        })
    }

    fn read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportError> {
        self.port.read(bytes).or_else(|error| {
            if matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) {
                Ok(0)
            } else if error.kind() == std::io::ErrorKind::BrokenPipe {
                Err(TransportError::Disconnected)
            } else {
                Err(TransportError::Io(error.to_string()))
            }
        })
    }
}

pub struct ConnectedDevice {
    pub port_name: String,
    pub initial_status: StatusResponse,
    pub client: DeviceClient<SerialTransport>,
}

fn candidate_ports() -> Result<Vec<String>, DeviceError> {
    let ports = serialport::available_ports()
        .map_err(|error| DeviceError::Transport(TransportError::Io(error.to_string())))?;
    Ok(ports
        .into_iter()
        .filter_map(|port| match port.port_type {
            serialport::SerialPortType::UsbPort(info)
                if info.vid == ESPRESSIF_USB_VID && info.pid == ESP32_S3_SERIAL_JTAG_PID =>
            {
                Some(port.port_name)
            }
            _ => None,
        })
        .collect())
}

pub fn connect(explicit_port: Option<&str>) -> Result<ConnectedDevice, DeviceError> {
    let ports = if let Some(port) = explicit_port {
        vec![port.to_owned()]
    } else {
        candidate_ports()?
    };
    if ports.is_empty() {
        return Err(DeviceError::NoDevice);
    }

    let mut last_error = DeviceError::NoDevice;
    for port_name in ports {
        let transport = match SerialTransport::open(&port_name) {
            Ok(transport) => transport,
            Err(error) => {
                last_error = error;
                continue;
            }
        };
        let mut client = DeviceClient::new(transport);
        match client.status() {
            Ok(initial_status) => {
                return Ok(ConnectedDevice {
                    port_name,
                    initial_status,
                    client,
                });
            }
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use protocol::{Frame, encode_frame};

    use super::*;

    #[derive(Default)]
    struct FakeTransport {
        reads: VecDeque<Result<Vec<u8>, TransportError>>,
        written: Vec<u8>,
        maximum_write: usize,
    }

    impl Transport for FakeTransport {
        fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError> {
            let count = if self.maximum_write == 0 {
                bytes.len()
            } else {
                bytes.len().min(self.maximum_write)
            };
            self.written.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        fn read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportError> {
            match self.reads.pop_front() {
                Some(Ok(chunk)) => {
                    assert!(chunk.len() <= bytes.len());
                    bytes[..chunk.len()].copy_from_slice(&chunk);
                    Ok(chunk.len())
                }
                Some(Err(error)) => Err(error),
                None => Ok(0),
            }
        }
    }

    fn status() -> StatusResponse {
        StatusResponse {
            firmware_version: "deskmate-m1".into(),
            uptime_ms: 123,
            free_heap: 456,
            display_width: 368,
            display_height: 448,
            rotation: 0,
            latest_revision: 7,
            valid_frames: 10,
            malformed_frames: 2,
            crc_errors: 1,
            overflow_frames: 1,
            ..protocol::test_support::sample_status_response()
        }
    }

    #[test]
    fn partial_reads_and_writes_complete_request() {
        let response = encode_message(1, &Message::StatusResponse(status())).unwrap();
        let split = response.len() / 2;
        let fake = FakeTransport {
            reads: VecDeque::from([
                Ok(response[..split].to_vec()),
                Ok(response[split..].to_vec()),
            ]),
            maximum_write: 3,
            ..FakeTransport::default()
        };
        let expected_request = encode_message(1, &Message::StatusRequest).unwrap();
        let mut client = DeviceClient::new(fake);
        assert_eq!(client.status().unwrap(), status());
        let fake = client.into_transport();
        assert_eq!(fake.written, expected_request);
    }

    #[test]
    fn malformed_response_is_reported() {
        let fake = FakeTransport {
            reads: VecDeque::from([Ok(vec![2, 1, 0])]),
            ..FakeTransport::default()
        };
        let mut client = DeviceClient::new(fake);
        assert!(matches!(
            client.status(),
            Err(DeviceError::MalformedResponse(_))
        ));
    }

    #[test]
    fn late_response_does_not_displace_the_current_response() {
        let stale = encode_message(9, &Message::StatusResponse(status())).unwrap();
        let response = encode_message(1, &Message::StatusResponse(status())).unwrap();
        let fake = FakeTransport {
            reads: VecDeque::from([Ok(stale), Ok(response)]),
            ..FakeTransport::default()
        };
        let mut client = DeviceClient::new(fake);
        assert_eq!(client.status().unwrap(), status());
    }

    #[test]
    fn two_late_responses_cannot_form_a_cascade() {
        let stale_first = encode_message(41, &Message::StatusResponse(status())).unwrap();
        let stale_second = encode_message(42, &Message::StatusResponse(status())).unwrap();
        let response = encode_message(1, &Message::StatusResponse(status())).unwrap();
        let fake = FakeTransport {
            reads: VecDeque::from([Ok(stale_first), Ok(stale_second), Ok(response)]),
            ..FakeTransport::default()
        };
        let mut client = DeviceClient::new(fake);
        assert_eq!(client.status().unwrap(), status());
    }

    #[test]
    fn timeout_is_distinct() {
        let fake = FakeTransport::default();
        let mut client = DeviceClient::with_timeout(fake, Duration::ZERO);
        assert_eq!(client.status(), Err(DeviceError::Timeout));
    }

    #[test]
    fn disconnect_is_distinct() {
        let fake = FakeTransport {
            reads: VecDeque::from([Err(TransportError::Disconnected)]),
            ..FakeTransport::default()
        };
        let mut client = DeviceClient::new(fake);
        assert_eq!(
            client.status(),
            Err(DeviceError::Transport(TransportError::Disconnected))
        );
    }

    #[test]
    fn version_mismatch_is_distinct() {
        let valid = encode_message(1, &Message::StatusResponse(status())).unwrap();
        let mut frame = protocol::decode_wire_frame(&valid).unwrap();
        frame.version = protocol::PROTOCOL_VERSION + 1;
        let response = encode_frame(&frame).unwrap();
        let fake = FakeTransport {
            reads: VecDeque::from([Ok(response)]),
            ..FakeTransport::default()
        };
        let mut client = DeviceClient::new(fake);
        assert_eq!(
            client.status(),
            Err(DeviceError::VersionMismatch(protocol::PROTOCOL_VERSION + 1))
        );
    }

    #[test]
    fn wrong_response_type_is_rejected() {
        let response =
            encode_message(1, &Message::HeartbeatAck(HeartbeatAck { uptime_ms: 1 })).unwrap();
        let fake = FakeTransport {
            reads: VecDeque::from([Ok(response)]),
            ..FakeTransport::default()
        };
        let mut client = DeviceClient::new(fake);
        assert_eq!(client.status(), Err(DeviceError::UnexpectedMessage));
    }

    #[test]
    fn frame_helper_accepts_nondefault_version() {
        let frame = Frame {
            version: 2,
            message_type: 1,
            flags: 0,
            request_id: 1,
            payload: vec![0xa0],
        };
        assert!(encode_frame(&frame).is_ok());
    }
}
