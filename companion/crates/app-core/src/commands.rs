use std::fmt;
use std::sync::mpsc::SyncSender;

use protocol::PushScene;
use serde::{Deserialize, Serialize};

use crate::{AppConfig, NetworkConfig, PersistenceState, ValidationIssue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PomodoroAction {
    Start,
    Pause,
    Toggle,
    Reset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RuntimeError {
    InvalidConfig { issues: Vec<ValidationIssue> },
    QueueFull,
    WorkerStopped,
    ResponseTimeout,
    DeviceDisconnected,
    UnknownCard { card_id: String },
    Device { message: String },
    ImageSource { message: String },
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig { issues } => write!(
                formatter,
                "configuration has {} validation issue(s)",
                issues.len()
            ),
            Self::QueueFull => formatter.write_str("runtime command queue is full"),
            Self::WorkerStopped => formatter.write_str("runtime worker has stopped"),
            Self::ResponseTimeout => formatter.write_str("runtime command response timed out"),
            Self::DeviceDisconnected => formatter.write_str("device is disconnected"),
            Self::UnknownCard { card_id } => write!(formatter, "unknown card {card_id:?}"),
            Self::Device { message } => write!(formatter, "device: {message}"),
            Self::ImageSource { message } => write!(formatter, "image source: {message}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

pub(crate) type CommandReply = SyncSender<Result<(), RuntimeError>>;

pub(crate) enum RuntimeCommand {
    ApplyConfig {
        config: AppConfig,
        reply: CommandReply,
    },
    SetPaused {
        paused: bool,
        reply: CommandReply,
    },
    SetAutostartPreference {
        enabled: bool,
        reply: CommandReply,
    },
    SetPersistenceState {
        persistence: PersistenceState,
        reply: CommandReply,
    },
    Pomodoro {
        card_id: String,
        action: PomodoroAction,
        reply: CommandReply,
    },
    /// One image source received a new picture. Carries no bytes: the handler
    /// reconciles the whole desired set from the host, which already owns them.
    ImageSourceUpdated {
        source_id: String,
        digest: [u8; 32],
        reply: CommandReply,
    },
    ActivateCard {
        card_id: String,
        reply: CommandReply,
    },
    PushScene {
        push: PushScene,
        reply: CommandReply,
    },
    Provision {
        config: NetworkConfig,
        reply: CommandReply,
    },
    FactoryReset {
        reply: CommandReply,
    },
    Shutdown {
        reply: CommandReply,
    },
}
