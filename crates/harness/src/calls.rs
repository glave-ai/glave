//! Running one of the model's tool calls: asking the person when the call needs it, running
//! the tool, and recording its result.

mod asking;
mod described;

use std::sync::Arc;

use glave_event::{CallId, Caution, Outcome, ToolCallCompleted};
use glave_ports::{
    BlobStore, Called, EventLog, Message, ModelProvider, ToolCall, ToolOutput, ToolSource,
};

use crate::approval::{ApprovalRequest, Approver, asking, assessed};
use crate::builtins::BUILTINS;
use crate::cancel::Cancel;
use crate::children::Ran;
use crate::compaction::compact_asked;
use crate::harness::{Harness, HarnessError, Thread};
use crate::hooks::ToolHook;
use crate::question::{self, QuestionRequest};
use crate::room::fit;
use crate::sandbox;
use crate::service::Round;
use crate::tools::Tool;
use crate::toolset::{Route, suggestion};
use crate::turn::{Live, Step};

/// The result of a tool call the turn never ran because the person cancelled it.
pub(crate) const NOT_RUN: &str = "Not run: the person cancelled the turn.";

/// What a call refused in plan mode shows, after its summary.
pub(crate) const NOT_IN_PLAN: &str = "not run in plan mode";

const NO_ANSWER: &str = "No one answered. Choose what seems best, go on, and say what you \
     assumed.";

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
        // A turn that calls a tool that can change files has its folder looked over as it ends,
        // for the files it made (`made.rs`).
        let at_home = thread.home.is_some();
        if let Some(turn) = thread.turn_open.as_mut() {
            turn.works |= calls
                .iter()
                .any(|call| crate::made::changes_files(&call.tool, at_home));
        }
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
                self.route_and_call(thread, first, step, cancel, live)
                    .await?
            };
            step.summary.tool_calls = step.summary.tool_calls.saturating_add(1);
            if let Some(path) = Tool::changes(&first.tool, &first.arguments)
                && !output.is_error
            {
                step.summary.changed_file(path);
            }
            self.record_result(thread, first.call_id.clone(), output)
                .await?;
            rest = rest.get(1..).unwrap_or_default();
        }
        Ok(())
    }

    /// Runs `call` as the tools route it: as it is, as the connected tool it stands for, as a
    /// search of the connected tools, or not at all.
    async fn route_and_call(
        &self,
        thread: &mut Thread,
        call: &ToolCall,
        step: &mut Step,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<ToolOutput, HarnessError> {
        match step.toolset.route(call) {
            Route::Run(call) => {
                let output = self.call_tool(thread, &call, step, cancel, live).await?;
                if step.toolset.get(&call.tool).map(|(index, _)| index) == Some(BUILTINS) {
                    Ok(self.with_nested_instructions(thread, &call, output).await)
                } else {
                    Ok(output)
                }
            }
            Route::Search { query, connector } => {
                let found = step.toolset.search(query.as_deref(), connector.as_deref());
                // What it suggests the person connect, glave's own records, before its result.
                let mut events = Vec::new();
                for it in &found.suggested {
                    self.stage(thread, &mut events, &suggestion(&call.call_id, it))?;
                }
                if !events.is_empty() {
                    let staged = u64::try_from(events.len()).unwrap_or(u64::MAX);
                    self.commit(thread, events).await?;
                    thread.unnumbered = thread.unnumbered.saturating_add(staged);
                }
                Ok(fit(found.output, self.tool_room(thread)))
            }
            Route::Refused(output) => Ok(output),
        }
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
                step.summary.absorb(&child);
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
        if step.toolset.is_question(&call.tool) && !self.approver.can_answer(thread.id) {
            return Ok(ToolOutput::text(true, question::NO_ONE));
        } else if step.toolset.is_question(&call.tool) {
            return self.put_questions(thread, call, cancel, live).await;
        } else if step.toolset.is_plan(&call.tool) {
            return Ok(self.update_plan(thread, call, step, cancel, live));
        } else if step.toolset.is_compact(&call.tool) {
            return Ok(compact_asked(call, step, live));
        } else if step.toolset.is_memory(&call.tool) {
            return self.call_memory(thread, call, cancel, live).await;
        } else if step.toolset.is_media(&call.tool) {
            return self.call_media(thread, call, step, cancel, live).await;
        } else if step.toolset.is_search(&call.tool) {
            return self.call_search(thread, call, step, cancel, live).await;
        } else if step.toolset.is_schedule(&call.tool) {
            return self.call_schedule(thread, call, step, cancel, live).await;
        }
        self.call_offered(thread, call, step, cancel, live).await
    }

    /// Runs one call of a tool a source offers, asking the person first when its effect needs
    /// them in this mode, when its source says it does, or when it may write to a connected
    /// service, unless it goes on from an answer.
    async fn call_offered(
        &self,
        thread: &mut Thread,
        call: &ToolCall,
        step: &mut Step,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<ToolOutput, HarnessError> {
        let round = step
            .round
            .take_if(|round| round.asked.call_id == call.call_id);
        let waited = step.waited(&call.call_id);
        let tools = &step.toolset;
        let Some((index, info)) = tools.get(&call.tool) else {
            return Ok(ToolOutput::text(true, tools.not_found(&call.tool)));
        };
        let Some(source) = self.source_for(thread, tools, index) else {
            return Ok(ToolOutput::text(true, format!("{:?} is gone", call.tool)));
        };
        let name = info.spec.name.as_str();
        let summary = self.describe(thread, tools, call);
        let hooked = ToolHook::of(call, name);
        // What of the sandbox it would run without, as its commands run now.
        let outside = (index == BUILTINS)
            .then(|| sandbox::outside(self.workspace_for(thread), name, &call.arguments))
            .flatten();
        let lifted = outside.as_ref().map(|outside| outside.lifted);
        if round.is_some() {
            // Its hooks ran and it was allowed before the service asked: only the answer is left.
            return self
                .run_call(thread, call, &source, &hooked, summary, round, cancel, live)
                .await;
        }
        if let Some(answered) = waited {
            // Its hooks ran and it asked before the turn stopped: its answer decides, as it
            // would have the moment it came, unless the sandbox changed meanwhile.
            let asked = answered.asked.outside_sandbox.as_ref().map(|o| o.lifted);
            if let Some(refused) = self.answered(thread, answered, cancel, live).await? {
                return Ok(refused);
            }
            if let Some(refused) =
                self.let_out_refused(thread, name, (lifted, asked), &summary, live)
            {
                return Ok(refused);
            }
            // What it was allowed without, as its log counts it once it begins.
            if let Some(asked) = asked {
                step.summary.let_out(asked);
            }
            return self
                .run_call(thread, call, &source, &hooked, summary, None, cancel, live)
                .await;
        }
        let hook = self.stopped_by_hook(thread, &hooked, &summary, cancel, live);
        if let Some(stopped) = hook.await? {
            return Ok(stopped);
        }
        let assessed = assessed(&*source, name, &call.arguments, info.effect).await;
        let (effect, caution, secret) = assessed;
        let (ask, acts) = asking(&*source, name, effect);
        if let Some(refused) = self.let_out_refused(thread, name, (lifted, lifted), &summary, live)
        {
            return Ok(refused);
        }
        let request = ApprovalRequest {
            thread: thread.id,
            call_id: call.call_id.clone(),
            tool: name.to_owned(),
            effect,
            summary: source.describe_in_full(name, &call.arguments),
            ask,
            secret,
            caution: Caution::of(caution.or(acts), lifted),
            outside_sandbox: outside,
            repeating: None,
            lines: None,
        };
        let turn = &mut step.summary;
        if let Some(refused) = self.approve(thread, &request, turn, cancel, live).await? {
            return Ok(refused);
        }
        if let Some(lifted) = lifted {
            step.summary.let_out(lifted);
        }
        self.run_call(thread, call, &source, &hooked, summary, None, cancel, live)
            .await
    }

    /// Runs `call` with its tool's `source`, once nothing stops it, and its `PostToolUse`
    /// hooks after.
    #[expect(
        clippy::too_many_arguments,
        reason = "one call's run: the thread, the call, its source and hook, what it shows, what its service last asked, and the turn's switches"
    )]
    async fn run_call(
        &self,
        thread: &mut Thread,
        call: &ToolCall,
        source: &Arc<dyn ToolSource>,
        hooked: &ToolHook<'_>,
        summary: String,
        round: Option<Round>,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<ToolOutput, HarnessError> {
        let given = if let Some(round) = round {
            // It goes on from what the person answered, and was told when it began.
            match self.given(thread, round, cancel, &summary, live).await? {
                Ok(given) => Some(given),
                Err(stopped) => return Ok(stopped),
            }
        } else {
            None
        };
        self.record_started(thread, &call.call_id).await?;
        if given.is_none() {
            live(Live::ToolStarted {
                summary: summary.clone(),
            });
        }
        // A command it starts in the background is recorded under this call.
        thread.commands.calling(call.call_id.clone());
        // A command, a write or an edit may change what the thread opens with, on its person's
        // own computer read again as the turn ends.
        if matches!(
            Tool::named(hooked.tool),
            Some(Tool::Bash | Tool::Write | Tool::Edit)
        ) {
            crate::homes::changed(thread);
        }
        let called = async {
            if given.is_some() || self.someone_answers(thread) {
                let arguments = &call.arguments;
                source
                    .call_asking(hooked.tool, arguments, given, cancel)
                    .await
            } else {
                Called::Done(source.call(hooked.tool, &call.arguments, cancel).await)
            }
        };
        let output = match crate::telemetry::tool_call(hooked.tool, called).await {
            Called::Done(output) => output,
            // The turn stops here: the answer comes to the log, and the turn goes on from it.
            Called::Asks(asks) => self.record_asked(thread, &call.call_id, asks).await?,
        };
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

    pub(crate) async fn record_result(
        &self,
        thread: &mut Thread,
        call_id: CallId,
        outcome: ToolOutput,
    ) -> Result<(), HarnessError> {
        // The harness's own secrets, however the thread's workspace was made.
        let outcome = self.workspace.hidden().masked(outcome);
        let outcome = self.gated(&thread.scope, thread.id, outcome).await;
        let outcome = fit(outcome, self.tool_room(thread));
        let outcome = self.keep_files(thread, outcome).await;
        // After what cuts a result to fit, and with nothing awaited before what it names is staged.
        let (outcome, told) = self.saying_what_stopped(thread, outcome).await;
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
        thread.commands.told(&told);
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
/// from outside labelled untrusted, an error as much as a success, since a failing command
/// or server can echo what someone else wrote.
pub(crate) fn completed(call_id: CallId, outcome: ToolOutput) -> ToolCallCompleted {
    let untrusted = outcome.untrusted;
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
