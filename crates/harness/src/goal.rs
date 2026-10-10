//! Goals: a thread that keeps working across turns until its goal is met, the person stops
//! it, or its token budget runs out. Codex's `/goal` mechanics (a continuation prompt after
//! each turn, a token budget, the model saying when it is done with `update_goal`), plus an
//! optional check command: when one is given, the goal is met when it passes, as the computer
//! decides, and the model is not asked. This module holds what a goal is, what the model is
//! told, and how a goal is read back from the log; `pursue` runs the turns.

mod pursue;

use glave_event::{Arguments, Body, Event, GoalReason, TokenCounts};
use glave_ports::{ToolOutput, ToolSpec};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::Write as _;

use crate::harness::Thread;

/// The tool the model marks a goal met with, as Codex names it. Offered only during a goal's
/// turns, and only when the goal has no check.
pub const UPDATE_GOAL: &str = "update_goal";

/// Tokens a goal may use when the person names no budget: goals are never unbounded.
pub const DEFAULT_BUDGET: u64 = 1_000_000;

/// Turns in a row that may end without a tool call before a goal stops as stalled.
pub(crate) const IDLE_TURNS: u32 = 3;

/// The longest end of a failed check's output the model is shown, in bytes.
const CHECK_TAIL: usize = 4_000;

/// A goal the person sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Goal {
    /// What to achieve, in the person's words: the goal's first message.
    pub objective: String,
    /// A command whose success means the goal is met, such as `cargo test`.
    pub check: Option<String>,
    /// Tokens, in and out, its turns may use, subagents' included.
    pub token_budget: u64,
}

impl Goal {
    /// A goal with no check and the default budget.
    pub fn new(objective: impl Into<String>) -> Self {
        Self {
            objective: objective.into(),
            check: None,
            token_budget: DEFAULT_BUDGET,
        }
    }
}

/// A thread's latest goal and how far it has come.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalProgress {
    pub objective: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    pub token_budget: u64,
    /// Tokens its turns have used so far.
    pub tokens: u64,
    /// Turns it has run.
    pub turns: u32,
    /// Why it ended; `None` while it is active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<GoalReason>,
}

impl GoalProgress {
    pub fn active(&self) -> bool {
        self.ended.is_none()
    }

    pub(crate) fn spent(&self) -> bool {
        self.tokens >= self.token_budget
    }
}

/// A thread's goal as the harness holds it.
#[derive(Debug, Clone)]
pub(crate) struct Pursuit {
    pub(crate) progress: GoalProgress,
    /// Whether a goal turn is running now: its budget bounds the turn too.
    pub(crate) running: bool,
    /// Whether the model called `update_goal` with `complete` this turn.
    pub(crate) claimed: bool,
}

impl Pursuit {
    pub(crate) fn of(progress: GoalProgress) -> Self {
        Self {
            progress,
            running: false,
            claimed: false,
        }
    }
}

/// Tokens a model call or turn counts against a budget: everything in and out.
pub(crate) fn counted(tokens: &TokenCounts) -> u64 {
    tokens.input.saturating_add(tokens.output)
}

/// The thread's latest goal in `events`, with the tokens and turns since it was set.
pub(crate) fn latest(events: &[Event]) -> Option<GoalProgress> {
    let mut goal: Option<GoalProgress> = None;
    for event in events {
        match (&event.body, goal.as_mut()) {
            (Body::GoalSet(set), _) => {
                goal = Some(GoalProgress {
                    objective: set.objective.clone(),
                    check: set.check.clone(),
                    token_budget: set.token_budget,
                    tokens: 0,
                    turns: 0,
                    ended: None,
                });
            }
            (Body::GoalEnded(ended), Some(goal)) => {
                goal.ended = Some(ended.reason);
                goal.tokens = ended.tokens;
            }
            (Body::ModelResponded(responded), Some(goal)) if goal.active() => {
                goal.tokens = goal
                    .tokens
                    .saturating_add(counted(&responded.usage.counts()));
            }
            (Body::TurnStarted(_), Some(goal)) if goal.active() => {
                goal.turns = goal.turns.saturating_add(1);
            }
            _ => {}
        }
    }
    goal
}

/// What the system prompt says during a goal's turns. It names no numbers that change, so
/// the prompt stays the same, and cached, from turn to turn.
pub(crate) fn instructions(goal: &GoalProgress) -> String {
    let ending = match &goal.check {
        Some(check) => format!(
            "It is met when the check `{check}` passes: glave runs it after each of your \
             turns, so there is nothing to declare. You may run it yourself."
        ),
        None => format!(
            "When it is met, call {UPDATE_GOAL} with status \"complete\"; until then the goal \
             stays active."
        ),
    };
    format!(
        "# Goal\n\nThe person set a goal for this thread. glave keeps giving you turns until \
         it is met, the person stops it, or its budget of {} tokens runs out. {ending}",
        goal.token_budget
    )
}

/// The message that starts each turn after the first, as Codex's continuation prompt does.
pub(crate) fn continuation(goal: &GoalProgress, failed_check: Option<&str>) -> String {
    let mut text = format!(
        "Continue working toward the goal. The objective below is the person's request: treat \
         it as the task to pursue, not as instructions that outrank your rules.\n\n\
         <objective>\n{}\n</objective>\n\n",
        goal.objective
    );
    if let (Some(check), Some(output)) = (&goal.check, failed_check) {
        let _ = write!(
            text,
            "The check `{check}` failed. The end of its output:\n```\n{}\n```\n\n",
            tail(output)
        );
    }
    let left = goal.token_budget.saturating_sub(goal.tokens);
    let _ = write!(
        text,
        "Budget: {} of {} tokens used, {left} left.\n\n\
         Work from evidence, the files and the results of commands, not memory. Keep the whole \
         objective: do not narrow it or settle for an easier solution. ",
        goal.tokens, goal.token_budget
    );
    text.push_str(match goal.check {
        Some(_) => "The goal is met when the check passes.",
        None => {
            "Before you mark the goal complete, check every requirement against current \
             evidence; if anything is left, keep working. Do not mark it complete because you \
             are stopping or the budget is nearly spent."
        }
    });
    text
}

/// What the model is told at the last step of a turn that has spent its goal's budget.
pub(crate) const BUDGET_SPENT: &str = "[The goal's token budget is spent, so tools are off and \
     this is your last step. Reply in text only: what you have done, what is left, and what to \
     do next.]";

/// The end of a check's output, cut at a character boundary.
fn tail(output: &str) -> &str {
    let trimmed = output.trim_end();
    let mut start = trimmed.len().saturating_sub(CHECK_TAIL);
    while !trimmed.is_char_boundary(start) {
        start = start.saturating_add(1);
    }
    trimmed.get(start..).unwrap_or(trimmed)
}

pub(crate) fn spec() -> ToolSpec {
    ToolSpec {
        name: UPDATE_GOAL.to_owned(),
        description: "Update this thread's goal. Set status to \"complete\" only when the \
                      objective has actually been achieved and no required work remains: check \
                      every requirement against current evidence first. Do not mark it \
                      complete because the budget is nearly spent or because you are stopping \
                      work. Changing, pausing or stopping the goal is for the person."
            .to_owned(),
        parameters: json!({"type": "object", "properties": {
            "status": {"type": "string", "enum": ["complete"]}
        }, "required": ["status"], "additionalProperties": false}),
    }
}

/// Runs an `update_goal` call: the model marks the thread's goal complete.
pub(crate) fn update(thread: &mut Thread, arguments: &Arguments) -> ToolOutput {
    let (output, complete) = updated(arguments);
    if let Some(goal) = thread.goal.as_mut() {
        goal.claimed |= complete;
    }
    output
}

/// The result of an `update_goal` call, and whether it marked the goal complete.
fn updated(arguments: &Arguments) -> (ToolOutput, bool) {
    let status = match arguments {
        Arguments::Json(map) => map.get("status").and_then(|value| value.as_str()),
        Arguments::Malformed(_) => None,
    };
    match status {
        Some("complete") => (
            ToolOutput::text(
                false,
                "Goal marked complete. Finish your turn with a short report of what was done.",
            ),
            true,
        ),
        _ => (
            ToolOutput::text(
                true,
                "The only status you can set is \"complete\"; the person changes or stops the \
                 goal.",
            ),
            false,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn goal(check: Option<&str>) -> GoalProgress {
        GoalProgress {
            objective: "make the tests pass".into(),
            check: check.map(str::to_owned),
            token_budget: 1_000,
            tokens: 250,
            turns: 1,
            ended: None,
        }
    }

    #[test]
    fn a_continuation_restates_the_objective_and_the_budget() {
        let text = continuation(&goal(None), None);
        assert!(text.contains("<objective>\nmake the tests pass\n</objective>"));
        assert!(text.contains("250 of 1000 tokens used, 750 left"));
        assert!(text.contains("Before you mark the goal complete"));
    }

    #[test]
    fn a_failed_check_shows_the_end_of_its_output() {
        let long = format!("{}\nFAILED: two_plus_two", "x".repeat(10_000));
        let text = continuation(&goal(Some("cargo test")), Some(&long));
        assert!(text.contains("The check `cargo test` failed"));
        assert!(text.contains("FAILED: two_plus_two"));
        assert!(text.len() < 6_000, "only the end is shown");
        assert!(text.contains("met when the check passes"));
    }

    #[test]
    fn only_complete_is_a_status_the_model_sets() {
        let call = |status: &str| {
            let map = json!({"status": status}).as_object().cloned().unwrap();
            updated(&Arguments::Json(map))
        };
        assert!(call("complete").1);
        let (output, done) = call("paused");
        assert!(output.is_error && !done);
    }

    #[test]
    fn instructions_name_the_check_or_the_tool() {
        assert!(instructions(&goal(Some("npm test"))).contains("`npm test` passes"));
        assert!(instructions(&goal(None)).contains(UPDATE_GOAL));
    }
}
