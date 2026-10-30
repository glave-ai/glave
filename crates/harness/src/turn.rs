//! One turn: the person's prompt, then model calls and tool calls until the model is done.

use glave_event::{
    BlobRef, CallId, Caution, EventId, MemoryId, MessageAdded, ModelRef, Outcome, Part,
    Permissions, Principal, Role, StopReason, ThreadId, ToolCallRequested, TurnResult, TurnStarted,
};
use glave_ports::{
    BlobStore, BoxFuture, Caller, Completion, Delta, EventLog, Failure, Message, ModelProvider,
    ModelRequest, Pieces, ProviderError, Purpose, ToolCall, ToolChoice, ToolSpec,
};
use std::time::Duration;

use serde::Serialize;

use crate::approval::Approver;
use crate::cancel::Cancel;
use crate::compaction::{Measured, chars};
use crate::files::refuses_files;
use crate::goal::{BUDGET_SPENT, counted};
use crate::harness::{Harness, HarnessError, Thread};
use crate::memory::{ConfigMemory, MemoryVerb};
use crate::question::Question;
use crate::repeats::Repeats;
use crate::sandbox::{Recorded, recorded};
use crate::telemetry;
use crate::thought::Thinking;
use crate::toolset::ToolSet;
use crate::waits::{Answered, Opened};
use tracing::Instrument as _;

/// What the model is told after a cancelled turn, as Codex tells it.
const CANCELLED: &str = "[The person cancelled this turn on purpose. A command that was stopped \
     may have done part of its work.]";

/// What the model is told at the last step a turn allows.
const STEP_LIMIT: &str = "[This turn has reached its step limit, so tools are off until the \
     person's next message. Reply in text only: say that you reached the step limit, what you \
     have done, what is left, and what to do next.]";

/// What the person sees while a turn runs. Never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Live {
    Text(String),
    Reasoning(String),
    /// The reasoning's headline changed: its last bold title, else its last whole sentence.
    Thinking(String),
    /// The reasoning ended, after this long: the model wrote something else, or its call ended.
    Thought(Duration),
    ToolStarted {
        summary: String,
    },
    ToolFinished {
        summary: String,
        is_error: bool,
    },
    /// An action that did not run: the person said no, or no one was there to ask. `caution`
    /// is why it asked, as the person reads it, when a caution did, and `reason` what it was
    /// written from, as its `approval.requested` records them.
    ToolDeclined {
        summary: String,
        caution: Option<String>,
        reason: Option<Caution>,
        no_one_there: bool,
    },
    /// The conversation was summarized to fit the context window; sizes are estimates.
    Compacted {
        before_tokens: u64,
        after_tokens: u64,
    },
    /// A needed compaction failed; the turn goes on without it.
    CompactionFailed(String),
    /// The model asked the person questions, and the turn waits for the answers.
    Asked {
        call_id: CallId,
        questions: Vec<Question>,
    },
    /// The call's `approval.requested` waits for the person's answer, which someone can give.
    /// One the judge let run is decided at once, and never pending.
    ApprovalPending {
        call_id: CallId,
    },
    /// A subagent started, in a child thread.
    SubagentStarted {
        thread: ThreadId,
        description: String,
    },
    /// What a subagent shows, under its own thread.
    Child {
        thread: ThreadId,
        update: Box<Live>,
    },
    /// A subagent is done; `ok` when it finished its work.
    SubagentEnded {
        thread: ThreadId,
        ok: bool,
    },
    /// A memory was saved, edited or forgotten in this turn: shown at once in every client.
    Memory {
        verb: MemoryVerb,
        memory: MemoryId,
        /// Its text, when it was saved or edited.
        text: Option<String>,
    },
    /// A message the person sent while the turn ran that the model will not read: a hook
    /// stopped it (`reason`, as the hook gave it), or the turn was stopped or failed before its
    /// next call (`None`). `id` is the one it was queued with.
    Unread {
        id: EventId,
        reason: Option<String>,
    },
    /// The model was asked, and nothing has come back yet.
    Waiting,
    /// A model call failed in a way that may pass, and is tried again after `wait`.
    Retrying {
        retry: u32,
        of: u32,
        wait: Duration,
        error: String,
    },
}

/// A person's message: its text, and the files they added to it, as `file` parts.
#[derive(Debug, Clone, Copy)]
pub struct Said<'a> {
    pub text: &'a str,
    pub files: &'a [Part],
}

/// What the person asked for, and who asked.
#[derive(Clone, Copy)]
pub(crate) struct Prompt<'a> {
    pub(crate) text: &'a str,
    /// The files the person added to it, as `file` parts after its text.
    pub(crate) files: &'a [Part],
    pub(crate) caller: &'a Caller,
    /// Whether glave wrote the prompt, as a goal's continuation: recorded as glave's, and
    /// never read as the person's say-so.
    pub(crate) by_glave: bool,
    /// The id its message is recorded with, when it was made already: a message sent while
    /// the turn before ran keeps the id it was queued with.
    pub(crate) id: Option<EventId>,
}

impl<'a> Prompt<'a> {
    /// `text`, as `caller` wrote it.
    pub(crate) fn new(text: &'a str, caller: &'a Caller) -> Self {
        Self {
            text,
            files: &[],
            caller,
            by_glave: false,
            id: None,
        }
    }
}

/// What a turn carries from step to step.
pub(crate) struct Step {
    pub(crate) toolset: ToolSet,
    pub(crate) repeats: Repeats,
    pub(crate) summary: TurnResult,
    /// The approval a turn going on from the log stopped at, with its answer once one came:
    /// for its call, the first the turn runs (`going_on.rs`).
    pub(crate) answered: Option<Answered>,
    /// Whether the model proposed a plan in this turn, in plan mode: the turn ends with it.
    pub(crate) proposed: bool,
}

/// The version of the turn configuration blob.
const CONFIG_VERSION: u32 = 2;

/// The most a turn's configuration may be: its system prompt and tools, many times over.
pub(crate) const CONFIG_LIMIT: u64 = 16 * 1024 * 1024;

/// The full configuration of a turn, stored once by hash and referenced from `turn.started`.
#[derive(Serialize)]
struct TurnConfig<'a> {
    /// The blob's format: 2 since it records memory. Blobs without `v` are version 1.
    v: u32,
    model: &'a str,
    provider: &'a str,
    system: &'a str,
    tools: &'a [ToolSpec],
    max_output_tokens: Option<u32>,
    /// What the agent could do without asking: the thread's mode.
    permissions: Permissions,
    /// The sandbox commands ran in; absent when there was none.
    #[serde(skip_serializing_if = "Option::is_none")]
    sandbox: Option<Recorded>,
    /// The memories the model knew at the turn's start, at their versions; absent when the
    /// thread reads no memory, so a configuration without memory is what it always was.
    #[serde(skip_serializing_if = "Option::is_none")]
    memory: Option<ConfigMemory>,
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
            proposed: false,
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
            self.compact_if_needed(thread, &step.toolset, &mut step.summary, cancel, live)
                .await?;
            self.read_queued(thread, cancel, live).await?;
            let last = self.last_step(thread, &step.summary);
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

    /// Why the next step is the turn's last, if it is: the step cap, or a goal's spent budget.
    fn last_step(&self, thread: &Thread, summary: &TurnResult) -> Option<&'static str> {
        if self.capped(summary.model_calls.saturating_add(1)) {
            return Some(STEP_LIMIT);
        }
        let goal = thread.goal.as_ref().filter(|goal| goal.running)?;
        let used = goal
            .progress
            .tokens
            .saturating_add(counted(&summary.tokens));
        (used >= goal.progress.token_budget).then_some(BUDGET_SPENT)
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

    /// After a refusal as too long for the model: the thread counts as full, so the next step
    /// compacts first and then tries again.
    fn mark_full(&self, thread: &mut Thread, refused: &ProviderError) {
        self.learn_window(&self.model_for(thread), refused);
        thread.measured = Some(Measured {
            tokens: u64::MAX,
            chars: chars(&thread.messages),
        });
    }

    /// Records what ended in the background since the last turn, stores the turn's
    /// configuration, then records `turn.started` and the person's message.
    async fn begin(
        &self,
        thread: &mut Thread,
        prompt: Prompt<'_>,
        offer: &ToolSet,
    ) -> Result<(), HarnessError> {
        let Prompt {
            text: prompt,
            files,
            caller,
            by_glave,
            id,
        } = prompt;
        // Where it works now that its project's folder moved, before the turn that works there.
        if let Some(bound) = thread.rebound.take() {
            self.record(thread, &bound).await?;
            // Glave's own record: call ids do not count it, so the model is sent the same.
            thread.unnumbered = thread.unnumbered.saturating_add(1);
        }
        // What ended in the background since the last turn is recorded before this one.
        self.check_commands(thread).await?;
        let sent_to = self.model_for(thread);
        let provider = self.current_default().provider;
        let config = self
            .store_config(thread, offer, &sent_to, &provider)
            .await?;
        let model = ModelRef {
            provider,
            name: sent_to,
        };
        // A new thread opens with the project's instructions.
        let opening = thread.messages.is_empty();
        let mut parts = thread
            .opening_parts()
            .filter(|_| opening)
            .unwrap_or_default();
        parts.push(Part::Text {
            text: prompt.to_owned(),
        });
        parts.extend_from_slice(files);
        let mut events = Vec::new();
        self.stage(
            thread,
            &mut events,
            &TurnStarted {
                model,
                config,
                trace: caller.trace.clone(),
                auth: caller.auth.clone(),
            },
        )?;
        let message = MessageAdded {
            role: Role::User,
            parts: parts.clone(),
            author: if by_glave {
                Some(Principal::glave())
            } else {
                Self::other_than_for(thread, caller.person)
            },
            thought: None,
        };
        match id {
            Some(id) => self.stage_as(thread, &mut events, &message, id)?,
            None => self.stage(thread, &mut events, &message)?,
        }
        self.commit(thread, events).await?;
        thread.turns = thread.turns.saturating_add(1);
        thread.messages.push(Message::User { parts });
        if by_glave {
            return Ok(());
        }
        thread.request = Some(prompt.to_owned());
        if !thread.delegated() {
            thread.asked.push(prompt.to_owned());
        }
        Ok(())
    }

    /// Stores the turn's full configuration, as sent to `model` at `provider`, and answers
    /// the reference `turn.started` holds.
    async fn store_config(
        &self,
        thread: &mut Thread,
        offer: &ToolSet,
        model: &str,
        provider: &str,
    ) -> Result<BlobRef, HarnessError> {
        let memory = self.look_at_memory(thread).await;
        let config = TurnConfig {
            v: CONFIG_VERSION,
            model,
            provider,
            system: &offer.system,
            tools: &offer.specs,
            max_output_tokens: self.settings.max_output_tokens,
            permissions: self.permissions_for(thread),
            sandbox: self.workspace_for(thread).sandbox().map(recorded),
            memory,
        };
        let bytes = serde_json::to_vec(&config).map_err(|e| HarnessError::Event(e.to_string()))?;
        let stored = self.blobs.put(
            &thread.scope,
            Some(thread.id),
            "application/json",
            CONFIG_LIMIT,
            Box::new(Pieces::from(bytes)),
        );
        Ok(stored.await?)
    }

    /// Sends the conversation to the model, forwarding live text as it arrives, and answers
    /// with how its reasoning went. When its
    /// provider refuses the images and PDFs sent inline, it is asked once more with every file
    /// by its name, and the model is taken for one that reads none: a file never stops a
    /// thread.
    async fn ask_model(
        &self,
        thread: &Thread,
        offer: &ToolSet,
        tool_choice: ToolChoice,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<(Completion, Thinking), HarnessError> {
        let model = self.model_for(thread);
        let (messages, files) = self
            .with_files(&thread.scope, &model, self.sent(thread))
            .await;
        let mut request = ModelRequest {
            model,
            system: offer.system.clone(),
            messages,
            tools: offer.specs.clone(),
            tool_choice,
            max_output_tokens: self.settings.max_output_tokens,
            purpose: if thread.delegated() {
                Purpose::Subagent
            } else {
                Purpose::Main
            },
            files,
            effort: thread.effort.clone(),
            payer: Some(thread.payer()),
        };
        let retries = self.settings.retries;
        let mut retry = 0;
        let mut named = false;
        loop {
            live(Live::Waiting);
            let mut thinking = Thinking::default();
            let mut forward = |delta: Delta| thinking.see(delta, self.clock.now(), live);
            let answered = self.provider.complete(&request, &mut forward).await;
            match answered {
                Ok(completion) => {
                    thinking.end(self.clock.now(), live);
                    if named {
                        self.learn_without_images(&request.model);
                    }
                    return Ok((completion, thinking));
                }
                Err(error) if !request.files.is_empty() && refuses_files(&error) => {
                    tracing::warn!(
                        model = %request.model,
                        "the provider refused the files sent inline: sent again by their names"
                    );
                    request.files.clear();
                    named = true;
                }
                Err(error) if error.is_transient() && retry < retries.attempts => {
                    retry += 1;
                    // Only the wait's jitter: without randomness it waits the plain backoff.
                    let jitter = self.entropy.bytes().unwrap_or_default();
                    let wait = retries.wait(retry, jitter);
                    live(Live::Retrying {
                        retry,
                        of: retries.attempts,
                        wait,
                        error: error.to_string(),
                    });
                    tokio::time::sleep(wait).await;
                }
                Err(error) => return Err(error.into()),
            }
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

    /// Records a finished model response: its message, with what its reasoning came to as
    /// `thinking` saw it, its tool calls, each with its line as `tools` describe it, then its
    /// usage.
    async fn record_response(
        &self,
        thread: &mut Thread,
        completion: &Completion,
        thinking: &Thinking,
        tools: &ToolSet,
    ) -> Result<Vec<ToolCall>, HarnessError> {
        let mut events = Vec::new();
        if !completion.parts.is_empty() {
            let message = MessageAdded {
                role: Role::Assistant,
                parts: completion.parts.clone(),
                author: None,
                thought: thinking.thought(&completion.parts),
            };
            self.stage(thread, &mut events, &message)?;
        }
        let mut calls = Vec::new();
        for requested in &completion.tool_calls {
            let seq = Self::seq_after(thread, &events)?;
            let call = ToolCall {
                // Numbered as the model has always seen them: bindings do not count, so where
                // a thread works never changes what the model is sent.
                call_id: CallId::new(format!(
                    "call_{}",
                    seq.get().saturating_sub(thread.unnumbered)
                ))?,
                tool: requested.tool.clone(),
                arguments: requested.arguments.clone(),
            };
            let payload = ToolCallRequested {
                call_id: call.call_id.clone(),
                provider_call_id: requested.provider_call_id.clone(),
                tool: call.tool.clone(),
                arguments: call.arguments.clone(),
                summary: self.describe(thread, tools, &call),
                kind: self.kind(thread, tools, &call),
            };
            self.stage(thread, &mut events, &payload)?;
            calls.push(call);
        }
        self.stage(thread, &mut events, &completion.responded())?;
        self.commit(thread, events).await?;
        thread.messages.push(Message::Assistant {
            parts: completion.parts.clone(),
            tool_calls: calls.clone(),
        });
        Ok(calls)
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

/// Remembers the size the provider reported, for estimating the next request.
fn measure(thread: &mut Thread, completion: &Completion) {
    let reported = completion.usage.counts();
    thread.measured = Some(Measured {
        tokens: reported.input.saturating_add(reported.output),
        chars: chars(&thread.messages),
    });
}
