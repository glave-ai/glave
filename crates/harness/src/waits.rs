//! A turn as its log left it open (`specs/one-server.md`, "Durable turns"): stopped at a wait
//! (an approval asked and not decided, the model's questions, or subagents' work while one of
//! them waits), or left open by a glave that stopped mid-call. A waiting turn holds nothing: an
//! answer is recorded in the log exactly as the turn would record it, and the turn goes on from
//! the log ([`Harness::go_on`]). What a turn did so far is read from the log too.

use std::collections::{HashMap, HashSet};

use glave_event::{
    ApprovalDecided, ApprovalRequested, Body, CallId, Event, Outcome, ParentCall, Part, Payload,
    Principal, PrincipalId, Seq, ThreadId, ToolCallCompleted, Verdict, decode,
};
use glave_ports::{
    BlobStore, CallKind, EventLog, LogError, ModelProvider, Scope, Standing, ThreadQuery,
    ThreadView, ToolCall, ToolOutput, WaitKind,
};

use crate::approval::{Approver, Decision};
use crate::harness::{Harness, HarnessError};
use crate::turn::{TurnSummary, absorb};

/// A turn its log has open: where it started, and its calls without a result, in order, with
/// the approval asked for the first of them when one was.
#[derive(Debug, Clone)]
pub(crate) struct Open {
    /// Where its `turn.started` is among the thread's events.
    pub(crate) started: usize,
    pub(crate) calls: Vec<ToolCall>,
    pub(crate) asked: Option<Answered>,
}

/// The approval a turn stopped at, and its answer once one is recorded.
#[derive(Debug, Clone)]
pub(crate) struct Answered {
    pub(crate) asked: ApprovalRequested,
    pub(crate) decided: Option<ApprovalDecided>,
}

/// How a thread's log left its turn: waiting, answered and going on, or left open by a glave
/// that stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Opened {
    Waits,
    /// Its wait was answered, its call not yet run: the work that goes on with it runs it. As
    /// a server starts, one is a crash's leftover, and is closed as one left open is.
    Answered,
    /// Its log, as read up to this event, ends in a turn no one ended.
    LeftOpen(Seq),
}

/// The turn `events` leave open, if they do.
pub(crate) fn open_turn(events: &[Event]) -> Option<Open> {
    let started = events
        .iter()
        .rposition(|event| matches!(event.body, Body::TurnStarted(_)))?;
    let turn = events.get(started..)?;
    if turn
        .iter()
        .any(|event| matches!(event.body, Body::TurnEnded(_)))
    {
        return None;
    }
    let completed: HashSet<&CallId> = turn
        .iter()
        .filter_map(|event| match &event.body {
            Body::ToolCallCompleted(done) => Some(&done.call_id),
            _ => None,
        })
        .collect();
    let calls: Vec<ToolCall> = turn
        .iter()
        .filter_map(|event| match &event.body {
            Body::ToolCallRequested(call) if !completed.contains(&call.call_id) => Some(ToolCall {
                call_id: call.call_id.clone(),
                tool: call.tool.clone(),
                arguments: call.arguments.clone(),
            }),
            _ => None,
        })
        .collect();
    let asked = calls
        .first()
        .and_then(|first| answered(turn, &first.call_id));
    Some(Open {
        started,
        calls,
        asked,
    })
}

/// The approval asked for `call` in `turn`, with its decision when one came.
fn answered(turn: &[Event], call: &CallId) -> Option<Answered> {
    let asked = turn.iter().find_map(|event| match &event.body {
        Body::ApprovalRequested(asked) if asked.call_id == *call => Some(asked.clone()),
        _ => None,
    })?;
    let decided = turn.iter().find_map(|event| match &event.body {
        Body::ApprovalDecided(decided) if decided.call_id == *call => Some(decided.clone()),
        _ => None,
    });
    Some(Answered { asked, decided })
}

/// The decision recorded for `decision`: allowing a command outside the sandbox for the
/// thread allows it once, as every such command asks.
pub(crate) fn verdict_of(decision: Decision, outside_sandbox: bool) -> Verdict {
    match decision {
        Decision::AllowOnce => Verdict::AllowOnce,
        Decision::AllowForThread if outside_sandbox => Verdict::AllowOnce,
        Decision::AllowForThread => Verdict::AllowForThread,
        Decision::Deny => Verdict::Deny,
        Decision::Unavailable => Verdict::NoOneThere,
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// How `events`, thread `id`'s, leave its turn: `None` when nothing is open.
    pub(crate) async fn opened(
        &self,
        scope: &Scope,
        id: ThreadId,
        events: &[Event],
    ) -> Result<Option<Opened>, HarnessError> {
        let left = events
            .last()
            .map(|event| Opened::LeftOpen(event.header.seq));
        let Some(open) = open_turn(events) else {
            return Ok(left.filter(|_| dangling(events)));
        };
        let Some(first) = open.calls.first() else {
            return Ok(left);
        };
        Ok(match (&open.asked, CallKind::of(&first.tool)) {
            (Some(answered), _) if answered.decided.is_some() => Some(Opened::Answered),
            (Some(_), _) | (None, CallKind::Question) => Some(Opened::Waits),
            (None, CallKind::Agent) if self.a_child_waits(scope, id, &open.calls).await? => {
                Some(Opened::Waits)
            }
            (None, CallKind::Agent | CallKind::Tool) => left,
        })
    }

    /// Whether a subagent of the calls side by side that `calls` start with waits, or was
    /// answered and goes on.
    async fn a_child_waits(
        &self,
        scope: &Scope,
        parent: ThreadId,
        calls: &[ToolCall],
    ) -> Result<bool, HarnessError> {
        let agents = calls
            .iter()
            .take_while(|call| CallKind::of(&call.tool) == CallKind::Agent);
        for call in agents {
            let child = self.child_of(scope, parent, &call.call_id).await?;
            let Some(turn) = child.and_then(|view| view.turn) else {
                continue;
            };
            let waits = turn.wait.is_some_and(|wait| wait.kind != WaitKind::Agent);
            let answered = turn.unfinished.first().is_some_and(|call| call.decided);
            if waits || answered {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The view of the subagent's thread `call` in `parent` started, if it started one.
    pub(crate) async fn child_of(
        &self,
        scope: &Scope,
        parent: ThreadId,
        call: &CallId,
    ) -> Result<Option<ThreadView>, HarnessError> {
        let query = ThreadQuery {
            standing: Standing::Active,
            limit: 1,
            after: None,
            open: false,
            parent: Some(ParentCall {
                thread: parent,
                call_id: call.clone(),
            }),
        };
        Ok(self
            .log
            .thread_views(scope, &query)
            .await?
            .into_iter()
            .next())
    }

    /// What the turn that starts at `turn`'s first event did so far, its subagents' work added
    /// in, as it would have counted it.
    pub(crate) async fn tallies(
        &self,
        scope: &Scope,
        thread: ThreadId,
        turn: &[Event],
    ) -> Result<TurnSummary, HarnessError> {
        let mut summary = own_tallies(turn);
        let agents: Vec<&CallId> = turn
            .iter()
            .filter_map(|event| match &event.body {
                Body::ToolCallRequested(call) if CallKind::of(&call.tool) == CallKind::Agent => {
                    Some(&call.call_id)
                }
                _ => None,
            })
            .collect();
        for call in agents {
            if let Some(child) = self.child_of(scope, thread, call).await? {
                let events = self.load(scope, child.thread).await?;
                absorb(&mut summary, &own_tallies(&events));
            }
        }
        Ok(summary)
    }

    /// What a subagent's call returns once its turn has ended, read from its thread: its last
    /// answer with how it ended, labelled untrusted when it read untrusted content, and what
    /// its turn did.
    pub(crate) async fn reported(
        &self,
        scope: &Scope,
        child: ThreadId,
    ) -> Result<(ToolOutput, TurnSummary), HarnessError> {
        let events = self.load(scope, child).await?;
        let summary = own_tallies(&events);
        let messages = crate::conversation::conversation(&events);
        let report = crate::subagent::report(&messages, summary.outcome);
        let mut output = ToolOutput::text(summary.outcome != Outcome::Completed, report);
        output.untrusted = events.iter().any(|event| match &event.body {
            Body::ToolCallCompleted(done) => done.untrusted,
            _ => false,
        });
        Ok((output, summary))
    }

    /// Records `decision`, by `person` when that is not the thread's own, for the approval
    /// `call` waits for in thread `id`, exactly as the turn waiting for it would: refused when
    /// no turn there waits for it, or another answer came first. The turn goes on from the log
    /// ([`Harness::go_on`]).
    pub async fn decide(
        &self,
        scope: Scope,
        id: ThreadId,
        call: &CallId,
        decision: Decision,
        person: Option<PrincipalId>,
    ) -> Result<(), HarnessError> {
        let refused = || HarnessError::NoApproval(call.clone());
        let (view, wait) = self.waiting(&scope, id, call, WaitKind::Approval).await?;
        let wait = wait.ok_or_else(refused)?;
        let asked = self.event_at(&scope, id, wait).await?;
        let Body::ApprovalRequested(asked) = asked.body else {
            return Err(refused());
        };
        let decided = ApprovalDecided {
            call_id: call.clone(),
            decision: verdict_of(decision, asked.outside_sandbox.is_some()),
            by: by(&view, person),
        };
        crate::telemetry::approval(&asked.tool, decided.decision);
        self.answer_with(scope, id, &view, &decided, refused).await
    }

    /// Records `answers` to the questions of `call` in thread `id`, as the call's result,
    /// exactly as the turn waiting for them would: refused when no turn there waits for them,
    /// or another answer came first. The turn goes on from the log ([`Harness::go_on`]).
    pub async fn answer(
        &self,
        scope: Scope,
        id: ThreadId,
        call: &CallId,
        answers: &[String],
    ) -> Result<(), HarnessError> {
        let refused = || HarnessError::NoQuestion(call.clone());
        let (view, wait) = self.waiting(&scope, id, call, WaitKind::Question).await?;
        let wait = wait.ok_or_else(refused)?;
        let asked = self.event_at(&scope, id, wait).await?;
        let Body::ToolCallRequested(asked) = asked.body else {
            return Err(refused());
        };
        let questions = crate::question::questions(&asked.arguments).map_err(|_| refused())?;
        let completed = ToolCallCompleted {
            call_id: call.clone(),
            is_error: false,
            output: vec![Part::Text {
                text: crate::question::answered(&questions, answers),
            }],
            structured: None,
            untrusted: false,
        };
        self.answer_with(scope, id, &view, &completed, refused)
            .await
    }

    /// Thread `id`'s view, and the `seq` its turn waits at when it waits at `call` for an
    /// answer of `kind`.
    async fn waiting(
        &self,
        scope: &Scope,
        id: ThreadId,
        call: &CallId,
        kind: WaitKind,
    ) -> Result<(ThreadView, Option<Seq>), HarnessError> {
        let view = self
            .log
            .thread_view(scope, id)
            .await?
            .ok_or(HarnessError::NotFound(id))?;
        let wait = view
            .turn
            .as_ref()
            .and_then(|turn| turn.wait.as_ref())
            .filter(|wait| wait.kind == kind && wait.call == *call)
            .map(|wait| wait.seq);
        Ok((view, wait))
    }

    /// Thread `id`'s event at `seq`.
    async fn event_at(&self, scope: &Scope, id: ThreadId, seq: Seq) -> Result<Event, HarnessError> {
        let before = Seq::new(seq.get().saturating_sub(1)).ok();
        let raw = self.log.read(scope, id, before, 1).await?;
        let raw = raw.into_iter().next().ok_or(HarnessError::NotFound(id))?;
        decode(raw).map_err(|e| HarnessError::Event(e.to_string()))
    }

    /// Appends `answer` right after `view`'s last event: the first answer wins, and one that
    /// meets another's write is refused as `refused` says.
    async fn answer_with<T: Payload>(
        &self,
        scope: Scope,
        id: ThreadId,
        view: &ThreadView,
        answer: &T,
        refused: impl Fn() -> HarnessError,
    ) -> Result<(), HarnessError> {
        let next = view
            .last_seq
            .next()
            .ok_or_else(|| HarnessError::Event(format!("thread {id} is full")))?;
        let mut staged = Vec::new();
        self.stage_next(id, next, &mut staged, answer)?;
        match self.log.append(&scope, &staged).await {
            Err(LogError::Conflict { .. }) => Err(refused()),
            appended => Ok(appended?),
        }
    }
}

/// Who answered, as a decision records them: absent for the person the thread is for.
fn by(view: &ThreadView, person: Option<PrincipalId>) -> Option<Principal> {
    let theirs = view.for_.as_ref().and_then(|whose| whose.id);
    person
        .filter(|person| Some(*person) != theirs)
        .map(Principal::person)
}

/// Whether `events` hold a call with no result outside any open turn.
fn dangling(events: &[Event]) -> bool {
    let completed: HashSet<&CallId> = events
        .iter()
        .filter_map(|event| match &event.body {
            Body::ToolCallCompleted(done) => Some(&done.call_id),
            _ => None,
        })
        .collect();
    events.iter().any(|event| match &event.body {
        Body::ToolCallRequested(call) => !completed.contains(&call.call_id),
        _ => false,
    })
}

/// What the latest turn in `events` did by itself: its model calls, tool calls, commands run
/// outside the sandbox, tokens and cost, and how it ended (completed while it has not).
pub(crate) fn own_tallies(events: &[Event]) -> TurnSummary {
    let started = events
        .iter()
        .rposition(|event| matches!(event.body, Body::TurnStarted(_)))
        .unwrap_or(0);
    let turn = events.get(started..).unwrap_or_default();
    let mut summary = TurnSummary::started();
    let mut outside: HashMap<&CallId, bool> = HashMap::new();
    for event in turn {
        match &event.body {
            Body::ModelResponded(responded) => {
                summary.model_calls = summary.model_calls.saturating_add(1);
                summary.tokens += responded.usage.counts();
                summary.estimated |= responded.usage.estimated();
                summary.cost = summary
                    .cost
                    .zip(responded.cost.as_ref())
                    .and_then(|(sum, cost)| sum.checked_add(cost.amount));
            }
            Body::ToolCallCompleted(_) => {
                summary.tool_calls = summary.tool_calls.saturating_add(1);
            }
            Body::ApprovalRequested(asked) => {
                outside.insert(&asked.call_id, asked.outside_sandbox.is_some());
            }
            Body::ApprovalDecided(decided)
                if outside.get(&decided.call_id).copied().unwrap_or(false)
                    && matches!(
                        decided.decision,
                        Verdict::AllowOnce | Verdict::AllowForThread
                    ) =>
            {
                summary.outside_sandbox = summary.outside_sandbox.saturating_add(1);
            }
            Body::TurnEnded(ended) => summary.outcome = ended.outcome,
            _ => {}
        }
    }
    summary
}
