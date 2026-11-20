//! A call of one of glave's own tools that reaches a service beyond the computer and spends
//! money, a media tool's or `web_search`'s: asked about as `web_fetch` is, its hooks first, and
//! told to the person as it starts.

use glave_ports::{BlobStore, Effect, EventLog, ModelProvider, ToolCall, ToolOutput};

use crate::approval::{ApprovalRequest, Approver, ask_of};
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::hooks::ToolHook;
use crate::turn::{Live, Step};

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Asks about `call` of `name`, described as `summary`: its hooks, then the person where
    /// this mode asks, or the answer the turn waited for. What refused it, when anything did;
    /// otherwise it is recorded as started and the person is told.
    pub(crate) async fn ask_beyond(
        &self,
        thread: &mut Thread,
        (call, name, summary): (&ToolCall, &str, &str),
        step: &mut Step,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Option<ToolOutput>, HarnessError> {
        let hooked = ToolHook::of(call, name);
        let refused = if let Some(answered) = step.waited(&call.call_id) {
            // Its hooks ran and it asked before the turn stopped: its answer decides.
            self.answered(thread, answered, cancel, live).await?
        } else if let Some(stopped) =
            (self.stopped_by_hook(thread, &hooked, summary, cancel, live)).await?
        {
            Some(stopped)
        } else {
            let request = ApprovalRequest {
                thread: thread.id,
                call_id: call.call_id.clone(),
                tool: name.to_owned(),
                effect: Effect::Runs,
                summary: summary.to_owned(),
                ask: ask_of(name),
                secret: false,
                caution: None,
                outside_sandbox: None,
                repeating: None,
            };
            let turn = &mut step.summary;
            self.approve(thread, &request, turn, cancel, live).await?
        };
        if refused.is_none() {
            self.record_started(thread, &call.call_id).await?;
            live(Live::ToolStarted {
                summary: summary.to_owned(),
            });
        }
        Ok(refused)
    }
}
