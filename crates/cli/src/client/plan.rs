//! A plan a turn proposed, in the terminal: the server's sentence, then each step on a line of
//! its own, as `thread/plan` gives it. With no one here, that is the turn's answer, and nothing
//! is built. With someone here, it asks as an approval does: `y` builds it (`plan/build`), any
//! other words change it (a turn with them, in plan mode still), and nothing leaves it.

use std::fmt::Write as _;

use glave_event::PlanStatus;
use glave_harness::Cancel;
use glave_session::protocol::{PlanInfo, Reply, Request};

use super::Client;
use crate::terminal::{DIM, Printer, paint};

/// What the person said to a proposed plan.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Build,
    /// Their words: what to change.
    Change(String),
    NotNow,
}

/// The plan as the terminal shows it: its sentence, its steps, and, when someone is here to
/// answer, the question, ending where the answer is typed.
pub(crate) fn shown(color: bool, plan: &PlanInfo, asks: bool) -> String {
    let mut text = format!(
        "\n\n{} {}\n",
        paint(color, "\x1b[33m", "plan"),
        plan.sentence
    );
    for (number, step) in (1_usize..).zip(&plan.steps) {
        let mark = match step.status {
            PlanStatus::Pending => " ",
            PlanStatus::InProgress => "~",
            PlanStatus::Completed => "x",
        };
        let _ = writeln!(text, "  {number}. [{mark}] {}", step.step);
    }
    if asks {
        let how = paint(color, DIM, "or type what to change");
        let _ = write!(text, "  build it? [y] yes  [N] not now, {how} › ");
    }
    text
}

/// What a typed line says: `y` or `yes` builds; other words change the plan; nothing, `n`,
/// `no` or the end of input leaves it.
pub(crate) fn answer(line: Option<&str>) -> Answer {
    let typed = line.map(str::trim).unwrap_or_default();
    match typed.to_ascii_lowercase().as_str() {
        "y" | "yes" => Answer::Build,
        "" | "n" | "no" => Answer::NotNow,
        _ => Answer::Change(typed.to_owned()),
    }
}

impl Client<'_> {
    /// After a turn that proposed a plan: shows the plan the thread has now, and, with someone
    /// here, what to run next as they answer. `None` when nothing follows.
    pub(crate) async fn after_plan(
        &self,
        printer: &mut Printer,
        interrupt: &Cancel,
    ) -> Option<Request> {
        let asked = Request::ThreadPlan {
            thread: self.thread,
        };
        let Ok(Reply::Plan { plan: Some(plan) }) = self.server.request(asked).await else {
            return None;
        };
        if !plan.proposed {
            return None;
        }
        printer.plan(&plan, self.asks);
        if !self.asks {
            return None;
        }
        // Ctrl-C leaves it, as no does.
        let line = interrupt.run(self.input.line()).await.flatten();
        match answer(line.as_deref()) {
            Answer::Build => {
                printer.status("\n· building the plan");
                Some(Request::PlanBuild {
                    thread: self.thread,
                })
            }
            Answer::Change(words) => Some(self.turn_start(&words)),
            Answer::NotNow => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_event::PlanStep;

    fn plan() -> PlanInfo {
        let step = |step: &str, status| PlanStep {
            step: step.to_owned(),
            status,
        };
        PlanInfo {
            steps: vec![
                step("Read the parser", PlanStatus::Completed),
                step("Add --verbose", PlanStatus::InProgress),
                step("Run the tests", PlanStatus::Pending),
            ],
            proposed: true,
            sentence: "Proposed plan: 3 steps".into(),
        }
    }

    #[test]
    fn a_plan_is_its_sentence_then_a_line_for_each_step() {
        let shown = shown(false, &plan(), false);
        assert_eq!(
            shown,
            "\n\nplan Proposed plan: 3 steps\n  1. [x] Read the parser\n  2. [~] Add --verbose\n  3. [ ] Run the tests\n"
        );
    }

    #[test]
    fn with_someone_here_it_asks_as_an_approval_does() {
        let shown = shown(false, &plan(), true);
        assert!(
            shown.ends_with("  build it? [y] yes  [N] not now, or type what to change › "),
            "{shown}"
        );
    }

    #[test]
    fn yes_builds_words_change_it_and_nothing_leaves_it() {
        assert_eq!(answer(Some(" Y ")), Answer::Build);
        assert_eq!(answer(Some("yes")), Answer::Build);
        assert_eq!(
            answer(Some(" skip step 2 ")),
            Answer::Change("skip step 2".into())
        );
        for nothing in [Some(""), Some("n"), Some("No"), None] {
            assert_eq!(answer(nothing), Answer::NotNow);
        }
    }
}
