//! Running a goal: its first turn, then a continuation after each turn until the check passes
//! or the model says it is met, the person stops it, its budget runs out, or it stalls.

use glave_event::{CallId, GoalEnded, GoalReason, GoalSet, Outcome, Principal, ThreadId};
use glave_ports::{BlobStore, EventLog, ModelProvider, Scope, ToolCall};
use serde_json::json;

use super::{Goal, GoalProgress, IDLE_TURNS, Pursuit, continuation, counted, instructions, latest};
use crate::approval::Approver;
use crate::caller::Caller;
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::repeats::Repeats;
use crate::tools::Tool;
use crate::turn::{Live, Prompt, Step, TurnSummary, absorb};

/// The longest a check may run, in seconds: the bash tool's own limit.
const CHECK_SECONDS: u64 = 600;

/// What a check found.
enum Checked {
    Passed,
    Failed(String),
    Cancelled,
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Sets `goal` on the thread, replacing any goal it had, and works toward it: the
    /// objective is the first turn's prompt, then each turn ends with a continuation until
    /// the goal ends. The summary adds up every turn. Cancelling stops the goal; an error
    /// leaves it active, to continue later with [`Harness::continue_goal`].
    pub async fn start_goal(
        &self,
        thread: &mut Thread,
        goal: Goal,
        caller: &Caller,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnSummary, HarnessError> {
        let set = GoalSet {
            objective: goal.objective.clone(),
            check: goal.check.clone(),
            token_budget: goal.token_budget,
            by: self.other_than_for(caller.person),
        };
        self.record(thread, &set).await?;
        thread.goal = Some(Pursuit::of(GoalProgress {
            objective: goal.objective.clone(),
            check: goal.check,
            token_budget: goal.token_budget,
            tokens: 0,
            turns: 0,
            ended: None,
        }));
        self.pursue(thread, Some(&goal.objective), caller, cancel, live)
            .await
    }

    /// Goes on working toward the thread's active goal, as after a restart: every turn,
    /// the first included, starts with a continuation.
    pub async fn continue_goal(
        &self,
        thread: &mut Thread,
        caller: &Caller,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnSummary, HarnessError> {
        if !thread.goal().is_some_and(GoalProgress::active) {
            return Err(HarnessError::NoGoal(thread.id));
        }
        self.pursue(thread, None, caller, cancel, live).await
    }

    /// Stops the thread's active goal for `caller`; false when it had none.
    pub async fn stop_goal(
        &self,
        thread: &mut Thread,
        caller: &Caller,
    ) -> Result<bool, HarnessError> {
        if !thread.goal().is_some_and(GoalProgress::active) {
            return Ok(false);
        }
        let by = self.other_than_for(caller.person);
        self.end_goal(thread, GoalReason::Stopped, by).await?;
        Ok(true)
    }

    /// Thread `id`'s latest goal, read from the log, whether it is running or not.
    pub async fn goal(
        &self,
        scope: Scope,
        id: ThreadId,
    ) -> Result<Option<GoalProgress>, HarnessError> {
        let events = self.load(&scope, id).await?;
        if events.is_empty() {
            return Err(HarnessError::NotFound(id));
        }
        Ok(latest(&events))
    }

    async fn pursue(
        &self,
        thread: &mut Thread,
        mut opening: Option<&str>,
        caller: &Caller,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnSummary, HarnessError> {
        let mut total = TurnSummary::started();
        let mut failed_check: Option<String> = None;
        let mut idle = 0;
        while let Some(goal) = thread.goal().filter(|goal| goal.active()).cloned() {
            if goal.spent() {
                self.end_goal(thread, GoalReason::Budget, None).await?;
                break;
            }
            let mut toolset = self.tools().await;
            toolset.system = format!("{}\n\n{}", toolset.system, instructions(&goal));
            if goal.check.is_none() {
                toolset.offer_goal();
            }
            let opened = opening.take();
            let text = match opened {
                Some(objective) => objective.to_owned(),
                None => continuation(&goal, failed_check.as_deref()),
            };
            let prompt = Prompt {
                text: &text,
                caller,
                by_glave: opened.is_none(),
            };
            set_running(thread, true);
            let ran = self.run_with(thread, prompt, toolset, cancel, live).await;
            set_running(thread, false);
            let turn = ran?;
            absorb(&mut total, &turn);
            total.outcome = turn.outcome;
            let claimed = note_turn(thread, &turn);
            let stopped = || self.other_than_for(caller.person);
            if turn.outcome == Outcome::Cancelled || cancel.is_cancelled() {
                self.end_goal(thread, GoalReason::Stopped, stopped())
                    .await?;
                break;
            }
            if let Some(check) = &goal.check {
                match self.check(thread, check, &mut total, cancel, live).await? {
                    Checked::Passed => {
                        self.end_goal(thread, GoalReason::MetByCheck, None).await?;
                        break;
                    }
                    Checked::Failed(output) => failed_check = Some(output),
                    Checked::Cancelled => {
                        self.end_goal(thread, GoalReason::Stopped, stopped())
                            .await?;
                        break;
                    }
                }
            } else if claimed {
                self.end_goal(thread, GoalReason::MetPerModel, None).await?;
                break;
            }
            idle = if turn.tool_calls == 0 { idle + 1 } else { 0 };
            if idle >= IDLE_TURNS {
                self.end_goal(thread, GoalReason::Stalled, None).await?;
                break;
            }
        }
        Ok(total)
    }

    /// Runs the goal's check through the bash tool, as the model's commands run: in the
    /// sandbox if there is one, asking the person when the approval mode says to.
    async fn check(
        &self,
        thread: &mut Thread,
        command: &str,
        total: &mut TurnSummary,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Checked, HarnessError> {
        let call = ToolCall {
            call_id: CallId::new(format!("check_{}", thread.next_seq.get()))?,
            tool: Tool::Bash.name().to_owned(),
            arguments: glave_event::Arguments::Json(
                json!({"command": command, "timeout_secs": CHECK_SECONDS})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        };
        let mut step = Step {
            toolset: self.tools().await,
            repeats: Repeats::default(),
            summary: TurnSummary::started(),
        };
        let output = self
            .call_tool(thread, &call, &mut step, cancel, live)
            .await?;
        absorb(total, &step.summary);
        // A judge asked about the check counts against the goal too.
        if let Some(goal) = thread.goal.as_mut() {
            let progress = &mut goal.progress;
            progress.tokens = progress
                .tokens
                .saturating_add(counted(&step.summary.tokens));
        }
        if cancel.is_cancelled() {
            return Ok(Checked::Cancelled);
        }
        if !output.is_error {
            return Ok(Checked::Passed);
        }
        let text: String = output
            .parts
            .iter()
            .filter_map(|part| match part {
                glave_event::Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Checked::Failed(text))
    }

    /// Records the goal's end with what it used.
    async fn end_goal(
        &self,
        thread: &mut Thread,
        reason: GoalReason,
        by: Option<Principal>,
    ) -> Result<(), HarnessError> {
        let tokens = thread.goal().map_or(0, |goal| goal.tokens);
        let by = by.or_else(|| (reason != GoalReason::Stopped).then(Principal::glave));
        self.record(thread, &GoalEnded { reason, tokens, by })
            .await?;
        if let Some(goal) = thread.goal.as_mut() {
            goal.progress.ended = Some(reason);
        }
        Ok(())
    }
}

fn set_running(thread: &mut Thread, running: bool) {
    if let Some(goal) = thread.goal.as_mut() {
        goal.running = running;
        if running {
            goal.claimed = false;
        }
    }
}

/// Adds a finished turn to the goal's progress; true when the model marked it complete.
fn note_turn(thread: &mut Thread, turn: &TurnSummary) -> bool {
    let Some(goal) = thread.goal.as_mut() else {
        return false;
    };
    let progress = &mut goal.progress;
    progress.tokens = progress.tokens.saturating_add(counted(&turn.tokens));
    progress.turns = progress.turns.saturating_add(1);
    std::mem::take(&mut goal.claimed)
}
