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
    UnknownWidget { widget_id: String },
    UnknownScreen { screen_id: String },
    UnknownCard { card_id: String },
    NotAPluginCard { card_id: String },
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
            Self::UnknownCard { card_id } => write!(formatter, "unknown card {card_id:?}"),
            Self::NotAPluginCard { card_id } => {
                write!(formatter, "card {card_id:?} is not a plugin card")
            }
            Self::Device { message } => write!(formatter, "device: {message}"),
            Self::Provider { message } => write!(formatter, "provider: {message}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

pub(crate) type CommandReply = SyncSender<Result<(), RuntimeError>>;

/// A preview answers with a rendered frame, so it cannot ride `CommandReply`,
/// which carries only success or failure.
pub(crate) type PreviewReply = SyncSender<Result<crate::runtime::CardPreview, RuntimeError>>;

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
    InjectPluginSnapshot {
        card_id: String,
        plugin_id: String,
        snapshot: providers::ProviderSnapshot<serde_json::Value>,
        reply: CommandReply,
    },
    RenderCardPreview {
        card_id: String,
        reply: PreviewReply,
    },
    ActivateScreen {
        screen_id: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Two distinct refusals, kept apart on purpose: one card does not exist,
    /// the other exists and is the wrong kind. Folding them into
    /// `UnknownWidget` would make a plugin-id typo and a clock card read
    /// identically in the log and on the wire. What HTTP status each becomes
    /// is the server's business, not app-core's.
    #[test]
    fn the_preview_card_errors_are_typed_and_name_the_card() {
        let unknown = RuntimeError::UnknownCard {
            card_id: "aqi".into(),
        };
        assert_eq!(unknown.to_string(), "unknown card \"aqi\"");
        assert_eq!(
            serde_json::to_string(&unknown).unwrap(),
            r#"{"kind":"unknown-card","card_id":"aqi"}"#
        );

        let wrong_kind = RuntimeError::NotAPluginCard {
            card_id: "clock".into(),
        };
        assert_eq!(
            wrong_kind.to_string(),
            "card \"clock\" is not a plugin card"
        );
        assert_eq!(
            serde_json::to_string(&wrong_kind).unwrap(),
            r#"{"kind":"not-a-plugin-card","card_id":"clock"}"#
        );
    }
}
