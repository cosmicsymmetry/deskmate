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
}

struct ProviderDeadline {
    interval: Option<Duration>,
    next: Option<Instant>,
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

    pub(crate) fn wait_duration(&self, now: Instant, maximum: Duration) -> Duration {
        let next = self
            .providers
            .values()
            .filter_map(|deadline| deadline.next)
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
}
