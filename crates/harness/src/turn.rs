//! One turn: the person's prompt, then model calls and tool calls until the model is done.

use glave_event::{
    CallId, Currency, MessageAdded, ModelRef, ModelResponded, Money, Outcome, Part, Role,
    StopReason, TokenCounts, ToolCallCompleted, ToolCallRequested, TurnEnded, TurnStarted,
};
use glave_ports::{
    BlobStore, Completion, Delta, EventLog, Message, ModelProvider, ModelRequest, ToolCall,
    ToolSpec,
};
use serde::Serialize;

use crate::approval::{ApprovalRequest, Approver, Decision, reads_secret, summarize};
use crate::harness::{Harness, HarnessError, Thread};
use crate::tools::{Tool, ToolOutcome, specs};

/// The same call this many times in a row is refused, so a stuck model changes course.
const REPEAT_LIMIT: u32 = 3;

/// What the person sees while a turn runs. Never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Live {
    Text(String),
    Reasoning(String),
    ToolStarted { summary: String },
    ToolFinished { summary: String, is_error: bool },
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

/// Tracks identical consecutive tool calls, so a stuck model is told to change course.
#[derive(Default)]
struct Repeats {
    last: Option<String>,
    count: u32,
}

impl Repeats {
    /// Notes `call`; true when it is the same call too many times in a row.
    fn refuse(&mut self, call: &ToolCall) -> bool {
        let signature = format!("{}{:?}", call.tool, call.arguments);
        if self.last.as_deref() == Some(signature.as_str()) {
            self.count = self.count.saturating_add(1);
        } else {
            self.last = Some(signature);
            self.count = 1;
        }
        self.count >= REPEAT_LIMIT
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
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Runs one turn. Live progress goes to `live`; everything durable goes to the log.
    pub async fn run_turn(
        &self,
        thread: &mut Thread,
        prompt: &str,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnSummary, HarnessError> {
        let tools = specs();
        self.begin(thread, prompt, &tools).await?;
        let mut summary = TurnSummary::started();
        let mut repeats = Repeats::default();
        summary.outcome = loop {
            if summary.model_calls >= self.settings.max_steps {
                break Outcome::LimitReached;
            }
            summary.model_calls = summary.model_calls.saturating_add(1);
            let completion = match self.ask_model(thread, &tools, live).await {
                Ok(completion) => completion,
                Err(error) => {
                    self.end(thread, Outcome::Failed).await?;
                    return Err(error);
                }
            };
            tally(&mut summary, &completion);
            let calls = self.record_response(thread, &completion).await?;
            if calls.is_empty() {
                break outcome_of(completion.stop);
            }
            for call in calls {
                let outcome = if repeats.refuse(&call) {
                    ToolOutcome::error(
                        "You made this exact call several times in a row. Change your approach.",
                    )
                } else {
                    self.call_tool(thread, &call, live).await
                };
                summary.tool_calls = summary.tool_calls.saturating_add(1);
                self.record_result(thread, call.call_id, outcome).await?;
            }
        };
        self.end(thread, summary.outcome).await?;
        Ok(summary)
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
        let parts = vec![Part::Text {
            text: prompt.to_owned(),
        }];
        let mut events = Vec::new();
        self.stage(thread, &mut events, &TurnStarted { model, config })?;
        let message = MessageAdded {
            role: Role::User,
            parts: parts.clone(),
        };
        self.stage(thread, &mut events, &message)?;
        self.commit(thread, events).await?;
        thread.messages.push(Message::User { parts });
        Ok(())
    }

    /// Sends the conversation to the model, forwarding live text as it arrives.
    async fn ask_model(
        &self,
        thread: &Thread,
        tools: &[ToolSpec],
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Completion, HarnessError> {
        let request = ModelRequest {
            model: self.settings.model.clone(),
            system: self.settings.system_prompt.clone(),
            messages: thread.messages.clone(),
            tools: tools.to_vec(),
            max_output_tokens: self.settings.max_output_tokens,
        };
        let mut forward = |delta: Delta| match delta {
            Delta::Text(text) => live(Live::Text(text)),
            Delta::Reasoning(text) => live(Live::Reasoning(text)),
        };
        Ok(self.provider.complete(&request, &mut forward).await?)
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
        let responded = ModelResponded {
            model: completion.model.clone(),
            response_id: completion.response_id.clone(),
            stop: completion.stop,
            provider_stop: completion.provider_stop.clone(),
            usage: completion.usage.clone(),
            cost: completion.cost.clone(),
        };
        self.stage(thread, &mut events, &responded)?;
        self.commit(thread, events).await?;
        thread.messages.push(Message::Assistant {
            parts: completion.parts.clone(),
            tool_calls: calls.clone(),
        });
        Ok(calls)
    }

    /// Runs one tool call, asking the person first when it would change anything or show the
    /// model a secret file.
    async fn call_tool(
        &self,
        thread: &mut Thread,
        call: &ToolCall,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> ToolOutcome {
        let Some(tool) = Tool::named(&call.tool) else {
            return ToolOutcome::error(format!(
                "There is no tool named {:?}. The tools are read, write, edit and bash.",
                call.tool
            ));
        };
        let summary = summarize(tool, &call.arguments);
        let secret = reads_secret(&self.workspace, tool, &call.arguments).await;
        if (tool.changes_things() || secret) && !thread.allowed.contains(&tool) {
            let request = ApprovalRequest {
                tool,
                summary: if secret {
                    format!("{summary} (this file usually holds secrets; the model would see them)")
                } else {
                    summary.clone()
                },
                secret,
            };
            match self.approver.decide(&request).await {
                Decision::AllowOnce => {}
                Decision::AllowForThread => {
                    thread.allowed.insert(tool);
                }
                Decision::Deny if secret => {
                    return ToolOutcome::error(
                        "Reading this file was declined: it usually holds secrets such as keys or passwords. Do not try to get its contents another way. If you need a specific value, ask the person for it.",
                    );
                }
                Decision::Deny => {
                    return ToolOutcome::error(
                        "The person declined this action. Ask them what they would like instead, or take a different approach.",
                    );
                }
            }
        }
        live(Live::ToolStarted {
            summary: summary.clone(),
        });
        let outcome = tool.run(&call.arguments, &self.workspace).await;
        live(Live::ToolFinished {
            summary,
            is_error: outcome.is_error,
        });
        outcome
    }

    async fn record_result(
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
