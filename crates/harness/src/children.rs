//! Running `agent` calls: each subagent in a child thread of its own, side by side, with what the person
//! sees of each child forwarded under the child's thread.

use std::sync::{Mutex, PoisonError};

use glave_event::{Outcome, ParentCall};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, ToolCall, ToolOutput};

use crate::approval::Approver;
use crate::calls::NOT_RUN;
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::join::{Running, all};
use crate::subagent::{self, AT_ONCE};
use crate::toolset::ToolSet;
use crate::turn::{Live, Prompt, TurnSummary};

/// What the person sees, shared by children running side by side.
type Shared<'a, 'b> = &'a Mutex<&'b mut (dyn FnMut(Live) + Send)>;

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Runs the `agent` calls of one response, at most `AT_ONCE` at a time. Each result comes
    /// back in its call's order, with the child's summary when it ran.
    pub(crate) async fn run_subagents(
        &self,
        parent: &Thread,
        calls: &[ToolCall],
        offer: &ToolSet,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Vec<(ToolOutput, Option<TurnSummary>)> {
        let shared = Mutex::new(live);
        let mut results = Vec::with_capacity(calls.len());
        for group in calls.chunks(AT_ONCE) {
            let runs: Vec<Running<'_, _>> = group
                .iter()
                .map(|call| -> Running<'_, _> {
                    Box::pin(self.run_subagent(parent, call, offer, cancel, &shared))
                })
                .collect();
            results.extend(all(runs).await);
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
    ) -> (ToolOutput, Option<TurnSummary>) {
        let asked = match subagent::asked(&call.arguments) {
            Ok(asked) => asked,
            Err(err) => return (ToolOutput::text(true, err), None),
        };
        let started = ParentCall {
            thread: parent.id,
            call_id: call.call_id.clone(),
        };
        let mut child = match self
            .start_thread_from(parent.scope, Some(started), &for_parent(parent), None)
            .await
        {
            Ok(child) => child,
            Err(err) => {
                return (
                    ToolOutput::text(true, format!("The subagent could not start: {err}")),
                    None,
                );
            }
        };
        // A child may do what the parent was allowed to, and no more, on the person's word.
        child.allowed = parent.allowed.clone();
        child.asked = parent.asked.clone();
        child.above.extend_from_slice(&parent.above);
        // A subagent works where its parent does.
        child.workspace.clone_from(&parent.workspace);
        child.note.clone_from(&parent.note);
        child.instructions.clone_from(&parent.instructions);
        child.project.clone_from(&parent.project);
        child.brought = std::sync::Arc::clone(&parent.brought);
        child.defaults = std::sync::Arc::clone(&parent.defaults);
        child.untrusted = parent.untrusted;
        child.read_secret = parent.read_secret;
        child.model.clone_from(&parent.model);
        child.approval = parent.approval;
        child.effort.clone_from(&parent.effort);
        let id = child.id;
        let send = |update: Live| {
            let mut live = live.lock().unwrap_or_else(PoisonError::into_inner);
            live(update);
        };
        send(Live::SubagentStarted {
            thread: id,
            description: asked.description.clone(),
        });
        let mut forward = |update: Live| {
            send(Live::Child {
                thread: id,
                update: Box::new(update),
            });
        };
        let tools = offer.for_child(asked.subagent_type);
        let brief = subagent::brief(&asked);
        // The parent's agent writes the brief: the child's `for` is implied, as for any prompt.
        let caller = Caller::default();
        let prompt = Prompt::new(&brief, &caller);
        let ran = self
            .run_with(&mut child, prompt, tools, cancel, &mut forward)
            .await;
        // What it left running in the background ends with its work.
        let _ = self.release_commands(&mut child).await;
        let result = match ran {
            Ok(summary) => {
                let failed = summary.outcome != Outcome::Completed;
                let report = subagent::report(&child.messages, summary.outcome);
                let mut output = ToolOutput::text(failed, report);
                // A child that read untrusted content may pass its words on: its report is
                // labelled as such, and the parent judged knowing it.
                output.untrusted = child.untrusted;
                (output, Some(summary))
            }
            Err(HarnessError::Cancelled) => (ToolOutput::text(true, NOT_RUN), None),
            Err(err) => (
                ToolOutput::text(true, format!("The subagent failed: {err}")),
                None,
            ),
        };
        send(Live::SubagentEnded {
            thread: id,
            ok: !result.0.is_error,
        });
        result
    }
}

/// Who starts a subagent's thread: the person its parent is for, so the child is for them too.
fn for_parent(parent: &Thread) -> Caller {
    Caller {
        person: parent.for_,
        auth: None,
        trace: None,
    }
}
