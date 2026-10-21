//! Probes: later threads that each ask for one earlier fact (`[[glave.threads]] probe`), each
//! graded on its own thread's answer and events, so a long task scores the share it got right
//! and passes when that share reaches `[glave] probe_share`.

use glave_ports::memories::RECALL;
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
    /// The probes, by number, whose thread searched with `recall`.
    #[serde(default)]
    pub recalled: Vec<u32>,
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
        recalled: Vec::new(),
    };
    let recall = format!("\"tool\":\"{RECALL}\"");
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
                if events.contains(&recall) {
                    probes.recalled.push(probes.of);
                }
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
            recalled: Vec::new(),
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

    /// How many of `probe`'s checks fail a thread that answered `answer`, held `events` and
    /// whose commands printed `printed`.
    fn failures(probe: &[Check], answer: &str, events: &str, printed: &str) -> usize {
        let trace = Trace {
            answer: answer.to_owned(),
            printed: printed.to_owned(),
            outcomes: vec![Outcome::Completed],
            threads: 1,
            ..Trace::default()
        };
        let cx = Context {
            trace: &trace,
            last: &trace,
            events,
            workspace: Path::new("/nonexistent"),
            original: Path::new("/nonexistent"),
            model: "m",
            memory: &MemoryReport::default(),
            requested: &[],
        };
        (probe.iter())
            .filter_map(|check| check.judge(&cx).err())
            .count()
    }

    /// Every repository task's probes against made-up threads: one that knows nothing fails
    /// each probe; one that says what the probe must find, and did what it must see done,
    /// passes it; and one that also says what it must not, or printed what it must never print,
    /// fails it. Their graders' oracle, as no model runs in CI.
    #[test]
    fn every_probe_fails_knowing_nothing_and_passes_knowing_the_current_fact() {
        let tasks = crate::task::load_all(Path::new("../../evals/tasks")).unwrap();
        let probes: Vec<(String, &[Check])> = (tasks.iter())
            .flat_map(|task| task.threads.iter().map(move |thread| (task, thread)))
            .filter(|(_, thread)| !thread.probe.is_empty())
            .map(|(task, thread)| {
                (
                    format!("{}: {:?}", task.name, thread.messages),
                    &thread.probe[..],
                )
            })
            .collect();
        assert!(probes.len() >= 10, "the month's probes are among them");
        for (what, probe) in probes {
            let texts = |pick: fn(&Check) -> Vec<&str>| -> Vec<&str> {
                probe.iter().flat_map(pick).collect()
            };
            let wanted = texts(|check| match check {
                Check::AnswerContains(text) => vec![text.as_str()],
                _ => Vec::new(),
            });
            let unwanted = texts(|check| match check {
                Check::AnswerLacks(texts) => texts.iter().map(String::as_str).collect(),
                _ => Vec::new(),
            });
            let done = texts(|check| match check {
                Check::Sent(text) => vec![text.as_str()],
                _ => Vec::new(),
            });
            let unprinted = texts(|check| match check {
                Check::NeverPrinted(text) => vec![text.as_str()],
                _ => Vec::new(),
            });
            assert!(!wanted.is_empty(), "{what} names nothing to find");
            assert!(
                failures(probe, "I don't know.", "", "") > 0,
                "{what} passes knowing nothing"
            );
            let (knows, events) = (wanted.join(" "), done.join("\n"));
            assert_eq!(
                failures(probe, &knows, &events, ""),
                0,
                "{what} fails {knows:?}"
            );
            for text in &unwanted {
                let stale = format!("{knows} {text}");
                assert!(
                    failures(probe, &stale, &events, "") > 0,
                    "{what} passes {stale:?}"
                );
            }
            for text in &unprinted {
                assert!(
                    failures(probe, &knows, &events, text) > 0,
                    "{what} passes printing {text:?}"
                );
            }
            if !done.is_empty() {
                assert!(
                    failures(probe, &knows, "", "") > 0,
                    "{what} passes doing nothing"
                );
            }
        }
    }
}
