//! Running a goal: its first turn, then a continuation after each turn until the model says
//! it is met or paused, the person stops it, its budget runs out, or it stalls.

use glave_event::{
    Arguments, Body, CallId, Event, GoalEnded, GoalReason, GoalResumed, GoalSet, Outcome,
    Principal, ThreadId, TurnResult,
};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, Scope};

use super::{
    Goal, GoalProgress, IDLE_TURNS, Pursuit, UPDATE_GOAL, Verdict, continuation, counted, latest,
    turn_at, update,
};
use crate::approval::Approver;
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::turn::{Live, Prompt};

/// Where a goal's turns start: a turn to run, with the objective when the goal is new, or one
/// that ran already and went on from the log after a wait.
enum First<'a> {
    Run(Option<&'a str>),
    Ran(TurnResult),
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
    /// leaves it active, to continue later with [`Harness::continue_goal`]. Refused in plan
    /// mode, whose turn stops at a plan for the person.
    pub async fn start_goal(
        &self,
        thread: &mut Thread,
        goal: Goal,
        caller: &Caller,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnResult, HarnessError> {
        if self.planning(thread) {
            return Err(HarnessError::GoalInPlan(thread.id));
        }
        self.close_left_open(thread).await?;
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
    /// continuation. Refused in plan mode, as [`Harness::start_goal`] is.
    pub async fn continue_goal(
        &self,
        thread: &mut Thread,
        caller: &Caller,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnResult, HarnessError> {
        if self.planning(thread) {
            return Err(HarnessError::GoalInPlan(thread.id));
        }
        self.close_left_open(thread).await?;
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
        self.goal_in(&scope, id, &events).await
    }

    /// The latest goal in `events`, thread `id`'s, with each of its turns counted as it was
    /// while the goal ran ([`Harness::turn_used`]).
    pub(crate) async fn goal_in(
        &self,
        scope: &Scope,
        id: ThreadId,
        events: &[Event],
    ) -> Result<Option<GoalProgress>, HarnessError> {
        let Some((mut progress, uncounted)) = latest(events) else {
            return Ok(None);
        };
        for started in uncounted {
            let used = self.turn_used(scope, id, turn_at(events, started)).await?;
            progress.tokens = progress.tokens.saturating_add(used);
        }
        Ok(Some(progress))
    }

    /// The tokens `turn`, thread `id`'s, used, as its log has it: its model calls and its
    /// subagents' ([`Harness::tallies`]). A goal counts each of its turns so, as it runs and
    /// read again alike.
    async fn turn_used(
        &self,
        scope: &Scope,
        id: ThreadId,
        turn: &[Event],
    ) -> Result<u64, HarnessError> {
        Ok(counted(&self.tallies(scope, id, turn).await?.tokens))
    }

    /// What `thread`'s latest turn used, as [`Harness::turn_used`] counts it.
    async fn last_turn_used(&self, thread: &Thread) -> Result<u64, HarnessError> {
        let events = self.load(&thread.scope, thread.id).await?;
        let started = events
            .iter()
            .rposition(|event| matches!(event.body, Body::TurnStarted(_)))
            .unwrap_or_default();
        let turn = turn_at(&events, started);
        self.turn_used(&thread.scope, thread.id, turn).await
    }

    async fn pursue(
        &self,
        thread: &mut Thread,
        opening: Option<&str>,
        caller: &Caller,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnResult, HarnessError> {
        self.pursue_from(thread, First::Run(opening), caller, cancel, live)
            .await
    }

    /// Goes on toward the thread's goal after `turn`, one of its turns that went on from the
    /// log after a wait (`going_on.rs`): taken in as the goal takes in each turn, then the
    /// goal's next turns.
    pub(crate) async fn pursue_after(
        &self,
        thread: &mut Thread,
        turn: TurnResult,
        caller: &Caller,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnResult, HarnessError> {
        self.pursue_from(thread, First::Ran(turn), caller, cancel, live)
            .await
    }

    /// The goal's turns until it ends, from `first`, then each with a continuation.
    async fn pursue_from(
        &self,
        thread: &mut Thread,
        first: First<'_>,
        caller: &Caller,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnResult, HarnessError> {
        let (mut opening, mut ran) = match first {
            First::Run(opening) => (opening, None),
            First::Ran(turn) => (None, Some(turn)),
        };
        let mut total = TurnResult::begun();
        let mut idle = 0;
        loop {
            let turn = match ran.take() {
                Some(turn) => turn,
                None => match self
                    .next_turn(thread, opening.take(), caller, cancel, live)
                    .await?
                {
                    Some(turn) => turn,
                    None => break,
                },
            };
            total.absorb(&turn);
            total.outcome = turn.outcome;
            let used = self.last_turn_used(thread).await?;
            let verdict = note_turn(thread, used);
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

    /// Runs the goal's next turn, the objective's when `opening` gives it, else a
    /// continuation's: `None` once the goal ended or spent its budget, or is paused because
    /// the thread went into plan mode while it waited.
    async fn next_turn(
        &self,
        thread: &mut Thread,
        opening: Option<&str>,
        caller: &Caller,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Option<TurnResult>, HarnessError> {
        let Some(goal) = thread.goal().filter(|goal| goal.active()).cloned() else {
            return Ok(None);
        };
        if goal.spent() {
            self.end_goal(thread, GoalReason::Budget, None, None)
                .await?;
            return Ok(None);
        }
        if self.planning(thread) {
            self.end_goal(thread, GoalReason::Paused, None, None)
                .await?;
            return Ok(None);
        }
        self.bring(thread).await?;
        let mut toolset = self.tools(thread).await;
        toolset.offer_goal();
        let text = match opening {
            Some(objective) => objective.to_owned(),
            None => continuation(&goal),
        };
        let prompt = Prompt {
            text: &text,
            files: &[],
            caller,
            by_glave: opening.is_none(),
            id: None,
        };
        set_running(thread, true);
        let ran = self.run_with(thread, prompt, toolset, cancel, live).await;
        set_running(thread, false);
        ran.map(Some)
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

/// Whether the turn that starts at `started` among `events`, thread's, is a turn of its goal:
/// one that began while the goal it has now was set and running.
pub(crate) fn of_turn(thread: &Thread, events: &[Event], started: usize) -> bool {
    let active = thread.goal().is_some_and(GoalProgress::active);
    let set = events
        .iter()
        .rposition(|event| matches!(event.body, Body::GoalSet(_) | Body::GoalResumed(_)));
    active && set.is_some_and(|set| set < started)
}

/// A goal's turn goes on from its log, its events so far `turn`, which used what `so_far`
/// tallies: it runs as the goal's turn, with how the model ended the goal in it already, if it
/// did, and is counted once it ends, as any of the goal's turns is (the log counted it, and
/// what it used so far, already).
pub(crate) fn going_on(thread: &mut Thread, turn: &[Event], so_far: &TurnResult) {
    set_running(thread, true);
    let completed: Vec<&CallId> = turn
        .iter()
        .filter_map(|event| match &event.body {
            Body::ToolCallCompleted(done) => Some(&done.call_id),
            _ => None,
        })
        .collect();
    let used: Vec<Arguments> = turn
        .iter()
        .filter_map(|event| match &event.body {
            Body::ToolCallRequested(call)
                if call.tool.trim().eq_ignore_ascii_case(UPDATE_GOAL)
                    && completed.contains(&&call.call_id) =>
            {
                Some(call.arguments.clone())
            }
            _ => None,
        })
        .collect();
    for arguments in &used {
        update(thread, arguments);
    }
    let spent = counted(&so_far.tokens);
    if let Some(goal) = thread.goal.as_mut() {
        goal.progress.tokens = goal.progress.tokens.saturating_sub(spent);
        goal.progress.turns = goal.progress.turns.saturating_sub(1);
    }
}

/// The goal's turn that went on from the log has ended.
pub(crate) fn stopped_running(thread: &mut Thread) {
    set_running(thread, false);
}

fn set_running(thread: &mut Thread, running: bool) {
    if let Some(goal) = thread.goal.as_mut() {
        goal.running = running;
        if running {
            goal.verdict = None;
        }
    }
}

/// Adds a finished turn, which used `used` tokens, to the goal's progress, with how the model
/// ended the goal, if it did.
fn note_turn(thread: &mut Thread, used: u64) -> Option<Verdict> {
    let goal = thread.goal.as_mut()?;
    let progress = &mut goal.progress;
    progress.tokens = progress.tokens.saturating_add(used);
    progress.turns = progress.turns.saturating_add(1);
    goal.verdict.take()
}
