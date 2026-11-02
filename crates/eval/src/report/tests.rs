use glave_event::GoalReason;

use super::*;
use crate::memory::MemoryUse;
use crate::probe::Probes;

fn attempt(task: &str, model: &str, number: u32, passed: bool) -> Attempt {
    let mut attempt = Attempt::unrun(task, model, number, false, "wrong".into());
    if passed {
        attempt.failures.clear();
    }
    attempt.cost_micros = Some(1_000);
    attempt.input_tokens = 1_200;
    attempt.output_tokens = 34;
    attempt.tool_calls = 2;
    attempt.asks = 1;
    attempt.seconds = f64::from(number);
    attempt
}

fn report(attempts: Vec<Attempt>) -> Report {
    run(&["a/one", "b/two"], attempts)
}

fn run(models: &[&str], attempts: Vec<Attempt>) -> Report {
    Report::new(
        "2026-10-08T12:00:00Z".into(),
        "openrouter".into(),
        models.iter().map(|&model| model.to_owned()).collect(),
        attempts,
    )
}

/// An attempt that cost `micros`, or reported no cost.
fn costing(model: &str, number: u32, passed: bool, micros: Option<i64>) -> Attempt {
    let mut attempt = attempt("fix", model, number, passed);
    attempt.cost_micros = micros;
    attempt
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
        table.contains("\n\n| task | one | two |\n| --- | --- | --- |\n"),
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
fn a_passed_task_costs_all_that_was_spent_over_the_passes() {
    let of = |attempts: &[Attempt]| PerPassedTask::of(&attempts.iter().collect::<Vec<_>>());
    let per = |cost_micros, tokens| {
        Some(PerPassedTask {
            cost_micros,
            tokens,
        })
    };
    // Two of three passed: what the failed one spent counts too.
    let attempts = [
        costing("a/one", 1, true, Some(1_000)),
        costing("a/one", 2, false, Some(1_000)),
        costing("a/one", 3, true, Some(1_000)),
    ];
    assert_eq!(of(&attempts), per(Some(1_500), 1_851));
    // A call that reported no cost, and a free route's $0, are no cost reported.
    assert_eq!(of(&[costing("a/one", 1, true, None)]), per(None, 1_234));
    assert_eq!(of(&[costing("a/one", 1, true, Some(0))]), per(None, 1_234));
    // None passed: nothing to divide by.
    assert_eq!(of(&[costing("a/one", 1, false, Some(1_000))]), None);
    assert_eq!(of(&[]), None);
}

#[test]
fn the_headline_names_the_cheapest_model_per_passed_task() {
    let table = run(
        &["lab/dear", "lab/cheap", "lab/free", "lab/failing"],
        vec![
            costing("lab/dear", 1, true, Some(3_000)),
            costing("lab/dear", 2, false, Some(1_000)),
            costing("lab/cheap", 1, true, Some(1_000)),
            costing("lab/cheap", 2, true, Some(1_000)),
            costing("lab/free", 1, true, Some(0)),
            costing("lab/free", 2, true, Some(0)),
            costing("lab/failing", 1, false, Some(2_000)),
            costing("lab/failing", 2, false, Some(2_000)),
        ],
    )
    .markdown(None);
    assert_eq!(
        table,
        "\
Cheapest per passed task: cheap, $0.0010 · 1,234 tokens, 2/2 passed. No cost reported: free.

| task | dear | cheap | free | failing |
| --- | --- | --- | --- | --- |
| fix | 1/2 | 2/2 | 2/2 | 0/2 |
| **passed** | **1/2** | **2/2** | **2/2** | **0/2** |
| per passed task | $0.0040 · 2,468 tokens | $0.0010 · 1,234 tokens | no cost reported · 1,234 tokens | - |
| cost | $0.0040 | $0.0020 | - | $0.0040 |
| tokens | 2,468 | 2,468 | 2,468 | 2,468 |
| tool calls | 4 | 4 | 4 | 4 |
| asks | 2 | 2 | 2 | 2 |
| blocked commands | 0 | 0 | 0 | 0 |
| median time | 2.0 s | 2.0 s | 2.0 s | 2.0 s |

Not passed:

- fix · lab/dear #2: wrong
- fix · lab/failing #1: wrong
- fix · lab/failing #2: wrong
"
    );
    let headline = |attempts| run(&["lab/one", "lab/two"], attempts).markdown(None);
    // Unpriced, and as the attempt's own line shows it.
    let free = costing("lab/one", 1, true, Some(0));
    assert!(!line(&free).contains('$'), "{}", line(&free));
    let table = headline(vec![free]);
    assert!(
        table.starts_with(
            "Cheapest per passed task: none, as no model passed a task with its cost \
             reported. No cost reported: one.\n\n"
        ),
        "{table}"
    );
    // A tie goes to the model listed first.
    let tied = headline(vec![
        costing("lab/one", 1, true, Some(1_000)),
        costing("lab/two", 1, true, Some(1_000)),
    ]);
    assert!(
        tied.starts_with("Cheapest per passed task: one, $0.0010"),
        "{tied}"
    );
}

#[test]
fn skipped_attempts_are_not_counted_and_fail_the_run() {
    let mut skipped = Attempt::unrun("fix", "a/one", 2, true, "skipped".into());
    skipped.seconds = 99.0;
    let now = report(vec![attempt("fix", "a/one", 1, true), skipped]);
    assert!(now.markdown(None).contains("| fix | 1/1 | - |"));
    assert!(!now.passed(), "an incomplete run is not a pass");
}

/// An attempt at a month of threads: 9 of 10 probes, and what memory did.
fn month(number: u32) -> Attempt {
    let mut month = attempt("memory-month", "a/one", number, true);
    month.probes = Some(Probes {
        passed: 9,
        of: 10,
        failed: vec!["probe 4 (thread 31): the answer does not mention \"Railway\"".into()],
        recalled: vec![3, 9],
    });
    month.memory = Some(MemoryUse {
        tidy_runs: 6,
        tidied_away: vec!["Atlas deploys to Heroku.".into()],
        used: [("personal".to_owned(), 870), ("project".to_owned(), 1_010)].into(),
        in_view: vec![0, 500, 1_900],
    });
    month
}

#[test]
fn reports_round_trip_through_json() {
    let now = report(vec![attempt("fix", "a/one", 1, true), month(1)]);
    let json = serde_json::to_string(&now).unwrap();
    assert_eq!(serde_json::from_str::<Report>(&json).unwrap(), now);
    assert!(
        json.contains(
            r#""per_passed_task":{"a/one":{"cost_micros":1000,"tokens":1234},"b/two":null}"#
        ),
        "{json}"
    );
    // A report written before it still reads, as a baseline.
    let mut older: serde_json::Value = serde_json::from_str(&json).unwrap();
    older.as_object_mut().unwrap().remove("per_passed_task");
    let older: Report = serde_json::from_value(older).unwrap();
    assert!(older.per_passed_task.is_empty());
    let plain = serde_json::to_string(&report(vec![attempt("fix", "a/one", 1, true)])).unwrap();
    assert!(
        !plain.contains("probes") && !plain.contains("memory"),
        "{plain}"
    );
}

#[test]
fn probes_and_memory_get_a_row_per_attempt_and_their_details() {
    let mut off = month(2);
    off.memory = None;
    let table = report(vec![attempt("fix", "a/one", 1, true), month(1), off]).markdown(None);
    assert!(
        table.contains("| memory-month · one #1 | 9/10 | 3, 9 | 6 | 1 | personal 870, project 1,010 | 800 on average, 1,900 at most |"),
        "{table}"
    );
    assert!(
        table.contains("| memory-month · one #2 | 9/10 | 3, 9 | - | - | - | - |"),
        "{table}"
    );
    assert!(!table.contains("| fix · one"), "{table}");
    assert!(
        table.contains(
            "- memory-month · one #1: probe 4 (thread 31): the answer does not mention \"Railway\""
        ),
        "{table}"
    );
    assert!(
        table.contains("- memory-month · one #1: tidied away \"Atlas deploys to Heroku.\""),
        "{table}"
    );
    assert!(line(&month(1)).contains(" · probes 9/10"));
    assert!(
        !report(vec![attempt("fix", "a/one", 1, true)])
            .markdown(None)
            .contains("Probes")
    );
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

#[test]
fn work_through_compaction_shows_its_compactions_tokens_repeats_and_goals() {
    let compacted = |number: u32, compactions: u32, repeated: u32, goal: GoalReason| {
        let mut attempt = attempt("long", "a/one", number, number == 1);
        attempt.compactions = compactions;
        attempt.repeated_steps = repeated;
        attempt.goal = Some(goal);
        attempt
    };
    let attempts = vec![
        compacted(1, 3, 1, GoalReason::Met),
        compacted(2, 4, 0, GoalReason::Budget),
        attempt("short", "a/one", 1, true),
    ];
    let table = report(attempts).markdown(None);
    assert!(
        table.contains("| long | one | 1/2 | 3.5 | 1,234 | 0.5 | Budget 1, Met 1 |"),
        "{table}"
    );
    assert!(!table.contains("| short | one | 1/1 | 0.0"), "{table}");
}

#[test]
fn nothing_compacted_shows_no_such_table() {
    let table = report(vec![attempt("fix", "a/one", 1, true)]).markdown(None);
    assert!(!table.contains("Through compaction"), "{table}");
}

#[test]
fn a_report_written_before_the_counts_still_reads_as_a_baseline() {
    let attempt = attempt("fix", "a/one", 1, true);
    let mut json = serde_json::to_value(&attempt).unwrap();
    let fields = json.as_object_mut().unwrap();
    fields.remove("compactions");
    fields.remove("repeated_steps");
    let read: Attempt = serde_json::from_value(json).unwrap();
    assert_eq!(read, attempt);
}
