//! Running one of the model's tool calls: asking the person when the call needs it, running
//! the tool, and recording its result.

use glave_event::{
    ApprovalDecided, ApprovalRequested, CallId, Principal, ToolCallCompleted, Verdict,
};
use glave_ports::{
    Assessment, BlobStore, Effect, EventLog, Message, ModelProvider, ToolCall, ToolOutput,
};

use crate::approval::{ApprovalMode, ApprovalRequest, Approver, Decision, no_one_there};
use crate::cancel::Cancel;
use crate::compaction::responded;
use crate::harness::{Harness, HarnessError, Thread};
use crate::hooks::ToolHook;
use crate::judge::{JudgeCase, Ruling, judge};
use crate::question::{self, QuestionRequest};
use crate::turn::{Live, Step, TurnSummary, absorb, tally};

/// The result of a tool call the turn never ran because the person cancelled it.
pub(crate) const NOT_RUN: &str = "Not run: the person cancelled the turn.";

const NO_ANSWER: &str = "No one answered. Choose what seems best, go on, and say what you \
     assumed.";

const DECLINED: &str = "The person declined this action. Ask them what they would like \
     instead, or take a different approach.";

const DECLINED_SECRET: &str = "Declined: this would show you secrets such as keys, tokens or \
     passwords. Do not try to get them another way. If you need a specific value, ask the \
     person for it.";

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
                self.call_tool(thread, first, step, cancel, live).await?
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
        step: &mut Step,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<ToolOutput, HarnessError> {
        let tools = &step.toolset;
        if tools.is_question(&call.tool) {
            return Ok(self.put_questions(thread, call, cancel, live).await);
        } else if tools.is_goal(&call.tool) {
            return Ok(crate::goal::update(thread, &call.arguments));
        } else if tools.is_memory(&call.tool) {
            return self.call_memory(thread, call, step, cancel, live).await;
        }
        let Some((index, info)) = tools.get(&call.tool) else {
            let (tool, names) = (&call.tool, tools.names().join(", "));
            let text = format!("There is no tool named {tool:?}. The tools are {names}.");
            return Ok(ToolOutput::text(true, text));
        };
        let Some(source) = self.source_for(thread, index) else {
            return Ok(ToolOutput::text(true, format!("{:?} is gone", call.tool)));
        };
        let name = info.spec.name.as_str();
        let summary = source.describe(name, &call.arguments);
        let hooked = ToolHook::of(call, name);
        let hook = self.stopped_by_hook(thread, &hooked, &summary, cancel, live);
        if let Some(stopped) = hook.await? {
            return Ok(stopped);
        }
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
            match self
                .judged(thread, &flagged, &mut step.summary, cancel)
                .await?
            {
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
        self.after_tool(thread, &hooked, output, cancel).await
    }

    /// Runs the `PreToolUse` hooks: the result the model is given instead of running the
    /// call, when one stopped it or the turn was cancelled meanwhile; `None` to go on. A hook
    /// only stops calls: what passes is still asked about as before.
    pub(crate) async fn stopped_by_hook(
        &self,
        thread: &mut Thread,
        hooked: &ToolHook<'_>,
        summary: &str,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Option<ToolOutput>, HarnessError> {
        let stopped = self.before_tool(thread, hooked, cancel).await?;
        if cancel.is_cancelled() {
            return Ok(Some(ToolOutput::text(true, NOT_RUN)));
        }
        Ok(stopped.map(|reason| {
            live(Live::ToolFinished {
                summary: format!("{summary} (stopped by a hook)"),
                is_error: true,
            });
            ToolOutput::text(
                true,
                format!("Not run: a hook the person set up stopped this call: {reason}"),
            )
        }))
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
    /// asked for it. The judge's call is recorded and counted in the turn, as model usage this
    /// thread caused; a cleared action is recorded as an approval glave decided.
    pub(crate) async fn judged(
        &self,
        thread: &mut Thread,
        flagged: &Flagged<'_>,
        turn: &mut TurnSummary,
        cancel: &Cancel,
    ) -> Result<Judged, HarnessError> {
        let Flagged {
            call,
            name,
            summary,
            reason,
        } = *flagged;
        if !self.settings.judge || self.settings.approval != ApprovalMode::Auto {
            return Ok(Judged::Ask);
        }
        let case = JudgeCase {
            asked: &thread.asked,
            rules: self.settings.instructions.as_deref(),
            action: summary,
            flagged: reason,
            untrusted: thread.untrusted,
        };
        let model = &self.settings.model;
        let asking = judge(&self.provider, model, &case, thread.lineage());
        let Some((ruling, answer)) = cancel.run(asking).await else {
            return Ok(Judged::Cancelled);
        };
        if let Some(answer) = &answer {
            turn.model_calls = turn.model_calls.saturating_add(1);
            tally(turn, answer);
            self.record(thread, &responded(answer)).await?;
        }
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
            by: Some(Principal::glave()),
        };
        self.record(thread, &decided).await?;
        Ok(Judged::Cleared)
    }

    /// Asks the person about `request`, recording the question and the answer. `None` when the
    /// call may go ahead; otherwise the result the model is given instead.
    pub(crate) async fn ask(
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
        let decided = cancel.run(self.approver.decide_by(request)).await;
        let (decision, person) = decided.map_or((None, None), |(d, by)| (Some(d), by));
        let verdict = match decision {
            None => Verdict::Cancelled,
            Some(Decision::AllowOnce) => Verdict::AllowOnce,
            Some(Decision::AllowForThread) => Verdict::AllowForThread,
            Some(Decision::Deny) => Verdict::Deny,
            Some(Decision::Unavailable) => Verdict::NoOneThere,
        };
        // The person decides, unless glave did: no one was there, or the turn was cancelled.
        let by = if matches!(verdict, Verdict::NoOneThere | Verdict::Cancelled) {
            Some(Principal::glave())
        } else {
            Self::other_than_for(thread, person)
        };
        let decided = ApprovalDecided {
            call_id: request.call_id.clone(),
            decision: verdict,
            by,
        };
        self.record(thread, &decided).await?;
        // The model is about to see a file that usually holds secrets.
        if request.secret && matches!(verdict, Verdict::AllowOnce | Verdict::AllowForThread) {
            thread.read_secret = true;
        }
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
        // Only content that came back is labelled; an error is glave's own words.
        let untrusted = outcome.untrusted && !outcome.is_error;
        let output = if untrusted {
            crate::untrusted::label(outcome.parts)
        } else {
            outcome.parts
        };
        let completed = ToolCallCompleted {
            call_id: call_id.clone(),
            is_error: outcome.is_error,
            output: output.clone(),
            // Only a memory call records anything here: which memory, at which version.
            structured: thread.memory.structured.take(),
            untrusted,
        };
        let mut events = Vec::new();
        self.stage(thread, &mut events, &completed)?;
        self.commit(thread, events).await?;
        thread.untrusted |= untrusted;
        thread.messages.push(Message::Tool {
            call_id,
            is_error: outcome.is_error,
            output,
        });
        Ok(())
    }
}

/// What the judge made of a soft caution.
pub(crate) enum Judged {
    /// The person asked for it: it runs without asking.
    Cleared,
    /// The person decides, as for any caution.
    Ask,
    /// The turn was cancelled while the judge thought.
    Cancelled,
}

/// A call a rule flagged, as the judge is told about it.
#[derive(Clone, Copy)]
pub(crate) struct Flagged<'a> {
    pub(crate) call: &'a ToolCall,
    pub(crate) name: &'a str,
    pub(crate) summary: &'a str,
    pub(crate) reason: &'a str,
}
