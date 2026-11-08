//! Keeping a thread within the model's context window: estimating how large the next request
//! will be, and restarting the conversation from a handoff summary when it nears the limit.
//! The summary is recorded as `thread.compacted`; nothing before it leaves the log.

mod recent;
mod summary;

use std::fmt::Write as _;

use glave_event::{Arguments, Part, Seq, ThreadCompacted, TurnResult};

use glave_ports::{BlobStore, EventLog, Message, ModelProvider, ProviderError, ToolSpec};

use crate::approval::Approver;
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::room::{stated_window, tool_room};
use crate::toolset::ToolSet;
use crate::turn::Live;

/// When to compact a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Compaction {
    /// The model's context window, in tokens.
    pub window_tokens: u64,
    /// Compact before a request that would fill this much of the window, in percent.
    pub at_percent: u8,
}

impl Default for Compaction {
    fn default() -> Self {
        Self {
            window_tokens: 128_000,
            at_percent: 80,
        }
    }
}

/// What a compaction did.
#[derive(Debug, Clone, PartialEq)]
pub struct Compacted {
    /// The estimated size of the next request before and after, in tokens.
    pub before_tokens: u64,
    pub after_tokens: u64,
}

/// A token count the provider reported, and how much text the conversation held then. Later
/// text is estimated from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Measured {
    pub tokens: u64,
    pub chars: u64,
}

/// Characters per token when nothing better is known. Code runs closer to 3, so estimates
/// run low; the threshold leaves room for that.
const CHARS_PER_TOKEN: u64 = 4;

/// The longest summary asked for, in tokens.
const SUMMARY_TOKENS: u32 = 4_000;

/// How much a request must grow after a compaction failed, in points of the window, before
/// another is tried: a failed summary costs as much as a good one, so it is not asked for at
/// every step while the request still fits. A refusal as too long tries again at once.
const REGROW: u8 = 5;

/// Characters of everything the model sees in `messages`.
pub(crate) fn chars(messages: &[Message]) -> u64 {
    let total: usize = messages
        .iter()
        .map(|message| match message {
            Message::User { parts } | Message::Tool { output: parts, .. } => parts_chars(parts),
            Message::Assistant { parts, tool_calls } => {
                let calls: usize = tool_calls
                    .iter()
                    .map(|call| {
                        call.tool
                            .len()
                            .saturating_add(arguments_chars(&call.arguments))
                    })
                    .sum();
                parts_chars(parts).saturating_add(calls)
            }
        })
        .sum();
    u64::try_from(total).unwrap_or(u64::MAX)
}

fn parts_chars(parts: &[Part]) -> usize {
    parts
        .iter()
        .map(|part| match part {
            Part::Text { text } => text.len(),
            Part::Reasoning { text, .. } => text.as_ref().map_or(0, String::len),
            Part::File { .. } => 0,
        })
        .sum()
}

fn arguments_chars(arguments: &Arguments) -> usize {
    match arguments {
        Arguments::Json(map) => serde_json::to_string(map).map_or(0, |json| json.len()),
        Arguments::Malformed(raw) => raw.len(),
    }
}

/// A tool call's arguments as the model wrote them.
fn arguments_text(arguments: &Arguments) -> String {
    match arguments {
        Arguments::Json(map) => serde_json::to_string(map).unwrap_or_default(),
        Arguments::Malformed(raw) => raw.clone(),
    }
}

/// Characters every request carries besides the conversation: instructions and tools.
/// The estimated size of the next request, in tokens: the last reported count plus what
/// was added since, or an estimate of everything when nothing was reported yet.
fn next_request_tokens(thread: &Thread, offer: &ToolSet) -> u64 {
    let now = chars(&thread.messages);
    match thread.measured {
        Some(measured) => measured
            .tokens
            .saturating_add(now.saturating_sub(measured.chars).div_ceil(CHARS_PER_TOKEN)),
        None => now
            .saturating_add(base_chars(&offer.system, &offer.specs))
            .div_ceil(CHARS_PER_TOKEN),
    }
}

fn base_chars(system: &str, tools: &[ToolSpec]) -> u64 {
    let tools = serde_json::to_string(tools).map_or(0, |json| json.len());
    u64::try_from(system.len().saturating_add(tools)).unwrap_or(u64::MAX)
}

/// Characters of the latest tool results a restart keeps: a quarter of what fills the window
/// enough to compact, so the summary, the results and the next steps fit well below it.
fn recent_room(window_tokens: u64, at_percent: u8) -> usize {
    let tokens = window_tokens.saturating_mul(u64::from(at_percent)) / 100 / 4;
    usize::try_from(tokens.saturating_mul(CHARS_PER_TOKEN)).unwrap_or(usize::MAX)
}

/// The message a compacted conversation restarts from: the summary, the latest tool results as
/// they were, and the person's latest request.
fn restart_text(summary: &str, recent: Option<&str>, request: Option<&str>) -> String {
    let mut text = format!(
        "[Earlier parts of this conversation were summarized to save space. The \
         summary holds what was done and learned: go on from its next step. Read only what that \
         step needs, one file at a time, not everything again at once, or the space runs out \
         again before anything is done.]\n\n{}",
        summary.trim()
    );
    if let Some(recent) = recent {
        let _ = write!(
            text,
            "\n\nYour latest tool calls and what they returned, as they were:{recent}"
        );
    }
    if let Some(request) = request {
        let _ = write!(
            text,
            "\n\nThe person's latest request, word for word:\n{request}"
        );
    }
    text
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Compacts now if the next request would pass the threshold, counting the summary's
    /// model calls in `summary`. A thread with nothing yet to summarize is left alone. A summary
    /// the provider fails to give is reported and the turn goes on: the request may still fit,
    /// and none is asked for again until it has grown by [`REGROW`] points of the window or the
    /// provider refuses it as too long.
    pub(crate) async fn compact_if_needed(
        &self,
        thread: &mut Thread,
        offer: &mut ToolSet,
        summary: &mut TurnResult,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<(), HarnessError> {
        let at_percent = self.settings.compaction.at_percent;
        let window_tokens = self.context_window(&self.model_for(thread));
        let tokens = next_request_tokens(thread, offer);
        let full =
            tokens.saturating_mul(100) >= window_tokens.saturating_mul(u64::from(at_percent));
        let regrow = window_tokens.saturating_mul(u64::from(REGROW)) / 100;
        let waits = (thread.summary_failed).is_some_and(|at| tokens < at.saturating_add(regrow));
        if !full || waits || thread.messages.len() < 3 {
            return Ok(());
        }
        match self
            .compact_with(thread, offer, summary, cancel, live)
            .await
        {
            Ok(_) => {
                // A thread of its own opens again with the prompt and tools on offer now; a
                // subagent's are its parent's.
                if !thread.delegated() {
                    *offer = self.tools(thread).await;
                }
                Ok(())
            }
            Err(HarnessError::Provider(error)) => {
                thread.summary_failed = Some(tokens);
                live(Live::CompactionFailed(error.to_string()));
                Ok(())
            }
            // The turn sees the cancel itself and stops.
            Err(HarnessError::Cancelled) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// The context window compaction aims for with `model`, in tokens: the configured one, or
    /// the model's real one if a refusal has stated a smaller one.
    pub fn context_window(&self, model: &str) -> u64 {
        let configured = self.settings.compaction.window_tokens;
        self.learned_windows
            .of(model)
            .map_or(configured, |learned| configured.min(learned))
    }

    /// The most bytes one tool output gives `thread`'s model.
    pub(crate) fn tool_room(&self, thread: &Thread) -> usize {
        tool_room(self.context_window(&self.model_for(thread)))
    }

    /// The longest summary to ask `model` for: at most a quarter of its window, since servers
    /// count the answer's room against the window too.
    fn summary_tokens(&self, model: &str) -> u32 {
        let quarter = u32::try_from(self.context_window(model) / 4).unwrap_or(u32::MAX);
        SUMMARY_TOKENS.min(quarter)
    }

    /// Believes the window a too-long refusal from `model` states, for the rest of the
    /// session.
    pub(crate) fn learn_window(&self, model: &str, refusal: &ProviderError) {
        if let Some(tokens) = stated_window(&refusal.to_string()) {
            self.learned_windows.learn(model, tokens);
        }
    }

    /// Summarizes the thread and restarts its conversation from the summary, whatever its size.
    /// For `/compact`. Cancelling leaves the thread as it was. What the summary calls used is
    /// in the log, as every call's is.
    pub async fn compact(
        &self,
        thread: &mut Thread,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Compacted, HarnessError> {
        self.close_left_open(thread).await?;
        let offer = self.tools(thread).await;
        let mut spent = TurnResult::begun();
        self.compact_with(thread, &offer, &mut spent, cancel, live)
            .await
    }

    /// Summarizes the whole conversation and restarts it from the summary, the latest tool
    /// results and the person's latest request, counting every summary call in `spent`.
    pub(crate) async fn compact_with(
        &self,
        thread: &mut Thread,
        offer: &ToolSet,
        spent: &mut TurnResult,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Compacted, HarnessError> {
        let before_tokens = next_request_tokens(thread, offer);
        let window_tokens = self.context_window(&self.model_for(thread));
        let at_percent = self.settings.compaction.at_percent;
        let (completion, summary) = self.summary(thread, offer, spent, cancel).await?;
        let through = thread
            .next_seq
            .get()
            .checked_sub(1)
            .and_then(|seq| Seq::new(seq).ok())
            .ok_or_else(|| HarnessError::Event("there is nothing to compact".into()))?;
        // The project's instructions open the restarted conversation, as they opened the first.
        let mut parts = thread.opening_parts().unwrap_or_default();
        parts.push(Part::Text {
            text: restart_text(
                &summary,
                recent::recent(&thread.messages, recent_room(window_tokens, at_percent)).as_deref(),
                thread.request.as_deref(),
            ),
        });
        let mut events = Vec::new();
        self.stage(thread, &mut events, &completion.responded())?;
        let compacted = ThreadCompacted {
            through,
            parts: parts.clone(),
            untrusted: thread.untrusted,
        };
        self.stage(thread, &mut events, &compacted)?;
        self.commit(thread, events).await?;
        thread.messages = vec![Message::User { parts }];
        thread.measured = None;
        thread.summary_failed = None;
        // The conversation starts again, and so may its prefix (`prefix.rs`).
        thread.prefix = crate::prefix::Prefixed::None;
        // The start shows every memory the thread knows now, and the notes fold away.
        if self.memory_for(thread).is_some() {
            thread.memory.restart();
        }
        let after_tokens = next_request_tokens(thread, offer);
        live(Live::Compacted {
            before_tokens,
            after_tokens,
        });
        Ok(Compacted {
            before_tokens,
            after_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_restart_message_keeps_the_request_word_for_word() {
        let text = restart_text(
            "  Did A and B.\n",
            Some("\n\n`read` a:\nA"),
            Some("fix the build"),
        );
        assert!(text.starts_with("[Earlier parts of this conversation were summarized"));
        assert!(
            text.ends_with("]\n\nDid A and B.\n\nYour latest tool calls and what they returned, as they were:\n\n`read` a:\nA\n\nThe person's latest request, word for word:\nfix the build"),
            "{text}"
        );
        let bare = restart_text("Did A.", None, None);
        assert!(!bare.contains("word for word") && !bare.contains("latest tool calls"));
    }

    #[test]
    fn characters_count_what_the_model_sees() {
        let messages = vec![
            Message::User {
                parts: vec![Part::Text {
                    text: "abcd".into(),
                }],
            },
            Message::Assistant {
                parts: vec![Part::Reasoning {
                    text: Some("xy".into()),
                    opaque: None,
                }],
                tool_calls: vec![glave_ports::ToolCall {
                    call_id: glave_event::CallId::new("call_1").unwrap(),
                    tool: "read".into(),
                    arguments: Arguments::Malformed("{p".into()),
                }],
            },
        ];
        assert_eq!(chars(&messages), 4 + 2 + 4 + 2);
    }
}
