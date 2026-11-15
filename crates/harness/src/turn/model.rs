//! One model call of a turn: the conversation sent, tried again when that may pass, stopped
//! when its answer repeats itself, and the answer recorded.

use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Poll;

use glave_event::{
    CallId, MessageAdded, ModelRef, Role, StopReason, TokenCounts, ToolCallRequested, Usage,
};
use glave_ports::{
    BlobStore, Completion, Delta, EventLog, Message, ModelProvider, ModelRequest, ProviderError,
    Purpose, ToolCall, ToolChoice,
};

use super::Live;
use crate::approval::Approver;
use crate::compaction::{CHARS_PER_TOKEN, Measured, chars, next_request_tokens};
use crate::files::refuses_files;
use crate::harness::{Harness, HarnessError, Thread};
use crate::looping::Looping;
use crate::thought::Thinking;
use crate::toolset::ToolSet;

/// What a model call came to.
pub(super) enum Answer {
    /// The model's answer, with how its reasoning went.
    Given(Completion, Thinking),
    /// An answer stopped as it streamed, since it repeated itself: none of its words, and what
    /// it used, estimated.
    Repeating(Completion),
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// After a refusal as too long for the model: the thread counts as full, so the next step
    /// compacts first and then tries again.
    pub(super) fn mark_full(&self, thread: &mut Thread, refused: &ProviderError) {
        self.learn_window(&self.model_for(thread), refused);
        thread.measured = Some(Measured {
            tokens: u64::MAX,
            chars: chars(&thread.messages),
        });
    }

    /// Sends the conversation to the model, forwarding live text as it arrives, and answers
    /// with how its reasoning went, or that its answer was stopped for repeating itself. When its
    /// provider refuses the images and PDFs sent inline, it is asked once more with every file
    /// by its name, and the model is taken for one that reads none: a file never stops a
    /// thread.
    pub(super) async fn ask_model(
        &self,
        thread: &Thread,
        offer: &ToolSet,
        tool_choice: ToolChoice,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Answer, HarnessError> {
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
            route: None,
            end_user: None,
        };
        let retries = self.settings.retries;
        let mut retry = 0;
        let mut named = false;
        loop {
            live(Live::Waiting);
            let (answered, mut thinking, came) = self.watched(&request, live).await;
            let Some(answered) = answered else {
                tracing::warn!(model = %request.model, "the answer repeated itself: stopped");
                thinking.end(self.clock.now(), live);
                let input = next_request_tokens(thread, offer);
                return self
                    .cut_off(&request.model, input, came)
                    .map(Answer::Repeating);
            };
            match answered {
                Ok(completion) => {
                    thinking.end(self.clock.now(), live);
                    if named {
                        self.learn_without_images(&request.model);
                    }
                    return Ok(Answer::Given(completion, thinking));
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

    /// Streams the answer to `request`, forwarding it live and watching its text for a loop:
    /// `None` when it repeated itself and was stopped; with how its reasoning went, and how many
    /// bytes of it came, text and reasoning.
    async fn watched(
        &self,
        request: &ModelRequest,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> (Option<Result<Completion, ProviderError>>, Thinking, u64) {
        let mut thinking = Thinking::default();
        let mut looping = Looping::default();
        let repeating = AtomicBool::new(false);
        let mut came: u64 = 0;
        let mut forward = |delta: Delta| {
            let (Delta::Text(text) | Delta::Reasoning(text)) = &delta;
            came = came.saturating_add(u64::try_from(text.len()).unwrap_or(u64::MAX));
            if let Delta::Text(text) = &delta
                && looping.see(text)
            {
                repeating.store(true, Ordering::Relaxed);
            }
            thinking.see(delta, self.clock.now(), live);
        };
        let answered = until(&repeating, self.provider.complete(request, &mut forward)).await;
        (answered, thinking, came)
    }

    /// An answer from `model` stopped for repeating itself, after `came` bytes of it, to a
    /// request of about `input` tokens: what it used, estimated, as its provider gave nothing.
    fn cut_off(&self, model: &str, input: u64, came: u64) -> Result<Completion, HarnessError> {
        let counts = TokenCounts {
            input,
            output: came.div_ceil(CHARS_PER_TOKEN),
            ..TokenCounts::default()
        };
        let usage =
            Usage::new(counts, true, None).map_err(|e| HarnessError::Event(e.to_string()))?;
        Ok(Completion {
            model: ModelRef {
                provider: self.current_default().provider,
                name: model.to_owned(),
            },
            response_id: None,
            parts: Vec::new(),
            tool_calls: Vec::new(),
            stop: StopReason::Repeating,
            provider_stop: None,
            usage,
            cost: None,
            served: None,
            upstream: None,
        })
    }

    /// After an answer stopped for repeating itself: records what it used, and none of its
    /// words, so the call asked again is sent what this one was. The first time, the turn goes
    /// on to ask once more; `again`, the turn ends, failed, saying why.
    pub(super) async fn stopped_repeating(
        &self,
        thread: &mut Thread,
        summary: &mut glave_event::TurnResult,
        cut: &Completion,
        again: bool,
    ) -> Result<(), HarnessError> {
        summary.take_in(&cut.responded());
        let mut events = Vec::new();
        self.stage(thread, &mut events, &cut.responded())?;
        self.commit(thread, events).await?;
        if !again {
            return Ok(());
        }
        summary.outcome = glave_event::Outcome::Failed;
        self.end(thread, summary).await?;
        Err(HarnessError::Repeating)
    }

    /// Records a finished model response: its message, with what its reasoning came to as
    /// `thinking` saw it, its tool calls, each with its line as `tools` describe it, then its
    /// usage.
    pub(super) async fn record_response(
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
                key: None,
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

/// Runs `work` until it finishes, `Some` of what it gave, or until `stop` is set as it runs:
/// `None` then, even when it finished in the same breath, and `work` is dropped, its stream
/// with it.
async fn until<T>(stop: &AtomicBool, work: impl Future<Output = T>) -> Option<T> {
    let mut work = pin!(work);
    poll_fn(|cx| {
        let done = work.as_mut().poll(cx);
        if stop.load(Ordering::Relaxed) {
            Poll::Ready(None)
        } else {
            done.map(Some)
        }
    })
    .await
}

/// Remembers the size the provider reported, for estimating the next request.
pub(super) fn measure(thread: &mut Thread, completion: &Completion) {
    let reported = completion.usage.counts();
    thread.measured = Some(Measured {
        tokens: reported.input.saturating_add(reported.output),
        chars: chars(&thread.messages),
    });
}
