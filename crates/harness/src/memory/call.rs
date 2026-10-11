//! Running the model's call of a memory tool. Memory is first-party: saving needs no approval
//! and is never hidden, but after the thread has read untrusted content or a secrets file, a
//! save is a soft caution (`specs/memory.md`, "Safety"): in auto mode the judge lets it run
//! only when the person asked for it to be remembered, in ask mode the person is asked, and
//! with no one there nothing is saved. Memory never changes an approval or what the judge sees.

use glave_ports::{BlobStore, Effect, EventLog, ModelProvider, ToolCall, ToolOutput};

use super::state::structured;
use super::{EDIT_MEMORY, MemoryChange as Change, REMEMBER};
use crate::approval::{ApprovalRequest, Approver};
use crate::calls::{Flagged, Judged, NOT_RUN};
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::hooks::ToolHook;
use crate::turn::{Live, Step};

/// Why a save asks after untrusted content: told to the judge, the person and the model.
const AFTER_UNTRUSTED: &str = "this thread has read untrusted content or a secrets file, so \
     saving to memory now could carry a planted instruction into every later thread; it runs \
     only if the person asked for this to be remembered";

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
    /// Runs a call of `remember`, `edit` or `forget`.
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
        let summary = glave_ports::describe_call(name, &call.arguments);
        let hooked = ToolHook::of(call, name);
        let hook = self.stopped_by_hook(thread, &hooked, &summary, cancel, live);
        if let Some(stopped) = hook.await? {
            return Ok(stopped);
        }
        let writes = name == REMEMBER || name == EDIT_MEMORY;
        if writes && on.untrusted_read {
            let flagged = Flagged {
                call,
                name,
                summary: &summary,
                reason: AFTER_UNTRUSTED,
            };
            match self
                .judged(thread, &flagged, &mut step.summary, cancel)
                .await?
            {
                Judged::Cleared => {}
                Judged::Cancelled => return Ok(ToolOutput::text(true, NOT_RUN)),
                Judged::Ask => {
                    let request = ApprovalRequest {
                        thread: thread.id,
                        call_id: call.call_id.clone(),
                        tool: name.to_owned(),
                        effect: Effect::Changes,
                        summary: format!("{summary} ({AFTER_UNTRUSTED})"),
                        secret: false,
                        caution: Some(AFTER_UNTRUSTED.to_owned()),
                    };
                    if let Some(refused) = self.ask(thread, &request, cancel, live).await? {
                        return Ok(refused);
                    }
                }
            }
        }
        live(Live::ToolStarted {
            summary: summary.clone(),
        });
        let on = super::MemoryContext {
            seq: Some(thread.next_seq),
            ..on
        };
        let done = memory.call(&on, name, &call.arguments).await;
        if let Some(change) = &done.change {
            thread.memory.apply(change);
            thread.memory.structured = Some(structured(change));
            live(shown(change));
        }
        live(Live::ToolFinished {
            summary,
            is_error: done.output.is_error,
        });
        self.after_tool(thread, &hooked, done.output, cancel).await
    }
}

/// The tool's own name, whatever case or spacing the model used.
fn canonical(name: &str) -> &'static str {
    let name = name.trim().to_ascii_lowercase();
    match name.as_str() {
        n if n == REMEMBER => REMEMBER,
        n if n == EDIT_MEMORY => EDIT_MEMORY,
        _ => super::FORGET,
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
