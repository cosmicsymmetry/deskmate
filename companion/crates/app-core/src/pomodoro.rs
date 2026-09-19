use std::fmt;
use std::time::{Duration, Instant};

use crate::config::{MAX_POMODORO_SECONDS, MIN_POMODORO_SECONDS};
use crate::state::PomodoroState;
use protocol::truncate_utf8_to_bytes;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PomodoroUpdate {
    pub(crate) state: PomodoroState,
    pub(crate) duration_seconds: u32,
    pub(crate) remaining_seconds: u32,
    pub(crate) completion_interrupt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PomodoroError {
    InvalidDuration,
}

impl fmt::Display for PomodoroError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDuration => write!(
                formatter,
                "pomodoro duration must be {MIN_POMODORO_SECONDS}..={MAX_POMODORO_SECONDS} seconds"
            ),
        }
    }
}

impl std::error::Error for PomodoroError {}

pub(crate) struct Pomodoro {
    label: String,
    duration: Duration,
    state: PomodoroState,
    accumulated: Duration,
    running_since: Option<Instant>,
    completion_emitted: bool,
}

impl Pomodoro {
    pub(crate) fn new(
        label: impl Into<String>,
        duration_seconds: u32,
    ) -> Result<Self, PomodoroError> {
        if !(MIN_POMODORO_SECONDS..=MAX_POMODORO_SECONDS).contains(&duration_seconds) {
            return Err(PomodoroError::InvalidDuration);
        }
        Ok(Self {
            label: {
                let label = label.into();
                truncate_utf8_to_bytes(&label, 64).to_owned()
            },
            duration: Duration::from_secs(u64::from(duration_seconds)),
            state: PomodoroState::Idle,
            accumulated: Duration::ZERO,
            running_since: None,
            completion_emitted: false,
        })
    }

    /// The truncated label this timer was created with. A host builds the
    /// card's face from it; protocol v2's device never sees it.
    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    pub(crate) fn matches_settings(&self, label: &str, duration_seconds: u32) -> bool {
        self.label == truncate_utf8_to_bytes(label, 64)
            && self.duration == Duration::from_secs(u64::from(duration_seconds))
    }

    pub(crate) fn start(&mut self, now: Instant) -> PomodoroUpdate {
        if matches!(self.state, PomodoroState::Idle | PomodoroState::Paused) {
            self.state = PomodoroState::Running;
            self.running_since = Some(now);
        }
        self.update(now)
    }

    pub(crate) fn pause(&mut self, now: Instant) -> PomodoroUpdate {
        self.settle(now);
        if self.state == PomodoroState::Running {
            self.accumulated = self.elapsed(now);
            self.running_since = None;
            self.state = PomodoroState::Paused;
        }
        self.update(now)
    }

    pub(crate) fn toggle(&mut self, now: Instant) -> PomodoroUpdate {
        match self.state {
            PomodoroState::Idle | PomodoroState::Paused => self.start(now),
            PomodoroState::Running => self.pause(now),
            PomodoroState::Completed => self.update(now),
        }
    }

    pub(crate) fn reset(&mut self, now: Instant) -> PomodoroUpdate {
        self.state = PomodoroState::Idle;
        self.accumulated = Duration::ZERO;
        self.running_since = None;
        self.completion_emitted = false;
        self.update(now)
    }

    pub(crate) fn update(&mut self, now: Instant) -> PomodoroUpdate {
        self.settle(now);
        let elapsed = self.elapsed(now).min(self.duration);
        let remaining = self.duration.saturating_sub(elapsed);
        let remaining_seconds = ceil_seconds(remaining);
        let completion_interrupt =
            if self.state == PomodoroState::Completed && !self.completion_emitted {
                self.completion_emitted = true;
                true
            } else {
                false
            };
        let duration_seconds = u32::try_from(self.duration.as_secs()).unwrap_or(u32::MAX);
        PomodoroUpdate {
            state: self.state,
            duration_seconds,
            remaining_seconds,
            completion_interrupt,
        }
    }

    fn settle(&mut self, now: Instant) {
        if self.state == PomodoroState::Running && self.elapsed(now) >= self.duration {
            self.accumulated = self.duration;
            self.running_since = None;
            self.state = PomodoroState::Completed;
        }
    }

    fn elapsed(&self, now: Instant) -> Duration {
        let current_run = self.running_since.map_or(Duration::ZERO, |started| {
            now.saturating_duration_since(started)
        });
        self.accumulated.saturating_add(current_run)
    }
}

fn ceil_seconds(duration: Duration) -> u32 {
    let seconds = duration.as_secs();
    let rounded = if duration.subsec_nanos() == 0 {
        seconds
    } else {
        seconds.saturating_add(1)
    };
    u32::try_from(rounded).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_pause_resume_and_reset_use_only_monotonic_elapsed_time() {
        let base = Instant::now();
        let mut timer = Pomodoro::new("Focus", 10).unwrap();
        let started = timer.start(base);
        assert_eq!(started.state, PomodoroState::Running);
        assert_eq!(started.remaining_seconds, 10);

        let paused = timer.pause(base + Duration::from_millis(3_250));
        assert_eq!(paused.state, PomodoroState::Paused);
        assert_eq!(paused.remaining_seconds, 7);
        let still_paused = timer.update(base + Duration::from_secs(30));
        assert_eq!(still_paused.remaining_seconds, 7);

        timer.start(base + Duration::from_secs(30));
        let resumed = timer.update(base + Duration::from_millis(36_749));
        assert_eq!(resumed.state, PomodoroState::Running);
        assert_eq!(resumed.remaining_seconds, 1);
        let reset = timer.reset(base + Duration::from_secs(40));
        assert_eq!(reset.state, PomodoroState::Idle);
        assert_eq!(reset.remaining_seconds, 10);
    }

    #[test]
    fn completion_interrupt_is_emitted_exactly_once_until_reset() {
        let base = Instant::now();
        let mut timer = Pomodoro::new("Focus", 2).unwrap();
        timer.start(base);
        let completed = timer.update(base + Duration::from_secs(2));
        assert_eq!(completed.state, PomodoroState::Completed);
        assert_eq!(completed.remaining_seconds, 0);
        assert!(completed.completion_interrupt);
        assert!(
            !timer
                .update(base + Duration::from_secs(3))
                .completion_interrupt
        );
        assert!(
            !timer
                .toggle(base + Duration::from_secs(4))
                .completion_interrupt
        );

        timer.reset(base + Duration::from_secs(5));
        timer.start(base + Duration::from_secs(5));
        assert!(
            timer
                .update(base + Duration::from_secs(7))
                .completion_interrupt
        );
    }

    /// An update carries everything a face and a `PushTimer` are built from.
    /// Protocol v1 had this method emit a wire field bag; v2 states the timer
    /// itself, and the label stays on the timer rather than being copied into
    /// every tick.
    #[test]
    fn output_is_a_complete_timer_snapshot() {
        let mut timer = Pomodoro::new("Focus", 25 * 60).unwrap();
        let update = timer.update(Instant::now());
        assert_eq!(timer.label(), "Focus");
        assert_eq!(update.duration_seconds, 25 * 60);
        assert_eq!(update.remaining_seconds, 25 * 60);
        assert_eq!(update.state, PomodoroState::Idle);
        assert!(!update.completion_interrupt);
    }

    #[test]
    fn duration_is_bounded_by_the_config_limits() {
        assert!(Pomodoro::new("bad", 0).is_err());
        assert!(Pomodoro::new("bad", MAX_POMODORO_SECONDS + 1).is_err());
    }

    #[test]
    fn settings_identity_distinguishes_state_preserving_reconfiguration() {
        let timer = Pomodoro::new("Focus", 60).unwrap();
        assert!(timer.matches_settings("Focus", 60));
        assert!(!timer.matches_settings("Break", 60));
        assert!(!timer.matches_settings("Focus", 61));
    }
}
