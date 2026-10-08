//! Running one of the model's tool calls: asking the person when the call needs it, running
//! the tool, and recording its result.

use glave_event::{ApprovalDecided, ApprovalRequested, CallId, Part, ToolCallCompleted, Verdict};
use glave_ports::{BlobStore, EventLog, Message, ModelProvider, ToolCall};

use crate::approval::{ApprovalRequest, Approver, Decision, no_one_there, reads_secret, summarize};
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::tools::{Tool, ToolOutcome};
use crate::turn::Live;

/// The result of a tool call the turn never ran because the person cancelled it.
pub(crate) const NOT_RUN: &str = "Not run: the person cancelled the turn.";

const DECLINED: &str = "The person declined this action. Ask them what they would like \
     instead, or take a different approach.";

const DECLINED_SECRET: &str = "Reading this file was declined: it usually holds secrets such \
     as keys or passwords. Do not try to get its contents another way. If you need a specific \
     value, ask the person for it.";

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Runs one tool call, asking the person first when it would change anything or show the
    /// model a secret file.
    pub(crate) async fn call_tool(
        &self,
        thread: &mut Thread,
        call: &ToolCall,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<ToolOutcome, HarnessError> {
        let Some(tool) = Tool::named(&call.tool) else {
            let names: Vec<&str> = Tool::ALL.iter().map(|tool| tool.name()).collect();
            return Ok(ToolOutcome::error(format!(
                "There is no tool named {:?}. The tools are {}.",
                call.tool,
                names.join(", ")
            )));
        };
        let summary = summarize(tool, &call.arguments);
        let secret = reads_secret(&self.workspace, tool, &call.arguments).await;
        if (self.settings.approval.asks_for(tool) || secret) && !thread.allowed.contains(&tool) {
            let request = ApprovalRequest {
                thread: thread.id,
                call_id: call.call_id.clone(),
                tool,
                summary: if secret {
                    format!("{summary} (this file usually holds secrets; the model would see them)")
                } else {
                    summary.clone()
                },
                secret,
            };
            if let Some(refused) = self.ask(thread, &request, cancel, live).await? {
                return Ok(refused);
            }
        }
        live(Live::ToolStarted {
            summary: summary.clone(),
        });
        let outcome = tool.run(&call.arguments, &self.workspace, cancel).await;
        live(Live::ToolFinished {
            summary,
            is_error: outcome.is_error,
        });
        Ok(outcome)
    }

    /// Asks the person about `request`, recording the question and the answer. `None` when the
    /// call may go ahead; otherwise the result the model is given instead.
    async fn ask(
        &self,
        thread: &mut Thread,
        request: &ApprovalRequest,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Option<ToolOutcome>, HarnessError> {
        let asked = ApprovalRequested {
            call_id: request.call_id.clone(),
            tool: request.tool.name().to_owned(),
            summary: request.summary.clone(),
            secret: request.secret,
        };
        self.record(thread, &asked).await?;
        let decision = cancel.run(self.approver.decide(request)).await;
        let verdict = match decision {
            None => Verdict::Cancelled,
            Some(Decision::AllowOnce) => Verdict::AllowOnce,
            Some(Decision::AllowForThread) => Verdict::AllowForThread,
            Some(Decision::Deny) => Verdict::Deny,
            Some(Decision::Unavailable) => Verdict::NoOneThere,
        };
        let decided = ApprovalDecided {
            call_id: request.call_id.clone(),
            decision: verdict,
        };
        self.record(thread, &decided).await?;
        Ok(match verdict {
            Verdict::AllowOnce => None,
            Verdict::AllowForThread => {
                thread.allowed.insert(request.tool);
                None
            }
            Verdict::Cancelled => Some(ToolOutcome::error(NOT_RUN)),
            Verdict::Deny | Verdict::NoOneThere => {
                live(Live::ToolDeclined {
                    summary: request.summary.clone(),
                    no_one_there: verdict == Verdict::NoOneThere,
                });
                Some(ToolOutcome::error(if verdict == Verdict::NoOneThere {
                    no_one_there(request.secret)
                } else if request.secret {
                    DECLINED_SECRET
                } else {
                    DECLINED
                }))
            }
        })
    }

    pub(crate) async fn record_result(
        &self,
        thread: &mut Thread,
        call_id: CallId,
        outcome: ToolOutcome,
    ) -> Result<(), HarnessError> {
        let output = vec![Part::Text { text: outcome.text }];
        let completed = ToolCallCompleted {
            call_id: call_id.clone(),
            is_error: outcome.is_error,
            output: output.clone(),
            structured: None,
        };
        let mut events = Vec::new();
        self.stage(thread, &mut events, &completed)?;
        self.commit(thread, events).await?;
        thread.messages.push(Message::Tool {
            call_id,
            is_error: outcome.is_error,
            output,
        });
        Ok(())
    }
}
