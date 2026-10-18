//! How tasks and schedules read in the terminal.

use glave_event::{Money, Outcome, When};
use glave_session::protocol::{ScheduleInfo, TaskInfo, TaskKind, TaskStatus};

/// One task: status, kind, elapsed time, tokens, cost, model, thread and title.
pub fn task_line(task: &TaskInfo) -> String {
    let status = match (task.status, task.outcome) {
        (TaskStatus::Running, _) => "running".to_owned(),
        (TaskStatus::Elsewhere, _) => "elsewhere".to_owned(),
        (TaskStatus::Finished, Some(outcome)) => outcome_name(outcome).to_owned(),
        (TaskStatus::Finished, None) => "finished".to_owned(),
    };
    let kind = match task.kind {
        TaskKind::Agent => "agent",
    };
    let tokens = task.tokens.input.saturating_add(task.tokens.output);
    let budget = task
        .budget
        .map(|budget| format!("/{}", short(budget)))
        .unwrap_or_default();
    format!(
        "{status:<13} {kind}  {:>7}  {:>6}{budget} tokens  {}  {}  {}  {}",
        elapsed(task.elapsed_ms),
        short(tokens),
        task.cost.map_or_else(|| "$?".to_owned(), dollars),
        task.model,
        task.thread,
        task.title
    )
}

/// One schedule: when it runs, what it runs, and when it runs next.
pub fn schedule_line(schedule: &ScheduleInfo) -> String {
    let when = match &schedule.when {
        When::Cron(cron) => format!("cron {cron:?} (UTC)"),
        When::At(at) => format!("once at {at}"),
    };
    let next = schedule
        .next
        .map_or_else(|| "has run".to_owned(), |next| format!("next {next}"));
    let goal = if schedule.goal { "goal: " } else { "" };
    format!(
        "{}  {when}  {next}  budget {}  {goal}{}",
        schedule.schedule,
        short(schedule.budget),
        schedule.prompt
    )
}

fn outcome_name(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Completed => "completed",
        Outcome::Cancelled => "stopped",
        Outcome::Failed => "failed",
        Outcome::Interrupted => "interrupted",
        Outcome::LimitReached => "out of budget",
    }
}

/// `12s`, `3m04s`, `2h05m`.
fn elapsed(millis: u64) -> String {
    let seconds = millis / 1_000;
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3_600 => format!("{}m{:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h{:02}m", seconds / 3_600, seconds / 60 % 60),
    }
}

/// `950`, `12.4k`, `3.1M`.
fn short(tokens: u64) -> String {
    #[expect(clippy::cast_precision_loss, reason = "a rounded figure to show")]
    let value = tokens as f64;
    match tokens {
        0..1_000 => tokens.to_string(),
        1_000..1_000_000 => format!("{:.1}k", value / 1_000.0),
        _ => format!("{:.1}M", value / 1_000_000.0),
    }
}

fn dollars(cost: Money) -> String {
    let cents = cost.micros / 10_000;
    format!("${}.{:02}", cents / 100, cents % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figures_read_short() {
        assert_eq!(elapsed(4_200), "4s");
        assert_eq!(elapsed(184_000), "3m04s");
        assert_eq!(elapsed(7_500_000), "2h05m");
        assert_eq!(short(950), "950");
        assert_eq!(short(12_400), "12.4k");
        assert_eq!(short(3_100_000), "3.1M");
        let cost = Money {
            micros: 1_234_567,
            currency: glave_event::Currency::USD,
        };
        assert_eq!(dollars(cost), "$1.23");
    }
}
