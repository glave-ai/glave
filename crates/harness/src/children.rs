//! Running `task` calls: each in a child thread of its own, side by side, with what the person
//! sees of each child forwarded under the child's thread.

use std::sync::{Mutex, PoisonError};

use glave_event::{Outcome, ParentCall};
use glave_ports::{BlobStore, EventLog, ModelProvider, ToolCall, ToolOutput};

use crate::approval::Approver;
use crate::calls::NOT_RUN;
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::join::{Running, all};
use crate::task::{self, AT_ONCE};
use crate::toolset::ToolSet;
use crate::turn::{Live, TurnSummary};

/// What the person sees, shared by children running side by side.
type Shared<'a, 'b> = &'a Mutex<&'b mut (dyn FnMut(Live) + Send)>;

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Runs the `task` calls of one response, at most `AT_ONCE` at a time. Each result comes
    /// back in its call's order, with the child's summary when it ran.
    pub(crate) async fn run_tasks(
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
                    Box::pin(self.run_task(parent, call, offer, cancel, &shared))
                })
                .collect();
            results.extend(all(runs).await);
        }
        results
    }

    async fn run_task(
        &self,
        parent: &Thread,
        call: &ToolCall,
        offer: &ToolSet,
        cancel: &Cancel,
        live: Shared<'_, '_>,
    ) -> (ToolOutput, Option<TurnSummary>) {
        let asked = match task::asked(&call.arguments) {
            Ok(asked) => asked,
            Err(err) => return (ToolOutput::text(true, err), None),
        };
        let started = ParentCall {
            thread: parent.id,
            call_id: call.call_id.clone(),
        };
        let mut child = match self.start_thread_from(parent.scope, Some(started)).await {
            Ok(child) => child,
            Err(err) => {
                return (
                    ToolOutput::text(true, format!("The task could not start: {err}")),
                    None,
                );
            }
        };
        // A child may do what the parent was allowed to, and no more.
        child.allowed = parent.allowed.clone();
        let id = child.id;
        let send = |update: Live| {
            let mut live = live.lock().unwrap_or_else(PoisonError::into_inner);
            live(update);
        };
        send(Live::TaskStarted {
            thread: id,
            description: asked.description.clone(),
        });
        let mut forward = |update: Live| {
            send(Live::Child {
                thread: id,
                update: Box::new(update),
            });
        };
        let tools = offer.for_child(asked.agent);
        let brief = task::brief(&asked);
        let ran = self
            .run_with(&mut child, &brief, tools, cancel, &mut forward)
            .await;
        let result = match ran {
            Ok(summary) => {
                let failed = summary.outcome != Outcome::Completed;
                let report = task::report(&child.messages, summary.outcome);
                (ToolOutput::text(failed, report), Some(summary))
            }
            Err(HarnessError::Cancelled) => (ToolOutput::text(true, NOT_RUN), None),
            Err(err) => (
                ToolOutput::text(true, format!("The task failed: {err}")),
                None,
            ),
        };
        send(Live::TaskEnded {
            thread: id,
            ok: !result.0.is_error,
        });
        result
    }
}
