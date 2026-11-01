//! What work that ran through compactions took: beside each task's pass rate on each model, its
//! compactions, tokens and repeated steps per attempt, and how its goals ended.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::num::NonZeroU64;

use glave_event::GoalReason;

use super::{Report, as_u64, passes, short, thousands, tokens};
use crate::run::Attempt;

impl Report {
    /// A row for each task and model with an attempt that compacted, every attempt of it
    /// counted; nothing when none did.
    pub(super) fn compacted(&self, out: &mut String) {
        let mut rows = String::new();
        for task in self.tasks() {
            for model in &self.models {
                let ran: Vec<&Attempt> = (self.attempts.iter())
                    .filter(|a| a.task == task && a.model == *model && !a.skipped)
                    .collect();
                if ran.iter().any(|attempt| attempt.compactions > 0) {
                    let _ = writeln!(rows, "| {task} | {} | {} |", short(model), row(&ran));
                }
            }
        }
        if !rows.is_empty() {
            let _ = writeln!(
                out,
                "\nThrough compaction, per attempt:\n\n| task | model | passed | compactions | \
                 tokens | repeated steps | goal ended |\n| --- | --- | --- | --- | --- | --- | --- \
                 |\n{}",
                rows.trim_end()
            );
        }
    }
}

/// The cells of one task on one model, `ran` being at least one attempt.
fn row(ran: &[&Attempt]) -> String {
    let count = NonZeroU64::new(as_u64(ran.len())).unwrap_or(NonZeroU64::MIN);
    // In tenths, rounded: whole numbers, so no precision is lost to a float.
    let mean = |total: u64| {
        let tenths = total.saturating_mul(10).saturating_add(count.get() / 2) / count;
        format!("{}.{}", tenths / 10, tenths % 10)
    };
    let compactions = ran.iter().map(|a| u64::from(a.compactions)).sum();
    let repeated = ran.iter().map(|a| u64::from(a.repeated_steps)).sum();
    format!(
        "{}/{} | {} | {} | {} | {}",
        passes(ran),
        ran.len(),
        mean(compactions),
        thousands(tokens(ran) / count),
        mean(repeated),
        goals(ran)
    )
}

/// How the attempts' goals ended, such as `Met 2, Budget 1`, or a dash when none ended.
fn goals(ran: &[&Attempt]) -> String {
    let mut ended: BTreeMap<String, u32> = BTreeMap::new();
    let reasons = ran.iter().filter_map(|attempt| attempt.goal);
    for reason in reasons.map(|reason: GoalReason| format!("{reason:?}")) {
        *ended.entry(reason).or_default() += 1;
    }
    if ended.is_empty() {
        return "-".to_owned();
    }
    let ended: Vec<String> = (ended.iter())
        .map(|(reason, count)| format!("{reason} {count}"))
        .collect();
    ended.join(", ")
}
