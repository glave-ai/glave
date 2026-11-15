//! How each model reasoned: the level asked of every model, the levels its calls were sent at,
//! and the reasoning tokens they spent, so a run at a chosen level shows the model reasoned.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::{Report, ran, short, thousands};
use crate::run::Attempt;
use crate::trace::DEFAULT_LEVEL;

impl Report {
    /// The level asked of every model, or each model's own, then each model some of whose
    /// calls were sent at another level, with the levels they were sent at.
    pub(super) fn reasoning(&self) -> String {
        let Some(asked) = &self.effort else {
            return "Reasoning: each model's own level.".to_owned();
        };
        let mut line = format!("Reasoning: {asked}, asked of every model.");
        let other: Vec<String> = (self.models.iter())
            .filter_map(|model| {
                let sent = sent_at(&ran(&self.attempts, model));
                sent.keys()
                    .any(|level| level != asked)
                    .then(|| format!("{} ({})", short(model), levels(&sent)))
            })
            .collect();
        if !other.is_empty() {
            let _ = write!(line, " Not sent it on every call: {}.", other.join(", "));
        }
        line
    }
}

/// The calls of `attempts` by the level they were sent at.
fn sent_at(attempts: &[&Attempt]) -> BTreeMap<String, u64> {
    let mut sent = BTreeMap::new();
    for (level, calls) in attempts.iter().flat_map(|a| &a.sent_at) {
        let count: &mut u64 = sent.entry(level.clone()).or_default();
        *count = count.saturating_add(u64::from(*calls));
    }
    sent
}

/// Levels and their calls in words: `high`, or `high 40 · model's own 2` when there are several.
fn levels(sent: &BTreeMap<String, u64>) -> String {
    let name = |level: &str| {
        if level == DEFAULT_LEVEL {
            "model's own".to_owned()
        } else {
            level.to_owned()
        }
    };
    match sent.len() {
        0 => "-".to_owned(),
        1 => sent.keys().map(|level| name(level)).collect(),
        _ => (sent.iter())
            .map(|(level, calls)| format!("{} {calls}", name(level)))
            .collect::<Vec<_>>()
            .join(" · "),
    }
}

/// The levels `attempts`' calls were sent at, as [`levels`] says them.
pub(super) fn sent(attempts: &[&Attempt]) -> String {
    levels(&sent_at(attempts))
}

/// The model calls of `attempts`.
fn calls(attempts: &[&Attempt]) -> u64 {
    attempts.iter().map(|a| u64::from(a.model_calls)).sum()
}

/// The reasoning tokens `attempts` spent in all.
pub(super) fn tokens(attempts: &[&Attempt]) -> String {
    thousands(attempts.iter().map(|a| a.reasoning_tokens).sum())
}

/// The reasoning tokens of a call, on average, rounded; a dash when there were no calls.
pub(super) fn per_call(attempts: &[&Attempt]) -> String {
    let calls = calls(attempts);
    if calls == 0 {
        return "-".to_owned();
    }
    let reasoning: u64 = attempts.iter().map(|a| a.reasoning_tokens).sum();
    thousands(reasoning.saturating_add(calls / 2) / calls)
}

/// The share of calls that reported no reasoning; a dash when there were no calls.
pub(super) fn without(attempts: &[&Attempt]) -> String {
    let calls = calls(attempts);
    if calls == 0 {
        return "-".to_owned();
    }
    let none: u64 = (attempts.iter())
        .map(|a| u64::from(a.calls_without_reasoning))
        .sum();
    format!("{}%", none.saturating_mul(100) / calls)
}
