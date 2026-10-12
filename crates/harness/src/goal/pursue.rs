//! Running a goal: its first turn, then a continuation after each turn until the model says
//! it is met or paused, the person stops it, its budget runs out, or it stalls.

use glave_event::{GoalEnded, GoalReason, GoalResumed, GoalSet, Outcome, Principal, ThreadId};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, Scope};

use super::{Goal, GoalProgress, IDLE_TURNS, Pursuit, Verdict, continuation, counted, latest};
use crate::approval::Approver;
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::turn::{Live, Prompt, TurnSummary, absorb};

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
            token_budget: goal.token_budget,
            by: Self::other_than_for(thread, caller.person),
        };
        self.record(thread, &set).await?;
        thread.goal = Some(Pursuit::of(GoalProgress {
            objective: goal.objective.clone(),
            token_budget: goal.token_budget,
            tokens: 0,
            turns: 0,
            ended: None,
            note: None,
        }));
        self.pursue(thread, Some(&goal.objective), caller, cancel, live)
            .await
    }

    /// Goes on working toward the thread's goal, as after a restart, or resumes it for
    /// `caller` if the model paused it: every turn, the first included, starts with a
    /// continuation.
    pub async fn continue_goal(
        &self,
        thread: &mut Thread,
        caller: &Caller,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnSummary, HarnessError> {
        let Some(goal) = thread.goal().filter(|goal| goal.resumable()) else {
            return Err(HarnessError::NoGoal(thread.id));
        };
        if !goal.active() {
            let by = Self::other_than_for(thread, caller.person);
            self.record(thread, &GoalResumed { by }).await?;
            if let Some(goal) = thread.goal.as_mut() {
                goal.progress.ended = None;
                goal.progress.note = None;
            }
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
        let by = Self::other_than_for(thread, caller.person);
        self.end_goal(thread, GoalReason::Stopped, by, None).await?;
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
        let mut idle = 0;
        while let Some(goal) = thread.goal().filter(|goal| goal.active()).cloned() {
            if goal.spent() {
                self.end_goal(thread, GoalReason::Budget, None, None)
                    .await?;
                break;
            }
            let mut toolset = self.tools(thread).await;
            toolset.offer_goal();
            let opened = opening.take();
            let text = match opened {
                Some(objective) => objective.to_owned(),
                None => continuation(&goal),
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
            let verdict = note_turn(thread, &turn);
            if turn.outcome == Outcome::Cancelled || cancel.is_cancelled() {
                let by = Self::other_than_for(thread, caller.person);
                self.end_goal(thread, GoalReason::Stopped, by, None).await?;
                break;
            }
            match verdict {
                Some(Verdict::Complete) => {
                    self.end_goal(thread, GoalReason::Met, None, None).await?;
                    break;
                }
                Some(Verdict::Paused(note)) => {
                    self.end_goal(thread, GoalReason::Paused, None, Some(note))
                        .await?;
                    break;
                }
                None => {}
            }
            idle = if turn.tool_calls == 0 { idle + 1 } else { 0 };
            if idle >= IDLE_TURNS {
                self.end_goal(thread, GoalReason::Stalled, None, None)
                    .await?;
                break;
            }
        }
        Ok(total)
    }

    /// Records the goal's end with what it used, and the model's note when it paused.
    async fn end_goal(
        &self,
        thread: &mut Thread,
        reason: GoalReason,
        by: Option<Principal>,
        note: Option<String>,
    ) -> Result<(), HarnessError> {
        let tokens = thread.goal().map_or(0, |goal| goal.tokens);
        let by = by.or_else(|| (reason != GoalReason::Stopped).then(Principal::glave));
        let ended = GoalEnded {
            reason,
            tokens,
            by,
            note,
        };
        self.record(thread, &ended).await?;
        if let Some(goal) = thread.goal.as_mut() {
            goal.progress.ended = Some(reason);
            goal.progress.note = ended.note;
        }
        Ok(())
    }
}

fn set_running(thread: &mut Thread, running: bool) {
    if let Some(goal) = thread.goal.as_mut() {
        goal.running = running;
        if running {
            goal.verdict = None;
        }
    }
}

/// Adds a finished turn to the goal's progress, with how the model ended the goal, if it did.
fn note_turn(thread: &mut Thread, turn: &TurnSummary) -> Option<Verdict> {
    let goal = thread.goal.as_mut()?;
    let progress = &mut goal.progress;
    progress.tokens = progress.tokens.saturating_add(counted(&turn.tokens));
    progress.turns = progress.turns.saturating_add(1);
    goal.verdict.take()
}
