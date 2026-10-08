//! One turn: the person's prompt, then model calls and tool calls until the model is done.

use glave_event::{
    CallId, Currency, MessageAdded, ModelRef, Money, Outcome, Part, Role, StopReason, TokenCounts,
    ToolCallRequested, TurnEnded, TurnStarted,
};
use glave_ports::{
    BlobStore, Completion, Delta, EventLog, Message, ModelProvider, ModelRequest, ToolCall,
    ToolChoice, ToolSpec,
};
use std::time::Duration;

use serde::Serialize;

use crate::approval::{ApprovalMode, Approver};
use crate::calls::NOT_RUN;
use crate::cancel::Cancel;
use crate::compaction::{Measured, chars, responded};
use crate::harness::{Harness, HarnessError, Thread};
use crate::repeats::Repeats;
use crate::sandbox::{Prepared, Recorded};
use crate::tools::{ToolOutcome, specs};

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
    ToolStarted {
        summary: String,
    },
    ToolFinished {
        summary: String,
        is_error: bool,
    },
    /// An action that did not run: the person said no, or no one was there to ask.
    ToolDeclined {
        summary: String,
        no_one_there: bool,
    },
    /// The conversation was summarized to fit the context window; sizes are estimates.
    Compacted {
        before_tokens: u64,
        after_tokens: u64,
    },
    /// A needed compaction failed; the turn goes on without it.
    CompactionFailed(String),
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

/// What a turn did and what it cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnSummary {
    pub outcome: Outcome,
    pub model_calls: u32,
    pub tool_calls: u32,
    pub tokens: TokenCounts,
    /// The total the provider reported, when it reported a cost for every call.
    pub cost: Option<Money>,
    /// True when any usage was estimated because a stream ended early.
    pub estimated: bool,
}

impl TurnSummary {
    fn started() -> Self {
        Self {
            outcome: Outcome::Completed,
            model_calls: 0,
            tool_calls: 0,
            tokens: TokenCounts::default(),
            cost: Some(Money {
                micros: 0,
                currency: Currency::USD,
            }),
            estimated: false,
        }
    }
}

/// The full configuration of a turn, stored once by hash and referenced from `turn.started`.
#[derive(Serialize)]
struct TurnConfig<'a> {
    model: &'a str,
    provider: &'a str,
    system: &'a str,
    tools: &'a [ToolSpec],
    max_output_tokens: Option<u32>,
    approval: ApprovalMode,
    /// The sandbox commands ran in; absent when there was none.
    #[serde(skip_serializing_if = "Option::is_none")]
    sandbox: Option<Recorded>,
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
    ) -> Result<TurnSummary, HarnessError> {
        let tools = specs();
        self.begin(thread, prompt, &tools).await?;
        let mut summary = TurnSummary::started();
        let mut repeats = Repeats::default();
        summary.outcome = loop {
            if cancel.is_cancelled() {
                break Outcome::Cancelled;
            }
            if summary.model_calls >= self.settings.max_steps {
                break Outcome::LimitReached;
            }
            if let Some(compaction) = self.compact_if_needed(thread, &tools, cancel, live).await? {
                summary.model_calls = summary.model_calls.saturating_add(1);
                tally(&mut summary, &compaction);
            }
            // The last step allowed answers in text: what was done and what is left, as
            // OpenCode asks for, rather than stopping mid-work without a word.
            let last = summary.model_calls.saturating_add(1) >= self.settings.max_steps;
            let choice = if last {
                self.add_note(thread, STEP_LIMIT).await?;
                ToolChoice::None
            } else {
                ToolChoice::Auto
            };
            // Asking the model writes nothing, so it can be dropped when the turn is cancelled.
            let Some(asked) = cancel
                .run(self.ask_model(thread, &tools, choice, live))
                .await
            else {
                break Outcome::Cancelled;
            };
            summary.model_calls = summary.model_calls.saturating_add(1);
            let mut completion = match asked {
                Ok(completion) => completion,
                Err(error) => {
                    self.end(thread, Outcome::Failed).await?;
                    return Err(error);
                }
            };
            if last {
                // Any call the model makes anyway is ignored, so none is left without a result.
                completion.tool_calls.clear();
            }
            tally(&mut summary, &completion);
            let calls = self.record_response(thread, &completion).await?;
            let reported = completion.usage.counts();
            thread.measured = Some(Measured {
                tokens: reported.input.saturating_add(reported.output),
                chars: chars(&thread.messages),
            });
            if last {
                break Outcome::LimitReached;
            }
            if calls.is_empty() {
                break outcome_of(completion.stop);
            }
            for call in calls {
                // Every call gets a result, so the thread stays valid after a cancel.
                let outcome = if cancel.is_cancelled() {
                    ToolOutcome::error(NOT_RUN)
                } else if repeats.refuse(&call) {
                    ToolOutcome::error(
                        "You made this exact call several times in a row. Change your approach.",
                    )
                } else {
                    self.call_tool(thread, &call, cancel, live).await
                };
                summary.tool_calls = summary.tool_calls.saturating_add(1);
                self.record_result(thread, call.call_id, outcome).await?;
            }
        };
        if summary.outcome == Outcome::Cancelled {
            self.add_note(thread, CANCELLED).await?;
        }
        self.end(thread, summary.outcome).await?;
        Ok(summary)
    }

    /// Tells the model something about the turn itself, as a recorded user message.
    async fn add_note(&self, thread: &mut Thread, note: &str) -> Result<(), HarnessError> {
        let parts = vec![Part::Text {
            text: note.to_owned(),
        }];
        let mut events = Vec::new();
        let message = MessageAdded {
            role: Role::User,
            parts: parts.clone(),
        };
        self.stage(thread, &mut events, &message)?;
        self.commit(thread, events).await?;
        thread.messages.push(Message::User { parts });
        Ok(())
    }

    /// Stores the turn's configuration, then records `turn.started` and the person's message.
    async fn begin(
        &self,
        thread: &mut Thread,
        prompt: &str,
        tools: &[ToolSpec],
    ) -> Result<(), HarnessError> {
        let config = TurnConfig {
            model: &self.settings.model,
            provider: &self.settings.provider,
            system: &self.settings.system_prompt,
            tools,
            max_output_tokens: self.settings.max_output_tokens,
            approval: self.settings.approval,
            sandbox: self.workspace.sandbox().map(Prepared::recorded),
        };
        let bytes = serde_json::to_vec(&config).map_err(|e| HarnessError::Event(e.to_string()))?;
        let config = self
            .blobs
            .put(&thread.scope, bytes, "application/json")
            .await?;
        let model = ModelRef {
            provider: self.settings.provider.clone(),
            name: self.settings.model.clone(),
        };
        // A new thread opens with the project's instructions.
        let opening = thread.messages.is_empty();
        let mut parts = self
            .instruction_parts()
            .filter(|_| opening)
            .unwrap_or_default();
        parts.push(Part::Text {
            text: prompt.to_owned(),
        });
        let mut events = Vec::new();
        self.stage(thread, &mut events, &TurnStarted { model, config })?;
        let message = MessageAdded {
            role: Role::User,
            parts: parts.clone(),
        };
        self.stage(thread, &mut events, &message)?;
        self.commit(thread, events).await?;
        thread.messages.push(Message::User { parts });
        thread.request = Some(prompt.to_owned());
        Ok(())
    }

    /// Sends the conversation to the model, forwarding live text as it arrives.
    async fn ask_model(
        &self,
        thread: &Thread,
        tools: &[ToolSpec],
        tool_choice: ToolChoice,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Completion, HarnessError> {
        let request = ModelRequest {
            model: self.settings.model.clone(),
            system: self.settings.system_prompt.clone(),
            messages: thread.messages.clone(),
            tools: tools.to_vec(),
            tool_choice,
            max_output_tokens: self.settings.max_output_tokens,
        };
        let retries = self.settings.retries;
        let mut retry = 0;
        loop {
            live(Live::Waiting);
            let mut forward = |delta: Delta| match delta {
                Delta::Text(text) => live(Live::Text(text)),
                Delta::Reasoning(text) => live(Live::Reasoning(text)),
            };
            match self.provider.complete(&request, &mut forward).await {
                Ok(completion) => return Ok(completion),
                Err(error) if error.is_transient() && retry < retries.attempts => {
                    retry += 1;
                    let wait = retries.wait(retry, self.entropy.bytes());
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

    /// Records `turn.ended`.
    async fn end(&self, thread: &mut Thread, outcome: Outcome) -> Result<(), HarnessError> {
        let mut events = Vec::new();
        self.stage(thread, &mut events, &TurnEnded { outcome })?;
        self.commit(thread, events).await
    }

    /// Records a finished model response: its message, its tool calls, then its usage.
    async fn record_response(
        &self,
        thread: &mut Thread,
        completion: &Completion,
    ) -> Result<Vec<ToolCall>, HarnessError> {
        let mut events = Vec::new();
        if !completion.parts.is_empty() {
            let message = MessageAdded {
                role: Role::Assistant,
                parts: completion.parts.clone(),
            };
            self.stage(thread, &mut events, &message)?;
        }
        let mut calls = Vec::new();
        for requested in &completion.tool_calls {
            let seq = Self::seq_after(thread, &events)?;
            let call = ToolCall {
                call_id: CallId::new(format!("call_{}", seq.get()))?,
                tool: requested.tool.clone(),
                arguments: requested.arguments.clone(),
            };
            let payload = ToolCallRequested {
                call_id: call.call_id.clone(),
                provider_call_id: requested.provider_call_id.clone(),
                tool: call.tool.clone(),
                arguments: call.arguments.clone(),
            };
            self.stage(thread, &mut events, &payload)?;
            calls.push(call);
        }
        self.stage(thread, &mut events, &responded(completion))?;
        self.commit(thread, events).await?;
        thread.messages.push(Message::Assistant {
            parts: completion.parts.clone(),
            tool_calls: calls.clone(),
        });
        Ok(calls)
    }
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

fn tally(summary: &mut TurnSummary, completion: &Completion) {
    let counts = completion.usage.counts();
    let total = &mut summary.tokens;
    total.input = total.input.saturating_add(counts.input);
    total.cache_read = total.cache_read.saturating_add(counts.cache_read);
    total.cache_write = total.cache_write.saturating_add(counts.cache_write);
    total.output = total.output.saturating_add(counts.output);
    total.reasoning = total.reasoning.saturating_add(counts.reasoning);
    summary.estimated |= completion.usage.estimated();
    summary.cost = match (summary.cost, &completion.cost) {
        (Some(sum), Some(cost)) if sum.currency == cost.amount.currency => Some(Money {
            micros: sum.micros.saturating_add(cost.amount.micros),
            currency: sum.currency,
        }),
        _ => None,
    };
}
