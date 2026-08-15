use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub(crate) struct Scheduler {
    pomodoro_interval: Duration,
    status_interval: Duration,
    time_sync_interval: Duration,
    next_pomodoro: Instant,
    next_status: Instant,
    next_time_sync: Instant,
    providers: BTreeMap<String, ProviderDeadline>,
    rotation: Option<RotationDeadline>,
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
    /// Per-card wake time for re-evaluating a `CardAlert::BeforeEvent`
    /// trigger even when no fresh provider data has landed. See
    /// `runtime::refresh_calendar_alert`: relying solely on provider refresh
    /// landings to observe the lead window missed most windows in practice
    /// (a 15-minute refresh interval trivially skips past a 5-minute lead
    /// window entirely).
    event_alert_checks: BTreeMap<String, Instant>,
}

struct ProviderDeadline {
    interval: Option<Duration>,
    next: Option<Instant>,
}

struct RotationDeadline {
    dwell: Duration,
    next: Instant,
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
            providers: BTreeMap::new(),
            rotation: None,
            alert_hold: None,
            event_alert_checks: BTreeMap::new(),
        }
    }

    pub(crate) fn replace_providers(
        &mut self,
        providers: impl IntoIterator<Item = (String, Option<Duration>)>,
        now: Instant,
    ) {
        self.providers = providers
            .into_iter()
            .map(|(widget_id, interval)| {
                (
                    widget_id,
                    ProviderDeadline {
                        interval,
                        next: Some(now),
                    },
                )
            })
            .collect();
    }

    pub(crate) fn schedule_provider_now(&mut self, widget_id: &str, now: Instant) -> bool {
        let Some(deadline) = self.providers.get_mut(widget_id) else {
            return false;
        };
        deadline.next = Some(now);
        true
    }

    pub(crate) fn due_providers(&self, now: Instant) -> Vec<String> {
        self.providers
            .iter()
            .filter(|(_, deadline)| deadline.next.is_some_and(|next| now >= next))
            .map(|(widget_id, _)| widget_id.clone())
            .collect()
    }

    pub(crate) fn provider_started(&mut self, widget_id: &str, now: Instant) {
        if let Some(deadline) = self.providers.get_mut(widget_id) {
            deadline.next = deadline.interval.map(|interval| now + interval);
        }
    }

    pub(crate) fn provider_retry(&mut self, widget_id: &str, now: Instant, delay: Duration) {
        if let Some(deadline) = self.providers.get_mut(widget_id) {
            deadline.next = Some(now + delay);
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

    /// Dwell is per-card, so the caller re-arms with a (possibly different)
    /// duration every time the rotation advances. `None` disarms rotation
    /// entirely, which is how `CarouselAdvance::Manual` is represented.
    pub(crate) fn set_rotation(&mut self, dwell: Option<Duration>, now: Instant) {
        self.rotation = dwell.map(|dwell| RotationDeadline {
            dwell,
            next: now + dwell,
        });
    }

    pub(crate) fn clear_rotation(&mut self) {
        self.rotation = None;
    }

    pub(crate) fn rotation_due(&mut self, now: Instant) -> bool {
        let Some(rotation) = self.rotation.as_mut() else {
            return false;
        };
        if now < rotation.next {
            return false;
        }
        rotation.next = now + rotation.dwell;
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

    /// Per-card wake time for re-evaluating a `CardAlert::BeforeEvent`
    /// trigger. `None` clears it (nothing left to wait for: no known future
    /// event, or the window is already open/past and was handled
    /// synchronously by the same call that set this).
    pub(crate) fn set_event_alert_deadline(&mut self, card_id: &str, deadline: Option<Instant>) {
        match deadline {
            Some(deadline) => {
                self.event_alert_checks.insert(card_id.to_owned(), deadline);
            }
            None => {
                self.event_alert_checks.remove(card_id);
            }
        }
    }

    /// Card IDs whose event-alert deadline has passed, removing them (the
    /// caller re-establishes a deadline, if still relevant, via
    /// `set_event_alert_deadline` after re-evaluating).
    pub(crate) fn due_event_alert_cards(&mut self, now: Instant) -> Vec<String> {
        let due: Vec<String> = self
            .event_alert_checks
            .iter()
            .filter(|(_, deadline)| now >= **deadline)
            .map(|(card_id, _)| card_id.clone())
            .collect();
        for card_id in &due {
            self.event_alert_checks.remove(card_id);
        }
        due
    }

    /// Drops event-alert deadlines for cards no longer live, mirroring
    /// `InterruptArbiter::retain_widgets`.
    pub(crate) fn retain_event_alert_checks(&mut self, mut retain: impl FnMut(&str) -> bool) {
        self.event_alert_checks.retain(|card_id, _| retain(card_id));
    }

    pub(crate) fn wait_duration(&self, now: Instant, maximum: Duration) -> Duration {
        let next = self
            .providers
            .values()
            .filter_map(|deadline| deadline.next)
            .chain(self.rotation.iter().map(|rotation| rotation.next))
            .chain(self.alert_hold.map(|(_, deadline)| deadline))
            .chain(self.event_alert_checks.values().copied())
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
    fn calendar_deadlines_are_replaced_and_manual_refresh_moves_only_one() {
        let now = Instant::now();
        let mut scheduler = Scheduler::new(
            now,
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(3),
        );
        scheduler.replace_providers(
            [
                ("work".into(), Some(Duration::from_mins(1))),
                ("home".into(), Some(Duration::from_mins(2))),
                ("manual".into(), None),
            ],
            now,
        );
        assert_eq!(scheduler.due_providers(now), ["home", "manual", "work"]);
        scheduler.provider_started("work", now);
        scheduler.provider_started("home", now);
        scheduler.provider_started("manual", now);
        assert!(scheduler.due_providers(now).is_empty());
        assert!(scheduler.schedule_provider_now("manual", now));
        assert_eq!(scheduler.due_providers(now), ["manual"]);
    }

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
    fn rotation_deadline_fires_once_per_dwell_and_bounds_the_wait() {
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

        // Timed advance: fires once at the deadline, then rearms.
        scheduler.set_rotation(Some(Duration::from_secs(20)), now);
        assert!(!scheduler.rotation_due(now + Duration::from_secs(19)));
        assert!(scheduler.rotation_due(now + Duration::from_secs(20)));
        assert!(!scheduler.rotation_due(now + Duration::from_secs(20)));
        assert!(scheduler.rotation_due(now + Duration::from_secs(40)));

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

    #[test]
    fn event_alert_deadlines_fire_once_bound_the_wait_and_are_pruned_with_their_card() {
        let now = Instant::now();
        let mut scheduler = Scheduler::new(
            now,
            Duration::from_hours(1),
            Duration::from_hours(1),
            Duration::from_hours(1),
        );
        assert!(scheduler.pomodoro_due(now));
        assert!(scheduler.status_due(now));
        assert!(scheduler.time_sync_due(now));

        // No deadlines: nothing due, no effect on the wait.
        assert!(
            scheduler
                .due_event_alert_cards(now + Duration::from_hours(1))
                .is_empty()
        );

        scheduler.set_event_alert_deadline("upnext", Some(now + Duration::from_mins(3)));
        scheduler.set_event_alert_deadline("later", Some(now + Duration::from_mins(10)));
        assert!(
            scheduler
                .due_event_alert_cards(now + Duration::from_mins(2))
                .is_empty()
        );
        assert_eq!(
            scheduler.due_event_alert_cards(now + Duration::from_mins(3)),
            ["upnext"]
        );
        // Consumed: "upnext" does not fire again on a later check, before
        // "later"'s own (still-armed, separate) deadline arrives.
        assert!(
            scheduler
                .due_event_alert_cards(now + Duration::from_mins(4))
                .is_empty()
        );

        // The loop cannot sleep past the surviving "later" deadline.
        assert_eq!(
            scheduler.wait_duration(now, Duration::from_hours(1)),
            Duration::from_mins(10)
        );

        // A card no longer in the live config is pruned, even with a deadline
        // still armed.
        scheduler.set_event_alert_deadline("later", Some(now + Duration::from_mins(10)));
        scheduler.retain_event_alert_checks(|card_id| card_id != "later");
        assert_eq!(
            scheduler.wait_duration(now, Duration::from_hours(1)),
            Duration::from_hours(1)
        );
    }
}
