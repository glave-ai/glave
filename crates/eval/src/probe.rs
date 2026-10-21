//! Probes: later threads that each ask for one earlier fact (`[[glave.threads]] probe`), each
//! graded on its own thread's answer and events, so a long task scores the share it got right
//! and passes when that share reaches `[glave] probe_share`.

use serde::{Deserialize, Serialize};

use crate::check::Context;
use crate::task::Task;
use crate::trace;

/// Slack when comparing shares, so 8 of 10 meets 0.8 however it rounds.
const SLACK: f64 = 1e-9;

/// How a task's probes went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Probes {
    pub passed: u32,
    pub of: u32,
    /// Why each probe that did not pass failed, in order.
    pub failed: Vec<String>,
}

impl Probes {
    /// The failure, when fewer passed than `share` of them.
    pub(crate) fn short_of(&self, share: f64) -> Option<String> {
        let needed = (f64::from(self.of) * share - SLACK).ceil();
        (f64::from(self.passed) < needed).then(|| {
            format!(
                "{} of {} probes passed, {needed} needed: {}",
                self.passed,
                self.of,
                self.failed.join("; ")
            )
        })
    }
}

/// Grades each probe of `task` on its own thread's events, `threads` holding each thread's in
/// the order they ran, the instruction's first; everything else comes from `cx`. `None` when
/// the task has no probes.
pub(crate) fn grade(task: &Task, threads: &[String], cx: &Context<'_>) -> Option<Probes> {
    let mut probes = Probes {
        passed: 0,
        of: 0,
        failed: Vec::new(),
    };
    for (index, thread) in task.threads.iter().enumerate() {
        if thread.probe.is_empty() {
            continue;
        }
        probes.of = probes.of.saturating_add(1);
        // The instruction's thread ran first; `task.threads` came after it.
        let ran = index.saturating_add(1);
        let why = match threads
            .get(ran)
            .map(|events| (events, trace::parse(events)))
        {
            None => Some("its thread never ran".to_owned()),
            Some((_, Err(error))) => Some(error),
            Some((events, Ok(trace))) => {
                let here = Context {
                    trace: &trace,
                    last: &trace,
                    events,
                    ..*cx
                };
                let failures: Vec<String> = (thread.probe.iter())
                    .filter_map(|check| check.judge(&here).err())
                    .collect();
                (!failures.is_empty()).then(|| failures.join(", "))
            }
        };
        match why {
            None => probes.passed = probes.passed.saturating_add(1),
            Some(why) => probes.failed.push(format!(
                "probe {} (thread {}): {why}",
                probes.of,
                ran.saturating_add(1)
            )),
        }
    }
    (probes.of > 0).then_some(probes)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use glave_event::Outcome;

    use super::*;
    use crate::check::Check;
    use crate::memory::MemoryReport;
    use crate::trace::Trace;

    #[test]
    fn the_share_is_met_at_the_bar_and_a_miss_names_every_failed_probe() {
        let probes = |passed| Probes {
            passed,
            of: 10,
            failed: vec!["probe 3 (thread 9): the answer does not mention \"Railway\"".into()],
        };
        assert_eq!(probes(8).short_of(0.8), None);
        assert_eq!(probes(10).short_of(1.0), None);
        assert_eq!(
            probes(7).short_of(0.8).as_deref(),
            Some(
                "7 of 10 probes passed, 8 needed: probe 3 (thread 9): the answer does not \
                 mention \"Railway\""
            )
        );
        assert!(probes(9).short_of(1.0).is_some(), "every probe");
    }

    /// Every repository task's probes against made-up answers: one that knows nothing fails
    /// each probe, one that says what the probe must find passes it, and one that also says what
    /// it must not fails it. Their graders' oracle, as no model runs in CI.
    #[test]
    fn every_probe_fails_knowing_nothing_and_passes_knowing_the_current_fact() {
        let tasks = crate::task::load_all(Path::new("../../evals/tasks")).unwrap();
        let mut seen = 0;
        for task in &tasks {
            for thread in task
                .threads
                .iter()
                .filter(|thread| !thread.probe.is_empty())
            {
                seen += 1;
                let (mut wanted, mut unwanted) = (Vec::new(), Vec::new());
                for check in &thread.probe {
                    match check {
                        Check::AnswerContains(text) => wanted.push(text.as_str()),
                        Check::AnswerLacks(texts) => {
                            unwanted.extend(texts.iter().map(String::as_str));
                        }
                        _ => {}
                    }
                }
                let what = format!("{}: {:?}", task.name, thread.messages);
                assert!(!wanted.is_empty(), "{what} names nothing to find");
                let failures = |answer: &str| {
                    let trace = Trace {
                        answer: answer.to_owned(),
                        outcomes: vec![Outcome::Completed],
                        threads: 1,
                        ..Trace::default()
                    };
                    let cx = Context {
                        trace: &trace,
                        last: &trace,
                        events: "",
                        workspace: Path::new("/nonexistent"),
                        original: Path::new("/nonexistent"),
                        model: "m",
                        memory: &MemoryReport::default(),
                        requested: &[],
                    };
                    (thread.probe.iter())
                        .filter_map(|check| check.judge(&cx).err())
                        .count()
                };
                assert!(
                    failures("I don't know.") > 0,
                    "{what} passes knowing nothing"
                );
                let knows = wanted.join(" ");
                assert_eq!(failures(&knows), 0, "{what} fails {knows:?}");
                if !unwanted.is_empty() {
                    let stale = format!("{knows} {}", unwanted.join(" "));
                    assert!(failures(&stale) > 0, "{what} passes {stale:?}");
                }
            }
        }
        assert!(seen >= 10, "the month's probes are among them");
    }
}
