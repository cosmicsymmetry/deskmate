use std::fmt;

use protocol::TriggerInterrupt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptSlot {
    Active,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedInterrupt {
    pub message: TriggerInterrupt,
    pub slot: InterruptSlot,
    pub acknowledged: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptError {
    Busy,
    TokenExhausted,
    InvalidWidgetId,
    UnknownToken,
    InvalidDismissal,
}

impl fmt::Display for InterruptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => formatter.write_str("active and pending interrupt slots are full"),
            Self::TokenExhausted => formatter.write_str("interrupt token counter is exhausted"),
            Self::InvalidWidgetId => formatter.write_str("interrupt widget ID is invalid"),
            Self::UnknownToken => formatter.write_str("unknown interrupt token"),
            Self::InvalidDismissal => {
                formatter.write_str("only the active interrupt can be dismissed")
            }
        }
    }
}

impl std::error::Error for InterruptError {}

#[derive(Debug, Default)]
pub struct InterruptArbiter {
    latest_token: u32,
    active: Option<TrackedInterrupt>,
    pending: Option<TrackedInterrupt>,
}

impl InterruptArbiter {
    pub fn with_latest_token(latest_token: u32) -> Self {
        Self {
            latest_token,
            ..Self::default()
        }
    }

    pub fn latest_token(&self) -> u32 {
        self.latest_token
    }

    pub fn advance_latest_token(&mut self, observed: u32) {
        self.latest_token = self.latest_token.max(observed);
    }

    pub fn active(&self) -> Option<&TrackedInterrupt> {
        self.active.as_ref()
    }

    pub fn pending(&self) -> Option<&TrackedInterrupt> {
        self.pending.as_ref()
    }

    pub fn schedule(
        &mut self,
        widget_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<TriggerInterrupt, InterruptError> {
        let widget_id = widget_id.into();
        if widget_id.is_empty() || widget_id.len() > protocol::MAX_WIDGET_ID_LEN {
            return Err(InterruptError::InvalidWidgetId);
        }
        let slot = if self.active.is_none() {
            InterruptSlot::Active
        } else if self.pending.is_none() {
            InterruptSlot::Pending
        } else {
            return Err(InterruptError::Busy);
        };
        let token = self
            .latest_token
            .checked_add(1)
            .ok_or(InterruptError::TokenExhausted)?;
        let message = TriggerInterrupt {
            widget_id,
            token,
            reason: truncate_utf8(&reason.into(), protocol::MAX_INTERRUPT_REASON_LEN),
        };
        let tracked = TrackedInterrupt {
            message: message.clone(),
            slot,
            acknowledged: false,
        };
        match slot {
            InterruptSlot::Active => self.active = Some(tracked),
            InterruptSlot::Pending => self.pending = Some(tracked),
        }
        self.latest_token = token;
        Ok(message)
    }

    pub fn acknowledge(&mut self, token: u32) -> Result<(), InterruptError> {
        self.find_mut(token)
            .ok_or(InterruptError::UnknownToken)?
            .acknowledged = true;
        Ok(())
    }

    pub fn mark_busy_for_retry(&mut self, token: u32) -> Result<TriggerInterrupt, InterruptError> {
        let tracked = self.find_mut(token).ok_or(InterruptError::UnknownToken)?;
        tracked.acknowledged = false;
        Ok(tracked.message.clone())
    }

    pub fn reject(&mut self, token: u32) -> Result<(), InterruptError> {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.message.token == token)
        {
            self.pending = None;
            return Ok(());
        }
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.message.token == token)
        {
            self.active = self.pending.take().map(|mut pending| {
                pending.slot = InterruptSlot::Active;
                pending
            });
            return Ok(());
        }
        Err(InterruptError::UnknownToken)
    }

    pub fn dismiss(&mut self, token: u32) -> Result<(), InterruptError> {
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
        self.active = self.pending.take().map(|mut pending| {
            pending.slot = InterruptSlot::Active;
            pending
        });
        Ok(())
    }

    pub fn replay(&self) -> Vec<TriggerInterrupt> {
        self.active
            .iter()
            .chain(self.pending.iter())
            .map(|tracked| tracked.message.clone())
            .collect()
    }

    pub fn clear_for_config_replacement(&mut self) {
        self.active = None;
        self.pending = None;
    }

    pub fn retain_widgets(&mut self, mut retain: impl FnMut(&str) -> bool) {
        if self
            .active
            .as_ref()
            .is_some_and(|active| !retain(&active.message.widget_id))
        {
            self.active = None;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| !retain(&pending.message.widget_id))
        {
            self.pending = None;
        }
        if self.active.is_none() {
            self.active = self.pending.take().map(|mut pending| {
                pending.slot = InterruptSlot::Active;
                pending
            });
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

fn truncate_utf8(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_owned();
    }
    let mut end = maximum_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::pomodoro::Pomodoro;

    use super::*;

    #[test]
    fn active_and_pending_slots_mirror_device_fifo_behavior() {
        let mut arbiter = InterruptArbiter::default();
        let active = arbiter.schedule("timer", "first").unwrap();
        let pending = arbiter.schedule("timer", "second").unwrap();
        assert_eq!(active.token, 1);
        assert_eq!(pending.token, 2);
        assert_eq!(
            arbiter.schedule("timer", "third"),
            Err(InterruptError::Busy)
        );
        assert_eq!(arbiter.latest_token(), 2);

        arbiter.acknowledge(1).unwrap();
        arbiter.acknowledge(2).unwrap();
        arbiter.dismiss(1).unwrap();
        assert_eq!(arbiter.active().unwrap().message.token, 2);
        assert_eq!(arbiter.active().unwrap().slot, InterruptSlot::Active);
        assert!(arbiter.pending().is_none());
    }

    #[test]
    fn busy_retry_and_reconnect_replay_reuse_identical_tokens() {
        let mut arbiter = InterruptArbiter::with_latest_token(40);
        let trigger = arbiter.schedule("timer", "done").unwrap();
        assert_eq!(trigger.token, 41);
        let retry = arbiter.mark_busy_for_retry(41).unwrap();
        assert_eq!(retry, trigger);
        assert_eq!(arbiter.replay(), vec![trigger]);
        assert_eq!(arbiter.latest_token(), 41);
    }

    #[test]
    fn pending_cannot_be_dismissed_before_active() {
        let mut arbiter = InterruptArbiter::default();
        arbiter.schedule("timer", "first").unwrap();
        let pending = arbiter.schedule("timer", "second").unwrap();
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
                arbiter.schedule("timer", "Timer finished").unwrap();
            }
        }
        assert_eq!(arbiter.replay().len(), 1);
        assert_eq!(arbiter.active().unwrap().message.token, 1);
    }

    #[test]
    fn config_replacement_retains_only_interrupts_for_live_widgets() {
        let mut arbiter = InterruptArbiter::default();
        arbiter.schedule("removed", "first").unwrap();
        let retained = arbiter.schedule("timer", "second").unwrap();

        arbiter.retain_widgets(|widget_id| widget_id == "timer");

        assert_eq!(arbiter.active().unwrap().message, retained);
        assert_eq!(arbiter.active().unwrap().slot, InterruptSlot::Active);
        assert!(arbiter.pending().is_none());
        assert_eq!(arbiter.latest_token(), 2);
    }

    #[test]
    fn observed_device_token_advances_but_never_rewinds_the_counter() {
        let mut arbiter = InterruptArbiter::with_latest_token(4);
        arbiter.advance_latest_token(9);
        assert_eq!(arbiter.schedule("timer", "done").unwrap().token, 10);
        arbiter.advance_latest_token(2);
        assert_eq!(arbiter.latest_token(), 10);
    }
}
