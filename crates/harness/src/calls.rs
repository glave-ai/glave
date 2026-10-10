//! Running one of the model's tool calls: asking the person when the call needs it, running
//! the tool, and recording its result.

use glave_event::{ApprovalDecided, ApprovalRequested, CallId, ToolCallCompleted, Verdict};
use glave_ports::{
    Assessment, BlobStore, Effect, EventLog, Message, ModelProvider, ToolCall, ToolOutput,
};

use crate::approval::{ApprovalMode, ApprovalRequest, Approver, Decision, no_one_there};
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::judge::{JudgeCase, Ruling, judge};
use crate::question::{self, QuestionRequest};
use crate::toolset::ToolSet;
use crate::turn::{Live, Step, absorb};

/// The result of a tool call the turn never ran because the person cancelled it.
pub(crate) const NOT_RUN: &str = "Not run: the person cancelled the turn.";

const NO_ANSWER: &str = "No one answered. Choose what seems best, go on, and say what you \
     assumed.";

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
    /// Runs the calls of one response in order, recording each result. Calls that start
    /// subagents and come one after another run side by side, and their children's work counts in the
    /// turn. Every call gets a result, so the thread stays valid after a cancel.
    pub(crate) async fn run_calls(
        &self,
        thread: &mut Thread,
        calls: &[ToolCall],
        step: &mut Step,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<(), HarnessError> {
        let mut rest = calls;
        while let Some(first) = rest.first() {
            let subagents = rest
                .iter()
                .take_while(|call| step.toolset.is_agent(&call.tool))
                .count();
            if subagents > 0 && !cancel.is_cancelled() {
                let (started, after) = rest.split_at_checked(subagents).unwrap_or((rest, &[]));
                let ran = self
                    .run_subagents(thread, started, &step.toolset, cancel, live)
                    .await;
                for (call, (output, child)) in started.iter().zip(ran) {
                    if let Some(child) = child {
                        absorb(&mut step.summary, &child);
                    }
                    step.summary.tool_calls = step.summary.tool_calls.saturating_add(1);
                    self.record_result(thread, call.call_id.clone(), output)
                        .await?;
                }
                rest = after;
                continue;
            }
            let output = if cancel.is_cancelled() {
                ToolOutput::text(true, NOT_RUN)
            } else if step.repeats.refuse(first) {
                ToolOutput::text(
                    true,
                    "You made this exact call several times in a row. Change your approach.",
                )
            } else {
                self.call_tool(thread, first, &step.toolset, cancel, live)
                    .await?
            };
            step.summary.tool_calls = step.summary.tool_calls.saturating_add(1);
            self.record_result(thread, first.call_id.clone(), output)
                .await?;
            rest = rest.get(1..).unwrap_or_default();
        }
        Ok(())
    }

    /// Runs one tool call, asking the person first when its effect needs them in this mode or
    /// when its source says it does (a read of a secret file).
    pub(crate) async fn call_tool(
        &self,
        thread: &mut Thread,
        call: &ToolCall,
        tools: &ToolSet,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<ToolOutput, HarnessError> {
        if tools.is_question(&call.tool) {
            return Ok(self.put_questions(thread, call, cancel, live).await);
        }
        let Some((index, info)) = tools.get(&call.tool) else {
            return Ok(ToolOutput::text(
                true,
                format!(
                    "There is no tool named {:?}. The tools are {}.",
                    call.tool,
                    tools.names().join(", ")
                ),
            ));
        };
        let Some(source) = self.sources.get(index) else {
            return Ok(ToolOutput::text(true, format!("{:?} is gone", call.tool)));
        };
        let name = info.spec.name.as_str();
        let summary = source.describe(name, &call.arguments);
        let (effect, mut caution, secret, firm) = match source.assess(name, &call.arguments).await {
            Assessment::AsDeclared => (info.effect, None, false, false),
            Assessment::Reads => (Effect::Reads, None, false, false),
            Assessment::Caution {
                reason,
                secret,
                firm,
            } => (info.effect, Some(reason), secret, firm),
        };
        if let Some(reason) = caution.as_deref().filter(|_| !firm) {
            let flagged = Flagged {
                call,
                name,
                summary: &summary,
                reason,
            };
            match self.judged(thread, &flagged, cancel).await? {
                Judged::Cleared => caution = None,
                Judged::Ask => {}
                Judged::Cancelled => return Ok(ToolOutput::text(true, NOT_RUN)),
            }
        }
        // A caution asks even when the tool was allowed for the rest of the thread.
        let asks = caution.is_some()
            || (self.settings.approval.asks_for(effect) && !thread.allowed.contains(name));
        if asks {
            let request = ApprovalRequest {
                thread: thread.id,
                call_id: call.call_id.clone(),
                tool: name.to_owned(),
                effect,
                summary: match &caution {
                    Some(reason) => format!("{summary} ({reason})"),
                    None => summary.clone(),
                },
                secret,
                caution,
            };
            if let Some(refused) = self.ask(thread, &request, cancel, live).await? {
                return Ok(refused);
            }
        }
        live(Live::ToolStarted {
            summary: summary.clone(),
        });
        let output = source.call(name, &call.arguments, cancel).await;
        live(Live::ToolFinished {
            summary,
            is_error: output.is_error,
        });
        Ok(output)
    }

    /// Puts the model's questions to the person and waits for the answers.
    async fn put_questions(
        &self,
        thread: &Thread,
        call: &ToolCall,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> ToolOutput {
        let questions = match question::questions(&call.arguments) {
            Ok(questions) => questions,
            Err(err) => return ToolOutput::text(true, err),
        };
        let request = QuestionRequest {
            thread: thread.id,
            call_id: call.call_id.clone(),
            questions,
        };
        let waiting = self.approver.answer(&request);
        // Waiting now, so a client told of the question can answer it at once.
        live(Live::Asked {
            call_id: request.call_id.clone(),
            questions: request.questions.clone(),
        });
        match cancel.run(waiting).await {
            None => ToolOutput::text(true, NOT_RUN),
            Some(None) => ToolOutput::text(true, NO_ANSWER),
            Some(Some(answers)) => {
                ToolOutput::text(false, question::answered(&request.questions, &answers))
            }
        }
    }

    /// In auto mode with a judge, asks it whether a soft caution may run because the person
    /// asked for it. A cleared action is recorded as an approval glave decided.
    async fn judged(
        &self,
        thread: &mut Thread,
        flagged: &Flagged<'_>,
        cancel: &Cancel,
    ) -> Result<Judged, HarnessError> {
        let Flagged {
            call,
            name,
            summary,
            reason,
        } = *flagged;
        let Some(model) = self.settings.judge.as_deref() else {
            return Ok(Judged::Ask);
        };
        if self.settings.approval != ApprovalMode::Auto {
            return Ok(Judged::Ask);
        }
        let case = JudgeCase {
            asked: &thread.asked,
            rules: self.settings.instructions.as_deref(),
            action: summary,
            flagged: reason,
        };
        let Some(ruling) = cancel.run(judge(&self.provider, model, &case)).await else {
            return Ok(Judged::Cancelled);
        };
        let Ruling::Run { reason: why } = ruling else {
            return Ok(Judged::Ask);
        };
        let asked = ApprovalRequested {
            call_id: call.call_id.clone(),
            tool: name.to_owned(),
            summary: format!("{summary} ({reason}; the judge let it run: {why})"),
            secret: false,
        };
        self.record(thread, &asked).await?;
        let decided = ApprovalDecided {
            call_id: call.call_id.clone(),
            decision: Verdict::AllowOnce,
        };
        self.record(thread, &decided).await?;
        Ok(Judged::Cleared)
    }

    /// Asks the person about `request`, recording the question and the answer. `None` when the
    /// call may go ahead; otherwise the result the model is given instead.
    async fn ask(
        &self,
        thread: &mut Thread,
        request: &ApprovalRequest,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Option<ToolOutput>, HarnessError> {
        let asked = ApprovalRequested {
            call_id: request.call_id.clone(),
            tool: request.tool.clone(),
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
                thread.allowed.insert(request.tool.clone());
                None
            }
            Verdict::Cancelled => Some(ToolOutput::text(true, NOT_RUN)),
            Verdict::Deny | Verdict::NoOneThere => {
                live(Live::ToolDeclined {
                    summary: request.summary.clone(),
                    no_one_there: verdict == Verdict::NoOneThere,
                });
                Some(ToolOutput::text(
                    true,
                    if verdict == Verdict::NoOneThere {
                        no_one_there(request)
                    } else if request.secret {
                        DECLINED_SECRET.to_owned()
                    } else {
                        DECLINED.to_owned()
                    },
                ))
            }
        })
    }

    pub(crate) async fn record_result(
        &self,
        thread: &mut Thread,
        call_id: CallId,
        outcome: ToolOutput,
    ) -> Result<(), HarnessError> {
        let output = outcome.parts;
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

/// What the judge made of a soft caution.
enum Judged {
    /// The person asked for it: it runs without asking.
    Cleared,
    /// The person decides, as for any caution.
    Ask,
    /// The turn was cancelled while the judge thought.
    Cancelled,
}

/// A call a rule flagged, as the judge is told about it.
#[derive(Clone, Copy)]
struct Flagged<'a> {
    call: &'a ToolCall,
    name: &'a str,
    summary: &'a str,
    reason: &'a str,
}
