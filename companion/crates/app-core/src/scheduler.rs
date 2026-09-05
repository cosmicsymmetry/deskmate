use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// A raster is a full-frame transfer: about five RLE chunks for a curated face and
/// about 161 raw chunks over the protocol's one-outstanding-request link. Spec §3
/// also rules any sub-30-second raster cadence dishonest for live faces (those are
/// refused instead of frozen), so static invalidations are coalesced at this floor.
pub(crate) const RASTER_MIN_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) struct Scheduler {
    pomodoro_interval: Duration,
    status_interval: Duration,
    time_sync_interval: Duration,
    next_pomodoro: Instant,
    next_status: Instant,
    next_time_sync: Instant,
    providers: BTreeMap<String, ProviderDeadline>,
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
    /// Per-card wake time for re-evaluating a `CardAlert::BeforeEvent`
    /// trigger even when no fresh provider data has landed. See
    /// `runtime::refresh_calendar_alert`: relying solely on provider refresh
    /// landings to observe the lead window missed most windows in practice
    /// (a 15-minute refresh interval trivially skips past a 5-minute lead
    /// window entirely).
    event_alert_checks: BTreeMap<String, Instant>,
    last_raster_push: Option<Instant>,
    raster_deadline: Option<Instant>,
}

struct ProviderDeadline {
    interval: Option<Duration>,
    next: Option<Instant>,
    skipped_while_paused: bool,
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
            last_raster_push: None,
            raster_deadline: None,
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
                        skipped_while_paused: false,
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

    /// Returns due provider jobs and advances each deadline before the caller
    /// decides whether that job can be submitted. A long-running in-flight job
    /// must not leave its next deadline in the past and pin the runtime loop.
    pub(crate) fn take_due_providers(&mut self, now: Instant) -> Vec<String> {
        let mut due = Vec::new();
        for (widget_id, deadline) in &mut self.providers {
            if deadline.next.is_some_and(|next| now >= next) {
                deadline.next = deadline.interval.map(|interval| now + interval);
                deadline.skipped_while_paused = false;
                due.push(widget_id.clone());
            }
        }
        due
    }

    /// Advances provider deadlines whose work is gated by pause, remembering
    /// only those skipped jobs so resume can run them promptly.
    pub(crate) fn skip_due_providers(&mut self, now: Instant) {
        for deadline in self.providers.values_mut() {
            if deadline.next.is_some_and(|next| now >= next) {
                deadline.next = deadline.interval.map(|interval| now + interval);
                deadline.skipped_while_paused = true;
            }
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

    pub(crate) fn schedule_status_now(&mut self, now: Instant) {
        self.next_status = now;
    }

    pub(crate) fn schedule_time_sync_now(&mut self, now: Instant) {
        self.next_time_sync = now;
    }

    /// Re-arms only work that actually became due while paused; manual providers
    /// that were never requested remain disarmed.
    pub(crate) fn schedule_skipped_providers_now(&mut self, now: Instant) {
        for deadline in self.providers.values_mut() {
            if deadline.skipped_while_paused {
                deadline.next = Some(now);
            }
        }
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

    /// Marks a static raster candidate dirty. The first activation is eligible
    /// immediately; after a successful push, every invalidation shares the one
    /// `last_push + floor` deadline, so newer snapshots replace pending work
    /// without extending it or queueing frames.
    pub(crate) fn invalidate_raster(&mut self, now: Instant) {
        let deadline = self
            .last_raster_push
            .map_or(now, |last| last + RASTER_MIN_INTERVAL);
        self.raster_deadline = Some(deadline);
    }

    pub(crate) fn raster_due(&mut self, now: Instant) -> bool {
        let Some(deadline) = self.raster_deadline else {
            return self.last_raster_push.is_none();
        };
        if now < deadline {
            return false;
        }
        self.raster_deadline = None;
        true
    }

    pub(crate) fn note_raster_pushed(&mut self, now: Instant) {
        self.last_raster_push = Some(now);
        self.raster_deadline = None;
    }

    /// Drops only the pending wake when the active candidate changes to a
    /// native/refused card. The last successful raster time remains the device's
    /// cadence floor if a raster card becomes active again.
    pub(crate) fn clear_raster_invalidation(&mut self) {
        self.raster_deadline = None;
    }

    pub(crate) fn wait_duration(&self, now: Instant, maximum: Duration) -> Duration {
        let next = self
            .providers
            .values()
            .filter_map(|deadline| deadline.next)
            .chain(self.rotation.iter().copied())
            .chain(self.alert_hold.map(|(_, deadline)| deadline))
            .chain(self.event_alert_checks.values().copied())
            .chain(self.raster_deadline)
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
        assert_eq!(
            scheduler.take_due_providers(now),
            ["home", "manual", "work"]
        );
        assert!(scheduler.take_due_providers(now).is_empty());
        assert!(scheduler.schedule_provider_now("manual", now));
        assert_eq!(scheduler.take_due_providers(now), ["manual"]);
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
    fn raster_floor_matrix_is_immediate_then_coalesces_until_exactly_thirty_seconds() {
        let start = Instant::now();
        let mut scheduler = Scheduler::new(
            start,
            Duration::from_hours(1),
            Duration::from_hours(1),
            Duration::from_hours(1),
        );

        assert!(
            scheduler.raster_due(start),
            "t=0 initial activation is immediate"
        );
        scheduler.note_raster_pushed(start);
        scheduler.invalidate_raster(start + Duration::from_secs(5));
        scheduler.invalidate_raster(start + Duration::from_millis(29_999));

        assert!(!scheduler.raster_due(start + Duration::from_secs(5)));
        assert!(!scheduler.raster_due(start + Duration::from_millis(29_999)));
        assert!(scheduler.raster_due(start + Duration::from_secs(30)));
        assert!(!scheduler.raster_due(start + Duration::from_secs(30)));
    }

    #[test]
    fn raster_floor_raises_five_seconds_but_does_not_shorten_two_minutes() {
        let start = Instant::now();
        let mut scheduler = Scheduler::new(
            start,
            Duration::from_hours(1),
            Duration::from_hours(1),
            Duration::from_hours(1),
        );
        scheduler.note_raster_pushed(start);

        scheduler.invalidate_raster(start + Duration::from_secs(5));
        assert!(!scheduler.raster_due(start + Duration::from_secs(5)));
        assert!(scheduler.raster_due(start + Duration::from_secs(30)));

        scheduler.note_raster_pushed(start);
        scheduler.invalidate_raster(start + Duration::from_mins(2));
        assert!(
            scheduler.raster_due(start + Duration::from_mins(2)),
            "a two-minute provider event stays eligible at two minutes"
        );
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
