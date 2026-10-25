//! Running `agent` calls: each subagent in a child thread of its own, side by side, with what the
//! person sees of each child forwarded under the child's thread. A call whose child already
//! started, before a wait or a restart, never starts another: an ended child's report is the
//! call's result, and one stopped at a wait goes on from its log (`going_on.rs`).

use std::sync::{Mutex, PoisonError};

use glave_event::{Outcome, ParentCall, ThreadId};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, ThreadView, ToolCall, ToolOutput};

use crate::approval::Approver;
use crate::calls::NOT_RUN;
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::join::{Running, all};
use crate::subagent::{self, AT_ONCE, Asked};
use crate::toolset::ToolSet;
use crate::turn::{Live, Prompt, TurnSummary};

/// What the person sees, shared by children running side by side.
type Shared<'a, 'b> = &'a Mutex<&'b mut (dyn FnMut(Live) + Send)>;

/// How a subagent's call came out.
pub(crate) enum Ran {
    /// Its result, with what its child's turn did when it ran.
    Done(ToolOutput, Option<TurnSummary>),
    /// Its child's turn waits for the person, so its parent's waits too.
    Waits,
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Runs the `agent` calls of one response, at most `AT_ONCE` at a time, each result in its
    /// call's order. A group where a child waits ends the run there: the groups after it start
    /// once the turn goes on.
    pub(crate) async fn run_subagents(
        &self,
        parent: &Thread,
        calls: &[ToolCall],
        offer: &ToolSet,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Vec<Ran> {
        let shared = Mutex::new(live);
        let mut results = Vec::with_capacity(calls.len());
        for group in calls.chunks(AT_ONCE) {
            let runs: Vec<Running<'_, _>> = group
                .iter()
                .map(|call| -> Running<'_, _> {
                    Box::pin(self.run_subagent(parent, call, offer, cancel, &shared))
                })
                .collect();
            let ran = all(runs).await;
            let waits = ran.iter().any(|ran| matches!(ran, Ran::Waits));
            results.extend(ran);
            if waits {
                break;
            }
        }
        results
    }

    async fn run_subagent(
        &self,
        parent: &Thread,
        call: &ToolCall,
        offer: &ToolSet,
        cancel: &Cancel,
        live: Shared<'_, '_>,
    ) -> Ran {
        let asked = match subagent::asked(&call.arguments) {
            Ok(asked) => asked,
            Err(err) => return Ran::Done(ToolOutput::text(true, err), None),
        };
        let send = |update: Live| {
            let mut live = live.lock().unwrap_or_else(PoisonError::into_inner);
            live(update);
        };
        let tools = offer.for_child(asked.subagent_type);
        let forwarding = |id: ThreadId| {
            move |update: Live| {
                send(Live::Child {
                    thread: id,
                    update: Box::new(update),
                });
            }
        };
        let existing = self.child_of(&parent.scope, parent.id, &call.call_id).await;
        let (mut child, ran) = match existing {
            Err(err) => return Ran::Done(failed(&err), None),
            // Ended, here or before a restart: its report is the call's result.
            Ok(Some(view)) if view.turn.is_none() => return self.report_of(parent, &view).await,
            // Stopped part way: it goes on from its log, never started again.
            Ok(Some(view)) => {
                let mut child = match self.resume_child(parent, &view).await {
                    Ok(Some(child)) => child,
                    Ok(None) => return self.report_of(parent, &view).await,
                    Err(err) => return Ran::Done(failed(&err), None),
                };
                let ran = self
                    .go_on_with(
                        &mut child,
                        Some(tools),
                        &for_parent(parent),
                        cancel,
                        &mut forwarding(view.thread),
                    )
                    .await;
                (child, ran)
            }
            Ok(None) if cancel.is_cancelled() => {
                return Ran::Done(ToolOutput::text(true, NOT_RUN), None);
            }
            Ok(None) => {
                let mut child = match self.new_child(parent, call).await {
                    Ok(child) => child,
                    Err(err) => {
                        let text = format!("The subagent could not start: {err}");
                        return Ran::Done(ToolOutput::text(true, text), None);
                    }
                };
                let id = child.id;
                send(Live::SubagentStarted {
                    thread: id,
                    description: asked.description.clone(),
                });
                let ran = self
                    .first_turn(&mut child, &asked, tools, cancel, &mut forwarding(id))
                    .await;
                (child, ran)
            }
        };
        if matches!(ran, Err(HarnessError::Waits(_))) {
            return Ran::Waits;
        }
        // What it left running in the background ends with its work.
        let _ = self.release_commands(&mut child).await;
        let result = result_of(&child, ran);
        let ok = matches!(&result, Ran::Done(output, _) if !output.is_error);
        send(Live::SubagentEnded {
            thread: child.id,
            ok,
        });
        result
    }

    /// A new child thread for `call` in `parent`, working where and as its parent does.
    async fn new_child(&self, parent: &Thread, call: &ToolCall) -> Result<Thread, HarnessError> {
        let started = ParentCall {
            thread: parent.id,
            call_id: call.call_id.clone(),
        };
        let mut child = self
            .start_thread_from(parent.scope, Some(started), &for_parent(parent), None)
            .await?;
        inherit(&mut child, parent);
        Ok(child)
    }

    /// The child `view` names, read from its log to go on, working where and as its parent
    /// does: `None` once what it left open is closed, a crash having stopped it mid-call, so its
    /// report is the call's result.
    async fn resume_child(
        &self,
        parent: &Thread,
        view: &ThreadView,
    ) -> Result<Option<Thread>, HarnessError> {
        let mut child = self.resume_thread(parent.scope, view.thread).await?;
        if child.left_open() {
            self.repair(parent.scope, view.thread).await?;
            return Ok(None);
        }
        inherit(&mut child, parent);
        Ok(Some(child))
    }

    /// The call's result once its child's turn has ended: the child's report from its log.
    async fn report_of(&self, parent: &Thread, view: &ThreadView) -> Ran {
        match self.reported(&parent.scope, view.thread).await {
            Ok((output, summary)) => Ran::Done(output, Some(summary)),
            Err(err) => Ran::Done(failed(&err), None),
        }
    }

    /// A new child's one turn: the brief its parent's agent wrote, with `tools`.
    async fn first_turn(
        &self,
        child: &mut Thread,
        asked: &Asked,
        tools: ToolSet,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnSummary, HarnessError> {
        let brief = subagent::brief(asked);
        // The parent's agent writes the brief: the child's `for` is implied, as for any prompt.
        let caller = Caller::default();
        let prompt = Prompt::new(&brief, &caller);
        self.run_with(child, prompt, tools, cancel, live).await
    }
}

/// A child works where its parent does, and may do what the parent was allowed to, and no
/// more, on the person's word.
fn inherit(child: &mut Thread, parent: &Thread) {
    child.allowed.extend(parent.allowed.iter().cloned());
    child.asked.clone_from(&parent.asked);
    child.above.extend_from_slice(&parent.above);
    child.workspace.clone_from(&parent.workspace);
    child.note.clone_from(&parent.note);
    child.instructions.clone_from(&parent.instructions);
    child.project.clone_from(&parent.project);
    child.brought = std::sync::Arc::clone(&parent.brought);
    child.defaults = std::sync::Arc::clone(&parent.defaults);
    child.untrusted |= parent.untrusted;
    child.read_secret |= parent.read_secret;
    child.model.clone_from(&parent.model);
    child.permissions = parent.permissions;
    child.effort.clone_from(&parent.effort);
}

/// What a subagent's call returns once its turn in `child` ended as `ran` says.
fn result_of(child: &Thread, ran: Result<TurnSummary, HarnessError>) -> Ran {
    match ran {
        Ok(summary) => {
            let failed = summary.outcome != Outcome::Completed;
            let report = subagent::report(&child.messages, summary.outcome);
            let mut output = ToolOutput::text(failed, report);
            // A child that read untrusted content may pass its words on: its report is
            // labelled as such, and the parent judged knowing it.
            output.untrusted = child.untrusted;
            Ran::Done(output, Some(summary))
        }
        Err(HarnessError::Cancelled) => Ran::Done(ToolOutput::text(true, NOT_RUN), None),
        Err(err) => Ran::Done(failed(&err), None),
    }
}

/// The result of a subagent's call that failed.
fn failed(err: &HarnessError) -> ToolOutput {
    ToolOutput::text(true, format!("The subagent failed: {err}"))
}

/// Who starts a subagent's thread: the person its parent is for, so the child is for them too.
fn for_parent(parent: &Thread) -> Caller {
    Caller {
        person: parent.for_,
        auth: None,
        trace: None,
    }
}
