//! The report: every attempt as JSON, and a Markdown table of pass rates per task and model,
//! compared with an earlier report when one is given.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use glave_event::{Currency, Money};
use glave_memory::BUDGET_TOKENS;
use serde::{Deserialize, Serialize};

use crate::run::Attempt;

/// One eval run, as saved to `report.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// When the run started, as an RFC 3339 timestamp.
    pub started: String,
    /// The provider's short name, such as `openrouter`.
    pub provider: String,
    pub models: Vec<String>,
    /// What a passed task took on each model, the run's headline; `None` for a model that
    /// passed none. Absent from reports written before it, which still read as baselines.
    #[serde(default)]
    pub(crate) per_passed_task: BTreeMap<String, Option<PerPassedTask>>,
    pub attempts: Vec<Attempt>,
}

/// What a passed task took on one model: everything its attempts spent, the failed ones
/// too, over the attempts that passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PerPassedTask {
    /// Micros of the reported currency; `None` when no cost was reported (see `spent`).
    pub(crate) cost_micros: Option<i64>,
    /// Input and output tokens.
    pub(crate) tokens: u64,
}

impl PerPassedTask {
    /// What a passed task took over one model's `attempts`; `None` when none passed.
    fn of(attempts: &[&Attempt]) -> Option<Self> {
        let passed = attempts.iter().filter(|a| a.passed()).count();
        let tokens = tokens(attempts).checked_div(u64::try_from(passed).ok()?)?;
        let cost_micros =
            spent(attempts).and_then(|micros| micros.checked_div(i64::try_from(passed).ok()?));
        Some(Self {
            cost_micros,
            tokens,
        })
    }
}

/// Passed and run attempts of one task on one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rate {
    passed: usize,
    ran: usize,
}

impl Report {
    /// The report of a run, with what a passed task took on each model worked out.
    pub(crate) fn new(
        started: String,
        provider: String,
        models: Vec<String>,
        attempts: Vec<Attempt>,
    ) -> Self {
        let per_passed_task = (models.iter())
            .map(|model| (model.clone(), PerPassedTask::of(&ran(&attempts, model))))
            .collect();
        Self {
            started,
            provider,
            models,
            per_passed_task,
            attempts,
        }
    }

    /// Whether every attempt ran and passed.
    pub fn passed(&self) -> bool {
        self.attempts.iter().all(Attempt::passed)
    }

    fn rate(&self, task: &str, model: &str) -> Option<Rate> {
        let ran: Vec<_> = self
            .attempts
            .iter()
            .filter(|a| a.task == task && a.model == model && !a.skipped)
            .collect();
        (!ran.is_empty()).then(|| Rate {
            passed: ran.iter().filter(|a| a.passed()).count(),
            ran: ran.len(),
        })
    }

    fn tasks(&self) -> BTreeSet<&str> {
        self.attempts.iter().map(|a| a.task.as_str()).collect()
    }

    /// The model cheapest per passed task, then the pass rate of each task on each model, the
    /// totals per model, and every failure. Rates that differ from `baseline` show what they
    /// were.
    pub fn markdown(&self, baseline: Option<&Self>) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{}\n", self.cheapest());
        let names: Vec<&str> = self.models.iter().map(|model| short(model)).collect();
        let _ = writeln!(out, "| task | {} |", names.join(" | "));
        let _ = writeln!(out, "| --- |{}", " --- |".repeat(names.len()));
        for task in self.tasks() {
            let cells: Vec<String> = self
                .models
                .iter()
                .map(|model| {
                    cell(
                        self.rate(task, model),
                        baseline.and_then(|b| b.rate(task, model)),
                    )
                })
                .collect();
            let _ = writeln!(out, "| {task} | {} |", cells.join(" | "));
        }
        self.totals(&mut out);
        self.probed(&mut out);
        let failed: Vec<&Attempt> = self.attempts.iter().filter(|a| !a.passed()).collect();
        if !failed.is_empty() {
            let _ = writeln!(out, "\nNot passed:\n");
            for attempt in failed {
                let _ = writeln!(
                    out,
                    "- {} · {} #{}: {}",
                    attempt.task,
                    attempt.model,
                    attempt.number,
                    attempt.failures.join("; ")
                );
            }
        }
        out
    }

    /// What a passed task took on `model`; `None` when none passed.
    fn per_passed(&self, model: &str) -> Option<PerPassedTask> {
        self.per_passed_task.get(model).copied().flatten()
    }

    /// The headline: the model whose passed tasks cost least, of those that reported a cost.
    fn cheapest(&self) -> String {
        let priced = self.models.iter().filter_map(|model| {
            let per = self.per_passed(model)?;
            Some((model, per, per.cost_micros?))
        });
        match priced.min_by_key(|(_, _, micros)| *micros) {
            Some((model, per, _)) => format!(
                "Cheapest per passed task: {}, {}.",
                short(model),
                per_task(Some(per))
            ),
            None => "Cheapest per passed task: none, as no model passed a task with its cost \
                     reported."
                .to_owned(),
        }
    }

    /// Rows of totals per model: passed, what a passed task took, cost, tokens, tool calls,
    /// median time.
    fn totals(&self, out: &mut String) {
        let per_model = |row: &Row<'_>| -> String {
            self.models
                .iter()
                .map(|model| row(model, &ran(&self.attempts, model)))
                .collect::<Vec<_>>()
                .join(" | ")
        };
        let rows: [(&str, &Row<'_>); 6] = [
            ("**passed**", &|_, a| {
                format!(
                    "**{}/{}**",
                    a.iter().filter(|a| a.passed()).count(),
                    a.len()
                )
            }),
            ("per passed task", &|model, _| {
                per_task(self.per_passed(model))
            }),
            ("cost", &|_, a| dollars(spent(a))),
            ("tokens", &|_, a| thousands(tokens(a))),
            ("tool calls", &|_, a| {
                a.iter()
                    .map(|a| u64::from(a.tool_calls))
                    .sum::<u64>()
                    .to_string()
            }),
            ("median time", &|_, a| median_seconds(a)),
        ];
        for (name, row) in rows {
            let _ = writeln!(out, "| {name} | {} |", per_model(row));
        }
    }

    /// A row for each attempt with probes: how many passed and which searched with `recall`,
    /// how often tidy-up ran and what it forgot, and memory's size at the end and at each
    /// thread's start; then each probe that failed and each memory tidy-up forgot.
    fn probed(&self, out: &mut String) {
        let probed: Vec<&Attempt> = (self.attempts.iter())
            .filter(|attempt| attempt.probes.is_some())
            .collect();
        if probed.is_empty() {
            return;
        }
        let _ = writeln!(
            out,
            "\nProbes and memory, in tokens as memory's budget counts them ({} a scope):\n\n\
             | attempt | probes | probes that recalled | tidy-up runs | tidied away | in force at \
             the end | at each thread's start |\n| --- | --- | --- | --- | --- | --- | --- |",
            thousands(as_u64(BUDGET_TOKENS))
        );
        let mut notes = Vec::new();
        for attempt in probed {
            let model = short(&attempt.model);
            let name = format!("{} · {model} #{}", attempt.task, attempt.number);
            let share = (attempt.probes.as_ref()).map_or_else(String::new, |probes| {
                let recalled: Vec<String> = probes.recalled.iter().map(u32::to_string).collect();
                let recalled = if recalled.is_empty() {
                    "none".to_owned()
                } else {
                    recalled.join(", ")
                };
                format!("{}/{} | {recalled}", probes.passed, probes.of)
            });
            let memory = attempt.memory.as_ref().map_or_else(
                || "- | - | - | -".to_owned(),
                |memory| {
                    format!(
                        "{} | {} | {} | {}",
                        memory.tidy_runs,
                        memory.tidied_away.len(),
                        used(&memory.used),
                        in_view(&memory.in_view)
                    )
                },
            );
            let _ = writeln!(out, "| {name} | {share} | {memory} |");
            let failed = attempt.probes.iter().flat_map(|probes| &probes.failed);
            notes.extend(failed.map(|failed| format!("- {name}: {failed}")));
            let away = attempt.memory.iter().flat_map(|memory| &memory.tidied_away);
            notes.extend(away.map(|text| format!("- {name}: tidied away {text:?}")));
        }
        if !notes.is_empty() {
            let _ = writeln!(out, "\n{}", notes.join("\n"));
        }
    }
}

/// The attempts of `model` that ran, skipped ones left out.
fn ran<'a>(attempts: &'a [Attempt], model: &str) -> Vec<&'a Attempt> {
    (attempts.iter())
        .filter(|a| a.model == model && !a.skipped)
        .collect()
}

/// What `attempts` cost in micros; `None` when a call reported no cost, or when all they
/// reported is $0, as a free route reports it: that says nothing of what the model costs,
/// so it never makes a model the cheapest.
fn spent(attempts: &[&Attempt]) -> Option<i64> {
    (attempts.iter())
        .try_fold(0_i64, |sum, a| sum.checked_add(a.cost_micros?))
        .filter(|&micros| micros != 0)
}

/// The input and output tokens `attempts` used.
fn tokens(attempts: &[&Attempt]) -> u64 {
    (attempts.iter())
        .map(|a| a.input_tokens.saturating_add(a.output_tokens))
        .sum()
}

/// What a passed task took: its cost and tokens, or a dash when none passed.
fn per_task(per: Option<PerPassedTask>) -> String {
    per.map_or_else(
        || "-".to_owned(),
        |per| {
            let cost = per.cost_micros.map_or_else(
                || "no cost reported".to_owned(),
                |micros| dollars(Some(micros)),
            );
            format!("{cost} · {} tokens", thousands(per.tokens))
        },
    )
}

/// A model's name without its maker: `one` for `a/one`.
fn short(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
}

fn cell(now: Option<Rate>, before: Option<Rate>) -> String {
    let Some(now) = now else {
        return "-".to_owned();
    };
    let text = format!("{}/{}", now.passed, now.ran);
    match before {
        // Compared as fractions: 2/3 then 4/6 is no change.
        Some(before) if now.passed * before.ran != before.passed * now.ran => {
            format!("{text} (was {}/{})", before.passed, before.ran)
        }
        _ => text,
    }
}

/// Micros as dollars to the nearest hundredth of a cent, or a dash when a cost was not
/// reported.
pub(crate) fn dollars(micros: Option<i64>) -> String {
    micros.map_or_else(
        || "-".to_owned(),
        |micros| {
            let cost = Money {
                micros,
                currency: Currency::USD,
            };
            format!("{cost:.4}")
        },
    )
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn median_seconds(attempts: &[&Attempt]) -> String {
    let mut seconds: Vec<f64> = attempts.iter().map(|a| a.seconds).collect();
    seconds.sort_by(f64::total_cmp);
    seconds
        .get(seconds.len() / 2)
        .map_or_else(|| "-".to_owned(), |s| format!("{s:.1} s"))
}

/// One row of totals: a model and its attempts in, a cell out.
type Row<'a> = dyn Fn(&str, &[&Attempt]) -> String + 'a;

/// Tokens in force per scope: `personal 870, project 1,010`.
fn used(used: &BTreeMap<String, usize>) -> String {
    let scopes: Vec<String> = (used.iter())
        .map(|(scope, tokens)| format!("{scope} {}", thousands(as_u64(*tokens))))
        .collect();
    scopes.join(", ")
}

/// Tokens at each thread's start: their mean and the most.
fn in_view(tokens: &[usize]) -> String {
    let mean = tokens.iter().sum::<usize>() / tokens.len().max(1);
    let most = tokens.iter().max().copied().unwrap_or_default();
    format!(
        "{} on average, {} at most",
        thousands(as_u64(mean)),
        thousands(as_u64(most))
    )
}

fn as_u64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// One line per finished attempt, for following a run as it goes.
pub(crate) fn line(attempt: &Attempt) -> String {
    let result = if attempt.skipped {
        "skip"
    } else if attempt.passed() {
        "pass"
    } else {
        "FAIL"
    };
    let mut line = format!(
        "{:<20} {:<32} {result:<4} {:>3} calls {:>3} tools {:>8} tokens {:>8} {:>6.1}s",
        attempt.task,
        format!("{}#{}", attempt.model, attempt.number),
        attempt.model_calls,
        attempt.tool_calls,
        attempt.input_tokens.saturating_add(attempt.output_tokens),
        dollars(attempt.cost_micros),
        attempt.seconds,
    );
    if let Some(probes) = &attempt.probes {
        let _ = write!(line, " · probes {}/{}", probes.passed, probes.of);
    }
    for failure in &attempt.failures {
        let _ = write!(line, "\n    {failure}");
    }
    line
}

#[cfg(test)]
mod tests;
