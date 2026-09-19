use std::fmt;

use protocol::TriggerInterrupt;

const TIMER_FINISHED_REASON: &str = "Timer finished";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrackedInterrupt {
    pub(crate) message: TriggerInterrupt,
    pub(crate) acknowledged: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InterruptError {
    Busy,
    TokenExhausted,
    UnknownToken,
    InvalidDismissal,
}

impl fmt::Display for InterruptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => formatter.write_str("active and pending interrupt slots are full"),
            Self::TokenExhausted => formatter.write_str("interrupt token counter is exhausted"),
            Self::UnknownToken => formatter.write_str("unknown interrupt token"),
            Self::InvalidDismissal => {
                formatter.write_str("only the active interrupt can be dismissed")
            }
        }
    }
}

impl std::error::Error for InterruptError {}

#[derive(Debug, Default)]
pub(crate) struct InterruptArbiter {
    latest_token: u32,
    active: Option<TrackedInterrupt>,
    pending: Option<TrackedInterrupt>,
}

impl InterruptArbiter {
    pub(crate) fn advance_latest_token(&mut self, observed: u32) {
        self.latest_token = self.latest_token.max(observed);
    }

    pub(crate) fn active(&self) -> Option<&TrackedInterrupt> {
        self.active.as_ref()
    }

    pub(crate) fn pending(&self) -> Option<&TrackedInterrupt> {
        self.pending.as_ref()
    }

    pub(crate) fn schedule_timer_finished(
        &mut self,
        card_id: impl Into<String>,
    ) -> Result<TriggerInterrupt, InterruptError> {
        let card_id = card_id.into();
        if self.active.is_some() && self.pending.is_some() {
            return Err(InterruptError::Busy);
        }
        let token = self
            .latest_token
            .checked_add(1)
            .ok_or(InterruptError::TokenExhausted)?;
        let message = TriggerInterrupt {
            card_id,
            token,
            reason: TIMER_FINISHED_REASON.to_owned(),
        };
        let tracked = TrackedInterrupt {
            message: message.clone(),
            acknowledged: false,
        };
        if self.active.is_none() {
            self.active = Some(tracked);
        } else {
            self.pending = Some(tracked);
        }
        self.latest_token = token;
        Ok(message)
    }

    pub(crate) fn acknowledge(&mut self, token: u32) -> Result<(), InterruptError> {
        self.find_mut(token)
            .ok_or(InterruptError::UnknownToken)?
            .acknowledged = true;
        Ok(())
    }

    pub(crate) fn dismiss(&mut self, token: u32) -> Result<(), InterruptError> {
        let Some(active) = &self.active else {
            return Err(InterruptError::UnknownToken);
        };
        if active.message.token != token {
            return if self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.message.token == token)
            {
                Err(InterruptError::InvalidDismissal)
            } else {
                Err(InterruptError::UnknownToken)
            };
        }
        self.active = self.pending.take();
        Ok(())
    }

    pub(crate) fn retain_widgets(&mut self, mut retain: impl FnMut(&str) -> bool) {
        if self
            .active
            .as_ref()
            .is_some_and(|active| !retain(&active.message.card_id))
        {
            self.active = None;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| !retain(&pending.message.card_id))
        {
            self.pending = None;
        }
        if self.active.is_none() {
            self.active = self.pending.take();
        }
    }

    fn find_mut(&mut self, token: u32) -> Option<&mut TrackedInterrupt> {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.message.token == token)
        {
            return self.active.as_mut();
        }
        self.pending
            .as_mut()
            .filter(|pending| pending.message.token == token)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::super::pomodoro::Pomodoro;

    use super::*;

    #[test]
    fn active_and_pending_slots_mirror_device_fifo_behavior() {
        let mut arbiter = InterruptArbiter::default();
        let active = arbiter.schedule_timer_finished("timer").unwrap();
        let pending = arbiter.schedule_timer_finished("timer").unwrap();
        assert_eq!(active.token, 1);
        assert_eq!(pending.token, 2);
        assert_eq!(
            arbiter.schedule_timer_finished("timer"),
            Err(InterruptError::Busy)
        );

        arbiter.acknowledge(1).unwrap();
        arbiter.acknowledge(2).unwrap();
        arbiter.dismiss(1).unwrap();
        assert_eq!(arbiter.active().unwrap().message.token, 2);
        assert!(arbiter.active().unwrap().acknowledged);
        assert!(arbiter.pending().is_none());
    }

    #[test]
    fn a_scheduled_interrupt_stays_unacknowledged_with_its_token_until_acknowledged() {
        let mut arbiter = InterruptArbiter::default();
        arbiter.advance_latest_token(40);
        let trigger = arbiter.schedule_timer_finished("timer").unwrap();
        assert_eq!(trigger.token, 41);
        assert_eq!(arbiter.active().unwrap().message, trigger);
        assert!(!arbiter.active().unwrap().acknowledged);
    }

    #[test]
    fn pending_cannot_be_dismissed_before_active() {
        let mut arbiter = InterruptArbiter::default();
        arbiter.schedule_timer_finished("timer").unwrap();
        let pending = arbiter.schedule_timer_finished("timer").unwrap();
        assert_eq!(
            arbiter.dismiss(pending.token),
            Err(InterruptError::InvalidDismissal)
        );
    }

    #[test]
    fn pomodoro_completion_schedules_exactly_one_interrupt() {
        let base = Instant::now();
        let mut timer = Pomodoro::new("Focus", 1).unwrap();
        let mut arbiter = InterruptArbiter::default();
        timer.start(base);
        for now in [
            base + Duration::from_secs(1),
            base + Duration::from_secs(2),
            base + Duration::from_secs(3),
        ] {
            if timer.update(now).completion_interrupt {
                arbiter.schedule_timer_finished("timer").unwrap();
            }
        }
        assert_eq!(arbiter.active().unwrap().message.token, 1);
        assert_eq!(arbiter.active().unwrap().message.reason, "Timer finished");
        assert!(arbiter.pending().is_none());
    }

    #[test]
    fn config_replacement_retains_only_interrupts_for_live_widgets() {
        let mut arbiter = InterruptArbiter::default();
        arbiter.schedule_timer_finished("removed").unwrap();
        let retained = arbiter.schedule_timer_finished("timer").unwrap();

        arbiter.retain_widgets(|card_id| card_id == "timer");

        assert_eq!(arbiter.active().unwrap().message, retained);
        assert!(arbiter.pending().is_none());

        arbiter.schedule_timer_finished("removed").unwrap();
        arbiter.retain_widgets(|card_id| card_id == "timer");
        assert_eq!(arbiter.active().unwrap().message, retained);
        assert!(arbiter.pending().is_none());
    }

    #[test]
    fn observed_device_token_advances_but_never_rewinds_the_counter() {
        let mut arbiter = InterruptArbiter::default();
        arbiter.advance_latest_token(4);
        arbiter.advance_latest_token(9);
        assert_eq!(arbiter.schedule_timer_finished("timer").unwrap().token, 10);
        arbiter.advance_latest_token(2);
        assert_eq!(arbiter.schedule_timer_finished("timer").unwrap().token, 11);
    }
}
