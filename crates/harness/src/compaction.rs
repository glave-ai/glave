//! Keeping a thread within the model's context window: estimating how large the next request
//! will be; once it nears the threshold, summarizing the older half of the conversation in the
//! background while the turn goes on, and swapping the summary in between model calls, the newer
//! half kept as it was; at the threshold, waiting for that summary, or summarizing the whole
//! conversation as before when there is none; and compacting when the person asks (`asked.rs`).
//! Every summary is recorded as `thread.compacted`; nothing before it leaves the log.

mod asked;
mod background;
mod recent;
mod restart;
mod size;
mod words;

use glave_event::{Part, Seq, TurnResult};

use glave_ports::{
    BlobStore, Completion, EventLog, Failure, Message, ModelProvider, ModelRequest, ProviderError,
    Purpose, ToolChoice,
};

pub(crate) use asked::{COMPACT, describe as described, spec};
pub(crate) use background::Background;
use restart::Restart;
pub(crate) use size::chars;
use size::{CHARS_PER_TOKEN, next_request_tokens, past};
use words::Covers;

use crate::approval::Approver;
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::room::{stated_window, tool_room, trim_oldest};
use crate::toolset::ToolSet;
use crate::turn::Live;

/// When to compact a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Compaction {
    /// The model's context window, in tokens.
    pub window_tokens: u64,
    /// Compact before a request that would fill this much of the window, in percent. A summary
    /// of the older half starts in the background a tenth of the window before it.
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
    /// The summary call, for receipts.
    pub completion: Completion,
}

/// A token count the provider reported, and how much text the conversation held then. Later
/// text is estimated from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Measured {
    pub tokens: u64,
    pub chars: u64,
}

/// The longest summary asked for, in tokens.
const SUMMARY_TOKENS: u32 = 4_000;

/// How far before the threshold, in points of the window, a summary of the older half starts in
/// the background: a tenth of it, 12,800 tokens of a 128,000-token window (Claude Code keeps a
/// 13,000-token buffer before it compacts), room for the steps a turn takes while the summary is
/// written. No request goes past the threshold for it: the model works with as much of the
/// window as before, and after the swap keeps the newer half as it was rather than a summary
/// alone.
const LEAD: u8 = 10;

/// The fewest tokens an older half is worth summarizing in the background for: less frees too
/// little to pay for the call, and is left to the compaction at the threshold.
const OLDER_MIN: u64 = 1_000;

/// Characters of the latest tool results a restart keeps: a quarter of what fills the window
/// enough to compact, so the summary, the results and the next steps fit well below it.
fn recent_room(window_tokens: u64, at_percent: u8) -> usize {
    let tokens = window_tokens.saturating_mul(u64::from(at_percent)) / 100 / 4;
    usize::try_from(tokens.saturating_mul(CHARS_PER_TOKEN)).unwrap_or(usize::MAX)
}

/// The summary's text, its text parts joined.
fn summary_text(completion: &Completion) -> String {
    let texts: Vec<&str> = completion
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    texts.join("\n")
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Keeps the next request within the window, at a boundary between model calls, counting
    /// any summary's model call in `summary`: swaps in a summary written in the background
    /// that is ready, and starts one once the request nears the threshold; at the threshold,
    /// waits for the one being written, and compacts the whole conversation when there is none
    /// or it freed too little. A thread with nothing yet to summarize is left alone. A summary
    /// the provider fails to give is reported and the turn goes on: the request may still fit.
    pub(crate) async fn compact_if_needed(
        &self,
        thread: &mut Thread,
        offer: &mut ToolSet,
        summary: &mut TurnResult,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<(), HarnessError> {
        if let Some(ended) = thread.background.ended().await
            && let Some((before_tokens, after_tokens)) =
                self.swap_in(thread, offer, summary, ended).await?
        {
            live(Live::Compacted {
                before_tokens,
                after_tokens,
                waited: false,
            });
        }
        if thread.messages.len() < 3 {
            return Ok(());
        }
        let at_percent = self.settings.compaction.at_percent;
        let window_tokens = self.context_window(&self.model_for(thread));
        let tokens = next_request_tokens(thread, offer);
        if !past(tokens, window_tokens, at_percent) {
            if past(tokens, window_tokens, at_percent.saturating_sub(LEAD))
                && thread.background.idle()
            {
                self.start_background(thread, offer).await;
            }
            return Ok(());
        }
        if thread.background.writing() {
            // The turn can go no further without it: dropped by a cancel, which the turn sees.
            let Some(waited) = cancel.run(thread.background.written()).await else {
                return Ok(());
            };
            if let Some(ended) = waited
                && let Some((before_tokens, after_tokens)) =
                    self.swap_in(thread, offer, summary, ended).await?
            {
                live(Live::Compacted {
                    before_tokens,
                    after_tokens,
                    waited: true,
                });
            }
            let tokens = next_request_tokens(thread, offer);
            if !past(tokens, window_tokens, at_percent) {
                return Ok(());
            }
        }
        self.compact_whole(thread, offer, summary, cancel, live)
            .await
    }

    /// Compacts the whole conversation, as it outgrows the window with no summary in the
    /// background to swap in.
    async fn compact_whole(
        &self,
        thread: &mut Thread,
        offer: &mut ToolSet,
        summary: &mut TurnResult,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<(), HarnessError> {
        match self.compact_with(thread, offer, cancel, live).await {
            Ok(compacted) => {
                summary.model_calls = summary.model_calls.saturating_add(1);
                summary.take_in(&compacted.completion.responded());
                self.offer_again(thread, offer).await;
                Ok(())
            }
            Err(HarnessError::Provider(error)) => {
                live(Live::CompactionFailed(error.to_string()));
                Ok(())
            }
            // The turn sees the cancel itself and stops.
            Err(HarnessError::Cancelled) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// After a compaction, a thread of its own opens again with the prompt and tools on offer
    /// now; a subagent's are its parent's.
    async fn offer_again(&self, thread: &mut Thread, offer: &mut ToolSet) {
        if !thread.delegated() {
            *offer = self.tools(thread).await;
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
    /// For `/compact`. Cancelling leaves the thread as it was.
    pub async fn compact(
        &self,
        thread: &mut Thread,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Compacted, HarnessError> {
        self.close_left_open(thread).await?;
        let offer = self.tools(thread).await;
        self.compact_with(thread, &offer, cancel, live).await
    }

    /// Summarizes the whole conversation and restarts it from the summary, the latest tool
    /// results and the person's latest request.
    pub(crate) async fn compact_with(
        &self,
        thread: &mut Thread,
        offer: &ToolSet,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Compacted, HarnessError> {
        let before_tokens = next_request_tokens(thread, offer);
        let window_tokens = self.context_window(&self.model_for(thread));
        let at_percent = self.settings.compaction.at_percent;
        let covers = thread.messages.len();
        let request = self.summary_request(thread, offer, covers, Covers::Whole);
        let completion = self.summarize(request, cancel).await?;
        let text = summary_text(&completion);
        if text.trim().is_empty() {
            return Err(ProviderError::Protocol("the model returned no summary".into()).into());
        }
        let through = thread
            .next_seq
            .get()
            .checked_sub(1)
            .and_then(|seq| Seq::new(seq).ok())
            .ok_or_else(|| HarnessError::Event("there is nothing to compact".into()))?;
        let recent = recent::recent(&thread.messages, recent_room(window_tokens, at_percent));
        let restart = Restart {
            completion: &completion,
            summary: &text,
            covers,
            through,
            how: Covers::Whole,
            recent: recent.as_deref(),
        };
        self.restart(thread, &restart).await?;
        thread.background.restarted();
        let after_tokens = next_request_tokens(thread, offer);
        live(Live::Compacted {
            before_tokens,
            after_tokens,
            waited: true,
        });
        Ok(Compacted {
            before_tokens,
            after_tokens,
            completion,
        })
    }

    /// Summarizes the conversation's first `covers` messages as `how` says, and restarts it
    /// from the summary, the rest kept as they were: for the person's own request (`asked.rs`).
    /// A log that does not hold the conversation as the thread does fails as a summary would.
    async fn compact_older(
        &self,
        thread: &mut Thread,
        offer: &mut ToolSet,
        covers: usize,
        how: Covers<'_>,
        cancel: &Cancel,
    ) -> Result<Compacted, HarnessError> {
        let unread = || ProviderError::Protocol("its log could not be read".into());
        let through = self
            .covered_through(thread, covers)
            .await
            .ok_or_else(unread)?;
        let before_tokens = next_request_tokens(thread, offer);
        let request = self.summary_request(thread, offer, covers, how);
        let completion = self.summarize(request, cancel).await?;
        let text = summary_text(&completion);
        if text.trim().is_empty() {
            return Err(ProviderError::Protocol("the model returned no summary".into()).into());
        }
        let restart = Restart {
            completion: &completion,
            summary: &text,
            covers,
            through,
            how,
            recent: None,
        };
        self.restart(thread, &restart).await?;
        thread.background.restarted();
        self.offer_again(thread, offer).await;
        Ok(Compacted {
            before_tokens,
            after_tokens: next_request_tokens(thread, offer),
            completion,
        })
    }

    /// What a summary of `thread`'s first `covers` messages is asked with: the thread's own
    /// prompt and tools, so the provider's cache still holds what they share, and the
    /// instruction for `how` as the last message.
    fn summary_request(
        &self,
        thread: &Thread,
        offer: &ToolSet,
        covers: usize,
        how: Covers<'_>,
    ) -> ModelRequest {
        let mut messages = thread.messages.get(..covers).unwrap_or_default().to_vec();
        messages.push(Message::User {
            parts: vec![Part::Text {
                text: words::instruction(how),
            }],
        });
        // The tools stay listed: some providers refuse a conversation with tool calls but no
        // tools. The instruction asks for text only, and any call is ignored.
        let model = self.model_for(thread);
        ModelRequest {
            max_output_tokens: Some(self.summary_tokens(&model)),
            model,
            system: offer.system.clone(),
            messages,
            tools: offer.specs.clone(),
            tool_choice: ToolChoice::None,
            purpose: Purpose::Summary,
            files: Vec::new(),
            effort: None,
            payer: Some(thread.payer()),
            route: None,
        }
    }

    /// Asks for the summary. A request refused as too long loses its oldest tool outputs and is
    /// asked again, until it fits or there are none left to lose. Only the model call can be
    /// dropped by a cancel: what follows it writes to the log.
    async fn summarize(
        &self,
        mut request: ModelRequest,
        cancel: &Cancel,
    ) -> Result<Completion, HarnessError> {
        loop {
            let asked = cancel
                .run(self.provider.complete(&request, &mut |_| {}))
                .await
                .ok_or(HarnessError::Cancelled)?;
            match asked {
                Err(error) if error.failure() == Failure::TooLong => {
                    self.learn_window(&request.model, &error);
                    if !trim_oldest(&mut request.messages) {
                        return Err(error.into());
                    }
                }
                other => return Ok(other?),
            }
        }
    }
}
