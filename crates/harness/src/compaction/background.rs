//! A summary of a thread's older half, written in a task of its own while the turn goes on, and
//! swapped in at the first boundary between model calls after it is ready (`compaction.rs`).
//! The thread owns the task: it is stopped when the thread lets go of it or is dropped, so it
//! never outlives the thread, and one runs at a time. It writes nothing: what it returns is
//! recorded only when it is swapped in.

use std::sync::Arc;
use std::time::Duration;

use glave_event::{Seq, TurnResult};
use glave_ports::{
    BlobStore, Completion, EventLog, Message, ModelProvider, ModelRequest, ProviderError,
};
use tokio::task::JoinSet;

use super::size::{CHARS_PER_TOKEN, chars, next_request_tokens, older_half, past};
use super::{Covers, LEAD, OLDER_MIN, Restart, summary_text};
use crate::approval::Approver;
use crate::harness::{Harness, HarnessError, Thread};
use crate::toolset::ToolSet;

/// The longest a summary in the background may take: a slow model writes the longest summary
/// asked for (4,000 tokens) in a few minutes. Past it the summary is given up, and the turn
/// compacts as it would have without one when it must.
const LIMIT: Duration = Duration::from_secs(300);

/// What a summary written in the background covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Older {
    /// How many of the conversation's first messages it summarizes; the rest stay as they were.
    pub(crate) covers: usize,
    /// The characters those messages held, so a summary of messages that changed meanwhile is
    /// never swapped in.
    pub(crate) chars: u64,
    /// The last event it covers.
    pub(crate) through: Seq,
    /// The most words it may take.
    pub(crate) words: u64,
}

/// A summary in the background that has ended: its answer, or why there is none.
pub(crate) type Ended = (Older, Result<Completion, ProviderError>);

/// A thread's summary in the background, if one is being written, and whether the next must
/// wait for a compaction first.
#[derive(Debug, Default)]
pub(crate) struct Background {
    /// The one being written, in a set that stops it when dropped.
    job: Option<(Older, JoinSet<Result<Completion, ProviderError>>)>,
    /// One failed, was given up, or freed too little since the conversation last restarted:
    /// none starts again until it does.
    spent: bool,
}

/// A copy of a thread has no summary in the background: the one being written stays with the
/// thread that started it.
impl Clone for Background {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl Background {
    /// Whether a summary may start now: none is being written, and none is spent.
    pub(crate) fn idle(&self) -> bool {
        self.job.is_none() && !self.spent
    }

    /// Whether one is being written.
    pub(crate) fn writing(&self) -> bool {
        self.job.is_some()
    }

    /// Starts writing `request`'s summary, of `older`, with `provider`, in a task of its own,
    /// in place of any being written. On a runtime of one thread the task runs only while the
    /// turn waits: a yield lets it send its request now.
    pub(crate) async fn start<P: ModelProvider>(
        &mut self,
        provider: &Arc<P>,
        request: ModelRequest,
        older: Older,
    ) {
        let provider = Arc::clone(provider);
        let mut task = JoinSet::new();
        task.spawn(async move {
            let mut unseen = |_| {};
            let asked = provider.complete(&request, &mut unseen);
            tokio::time::timeout(LIMIT, asked)
                .await
                .unwrap_or(Err(ProviderError::Timeout {
                    seconds: LIMIT.as_secs(),
                }))
        });
        self.job = Some((older, task));
        tokio::task::yield_now().await;
    }

    /// The summary, if it has ended, without waiting for one still being written. On a runtime
    /// of one thread the task runs only while the turn waits, so one that is ready gets the
    /// chance to finish first, and is swapped in now rather than a step later.
    pub(crate) async fn ended(&mut self) -> Option<Ended> {
        self.job.as_ref()?;
        tokio::task::yield_now().await;
        let (_, task) = self.job.as_mut()?;
        let joined = task.try_join_next()?;
        let (older, _) = self.job.take()?;
        Some((older, flatten(joined)))
    }

    /// Waits for the summary being written, if there is one: the turn can go no further
    /// without it. Bounded by the task's own limit.
    pub(crate) async fn wait(&mut self) -> Option<Ended> {
        let (_, task) = self.job.as_mut()?;
        let joined = tokio::time::timeout(LIMIT, task.join_next()).await.ok()??;
        let (older, _) = self.job.take()?;
        Some((older, flatten(joined)))
    }

    /// Stops the one being written, if any: another compaction takes its place.
    pub(crate) fn stop(&mut self) {
        self.job = None;
    }

    /// No summary starts in the background until the conversation restarts.
    pub(crate) fn spend(&mut self) {
        self.spent = true;
    }

    /// The conversation restarted from a summary of the whole: one may start again once it
    /// fills.
    pub(crate) fn restarted(&mut self) {
        self.job = None;
        self.spent = false;
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Starts summarizing `thread`'s older half in the background, when it has one worth it:
    /// its first messages up to a response, at least half of what it holds.
    pub(super) async fn start_background(&self, thread: &mut Thread, offer: &ToolSet) {
        let Some(covers) = older_half(&thread.messages) else {
            return;
        };
        let older_chars = chars(thread.messages.get(..covers).unwrap_or_default());
        let older_tokens = older_chars.div_ceil(CHARS_PER_TOKEN);
        if older_tokens < OLDER_MIN {
            return;
        }
        let Some(through) = self.covered_through(thread, covers).await else {
            thread.background.spend();
            return;
        };
        // A third of the words it replaces, at about three words to four tokens.
        let words = older_tokens / 4;
        let request = self.summary_request(thread, offer, covers, Covers::Older { words });
        let older = Older {
            covers,
            chars: older_chars,
            through,
            words,
        };
        thread
            .background
            .start(&self.provider, request, older)
            .await;
    }

    /// Takes in a summary written in the background, `ended`: its usage, always, and the
    /// summary in place of the older half it covers, when it came and frees at least half of
    /// that half, answering the next request's estimated size before and after. One that did
    /// not, or leaves the request still nearing the threshold, keeps another from starting in
    /// the background until the conversation next restarts.
    pub(super) async fn swap_in(
        &self,
        thread: &mut Thread,
        offer: &mut ToolSet,
        summary: &mut TurnResult,
        ended: Ended,
    ) -> Result<Option<(u64, u64)>, HarnessError> {
        let (older, answered) = ended;
        let completion = match answered {
            Ok(completion) => completion,
            Err(error) => {
                tracing::warn!(%error, "the summary in the background failed");
                thread.background.spend();
                return Ok(None);
            }
        };
        summary.model_calls = summary.model_calls.saturating_add(1);
        summary.take_in(&completion.responded());
        let text = summary_text(&completion);
        let size = u64::try_from(text.trim().len()).unwrap_or(u64::MAX);
        let same = thread.messages.get(..older.covers).map(chars) == Some(older.chars);
        let opens = matches!(
            thread.messages.get(older.covers),
            Some(Message::Assistant { .. })
        );
        if size == 0 || size.saturating_mul(2) > older.chars || !same || !opens {
            thread.background.spend();
            let mut events = Vec::new();
            self.stage(thread, &mut events, &completion.responded())?;
            self.commit(thread, events).await?;
            return Ok(None);
        }
        let before_tokens = next_request_tokens(thread, offer);
        let restart = Restart {
            completion: &completion,
            summary: &text,
            covers: older.covers,
            through: older.through,
            how: Covers::Older { words: older.words },
            recent: None,
        };
        self.restart(thread, &restart).await?;
        self.offer_again(thread, offer).await;
        let after_tokens = next_request_tokens(thread, offer);
        let at_percent = self.settings.compaction.at_percent;
        let window_tokens = self.context_window(&self.model_for(thread));
        if past(after_tokens, window_tokens, at_percent.saturating_sub(LEAD)) {
            thread.background.spend();
        }
        Ok(Some((before_tokens, after_tokens)))
    }
}

/// A finished task's answer, or why there is none.
fn flatten(
    joined: Result<Result<Completion, ProviderError>, tokio::task::JoinError>,
) -> Result<Completion, ProviderError> {
    joined.unwrap_or_else(|error| {
        Err(ProviderError::Protocol(format!(
            "the summary in the background stopped: {error}"
        )))
    })
}

#[cfg(test)]
mod tests {
    use glave_event::{ModelRef, Part, StopReason, TokenCounts, Usage};
    use glave_ports::testing::ScriptedModel;
    use tokio::sync::Notify;

    use super::*;

    fn said(text: &str) -> Completion {
        Completion {
            model: ModelRef {
                provider: "script".into(),
                name: "test-model".into(),
            },
            response_id: None,
            parts: vec![Part::Text { text: text.into() }],
            tool_calls: Vec::new(),
            stop: StopReason::EndTurn,
            provider_stop: None,
            usage: Usage::new(TokenCounts::default(), false, None).unwrap(),
            cost: None,
            served: None,
            upstream: None,
        }
    }

    fn request() -> ModelRequest {
        serde_json::from_value(serde_json::json!({
            "model": "test-model", "system": "", "messages": [], "tools": [],
            "purpose": "summary"
        }))
        .unwrap()
    }

    fn older() -> Older {
        Older {
            covers: 2,
            chars: 100,
            through: Seq::FIRST,
            words: 10,
        }
    }

    #[test]
    fn nothing_waits_for_a_summary_until_the_turn_needs_it() {
        glave_ports::block_on!(async {
            let gate = Arc::new(Notify::new());
            let script = ScriptedModel::new(vec![said("Older.")]).gated(Arc::clone(&gate));
            let model = Arc::new(script);
            let mut background = Background::default();
            assert!(background.idle() && background.ended().await.is_none());
            background.start(&model, request(), older()).await;
            assert!(background.writing() && !background.idle());
            assert!(background.ended().await.is_none(), "not written yet");
            assert!(background.clone().idle(), "a copy of the thread has none");
            gate.notify_one();
            let (covered, answer) = background.wait().await.unwrap();
            assert_eq!(covered, older());
            assert_eq!(answer.unwrap().parts, said("Older.").parts);
            assert!(background.idle(), "one may start again");
            background.spend();
            assert!(!background.idle());
            background.restarted();
            assert!(background.idle());
        });
    }

    #[test]
    fn a_summary_being_written_stops_with_its_thread() {
        glave_ports::block_on!(async {
            let gate = Arc::new(Notify::new());
            let model = Arc::new(ScriptedModel::new(vec![said("Older.")]).gated(gate));
            let mut background = Background::default();
            background.start(&model, request(), older()).await;
            assert_eq!(Arc::strong_count(&model), 2, "the task holds the model");
            drop(background);
            tokio::task::yield_now().await;
            assert_eq!(Arc::strong_count(&model), 1, "stopped, it holds nothing");
        });
    }
}
