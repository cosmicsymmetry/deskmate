use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub(crate) struct Scheduler {
    pomodoro_interval: Duration,
    status_interval: Duration,
    time_sync_interval: Duration,
    next_pomodoro: Instant,
    next_status: Instant,
    next_time_sync: Instant,
    calendars: BTreeMap<String, CalendarDeadline>,
}

struct CalendarDeadline {
    interval: Duration,
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
            calendars: BTreeMap::new(),
        }
    }

    pub(crate) fn replace_calendars(
        &mut self,
        calendars: impl IntoIterator<Item = (String, Duration)>,
        now: Instant,
    ) {
        self.calendars = calendars
            .into_iter()
            .map(|(widget_id, interval)| {
                (
                    widget_id,
                    CalendarDeadline {
                        interval,
                        next: now,
                    },
                )
            })
            .collect();
    }

    pub(crate) fn schedule_calendar_now(&mut self, widget_id: &str, now: Instant) -> bool {
        let Some(deadline) = self.calendars.get_mut(widget_id) else {
            return false;
        };
        deadline.next = now;
        true
    }

    pub(crate) fn due_calendars(&self, now: Instant) -> Vec<String> {
        self.calendars
            .iter()
            .filter(|(_, deadline)| now >= deadline.next)
            .map(|(widget_id, _)| widget_id.clone())
            .collect()
    }

    pub(crate) fn calendar_started(&mut self, widget_id: &str, now: Instant) {
        if let Some(deadline) = self.calendars.get_mut(widget_id) {
            deadline.next = now + deadline.interval;
        }
    }

    pub(crate) fn calendar_retry(&mut self, widget_id: &str, now: Instant, delay: Duration) {
        if let Some(deadline) = self.calendars.get_mut(widget_id) {
            deadline.next = now + delay;
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
            .calendars
            .values()
            .map(|deadline| deadline.next)
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
        scheduler.replace_calendars(
            [
                ("work".into(), Duration::from_mins(1)),
                ("home".into(), Duration::from_mins(2)),
            ],
            now,
        );
        assert_eq!(scheduler.due_calendars(now), ["home", "work"]);
        scheduler.calendar_started("work", now);
        scheduler.calendar_started("home", now);
        assert!(scheduler.due_calendars(now).is_empty());
        assert!(scheduler.schedule_calendar_now("work", now));
        assert_eq!(scheduler.due_calendars(now), ["work"]);
    }
}
