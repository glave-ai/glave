//! The report: every attempt as JSON, and a Markdown table of pass rates per task and model,
//! compared with an earlier report when one is given.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use glave_event::{Currency, Money};
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
    pub attempts: Vec<Attempt>,
}

/// Passed and run attempts of one task on one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rate {
    passed: usize,
    ran: usize,
}

impl Report {
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

    /// The pass rate of each task on each model, the totals per model, and every failure.
    /// Rates that differ from `baseline` show what they were.
    pub fn markdown(&self, baseline: Option<&Self>) -> String {
        let mut out = String::new();
        let short: Vec<&str> = self
            .models
            .iter()
            .map(|model| model.rsplit('/').next().unwrap_or(model))
            .collect();
        let _ = writeln!(out, "| task | {} |", short.join(" | "));
        let _ = writeln!(out, "| --- |{}", " --- |".repeat(short.len()));
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

    /// Rows of totals per model: passed, cost, tokens, tool calls, median time.
    fn totals(&self, out: &mut String) {
        let per_model = |row: &Row| -> String {
            self.models
                .iter()
                .map(|model| {
                    let attempts: Vec<&Attempt> = self
                        .attempts
                        .iter()
                        .filter(|a| &a.model == model && !a.skipped)
                        .collect();
                    row(&attempts)
                })
                .collect::<Vec<_>>()
                .join(" | ")
        };
        let rows: [(&str, &Row); 5] = [
            ("**passed**", &|a| {
                format!(
                    "**{}/{}**",
                    a.iter().filter(|a| a.passed()).count(),
                    a.len()
                )
            }),
            ("cost", &|a| {
                dollars(
                    a.iter()
                        .try_fold(0_i64, |sum, a| sum.checked_add(a.cost_micros?)),
                )
            }),
            ("tokens", &|a| {
                thousands(
                    a.iter()
                        .map(|a| a.input_tokens.saturating_add(a.output_tokens))
                        .sum(),
                )
            }),
            ("tool calls", &|a| {
                a.iter()
                    .map(|a| u64::from(a.tool_calls))
                    .sum::<u64>()
                    .to_string()
            }),
            ("median time", &|a| median_seconds(a)),
        ];
        for (name, row) in rows {
            let _ = writeln!(out, "| {name} | {} |", per_model(row));
        }
    }
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

/// One row of totals: a model's attempts in, a cell out.
type Row = dyn Fn(&[&Attempt]) -> String;

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
    for failure in &attempt.failures {
        let _ = write!(line, "\n    {failure}");
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(task: &str, model: &str, number: u32, passed: bool) -> Attempt {
        let mut attempt = Attempt::unrun(task, model, number, false, "wrong".into());
        if passed {
            attempt.failures.clear();
        }
        attempt.cost_micros = Some(1_000);
        attempt.input_tokens = 1_200;
        attempt.output_tokens = 34;
        attempt.tool_calls = 2;
        attempt.seconds = f64::from(number);
        attempt
    }

    fn report(attempts: Vec<Attempt>) -> Report {
        Report {
            started: "2026-10-08T12:00:00Z".into(),
            provider: "openrouter".into(),
            models: vec!["a/one".into(), "b/two".into()],
            attempts,
        }
    }

    #[test]
    fn the_table_shows_rates_totals_changes_and_failures() {
        let now = report(vec![
            attempt("fix", "a/one", 1, true),
            attempt("fix", "a/one", 2, true),
            attempt("fix", "b/two", 1, false),
            attempt("fix", "b/two", 2, true),
        ]);
        let before = report(vec![
            attempt("fix", "a/one", 1, true),
            attempt("fix", "b/two", 1, true),
        ]);
        let table = now.markdown(Some(&before));
        assert!(
            table.starts_with("| task | one | two |\n| --- | --- | --- |\n"),
            "{table}"
        );
        assert!(table.contains("| fix | 2/2 | 1/2 (was 1/1) |"), "{table}");
        assert!(
            table.contains("| **passed** | **2/2** | **1/2** |"),
            "{table}"
        );
        assert!(table.contains("| cost | $0.0020 | $0.0020 |"), "{table}");
        assert!(table.contains("| tokens | 2,468 | 2,468 |"), "{table}");
        assert!(table.contains("| tool calls | 4 | 4 |"), "{table}");
        assert!(table.contains("- fix · b/two #1: wrong"), "{table}");
        assert!(!now.passed());
    }

    #[test]
    fn skipped_attempts_are_not_counted_and_fail_the_run() {
        let mut skipped = Attempt::unrun("fix", "a/one", 2, true, "skipped".into());
        skipped.seconds = 99.0;
        let now = report(vec![attempt("fix", "a/one", 1, true), skipped]);
        assert!(now.markdown(None).contains("| fix | 1/1 | - |"));
        assert!(!now.passed(), "an incomplete run is not a pass");
    }

    #[test]
    fn reports_round_trip_through_json() {
        let now = report(vec![attempt("fix", "a/one", 1, true)]);
        let json = serde_json::to_string(&now).unwrap();
        assert_eq!(serde_json::from_str::<Report>(&json).unwrap(), now);
    }

    #[test]
    fn numbers_read_easily() {
        assert_eq!(dollars(Some(7_731)), "$0.0077");
        assert_eq!(dollars(Some(1_234_500)), "$1.2345");
        assert_eq!(dollars(None), "-");
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
