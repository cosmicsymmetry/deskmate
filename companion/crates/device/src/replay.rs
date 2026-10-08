use std::time::Instant;

use protocol::{
    ActivateCard, ApplyConfig, DeviceEvent, EventAction, EventKind, Message, PushTimer, TimeSync,
    TriggerInterrupt,
};

#[derive(Clone, Default)]
pub struct ReplayState {
    pub time_sync: Option<(TimeSync, Instant)>,
    pub config: Option<ApplyConfig>,
    pub pushes: Vec<PushTimer>,
    pub active_card: Option<ActivateCard>,
    pub interrupts: Vec<TriggerInterrupt>,
}

impl ReplayState {
    /// Commit rebased revisions without overwriting events received during replay.
    pub fn commit_replay(&mut self, replay: Self) {
        self.config = replay.config;
        self.pushes = replay.pushes;
    }

    /// Record replay state only after validating the ACK's request type and revision.
    pub fn remember_success(&mut self, request: &Message) {
        match request {
            Message::TimeSync(sync) => self.time_sync = Some((*sync, Instant::now())),
            Message::ApplyConfig(config) => {
                let changes_live_model = self.config.as_ref() != Some(config);
                self.config = Some(config.clone());
                if changes_live_model {
                    self.pushes.clear();
                    self.interrupts.clear();
                    if self.active_card.as_ref().is_some_and(|active| {
                        !config
                            .cards
                            .iter()
                            .any(|card| card.card_id == active.card_id)
                    }) {
                        self.active_card = None;
                    }
                }
            }
            Message::PushTimer(push) => {
                self.pushes.retain(|cached| cached.card_id != push.card_id);
                self.pushes.push(push.clone());
                self.pushes.sort_unstable_by_key(|cached| cached.revision);
            }
            Message::ActivateCard(activation) => {
                self.active_card = Some(activation.clone());
            }
            Message::TriggerInterrupt(interrupt) => {
                self.interrupts
                    .retain(|cached| cached.token != interrupt.token);
                self.interrupts.push(interrupt.clone());
                self.interrupts.sort_unstable_by_key(|cached| cached.token);
            }
            _ => {}
        }
    }

    pub fn observe_event(&mut self, event: &DeviceEvent) {
        if event.kind == EventKind::Navigation
            && matches!(
                event.action,
                EventAction::NavigatePrevious | EventAction::NavigateNext
            )
            && self.config.as_ref().is_some_and(|config| {
                config
                    .cards
                    .iter()
                    .any(|card| card.card_id == event.card_id)
            })
        {
            self.active_card = Some(ActivateCard {
                card_id: event.card_id.clone(),
            });
        }
        if event.kind == EventKind::InterruptDismissed
            && let Some(token) = event.interrupt_token
        {
            self.interrupts.retain(|interrupt| interrupt.token != token);
        }
    }
}

#[cfg(test)]
mod tests {
    use protocol::{CardConfig, TapAction};

    use super::*;

    fn config(revision: u32) -> ApplyConfig {
        ApplyConfig {
            brightness: None,
            revision,
            rotation: 90,
            cards: vec![CardConfig {
                card_id: "timer".into(),
                tap_action: TapAction::StartPause,
            }],
        }
    }

    fn timer_push(card_id: &str, revision: u32) -> PushTimer {
        PushTimer {
            card_id: card_id.into(),
            revision,
            total_ms: 60_000,
            remaining_ms: 30_000,
            running: false,
        }
    }

    fn interrupt(token: u32) -> TriggerInterrupt {
        TriggerInterrupt {
            card_id: "timer".into(),
            token,
            reason: format!("interrupt-{token}"),
        }
    }

    #[test]
    fn changed_config_invalidates_dependent_replay_but_identical_config_preserves_it() {
        let mut replay = ReplayState::default();
        let current = config(1);
        replay.remember_success(&Message::ApplyConfig(current.clone()));
        replay.remember_success(&Message::PushTimer(timer_push("timer", 2)));
        replay.remember_success(&Message::TriggerInterrupt(interrupt(4)));
        replay.remember_success(&Message::ActivateCard(ActivateCard {
            card_id: "timer".into(),
        }));

        replay.remember_success(&Message::ApplyConfig(current.clone()));
        assert_eq!(replay.pushes.len(), 1);
        assert_eq!(replay.interrupts.len(), 1);
        assert_eq!(replay.active_card.as_ref().unwrap().card_id, "timer");

        let changed = ApplyConfig {
            brightness: None,
            revision: 2,
            rotation: current.rotation,
            cards: vec![CardConfig {
                card_id: "calendar".into(),
                tap_action: TapAction::None,
            }],
        };
        replay.remember_success(&Message::ApplyConfig(changed));
        assert!(replay.pushes.is_empty());
        assert!(replay.interrupts.is_empty());
        assert!(replay.active_card.is_none());
    }

    #[test]
    fn timer_replay_replaces_by_card_and_sorts_by_revision() {
        let mut replay = ReplayState::default();
        replay.remember_success(&Message::PushTimer(timer_push("clock", 5)));
        replay.remember_success(&Message::PushTimer(timer_push("timer", 3)));
        replay.remember_success(&Message::PushTimer(timer_push("clock", 1)));

        assert_eq!(
            replay
                .pushes
                .iter()
                .map(|push| (push.card_id.as_str(), push.revision))
                .collect::<Vec<_>>(),
            vec![("clock", 1), ("timer", 3)]
        );
    }

    #[test]
    fn interrupt_replay_replaces_by_token_and_sorts_by_token() {
        let mut replay = ReplayState::default();
        replay.remember_success(&Message::TriggerInterrupt(interrupt(5)));
        replay.remember_success(&Message::TriggerInterrupt(interrupt(3)));
        assert_eq!(
            replay
                .interrupts
                .iter()
                .map(|cached| cached.token)
                .collect::<Vec<_>>(),
            vec![3, 5]
        );

        let mut replacement = interrupt(5);
        replacement.reason = "replacement".into();
        replay.remember_success(&Message::TriggerInterrupt(replacement));

        assert_eq!(
            replay
                .interrupts
                .iter()
                .map(|cached| (cached.token, cached.reason.as_str()))
                .collect::<Vec<_>>(),
            vec![(3, "interrupt-3"), (5, "replacement")]
        );
    }

    #[test]
    fn interrupt_dismissal_removes_only_the_matching_token() {
        let mut replay = ReplayState {
            interrupts: vec![interrupt(3), interrupt(5), interrupt(7)],
            ..ReplayState::default()
        };

        replay.observe_event(&DeviceEvent {
            view_index: None,
            sequence: 1,
            kind: EventKind::InterruptDismissed,
            card_id: "timer".into(),
            action: EventAction::DismissInterrupt,
            interrupt_token: Some(5),
        });

        assert_eq!(
            replay
                .interrupts
                .iter()
                .map(|cached| cached.token)
                .collect::<Vec<_>>(),
            vec![3, 7]
        );
    }

    #[test]
    fn navigation_to_a_card_absent_from_config_does_not_change_active_card() {
        let mut replay = ReplayState {
            config: Some(config(1)),
            active_card: Some(ActivateCard {
                card_id: "timer".into(),
            }),
            ..ReplayState::default()
        };

        replay.observe_event(&DeviceEvent {
            view_index: None,
            sequence: 1,
            kind: EventKind::Navigation,
            card_id: "absent".into(),
            action: EventAction::NavigateNext,
            interrupt_token: None,
        });

        assert_eq!(replay.active_card.unwrap().card_id, "timer");
    }
}
