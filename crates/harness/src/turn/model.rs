//! One model call of a turn: the conversation sent, tried again when that may pass, and the
//! answer recorded.

use glave_event::{CallId, MessageAdded, Role, ToolCallRequested};
use glave_ports::{
    BlobStore, Completion, Delta, EventLog, Message, ModelProvider, ModelRequest, ProviderError,
    Purpose, ToolCall, ToolChoice,
};

use super::Live;
use crate::approval::Approver;
use crate::compaction::{Measured, chars};
use crate::files::refuses_files;
use crate::harness::{Harness, HarnessError, Thread};
use crate::thought::Thinking;
use crate::toolset::ToolSet;

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
    /// with how its reasoning went. When its
    /// provider refuses the images and PDFs sent inline, it is asked once more with every file
    /// by its name, and the model is taken for one that reads none: a file never stops a
    /// thread.
    pub(super) async fn ask_model(
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
            route: None,
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

/// Remembers the size the provider reported, for estimating the next request.
pub(super) fn measure(thread: &mut Thread, completion: &Completion) {
    let reported = completion.usage.counts();
    thread.measured = Some(Measured {
        tokens: reported.input.saturating_add(reported.output),
        chars: chars(&thread.messages),
    });
}
