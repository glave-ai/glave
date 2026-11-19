//! A schedule as lists show it, and its times: when a run is due, and when it runs next.

use glave_event::{Ask, At, Outcome, Skipped, When};
use glave_ports::{Located, title_from};
use jiff::Timestamp;

use super::card::shown;
use crate::cron::Cron;
use crate::ledger::{Ledger, Recorded, Scheduled};
use crate::protocol::{LastRun, RunStatus, ScheduleInfo, Told};
use crate::words::Sentence;
use crate::zone::time_zone;

/// Occurrences of a cron expression walked to find the latest one missed.
const MOST_MISSED: usize = 100_000;

/// The time a run of `scheduled` is due by `now`, if one is: for a cron schedule that missed
/// several, the latest of them. Never for one with no chat of its own, which runs no more.
pub(super) fn due(scheduled: &Scheduled, now: Timestamp) -> Option<Timestamp> {
    let next = next_fire(scheduled)?;
    if next > now {
        return None;
    }
    let When::Cron(text) = &scheduled.created.when else {
        return Some(next);
    };
    let cron = Cron::parse(text).ok()?;
    let zone = time_zone(&scheduled.created.zone).ok()?;
    let mut at = next;
    for _ in 0..MOST_MISSED {
        match cron.next_after(at, &zone) {
            Some(later) if later <= now => at = later,
            _ => return Some(at),
        }
    }
    Some(now)
}

/// The first time after the runs counted so far that `scheduled` fires, overdue or not: `None`
/// while it is paused, once a one-off has run, was held back or was missed while paused, and
/// for one with no chat of its own.
pub(super) fn next_fire(scheduled: &Scheduled) -> Option<Timestamp> {
    if is_paused(scheduled) {
        return None;
    }
    match &scheduled.created.when {
        When::At(at) => {
            let waits = scheduled.last.or(scheduled.held).is_none() && scheduled.missed.is_none();
            waits.then(|| at.timestamp())
        }
        When::Cron(text) => {
            let zone = time_zone(&scheduled.created.zone).ok()?;
            Cron::parse(text).ok()?.next_after(since(scheduled), &zone)
        }
    }
}

/// When `scheduled` runs next, as lists show it: an overdue run as now.
pub(super) fn next_run(scheduled: &Scheduled, now: Timestamp) -> Option<Timestamp> {
    next_fire(scheduled).map(|next| next.max(now))
}

/// Whether `scheduled` starts no runs: paused, or made before schedules had chats.
fn is_paused(scheduled: &Scheduled) -> bool {
    scheduled.paused || scheduled.created.thread.is_none()
}

/// Whether `ledger` holds a run still open, as one waiting for the person is, or a schedule that
/// will still run, or will once resumed, at `now`.
pub(super) fn live(ledger: &Ledger, now: Timestamp) -> bool {
    !ledger.open.is_empty() || (ledger.schedules.values()).any(|scheduled| kept(scheduled, now))
}

/// Since when `ledger` has held nothing to run, as far as it says: its last schedule's going, or
/// the latest a schedule of it was made or due to run.
pub(super) fn idle_since(ledger: &Ledger) -> Option<At> {
    let latest = (ledger.schedules.values())
        .map(|scheduled| {
            scheduled
                .last
                .map_or(scheduled.at, |last| last.max(scheduled.at))
        })
        .max();
    latest.max(ledger.emptied)
}

/// Whether `scheduled` counts toward the schedules a person keeps at `now`: it will still run,
/// or will once resumed. One made before schedules had chats runs no more, and doesn't.
pub(super) fn kept(scheduled: &Scheduled, now: Timestamp) -> bool {
    let resumable = scheduled.paused && scheduled.created.thread.is_some();
    resumable || next_run(scheduled, now).is_some()
}

/// Runs are counted from the latest one, or the latest held back, or from when the schedule
/// was resumed or made.
fn since(scheduled: &Scheduled) -> Timestamp {
    (scheduled.last.max(scheduled.held).max(scheduled.resumed))
        .unwrap_or(scheduled.at)
        .timestamp()
}

/// `scheduled` of `project`, as lists show it, at `now`.
pub(super) fn info(
    project: Located,
    scheduled: &Scheduled,
    ledger: &Ledger,
    now: Timestamp,
) -> ScheduleInfo {
    let created = &scheduled.created;
    // One made before schedules had chats has no name of its own.
    let name = Some(created.name.clone())
        .filter(|name| !name.is_empty())
        .or_else(|| title_from(&created.prompt))
        .unwrap_or_default();
    let note = created
        .thread
        .is_none()
        .then(|| Told::from(Sentence::ScheduleNoChat));
    let name_shown = shown(&name);
    ScheduleInfo {
        schedule: created.schedule,
        stop: Told::from(Ask::StopRepeating {
            name: name_shown.clone(),
        }),
        name_shown,
        name,
        project: project.project,
        thread: created.thread,
        from: created.from,
        when: created.when.clone(),
        zone: created.zone.clone(),
        repeats: Told::repeats(created.when.clone(), true),
        prompt: created.prompt.clone(),
        prompt_shown: shown(&created.prompt),
        budget: created.budget,
        created_at: scheduled.at,
        paused: is_paused(scheduled),
        last: last_run(scheduled, ledger),
        next: next_run(scheduled, now).map(At::from),
        note,
    }
}

/// What came of `scheduled`'s latest run, as `ledger` has it.
fn last_run(scheduled: &Scheduled, ledger: &Ledger) -> Option<LastRun> {
    if let Some(due) = scheduled.missed {
        let said = Sentence::RunMissed.into();
        let (status, outcome) = (RunStatus::Missed, None);
        return Some(LastRun {
            due,
            status,
            outcome,
            said,
        });
    }
    let due = scheduled.last?;
    let schedule = scheduled.created.schedule;
    let (status, outcome, said) = match &scheduled.skipped {
        Some(Skipped::Going) => (RunStatus::Skipped, None, Sentence::RunSkipped),
        Some(Skipped::Refused { reason }) => {
            let reason = reason.clone();
            (RunStatus::Refused, None, Sentence::RunRefused { reason })
        }
        None => {
            let started = |task: &&Recorded| {
                (task.started.schedule)
                    .is_some_and(|fired| fired.schedule == schedule && fired.due == due)
            };
            if ledger.open.values().any(|task| started(&task)) {
                (RunStatus::Running, None, Sentence::RunRunning)
            } else {
                let ended = ledger.finished.iter().rev().find(started);
                match ended.and_then(|task| task.ended.as_ref()) {
                    Some((ended, _)) => (RunStatus::Ended, Some(ended.outcome), ran(ended.outcome)),
                    None => (RunStatus::Started, None, Sentence::RunStarted),
                }
            }
        }
    };
    Some(LastRun {
        due,
        status,
        outcome,
        said: said.into(),
    })
}

/// How a run that ended with `outcome` went, in a few words.
fn ran(outcome: Outcome) -> Sentence {
    let outcome = match outcome {
        Outcome::Completed => return Sentence::RunDone,
        Outcome::Cancelled => "cancelled",
        Outcome::Failed => "failed",
        Outcome::Interrupted => "interrupted",
        Outcome::LimitReached => "limit_reached",
    };
    Sentence::RunUnfinished {
        outcome: outcome.to_owned(),
    }
}
