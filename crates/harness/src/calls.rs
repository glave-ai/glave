//! Running one of the model's tool calls: asking the person when the call needs it, running
//! the tool, and recording its result.

use std::sync::Arc;

use glave_event::{
    ApprovalDecided, ApprovalRequested, CallId, Lifted, Outcome, Principal, ToolCallCompleted,
    Verdict,
};
use glave_ports::settings::Judge;
use glave_ports::{
    Assessment, BlobStore, Effect, EventLog, Message, ModelProvider, ToolCall, ToolOutput,
    ToolSource,
};

use crate::approval::{ApprovalMode, ApprovalRequest, Approver, asks_for, no_one_there, sentence};
use crate::builtins::BUILTINS;
use crate::cancel::Cancel;
use crate::children::Ran;
use crate::harness::{Harness, HarnessError, Thread};
use crate::hooks::ToolHook;
use crate::judge::{JudgeCase, Ruling, judge};
use crate::question::{self, QuestionRequest};
use crate::sandbox::{self, Asks};
use crate::tools::Tool;
use crate::turn::{Live, Step, TurnSummary, absorb, tally};
use crate::waits::{Answered, verdict_of};

/// The result of a tool call the turn never ran because the person cancelled it.
pub(crate) const NOT_RUN: &str = "Not run: the person cancelled the turn.";

/// What a call refused in plan mode shows, after its summary.
const NOT_IN_PLAN: &str = "not run in plan mode";

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
    /// turn. Every call gets a result, so the thread stays valid after a cancel. A call that
    /// waits for the person, or whose subagent does, stops the turn there (`HarnessError::Waits`).
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
            if subagents > 0 {
                let (started, after) = rest.split_at_checked(subagents).unwrap_or((rest, &[]));
                self.run_agents(thread, started, step, cancel, live).await?;
                rest = after;
                continue;
            }
            // The approval the turn stopped at is settled even when it was stopped.
            let waited = step.waits_on(&first.call_id);
            let output = if cancel.is_cancelled() && !waited {
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

    /// Runs `calls`, which start subagents side by side, and records their results: refused as
    /// waiting while one of their subagents waits, unless the person stopped the turn, which
    /// declines what they wait for and ends them.
    async fn run_agents(
        &self,
        thread: &mut Thread,
        calls: &[ToolCall],
        step: &mut Step,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<(), HarnessError> {
        let waits = |ran: &[Ran]| ran.iter().any(|ran| matches!(ran, Ran::Waits));
        let mut ran = self
            .run_subagents(thread, calls, &step.toolset, cancel, live)
            .await;
        if waits(&ran) && cancel.is_cancelled() && cancel.outcome() == Outcome::Cancelled {
            ran = self
                .run_subagents(thread, calls, &step.toolset, cancel, live)
                .await;
        }
        if waits(&ran) {
            return Err(HarnessError::Waits(thread.id));
        }
        for (call, ran) in calls.iter().zip(ran) {
            let Ran::Done(output, child) = ran else {
                continue;
            };
            if let Some(child) = child {
                absorb(&mut step.summary, &child);
            }
            step.summary.tool_calls = step.summary.tool_calls.saturating_add(1);
            self.record_result(thread, call.call_id.clone(), output)
                .await?;
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
        if step.toolset.is_question(&call.tool) {
            return self.put_questions(thread, call, cancel, live).await;
        } else if step.toolset.is_goal(&call.tool) {
            return Ok(crate::goal::update(thread, &call.arguments));
        } else if step.toolset.is_plan(&call.tool) {
            return Ok(self.update_plan(thread, call, step, cancel, live));
        } else if step.toolset.is_memory(&call.tool) {
            return self.call_memory(thread, call, step, cancel, live).await;
        }
        let waited = step.waited(&call.call_id);
        let tools = &step.toolset;
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
        // What of the sandbox it would run without, as its commands run now.
        let outside = (index == BUILTINS)
            .then(|| sandbox::outside(self.workspace_for(thread), name, &call.arguments))
            .flatten();
        let lifted = outside.as_ref().map(|outside| outside.lifted);
        if let Some(answered) = waited {
            // Its hooks ran and it asked before the turn stopped: its answer decides, as it
            // would have the moment it came, unless the sandbox changed meanwhile.
            let asked = answered.asked.outside_sandbox.as_ref().map(|o| o.lifted);
            if let Some(refused) = self.answered(thread, answered, cancel, live).await? {
                return Ok(refused);
            }
            if let Some(refused) = self.let_out_refused(thread, lifted, asked, &summary, live) {
                return Ok(refused);
            }
            if let Some(lifted) = lifted {
                step.summary.let_out(lifted);
            }
            return self
                .run_call(thread, call, &source, &hooked, summary, cancel, live)
                .await;
        }
        let hook = self.stopped_by_hook(thread, &hooked, &summary, cancel, live);
        if let Some(stopped) = hook.await? {
            return Ok(stopped);
        }
        let (effect, caution, secret, firm) = match source.assess(name, &call.arguments).await {
            Assessment::AsDeclared => (info.effect, None, false, false),
            Assessment::Reads => (Effect::Reads, None, false, false),
            Assessment::Caution {
                reason,
                secret,
                firm,
            } => (info.effect, Some(reason), secret, firm),
        };
        if let Some(refused) = self.let_out_refused(thread, lifted, lifted, &summary, live) {
            return Ok(refused);
        }
        let request = ApprovalRequest {
            thread: thread.id,
            call_id: call.call_id.clone(),
            tool: name.to_owned(),
            effect,
            summary: summary.clone(),
            secret,
            caution: sandbox::with_outside(caution, lifted),
            outside_sandbox: outside,
        };
        let needs = Needs { request, firm };
        let turn = &mut step.summary;
        if let Some(refused) = self.approve(thread, &needs, turn, cancel, live).await? {
            return Ok(refused);
        }
        if let Some(lifted) = lifted {
            step.summary.let_out(lifted);
        }
        self.run_call(thread, call, &source, &hooked, summary, cancel, live)
            .await
    }

    /// The result the model is given instead of running a call that would run without
    /// `lifted` of the sandbox, where the person is asked, or was, about `asked`: refused when
    /// it would leave the sandbox otherwise than they were asked, since the sandbox changed
    /// while it waited, and when it would leave it at all where its commands may not ask to.
    /// `None` when it may go on.
    fn let_out_refused(
        &self,
        thread: &Thread,
        lifted: Option<Lifted>,
        asked: Option<Lifted>,
        summary: &str,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Option<ToolOutput> {
        let asks = Asks::of(self.workspace_for(thread));
        let (refused, why) = if lifted.is_some() && lifted != asked {
            (sandbox::CHANGED, "not run: the sandbox changed")
        } else if lifted.is_some() && asks == Asks::Never {
            (sandbox::PLAN_STAYS, NOT_IN_PLAN)
        } else if lifted.is_some() && asks == Asks::Nothing {
            (sandbox::NOT_ALLOWED, "not allowed here")
        } else {
            return None;
        };
        live(Live::ToolFinished {
            summary: format!("{summary} ({why})"),
            is_error: true,
        });
        Some(ToolOutput::text(true, refused))
    }

    /// Runs `call` with its tool's `source`, once nothing stops it, and its `PostToolUse`
    /// hooks after.
    #[expect(
        clippy::too_many_arguments,
        reason = "one call's run: the thread, the call, its source and hook, what it shows, and the turn's switches"
    )]
    async fn run_call(
        &self,
        thread: &mut Thread,
        call: &ToolCall,
        source: &Arc<dyn ToolSource>,
        hooked: &ToolHook<'_>,
        summary: String,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<ToolOutput, HarnessError> {
        live(Live::ToolStarted {
            summary: summary.clone(),
        });
        // A command it starts in the background is recorded under this call.
        thread.commands.calling(call.call_id.clone());
        let output = source.call(hooked.tool, &call.arguments, cancel);
        let output = crate::telemetry::tool_call(hooked.tool, output).await;
        live(Live::ToolFinished {
            summary,
            is_error: output.is_error,
        });
        self.after_tool(thread, hooked, output, cancel).await
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

    /// Puts the model's questions to the person and waits for the answers: where they are
    /// answered in the log, the turn stops here, its call the wait, and goes on once they are.
    async fn put_questions(
        &self,
        thread: &Thread,
        call: &ToolCall,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<ToolOutput, HarnessError> {
        if cancel.is_cancelled() {
            return Ok(ToolOutput::text(true, NOT_RUN));
        }
        let questions = match question::questions(&call.arguments) {
            Ok(questions) => questions,
            Err(err) => return Ok(ToolOutput::text(true, err)),
        };
        let request = QuestionRequest {
            thread: thread.id,
            call_id: call.call_id.clone(),
            questions,
        };
        if self.approver.waits(thread.id) {
            live(Live::Asked {
                call_id: request.call_id,
                questions: request.questions,
            });
            return Err(HarnessError::Waits(thread.id));
        }
        let waiting = self.approver.answer(&request);
        // Waiting now, so a client told of the question can answer it at once.
        live(Live::Asked {
            call_id: request.call_id.clone(),
            questions: request.questions.clone(),
        });
        Ok(match cancel.run(waiting).await {
            None => ToolOutput::text(true, NOT_RUN),
            Some(None) => ToolOutput::text(true, NO_ANSWER),
            Some(Some(answers)) => question::answered(&request.questions, &answers),
        })
    }

    /// Asks the person about a call when it needs them: for its caution, unless the judge clears
    /// it, or for the mode; Full access asks no one ([`Self::ask`]). `None` when the call may run;
    /// otherwise the result the model is given instead.
    async fn approve(
        &self,
        thread: &mut Thread,
        needs: &Needs,
        turn: &mut TurnSummary,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Option<ToolOutput>, HarnessError> {
        let request = &needs.request;
        let mut cautioned = request.caution.is_some();
        if cautioned && self.judgeable(thread, needs) {
            match self.judged(thread, request, turn, cancel).await? {
                Judged::Cleared => cautioned = false,
                Judged::Ask => {}
                Judged::Cancelled => return Ok(Some(ToolOutput::text(true, NOT_RUN))),
            }
        }
        // A command that stays in the thread's sandbox, which Ask first lets run: `bash` is
        // always the built-in, the first source that names a tool keeping it.
        let confined = request.tool == Tool::Bash.name()
            && request.outside_sandbox.is_none()
            && self.workspace_for(thread).sandbox().is_some();
        // A caution asks even when the tool was allowed for the rest of the thread; in plan
        // mode, where it is refused, nothing allowed before counts.
        let allowed = thread.allowed.contains(&request.tool) && !self.planning(thread);
        let asks = cautioned
            || (asks_for(self.approval_for(thread), request.effect, confined) && !allowed);
        if !asks {
            return Ok(None);
        }
        self.ask(thread, request, cancel, live).await
    }

    /// The model that judges `thread`'s flagged actions, or none when no judge does: its
    /// project's choice, else the harness's. The main model is its project's, else the
    /// default.
    fn judge_for(&self, thread: &Thread) -> Option<String> {
        let settled = &thread.brought.settled;
        let main = || {
            settled
                .model
                .clone()
                .unwrap_or_else(|| self.default_model())
        };
        match settled.judge.as_ref().unwrap_or(&thread.defaults.judge) {
            Judge::Off => None,
            Judge::Main => Some(main()),
            Judge::Model(model) => Some(model.clone()),
        }
    }

    /// Whether the judge may clear a call's caution: never a firm one. One that would run a
    /// command outside the sandbox only while someone could be asked instead, and only in a
    /// thread that has read nothing untrusted.
    fn judgeable(&self, thread: &Thread, needs: &Needs) -> bool {
        let unsandboxed = needs.request.outside_sandbox.is_some();
        !needs.firm && (!unsandboxed || (!thread.untrusted && self.approver.can_answer(thread.id)))
    }

    /// In auto mode with a judge, asks it whether a soft caution may run because the person
    /// asked for it. The judge's call is recorded and counted in the turn, as model usage this
    /// thread caused; a cleared action is recorded as an approval glave decided, its question
    /// and answer in one write.
    pub(crate) async fn judged(
        &self,
        thread: &mut Thread,
        request: &ApprovalRequest,
        turn: &mut TurnSummary,
        cancel: &Cancel,
    ) -> Result<Judged, HarnessError> {
        let Some(reason) = request.caution.as_deref() else {
            return Ok(Judged::Ask);
        };
        let Some(model) = self.judge_for(thread) else {
            return Ok(Judged::Ask);
        };
        if self.approval_for(thread) != ApprovalMode::Risky {
            return Ok(Judged::Ask);
        }
        let case = JudgeCase {
            asked: &thread.asked,
            rules: thread.instructions.as_deref(),
            action: &request.summary,
            flagged: reason,
            untrusted: thread.untrusted,
            outside_sandbox: request.outside_sandbox.as_ref().map(|o| o.lifted),
        };
        let asking = judge(&self.provider, &model, &case);
        let Some((ruling, answer)) = cancel.run(asking).await else {
            return Ok(Judged::Cancelled);
        };
        if let Some(answer) = &answer {
            turn.model_calls = turn.model_calls.saturating_add(1);
            tally(turn, answer);
            self.record(thread, &answer.responded()).await?;
        }
        let Ruling::Run { reason: why } = ruling else {
            return Ok(Judged::Ask);
        };
        // Why it asks, then the judge's own words for why it need not.
        let cleared = match why.trim() {
            "" => sentence("allowed without asking"),
            why => sentence(&format!("allowed without asking: {why}")),
        };
        let asked = ApprovalRequested {
            caution: Some(format!("{} {cleared}", sentence(reason))),
            ..request.asked()
        };
        let decided = ApprovalDecided {
            call_id: request.call_id.clone(),
            decision: Verdict::AllowOnce,
            by: Some(Principal::glave()),
        };
        // Written together, so that no stop between them can leave it waiting for anyone.
        let mut events = Vec::new();
        self.stage(thread, &mut events, &asked)?;
        self.stage(thread, &mut events, &decided)?;
        self.commit(thread, events).await?;
        Ok(Judged::Cleared)
    }

    /// Asks the person about `request`, recording the question and the answer. `None` when the
    /// call may go ahead; otherwise the result the model is given instead. Full access asks no
    /// one: the call goes ahead, unless it would show the model a secret ([`Self::unasked`]).
    /// Plan mode asks no one either: what would ask is refused, since nothing changes until
    /// the person approves the plan.
    pub(crate) async fn ask(
        &self,
        thread: &mut Thread,
        request: &ApprovalRequest,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Option<ToolOutput>, HarnessError> {
        if self.approval_for(thread) == ApprovalMode::Never {
            return self.unasked(thread, request, live).await;
        }
        if self.planning(thread) {
            live(Live::ToolFinished {
                summary: format!("{} ({NOT_IN_PLAN})", request.summary),
                is_error: true,
            });
            return Ok(Some(ToolOutput::text(true, crate::plan::refused(request))));
        }
        let asked = request.asked();
        self.record(thread, &asked).await?;
        if self.approver.waits(thread.id) {
            // The turn stops here: the answer comes to the log, and the turn goes on from it.
            live(Live::ApprovalPending {
                call_id: request.call_id.clone(),
            });
            return Err(HarnessError::Waits(thread.id));
        }
        let deciding = self.approver.decide_by(request);
        // Pending now, so a client told of it can answer it at once.
        if self.approver.can_answer(thread.id) {
            live(Live::ApprovalPending {
                call_id: request.call_id.clone(),
            });
        }
        let decided = cancel.run(deciding).await;
        let (decision, person) = decided.map_or((None, None), |(d, by)| (Some(d), by));
        // Running outside the sandbox is allowed one command at a time.
        let verdict = decision.map_or(Verdict::Cancelled, |decision| {
            verdict_of(decision, request.outside_sandbox.is_some())
        });
        // The person decides, unless glave did: no one was there, or the turn was cancelled.
        let by = if matches!(verdict, Verdict::NoOneThere | Verdict::Cancelled) {
            Some(Principal::glave())
        } else {
            Self::other_than_for(thread, person)
        };
        crate::telemetry::approval(&request.tool, verdict);
        let decided = ApprovalDecided {
            call_id: request.call_id.clone(),
            decision: verdict,
            by,
        };
        self.record(thread, &decided).await?;
        let no_one = (verdict == Verdict::NoOneThere).then(|| no_one_there(request));
        Ok(settle(thread, &asked, verdict, no_one, live))
    }

    /// What Full access does where another mode would ask about `request`: nothing, so the call
    /// goes ahead, unless it would show the model a secret, which only a person's yes does. Then
    /// glave declines it at once, saying so, its question and answer written together so that
    /// no stop between them can leave the thread waiting for anyone.
    async fn unasked(
        &self,
        thread: &mut Thread,
        request: &ApprovalRequest,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Option<ToolOutput>, HarnessError> {
        if !request.secret {
            return Ok(None);
        }
        let mut asked = request.asked();
        let why = sentence("declined without asking: Full access never shows the model a secret");
        asked.caution = Some(match asked.caution.take() {
            Some(caution) => format!("{caution} {why}"),
            None => why,
        });
        let decided = ApprovalDecided {
            call_id: request.call_id.clone(),
            decision: Verdict::Deny,
            by: Some(Principal::glave()),
        };
        crate::telemetry::approval(&request.tool, Verdict::Deny);
        let mut events = Vec::new();
        self.stage(thread, &mut events, &asked)?;
        self.stage(thread, &mut events, &decided)?;
        self.commit(thread, events).await?;
        Ok(settle(thread, &asked, Verdict::Deny, None, live))
    }

    /// Takes in the answer to the approval a turn going on from the log stopped at: the one
    /// recorded for it, or, once the turn was stopped, a cancellation recorded now. `None` when
    /// the call may go ahead; otherwise the result the model is given instead. With no answer
    /// yet and the turn not stopped, it waits on.
    pub(crate) async fn answered(
        &self,
        thread: &mut Thread,
        answered: Answered,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Option<ToolOutput>, HarnessError> {
        let verdict = match answered.decided {
            Some(decided) => decided.decision,
            None if cancel.is_cancelled() => {
                let decided = ApprovalDecided {
                    call_id: answered.asked.call_id.clone(),
                    decision: Verdict::Cancelled,
                    by: Some(Principal::glave()),
                };
                crate::telemetry::approval(&answered.asked.tool, decided.decision);
                self.record(thread, &decided).await?;
                Verdict::Cancelled
            }
            None => return Err(HarnessError::Waits(thread.id)),
        };
        Ok(settle(thread, &answered.asked, verdict, None, live))
    }

    pub(crate) async fn record_result(
        &self,
        thread: &mut Thread,
        call_id: CallId,
        outcome: ToolOutput,
    ) -> Result<(), HarnessError> {
        let outcome = self.keep_files(thread, outcome).await;
        let completed = completed(call_id, outcome);
        let mut events = Vec::new();
        // A command the call started or moved to the background, or saw end, first.
        let staged = self.stage_commands(thread, &mut events)?;
        // The plan the call set, with its result.
        let plan = thread.plan.staged.take();
        if let Some(plan) = &plan {
            self.stage(thread, &mut events, plan)?;
        }
        self.stage(thread, &mut events, &completed)?;
        self.commit(thread, events).await?;
        staged.recorded(thread);
        if let Some(plan) = &plan {
            // Glave's own record: call ids do not count it, so the model is sent the same.
            thread.unnumbered = thread.unnumbered.saturating_add(1);
            thread.plan.recorded(plan);
        }
        thread.untrusted |= completed.untrusted;
        thread.messages.push(Message::Tool {
            call_id: completed.call_id,
            is_error: completed.is_error,
            output: completed.output,
        });
        Ok(())
    }
}

/// The result of call `call_id` as it is recorded, from what it gave: content that came back
/// from outside labelled untrusted (an error is glave's own words, never labelled).
pub(crate) fn completed(call_id: CallId, outcome: ToolOutput) -> ToolCallCompleted {
    let untrusted = outcome.untrusted && !outcome.is_error;
    let output = if untrusted {
        crate::untrusted::label(outcome.parts)
    } else {
        outcome.parts
    };
    ToolCallCompleted {
        call_id,
        is_error: outcome.is_error,
        output,
        structured: outcome.structured,
        untrusted,
    }
}

/// What `verdict` on the approval `asked` means for its call: `None` when it may run; otherwise
/// the result the model is given instead, `no_one` when no one was there to answer.
fn settle(
    thread: &mut Thread,
    asked: &ApprovalRequested,
    verdict: Verdict,
    no_one: Option<String>,
    live: &mut (dyn FnMut(Live) + Send),
) -> Option<ToolOutput> {
    // The model is about to see a file that usually holds secrets.
    if asked.secret && matches!(verdict, Verdict::AllowOnce | Verdict::AllowForThread) {
        thread.read_secret = true;
    }
    match verdict {
        Verdict::AllowOnce => None,
        Verdict::AllowForThread => {
            thread.allowed.insert(asked.tool.clone());
            None
        }
        Verdict::Cancelled => Some(ToolOutput::text(true, NOT_RUN)),
        Verdict::Deny | Verdict::NoOneThere => {
            live(Live::ToolDeclined {
                summary: asked.summary.clone(),
                caution: asked.caution.clone(),
                no_one_there: verdict == Verdict::NoOneThere,
            });
            let declined = if asked.secret {
                DECLINED_SECRET
            } else {
                DECLINED
            };
            Some(ToolOutput::text(
                true,
                no_one.unwrap_or_else(|| declined.to_owned()),
            ))
        }
    }
}

/// What a call needs before it runs: the person's yes, asked as `request`, unless the judge may
/// clear its caution and does. A `firm` caution only the person can clear.
struct Needs {
    request: ApprovalRequest,
    firm: bool,
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
