//! One turn: the person's prompt, then model calls and tool calls until the model is done.

use glave_event::{MessageAdded, Outcome, Part, Principal, Role, StopReason, TurnResult};
use glave_ports::{
    BlobStore, BoxFuture, Caller, EventLog, Failure, Message, ModelProvider, ToolCall, ToolChoice,
};

use crate::approval::Approver;
use crate::cancel::Cancel;
use crate::compaction::Asked;
use crate::harness::{Harness, HarnessError, Thread};
use crate::repeats::Repeats;
use crate::service::Round;
use crate::telemetry;
use crate::toolset::ToolSet;
use crate::waits::{Answered, Opened};
use tracing::Instrument as _;

mod begin;
mod live;
mod model;

pub(crate) use begin::CONFIG_LIMIT;
pub(crate) use live::Prompt;
pub use live::{Live, Said};
use model::measure;

/// What the model is told after a cancelled turn, as Codex tells it.
const CANCELLED: &str = "[The person cancelled this turn on purpose. A command that was stopped \
     may have done part of its work.]";

/// What the model is told at the last step a turn allows.
const STEP_LIMIT: &str = "[This turn has reached its step limit, so tools are off until the \
     person's next message. Reply in text only: say that you reached the step limit, what you \
     have done, what is left, and what to do next.]";

/// What a turn carries from step to step.
pub(crate) struct Step {
    pub(crate) toolset: ToolSet,
    pub(crate) repeats: Repeats,
    pub(crate) summary: TurnResult,
    /// The approval a turn going on from the log stopped at, with its answer once one came:
    /// for its call, the first the turn runs (`going_on.rs`).
    pub(crate) answered: Option<Answered>,
    /// What the service of that call last asked the person, when it did, with their answer once
    /// one came: the call goes on from it (`service.rs`).
    pub(crate) round: Option<Round>,
    /// Whether the model proposed a plan in this turn, in plan mode: the turn ends with it.
    pub(crate) proposed: bool,
    /// The compaction the person asked for in this turn, if they did (`compaction/asked.rs`).
    pub(crate) compact: Asked,
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Runs one turn. Live progress goes to `live`; everything durable goes to the log. Once
    /// `cancel` fires, the turn stops at its next step, ends any running command, and is
    /// recorded as cancelled, leaving the thread ready for the next turn.
    pub async fn run_turn(
        &self,
        thread: &mut Thread,
        prompt: &str,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnResult, HarnessError> {
        self.run_turn_as(thread, prompt, &Caller::default(), cancel, live)
            .await
    }

    /// Runs one turn for `caller`: their sign-in is recorded with the turn, and they are the
    /// prompt's `author` when they are not the person the thread is for.
    pub async fn run_turn_as(
        &self,
        thread: &mut Thread,
        prompt: &str,
        caller: &Caller,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnResult, HarnessError> {
        let said = Said {
            text: prompt,
            files: &[],
        };
        self.run_turn_with_files(thread, said, caller, cancel, live)
            .await
    }

    /// Runs one turn for `caller`'s message `said`: its text, and the files they added to
    /// it (`file` parts, already in the blob store), which the model is sent as it can read
    /// them.
    pub async fn run_turn_with_files(
        &self,
        thread: &mut Thread,
        said: Said<'_>,
        caller: &Caller,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnResult, HarnessError> {
        let prompt = Prompt {
            files: said.files,
            ..Prompt::new(said.text, caller)
        };
        self.run_prompt(thread, prompt, cancel, live).await
    }

    /// Runs one turn for `prompt`, as the person wrote it: a command is sent as its text.
    pub(crate) async fn run_prompt(
        &self,
        thread: &mut Thread,
        prompt: Prompt<'_>,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnResult, HarnessError> {
        self.bring(thread).await?;
        let toolset: ToolSet = self.tools(thread).await;
        let text = self
            .expand(thread, prompt.text, cancel)
            .await
            .map_err(HarnessError::PromptFailed)?;
        let prompt = Prompt {
            text: &text,
            ..prompt
        };
        self.run_with(thread, prompt, toolset, cancel, live).await
    }

    /// Runs one turn with `toolset`. Boxed, so a child's turn can run inside its parent's.
    pub(crate) fn run_with<'a>(
        &'a self,
        thread: &'a mut Thread,
        prompt: Prompt<'a>,
        toolset: ToolSet,
        cancel: &'a Cancel,
        live: &'a mut (dyn FnMut(Live) + Send),
    ) -> BoxFuture<'a, Result<TurnResult, HarnessError>> {
        Box::pin(async move {
            let span = telemetry::turn(thread.id, prompt.caller.trace.as_ref());
            let began = std::time::Instant::now();
            let ran = self
                .turn(thread, prompt, toolset, cancel, live)
                .instrument(span.clone())
                .await;
            let ended = ran.as_ref().ok();
            let ended = ended.map(|turn| (turn.outcome, turn.model_calls, turn.tool_calls));
            telemetry::turn_ended(&span, began, ended);
            ran
        })
    }

    async fn turn(
        &self,
        thread: &mut Thread,
        prompt: Prompt<'_>,
        toolset: ToolSet,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnResult, HarnessError> {
        self.close_left_open(thread).await?;
        self.reopen_removed(thread).await?;
        self.pass_hooks(thread, prompt, cancel).await?;
        self.begin(thread, prompt, &toolset).await?;
        let step = Step {
            toolset,
            repeats: Repeats::default(),
            summary: TurnResult::begun(),
            answered: None,
            round: None,
            proposed: false,
            compact: Asked::No,
        };
        self.steps(thread, step, &[], cancel, live).await
    }

    /// A turn's steps: first `pending`, the rest of a response's calls when the turn goes on
    /// from its log at them (`going_on.rs`), then a model call and its tools' calls after
    /// another until the turn ends, which it records. One that stops at a wait records
    /// nothing more (`HarnessError::Waits`); one that fails otherwise leaves the turn open in
    /// the log, which the next work on the thread closes first.
    pub(crate) async fn steps(
        &self,
        thread: &mut Thread,
        mut step: Step,
        pending: &[ToolCall],
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnResult, HarnessError> {
        // Open until the turn ends or stops at a wait, whatever fails between.
        thread.opened = Some(Opened::LeftOpen);
        self.offline_if_untrusted(thread).await?;
        if !pending.is_empty() {
            let ran = self
                .run_calls(thread, pending, &mut step, cancel, live)
                .await;
            waits_if(thread, ran)?;
        }
        // Whether this step was refused as too long once already.
        let mut squeezed = false;
        step.summary.outcome = loop {
            if cancel.is_cancelled() {
                break cancel.outcome();
            }
            // In plan mode the turn ends with the plan it proposed: the person decides next.
            if step.proposed {
                break Outcome::Completed;
            }
            if self.capped(step.summary.model_calls) {
                break Outcome::LimitReached;
            }
            self.compact_if_needed(thread, &mut step, cancel, live)
                .await?;
            self.read_queued(thread, cancel, live).await?;
            let last = self.last_step(&step.summary);
            let choice = self.choice(thread, last).await?;
            let last = last.is_some();
            // Asking the model writes nothing, so it can be dropped when the turn is cancelled.
            let Some(asked) = cancel
                .run(self.ask_model(thread, &step.toolset, choice, live))
                .await
            else {
                break cancel.outcome();
            };
            step.summary.model_calls = step.summary.model_calls.saturating_add(1);
            let (mut completion, thinking) = match asked {
                Ok(asked) => asked,
                Err(HarnessError::Provider(refused))
                    if refused.failure() == Failure::TooLong
                        && !squeezed
                        && thread.messages.len() >= 3 =>
                {
                    self.mark_full(thread, &refused);
                    squeezed = true;
                    continue;
                }
                Err(error) => {
                    step.summary.outcome = Outcome::Failed;
                    self.end(thread, &step.summary).await?;
                    return Err(error);
                }
            };
            squeezed = false;
            if last {
                // Any call the model makes anyway is ignored, so none is left without a result.
                completion.tool_calls.clear();
            }
            step.summary.take_in(&completion.responded());
            let calls = self
                .record_response(thread, &completion, &thinking, &step.toolset)
                .await?;
            measure(thread, &completion);
            if last {
                break Outcome::LimitReached;
            }
            if calls.is_empty() {
                break outcome_of(completion.stop);
            }
            let ran = self
                .run_calls(thread, &calls, &mut step, cancel, live)
                .await;
            waits_if(thread, ran)?;
        };
        // Stopped from outside: by the person (cancelled), or by glave stopping (interrupted).
        let stopped = cancel.is_cancelled() && step.summary.outcome == cancel.outcome();
        if stopped && step.summary.outcome == Outcome::Cancelled {
            self.add_note(thread, CANCELLED).await?;
        }
        self.end(thread, &step.summary).await?;
        if !thread.delegated() && !stopped {
            self.on_stop(thread, step.summary.outcome, cancel).await?;
        }
        Ok(step.summary)
    }

    /// The person's own prompts pass the `UserPromptSubmit` hooks first: one a hook stops
    /// never reaches the model.
    async fn pass_hooks(
        &self,
        thread: &mut Thread,
        prompt: Prompt<'_>,
        cancel: &Cancel,
    ) -> Result<(), HarnessError> {
        if !prompt.by_glave
            && !thread.delegated()
            && let Some(reason) = self.on_prompt(thread, prompt.text, cancel).await?
        {
            return Err(HarnessError::PromptBlocked(reason));
        }
        Ok(())
    }

    /// Whether a turn that has made `calls` model calls is at its cap; never, with no cap.
    fn capped(&self, calls: u32) -> bool {
        self.settings.max_steps.is_some_and(|max| calls >= max)
    }

    /// Why the next step is the turn's last, if it is: the step cap.
    fn last_step(&self, summary: &TurnResult) -> Option<&'static str> {
        self.capped(summary.model_calls.saturating_add(1))
            .then_some(STEP_LIMIT)
    }

    /// Tells the model something about the turn itself, as a recorded user message.
    async fn add_note(&self, thread: &mut Thread, note: &str) -> Result<(), HarnessError> {
        let parts = vec![Part::Text {
            text: note.to_owned(),
        }];
        let mut events = Vec::new();
        // Glave's own words, never the person's say-so.
        let message = MessageAdded {
            role: Role::User,
            parts: parts.clone(),
            author: Some(Principal::glave()),
            thought: None,
        };
        self.stage(thread, &mut events, &message)?;
        self.commit(thread, events).await?;
        thread.messages.push(Message::User { parts });
        Ok(())
    }

    /// The tools the model may call this step. The last step allowed answers in text: what
    /// was done and what is left, as OpenCode asks for, rather than stopping mid-work without
    /// a word.
    async fn choice(
        &self,
        thread: &mut Thread,
        last: Option<&str>,
    ) -> Result<ToolChoice, HarnessError> {
        if let Some(note) = last {
            self.add_note(thread, note).await?;
            Ok(ToolChoice::None)
        } else {
            Ok(ToolChoice::Auto)
        }
    }

    /// Records `turn.ended`, with what the turn did and cost: its `result`.
    async fn end(&self, thread: &mut Thread, result: &TurnResult) -> Result<(), HarnessError> {
        let mut events = Vec::new();
        let staged = self.stage_commands(thread, &mut events)?;
        self.stage(thread, &mut events, result)?;
        self.commit(thread, events).await?;
        staged.recorded(thread);
        // Nothing is open any more, whatever was when the thread was read.
        thread.opened = None;
        // On its person's own computer, read again after a turn that changed files there: what
        // the next thread in its folder opens with.
        self.refresh_at_home(thread).await;
        Ok(())
    }
}

/// `ran`, a turn's calls: one that stopped at a wait leaves the thread waiting, as its log
/// has it.
fn waits_if(thread: &mut Thread, ran: Result<(), HarnessError>) -> Result<(), HarnessError> {
    if matches!(ran, Err(HarnessError::Waits(_))) {
        thread.opened = Some(Opened::Waits);
    }
    ran
}

/// How a turn ends when the model stops without calling a tool.
fn outcome_of(stop: StopReason) -> Outcome {
    match stop {
        StopReason::EndTurn
        | StopReason::StopSequence
        | StopReason::Refusal
        | StopReason::ContentFilter
        | StopReason::ToolUse => Outcome::Completed,
        StopReason::MaxTokens | StopReason::ContextExceeded => Outcome::LimitReached,
        StopReason::Interrupted | StopReason::Cancelled => Outcome::Interrupted,
        StopReason::Error | StopReason::MalformedToolCall | StopReason::Pause => Outcome::Failed,
    }
}
