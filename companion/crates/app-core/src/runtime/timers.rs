//! Pomodoro state, the alerts it raises, and the interrupts they become.
//!
//! Split out of `runtime.rs` unchanged on 2026-09-11. `use super::*` keeps
//! name resolution identical to when this was one file.

// The glob is what makes this file a MOVE rather than a rewrite: name
// resolution inside it is identical to when all of this lived in one
// `runtime.rs`. Enumerating thirty parent imports would make the split a
// diff nobody can read against the original.
#[allow(clippy::wildcard_imports)]
use super::*;

pub(super) fn update_pomodoros(state: &mut WorkerState, now: Instant) {
    let ids: Vec<String> = state.pomodoros.keys().cloned().collect();
    for widget_id in ids {
        let Some(timer) = state.pomodoros.get_mut(&widget_id) else {
            continue;
        };
        let update = timer.update(now);
        let changed = state.latest_fields.get(&widget_id) != Some(&update.fields);
        let completion_interrupt = record_pomodoro_update(state, &widget_id, update);
        if changed {
            state.dirty_widgets.insert(widget_id.clone());
        }
        if completion_interrupt {
            let _ = state
                .interrupts
                .schedule(widget_id.clone(), "Timer finished");
        }
    }
}

pub(super) fn control_pomodoro(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    device: &mut dyn RuntimeDevice,
    widget_id: &str,
    action: PomodoroAction,
    now: Instant,
) -> Result<(), RuntimeError> {
    let Some(timer) = state.pomodoros.get_mut(widget_id) else {
        return Err(RuntimeError::UnknownWidget {
            widget_id: widget_id.into(),
        });
    };
    let update = match action {
        PomodoroAction::Start => timer.start(now),
        PomodoroAction::Pause => timer.pause(now),
        PomodoroAction::Toggle => timer.toggle(now),
        PomodoroAction::Reset => timer.reset(now),
    };
    let completion_interrupt = record_pomodoro_update(state, widget_id, update);
    state.dirty_widgets.insert(widget_id.into());
    if completion_interrupt {
        state
            .interrupts
            .schedule(widget_id, "Timer finished")
            .map_err(|error| RuntimeError::Device {
                message: error.to_string(),
            })?;
    }
    if state.connected && !state.config.preferences.paused {
        push_dirty_widgets(state, device)?;
        flush_interrupts(state, scheduler, device, now)?;
    }
    Ok(())
}

pub(super) fn record_pomodoro_update(
    state: &mut WorkerState,
    widget_id: &str,
    update: engine::pomodoro::PomodoroUpdate,
) -> bool {
    let completion_interrupt =
        update.completion_interrupt && card_wants_completion_interrupt(&state.config, widget_id);
    state.latest_fields.insert(widget_id.into(), update.fields);
    state.pomodoro_snapshots.insert(
        widget_id.into(),
        PomodoroSnapshot {
            widget_id: widget_id.into(),
            state: pomodoro_state(update.state),
            duration_seconds: update.duration_seconds,
            remaining_seconds: update.remaining_seconds,
        },
    );
    completion_interrupt
}

/// The configured alert for a card, or `CardAlert::None` if the card is
/// unknown (e.g. it was removed from config between scheduling and firing).
pub(super) fn card_alert(config: &AppConfig, widget_id: &str) -> CardAlert {
    config
        .cards
        .iter()
        .find(|card| card.id() == widget_id)
        .map_or(CardAlert::None, CardSettings::alert)
}

/// A pomodoro's completion only becomes a host-triggered interrupt when its
/// card is configured to alert specifically on timer finish.
pub(super) fn card_wants_completion_interrupt(config: &AppConfig, widget_id: &str) -> bool {
    matches!(
        card_alert(config, widget_id),
        CardAlert::OnTimerFinish { .. }
    )
}

/// Arms the scheduler's bounded alert-hold deadline for `token`, but only if
/// `token` actually is the arbiter's *active* interrupt. Delivery of a Pending
/// interrupt does not run a deadline of its own; one is armed for it only once
/// it is promoted to Active (see `sync_alert_hold_to_active_interrupt`). This
/// guard, and keying the hold to
/// a specific token in the first place, is what fixes Task 6 review
/// Critical-1: a single unkeyed, unconditionally-armed hold let scheduling
/// *any* interrupt (including one that landed Pending) clobber the deadline
/// belonging to a *different*, already-Active interrupt — auto-dismissing an
/// `UntilDismissed` alert early, or silently losing a bounded alert's
/// deadline entirely.
///
/// `AlertHold::Seconds` arms an absolute deadline from successful delivery;
/// `AlertHold::UntilDismissed` (or no hold at all) disarms it.
///
/// BY DESIGN (decided 2026-08-06, design spec §3.2): expiring this deadline
/// only frees the host's arbiter slot for the next alert and re-syncs the
/// saved carousel screen id host-side (see the `alert_hold_due` handling in
/// `run_scheduled_work`). It does not clear the panel. Per
/// `docs/protocol/v1.md`'s `ActivateScreen` section and firmware's
/// `protocol_task.c` (`show_carousel_screen` is skipped whenever an interrupt
/// is active), the device yields the overlay on tap alone. Confirmed on
/// hardware — see `docs/hardware/board-notes.md`, card-model §6.
///
/// The consequence to keep in mind: dismissing host-side while the device
/// still shows the overlay leaves host and device interrupt state diverged
/// (the device still expects a tap; a further host-triggered alert can be
/// rejected `Busy` even though the host arbiter believes its slot is free)
/// until the next tap or a full resync. Do not "fix" this by adding a wire
/// dismissal message — that was considered and rejected for v1, because an
/// alert worth interrupting the user is worth acknowledging.
pub(super) fn arm_alert_hold(
    scheduler: &mut Scheduler,
    token: u32,
    hold: Option<AlertHold>,
    now: Instant,
) {
    let deadline = match hold {
        Some(AlertHold::Seconds { value }) => Some(now + Duration::from_secs(u64::from(value))),
        Some(AlertHold::UntilDismissed) | None => None,
    };
    scheduler.set_alert_hold(deadline.map(|deadline| (token, deadline)));
}

/// See `arm_alert_hold`'s doc comment for the Active-only invariant this
/// enforces.
pub(super) fn arm_alert_hold_if_active(
    interrupts: &InterruptArbiter,
    scheduler: &mut Scheduler,
    token: u32,
    hold: Option<AlertHold>,
    now: Instant,
) {
    if interrupts
        .active()
        .is_some_and(|active| active.message.token == token)
    {
        arm_alert_hold(scheduler, token, hold, now);
    }
}

/// Recomputes the scheduler's alert-hold deadline from scratch against
/// whichever interrupt is currently Active. An already-acknowledged promoted
/// interrupt was delivered while Pending, so its own configured hold starts
/// fresh from `now`; an unacknowledged promoted interrupt has no deadline until
/// `flush_interrupts` delivers it. Callers only invoke this when the active
/// interrupt has actually just changed — after a dismissal promotes (or fails
/// to promote) a Pending interrupt, or after config replacement prunes the
/// previously-active interrupt's widget — never on every call, since
/// recomputing unconditionally would reset an unrelated, still-active
/// interrupt's in-flight countdown on every unrelated event.
pub(super) fn sync_alert_hold_to_active_interrupt(
    state: &WorkerState,
    scheduler: &mut Scheduler,
    now: Instant,
) {
    match state.interrupts.active() {
        Some(active) if active.acknowledged => {
            let hold = card_alert(&state.config, &active.message.widget_id).hold();
            arm_alert_hold(scheduler, active.message.token, hold, now);
        }
        Some(_) | None => scheduler.set_alert_hold(None),
    }
}

pub(super) fn flush_interrupts(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    device: &mut dyn RuntimeDevice,
    now: Instant,
) -> Result<(), RuntimeError> {
    if ownership_was_refused(state) {
        return Ok(());
    }
    loop {
        let pending = state
            .interrupts
            .active()
            .filter(|tracked| !tracked.acknowledged)
            .or_else(|| {
                state
                    .interrupts
                    .pending()
                    .filter(|tracked| !tracked.acknowledged)
            })
            .map(|tracked| tracked.message.clone());
        let Some(interrupt) = pending else {
            return Ok(());
        };
        match device.trigger_interrupt(interrupt.clone()) {
            Ok(()) => {
                state
                    .interrupts
                    .acknowledge(interrupt.token)
                    .map_err(|error| RuntimeError::Device {
                        message: error.to_string(),
                    })?;
                arm_alert_hold_if_active(
                    &state.interrupts,
                    scheduler,
                    interrupt.token,
                    card_alert(&state.config, &interrupt.widget_id).hold(),
                    now,
                );
            }
            Err(error) if is_wrong_tier(&error) => {
                mark_ownership_refused(state);
                return Ok(());
            }
            Err(DeviceError::Rejected(error)) if error.code == protocol::ErrorCode::Busy => {
                state
                    .interrupts
                    .mark_busy_for_retry(interrupt.token)
                    .map_err(|error| RuntimeError::Device {
                        message: error.to_string(),
                    })?;
                return Ok(());
            }
            Err(error) => return Err(device_runtime_error(&error)),
        }
    }
}

pub(super) fn pomodoro_state(state: EnginePomodoroState) -> PomodoroState {
    match state {
        EnginePomodoroState::Idle => PomodoroState::Idle,
        EnginePomodoroState::Running => PomodoroState::Running,
        EnginePomodoroState::Paused => PomodoroState::Paused,
        EnginePomodoroState::Completed => PomodoroState::Completed,
    }
}
