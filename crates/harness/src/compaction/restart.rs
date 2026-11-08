//! Restarting a conversation from a summary: the thread's opening instructions and the summary
//! first, then the messages it does not cover as they were, recorded as `thread.compacted` with
//! the summary's usage before it.

use glave_event::{Part, Seq, ThreadCompacted};
use glave_ports::{BlobStore, Completion, EventLog, Message, ModelProvider};

use super::Covers;
use super::words;
use crate::approval::Approver;
use crate::harness::{Harness, HarnessError, Thread};

/// How a conversation restarts from a summary.
pub(super) struct Restart<'a> {
    /// The summary call's answer, whose usage is recorded.
    pub(super) completion: &'a Completion,
    /// Its text.
    pub(super) summary: &'a str,
    /// How many of the conversation's first messages it covers; the rest stay as they were.
    pub(super) covers: usize,
    /// The last event it covers.
    pub(super) through: Seq,
    pub(super) how: Covers<'a>,
    /// The latest tool results, kept word for word in the restart message.
    pub(super) recent: Option<&'a str>,
}

/// Whether `messages` hold `request`, the person's latest, in a message of their side.
fn holds(messages: &[Message], request: &str) -> bool {
    messages.iter().any(|message| match message {
        Message::User { parts } => parts
            .iter()
            .any(|part| matches!(part, Part::Text { text } if text.contains(request))),
        _ => false,
    })
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// The last event before `thread`'s message `covers` began, as its log has it: what a
    /// summary of the messages before it covers. `None` when the log cannot be read, or holds
    /// the conversation otherwise than the thread does.
    pub(super) async fn covered_through(&self, thread: &Thread, covers: usize) -> Option<Seq> {
        let events = match self.load(&thread.scope, thread.id).await {
            Ok(events) => events,
            Err(error) => {
                tracing::warn!(%error, "the log could not be read for a compaction");
                return None;
            }
        };
        let began = crate::conversation::beginnings(&events);
        if began.len() != thread.messages.len() {
            tracing::warn!("the log holds the conversation otherwise: it is compacted whole");
            return None;
        }
        Seq::new(began.get(covers)?.get().checked_sub(1)?).ok()
    }

    /// Restarts `thread`'s conversation as `restart` says: from the summary, after the
    /// thread's opening instructions, then the messages it does not cover as they were.
    /// Records the summary's usage, then `thread.compacted`.
    pub(super) async fn restart(
        &self,
        thread: &mut Thread,
        restart: &Restart<'_>,
    ) -> Result<(), HarnessError> {
        let kept = thread.messages.get(restart.covers..).unwrap_or_default();
        let request = (thread.request.as_deref()).filter(|request| !holds(kept, request));
        // The project's instructions open the restarted conversation, as they opened the first.
        let mut parts = thread.opening_parts().unwrap_or_default();
        parts.push(Part::Text {
            text: words::restart(restart.how, restart.summary, restart.recent, request),
        });
        let compacted = ThreadCompacted {
            through: restart.through,
            parts: parts.clone(),
            kept: u32::try_from(kept.len()).unwrap_or(u32::MAX),
            untrusted: thread.untrusted,
        };
        let kept = kept.to_vec();
        let mut events = Vec::new();
        self.stage(thread, &mut events, &restart.completion.responded())?;
        self.stage(thread, &mut events, &compacted)?;
        self.commit(thread, events).await?;
        thread.messages = std::iter::once(Message::User { parts })
            .chain(kept)
            .collect();
        thread.measured = None;
        // The conversation starts again, and so may its prefix (`prefix.rs`).
        thread.prefix = crate::prefix::Prefixed::None;
        // The start shows every memory the thread knows now, and the notes fold away.
        if self.memory_for(thread).is_some() {
            thread.memory.restart();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_event::CallId;

    #[test]
    fn the_latest_request_is_held_word_for_word_or_not_at_all() {
        let text = |text: &str| vec![Part::Text { text: text.into() }];
        let messages = [Message::User {
            parts: text("please fix the build now"),
        }];
        assert!(holds(&messages, "fix the build"));
        assert!(!holds(&messages, "deploy it"));
        let result = Message::Tool {
            call_id: CallId::new("call_1").unwrap(),
            is_error: false,
            output: text("fix the build"),
        };
        assert!(
            !holds(&[result], "fix the build"),
            "a tool's words are not the person's"
        );
    }
}
