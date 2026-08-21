use std::fmt;
use std::sync::mpsc::SyncSender;

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
    UnknownWidget { widget_id: String },
    UnknownScreen { screen_id: String },
    Device { message: String },
    Provider { message: String },
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
            Self::UnknownWidget { widget_id } => write!(formatter, "unknown widget {widget_id:?}"),
            Self::UnknownScreen { screen_id } => write!(formatter, "unknown screen {screen_id:?}"),
            Self::Device { message } => write!(formatter, "device: {message}"),
            Self::Provider { message } => write!(formatter, "provider: {message}"),
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
        widget_id: String,
        action: PomodoroAction,
        reply: CommandReply,
    },
    RefreshProvider {
        widget_id: String,
        reply: CommandReply,
    },
    ActivateScreen {
        screen_id: String,
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
