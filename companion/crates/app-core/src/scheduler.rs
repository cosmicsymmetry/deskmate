use std::time::{Duration, Instant};

pub(crate) struct Scheduler {
    pomodoro_interval: Duration,
    status_interval: Duration,
    time_sync_interval: Duration,
    next_pomodoro: Instant,
    next_status: Instant,
    next_time_sync: Instant,
    rotation: Option<Instant>,
    /// Keyed to the specific interrupt token the hold belongs to. A code
    /// review of the first version of this feature found that a single
    /// unkeyed `Option<Instant>` cross-talks between the arbiter's active and
    /// pending slots: arming it for whatever was just scheduled (even into
    /// Pending) let an unrelated bounded hold auto-dismiss an
    /// `UntilDismissed` interrupt, and let an unrelated `UntilDismissed`
    /// schedule silently swallow a bounded hold's deadline. Keying to the
    /// token that earned it, and only ever arming/checking it against
    /// whichever interrupt is actually `Active`, makes those two interrupts
    /// independent regardless of arrival order.
    alert_hold: Option<(u32, Instant)>,
}

impl Scheduler {
    pub(crate) fn new(
        now: Instant,
        pomodoro_interval: Duration,
        status_interval: Duration,
        time_sync_interval: Duration,
    ) -> Self {
        Self {
            pomodoro_interval,
            status_interval,
            time_sync_interval,
            next_pomodoro: now,
            next_status: now,
            next_time_sync: now,
            rotation: None,
            alert_hold: None,
        }
    }

    pub(crate) fn pomodoro_due(&mut self, now: Instant) -> bool {
        take_deadline(&mut self.next_pomodoro, self.pomodoro_interval, now)
    }

    pub(crate) fn status_due(&mut self, now: Instant) -> bool {
        take_deadline(&mut self.next_status, self.status_interval, now)
    }

    pub(crate) fn time_sync_due(&mut self, now: Instant) -> bool {
        take_deadline(&mut self.next_time_sync, self.time_sync_interval, now)
    }

    pub(crate) fn schedule_status_now(&mut self, now: Instant) {
        self.next_status = now;
    }

    pub(crate) fn schedule_time_sync_now(&mut self, now: Instant) {
        self.next_time_sync = now;
    }

    /// Dwell is per-card, so the caller re-arms with a (possibly different)
    /// duration every time the rotation advances. `None` disarms rotation
    /// entirely, which is how `CarouselAdvance::Manual` is represented.
    pub(crate) fn set_rotation(&mut self, dwell: Option<Duration>, now: Instant) {
        self.rotation = dwell.map(|dwell| now + dwell);
    }

    pub(crate) fn clear_rotation(&mut self) {
        self.rotation = None;
    }

    pub(crate) fn rotation_due(&mut self, now: Instant) -> bool {
        let Some(deadline) = self.rotation else {
            return false;
        };
        if now < deadline {
            return false;
        }
        self.rotation = None;
        true
    }

    /// Bounded auto-dismiss deadline for the interrupt currently holding
    /// token `token`, armed by the caller when that interrupt is delivered
    /// with `AlertHold::Seconds`, or when an already-delivered interrupt is
    /// promoted from Pending to Active. `None` disarms it entirely — used for
    /// `AlertHold::UntilDismissed`, and whenever the token it was keyed to is
    /// no longer the active interrupt (see `runtime::sync_alert_hold_to_active_interrupt`).
    pub(crate) fn set_alert_hold(&mut self, hold: Option<(u32, Instant)>) {
        self.alert_hold = hold;
    }

    /// Returns the token whose hold just expired, or `None`. The caller must
    /// only dismiss that exact token (defensive: this scheduler and the
    /// arbiter's active slot are kept in sync by the caller, but the token
    /// check means a bug in that synchronization fails safe instead of
    /// dismissing the wrong interrupt).
    pub(crate) fn alert_hold_due(&mut self, now: Instant) -> Option<u32> {
        let (token, deadline) = self.alert_hold?;
        if now < deadline {
            return None;
        }
        self.alert_hold = None;
        Some(token)
    }

    pub(crate) fn wait_duration(&self, now: Instant, maximum: Duration) -> Duration {
        let next = self
            .rotation
            .iter()
            .copied()
            .chain(self.alert_hold.map(|(_, deadline)| deadline))
            .chain([self.next_pomodoro, self.next_status, self.next_time_sync])
            .min()
            .unwrap_or(now + maximum);
        next.saturating_duration_since(now).min(maximum)
    }
}

fn take_deadline(deadline: &mut Instant, interval: Duration, now: Instant) -> bool {
    if now < *deadline {
        return false;
    }
    *deadline = now + interval;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overdue_periodic_deadlines_fire_once_then_move_past_wake_time() {
        let now = Instant::now();
        let mut scheduler = Scheduler::new(
            now,
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(3),
        );
        let after_sleep = now + Duration::from_hours(8);

        assert!(scheduler.pomodoro_due(after_sleep));
        assert!(!scheduler.pomodoro_due(after_sleep));
        assert!(scheduler.status_due(after_sleep));
        assert!(!scheduler.status_due(after_sleep));
        assert!(scheduler.time_sync_due(after_sleep));
        assert!(!scheduler.time_sync_due(after_sleep));
    }

    #[test]
    fn rotation_deadline_fires_once_and_bounds_the_wait_until_the_caller_rearms() {
        let now = Instant::now();
        let mut scheduler = Scheduler::new(
            now,
            Duration::from_mins(1),
            Duration::from_mins(1),
            Duration::from_mins(1),
        );
        // `Scheduler::new` marks the periodic deadlines due immediately (so they fire
        // once on cold boot). Consume that initial firing so the assertions below
        // isolate the rotation deadline's contribution to `wait_duration`.
        assert!(scheduler.pomodoro_due(now));
        assert!(scheduler.status_due(now));
        assert!(scheduler.time_sync_due(now));

        // Manual advance: no rotation deadline exists.
        scheduler.set_rotation(None, now);
        assert!(!scheduler.rotation_due(now + Duration::from_hours(1)));

        // Timed advance: fires once at the deadline, then the caller must re-arm it.
        scheduler.set_rotation(Some(Duration::from_secs(20)), now);
        assert!(!scheduler.rotation_due(now + Duration::from_secs(19)));
        assert!(scheduler.rotation_due(now + Duration::from_secs(20)));
        assert!(!scheduler.rotation_due(now + Duration::from_secs(20)));

        // The loop cannot sleep past a pending rotation.
        scheduler.set_rotation(Some(Duration::from_secs(5)), now);
        assert_eq!(
            scheduler.wait_duration(now, Duration::from_hours(1)),
            Duration::from_secs(5)
        );

        scheduler.clear_rotation();
        assert!(!scheduler.rotation_due(now + Duration::from_hours(1)));
    }

    #[test]
    fn alert_hold_fires_once_at_its_deadline_and_bounds_the_wait() {
        let now = Instant::now();
        let mut scheduler = Scheduler::new(
            now,
            Duration::from_hours(1),
            Duration::from_hours(1),
            Duration::from_hours(1),
        );
        // `Scheduler::new` marks the periodic deadlines due immediately (so they fire
        // once on cold boot). Consume that initial firing so the `wait_duration`
        // assertions below isolate the alert hold's contribution.
        assert!(scheduler.pomodoro_due(now));
        assert!(scheduler.status_due(now));
        assert!(scheduler.time_sync_due(now));

        // No hold armed: never due, no effect on the wait.
        assert_eq!(
            scheduler.alert_hold_due(now + Duration::from_hours(1)),
            None
        );

        // A bounded hold fires exactly once at its deadline, then disarms itself
        // (unlike rotation, it does not rearm — the next interrupt arms a fresh one),
        // and reports which token expired.
        scheduler.set_alert_hold(Some((7, now + Duration::from_secs(30))));
        assert_eq!(
            scheduler.alert_hold_due(now + Duration::from_secs(29)),
            None
        );
        assert_eq!(
            scheduler.alert_hold_due(now + Duration::from_secs(30)),
            Some(7)
        );
        assert_eq!(
            scheduler.alert_hold_due(now + Duration::from_secs(30)),
            None
        );
        assert_eq!(scheduler.alert_hold_due(now + Duration::from_mins(1)), None);

        // The loop cannot sleep past a pending hold deadline.
        scheduler.set_alert_hold(Some((9, now + Duration::from_secs(5))));
        assert_eq!(
            scheduler.wait_duration(now, Duration::from_hours(1)),
            Duration::from_secs(5)
        );

        // `AlertHold::UntilDismissed` (or "this token is no longer active") is
        // represented as `None`: never due, and it does not bound the wait.
        scheduler.set_alert_hold(None);
        assert_eq!(
            scheduler.alert_hold_due(now + Duration::from_hours(1)),
            None
        );
        assert_eq!(
            scheduler.wait_duration(now, Duration::from_millis(5)),
            Duration::from_millis(5)
        );
    }
}
