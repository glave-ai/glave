//! Running the model's call of a memory tool. Memory is first-party: saving needs no approval
//! and is never hidden, but after the thread has read untrusted content or a secrets file, a
//! save is a soft caution (`specs/memory.md`, "Safety"): in Auto the judge lets it run only
//! when the person asked for it to be remembered, otherwise the person is asked, and with no
//! one there nothing is saved; Full access asks no one and saves. Memory never changes an
//! approval or what the judge sees.

use glave_event::{Arguments, Caution, Risk};
use glave_ports::memories::{
    EDIT_MEMORY, FORGET, MemoryChange as Change, MemoryContext, MemoryOutput, RECALL, REMEMBER,
};
use glave_ports::{BlobStore, Effect, EventLog, ModelProvider, ToolCall, ToolOutput};

use super::state::recorded;
use crate::approval::{ApprovalRequest, Approver};
use crate::calls::{Judged, NOT_RUN};
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::hooks::ToolHook;
use crate::turn::{Live, Step};

/// What the person sees when a memory changes, in every client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryVerb {
    Remembered,
    Edited,
    Forgot,
}

impl MemoryVerb {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Remembered => "remembered",
            Self::Edited => "edited",
            Self::Forgot => "forgot",
        }
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Runs a call of `remember`, `edit`, `forget` or `recall`. A recall that found untrusted
    /// output comes back marked so, and the thread counts it as untrusted content read.
    pub(crate) async fn call_memory(
        &self,
        thread: &mut Thread,
        call: &ToolCall,
        step: &mut Step,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<ToolOutput, HarnessError> {
        let Some((memory, on)) = self.memory_for(thread) else {
            return Ok(ToolOutput::text(true, "Memory is off here."));
        };
        let name = canonical(&call.tool);
        let summary = described(&call.tool, &call.arguments);
        let hooked = ToolHook::of(call, name);
        let writes = name == REMEMBER || name == EDIT_MEMORY;
        let refused = if let Some(answered) = step.waited(&call.call_id) {
            // Its hooks ran and it asked before the turn stopped: its answer decides, as it
            // would have the moment it came.
            self.answered(thread, answered, cancel, live).await?
        } else if let Some(stopped) =
            (self.stopped_by_hook(thread, &hooked, &summary, cancel, live)).await?
        {
            Some(stopped)
        } else if writes && on.untrusted_read {
            let request = ApprovalRequest {
                thread: thread.id,
                call_id: call.call_id.clone(),
                tool: name.to_owned(),
                effect: Effect::Changes,
                summary: summary.clone(),
                secret: false,
                caution: Caution::of(Some(Risk::RemembersAfterUntrusted), None),
                outside_sandbox: None,
            };
            let turn = &mut step.summary;
            match self.judged(thread, &request, turn, cancel).await? {
                Judged::Cleared => None,
                Judged::Cancelled => Some(ToolOutput::text(true, NOT_RUN)),
                Judged::Ask => self.ask(thread, &request, cancel, live).await?,
            }
        } else {
            None
        };
        if let Some(refused) = refused {
            return Ok(refused);
        }
        live(Live::ToolStarted {
            summary: summary.clone(),
        });
        let on = MemoryContext {
            seq: Some(thread.next_seq),
            ..on
        };
        let MemoryOutput { output, change } = memory.call(&on, name, &call.arguments).await;
        if let Some(change) = &change {
            thread.memory.apply(change);
        }
        live(Live::ToolFinished {
            summary,
            is_error: output.is_error,
        });
        if let Some(change) = &change {
            live(shown(change));
        }
        let mut output = self.after_tool(thread, &hooked, output, cancel).await?;
        output.structured = Some(recorded(change.as_ref()));
        Ok(output)
    }
}

/// A call of memory tool `tool` in a line, by the tool's own name: `remember {"text":"…"}`.
pub(crate) fn described(tool: &str, arguments: &Arguments) -> String {
    glave_ports::describe_call(canonical(tool), arguments)
}

/// The tool's own name, whatever case or spacing the model used.
fn canonical(name: &str) -> &'static str {
    let name = name.trim().to_ascii_lowercase();
    match name.as_str() {
        n if n == REMEMBER => REMEMBER,
        n if n == EDIT_MEMORY => EDIT_MEMORY,
        n if n == RECALL => RECALL,
        _ => FORGET,
    }
}

/// The change as the person sees it: `remembered: "prefers tabs" · <id>`.
fn shown(change: &Change) -> Live {
    match change {
        Change::Saved { memory, .. } => Live::Memory {
            verb: MemoryVerb::Remembered,
            memory: memory.memory,
            text: Some(memory.text.clone()),
        },
        Change::Edited(memory) => Live::Memory {
            verb: MemoryVerb::Edited,
            memory: memory.memory,
            text: Some(memory.text.clone()),
        },
        Change::Forgotten(memory) => Live::Memory {
            verb: MemoryVerb::Forgot,
            memory: *memory,
            text: None,
        },
    }
}
