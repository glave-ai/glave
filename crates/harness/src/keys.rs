//! The idempotency keys a thread's messages were sent with (`specs/invariants.md`, 4): each
//! key a person sent a message with is recorded on its `message.added`, so a retry with the
//! same key finds it in the log, on whatever server it reaches, and starts nothing. A thread
//! read again from its log reads them back; one held here keeps them as it records them. Only
//! the most recent are kept, each with how the turn that read its message ended.

use std::collections::VecDeque;

use glave_event::{Body, Event, PrincipalId, TurnResult};

use crate::harness::Thread;
use crate::inbox::Queued;

/// Keys a thread keeps, most recent last: a key older than these is sent anew, as a server
/// that remembers keys in memory forgets them.
const KEPT: usize = 256;

/// One message sent with a key: who sent it, when not the person the thread is for, the key,
/// and how the turn that read it ended, once it has.
#[derive(Debug, Clone)]
struct Keyed {
    author: Option<PrincipalId>,
    key: String,
    ended: Option<TurnResult>,
}

/// The keys a thread's messages were sent with, most recent last.
#[derive(Debug, Clone, Default)]
pub(crate) struct Keys {
    recent: VecDeque<Keyed>,
}

/// What a message sent with a key came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SentBefore {
    /// The turn that read it ended so.
    Ended(TurnResult),
    /// The turn that read it has not ended.
    Open,
}

impl Keys {
    /// The keys `events`, a thread's log, record.
    pub(crate) fn of(events: &[Event]) -> Self {
        let mut keys = Self::default();
        for event in events {
            match &event.body {
                Body::MessageAdded(added) => {
                    if let Some(key) = &added.key {
                        let author = added.author.as_ref().and_then(|author| author.id);
                        keys.sent(author, key);
                    }
                }
                Body::TurnEnded(ended) => keys.ended(ended),
                _ => {}
            }
        }
        keys
    }

    /// Keeps that `author` sent a message with `key`, read by a turn not yet ended.
    pub(crate) fn sent(&mut self, author: Option<PrincipalId>, key: &str) {
        if self.recent.len() >= KEPT {
            self.recent.pop_front();
        }
        self.recent.push_back(Keyed {
            author,
            key: key.to_owned(),
            ended: None,
        });
    }

    /// Keeps that the turn that read every message not yet ended `ended` so.
    pub(crate) fn ended(&mut self, ended: &TurnResult) {
        for keyed in self.recent.iter_mut().rev() {
            if keyed.ended.is_some() {
                break;
            }
            keyed.ended = Some(ended.clone());
        }
    }
}

impl Thread {
    /// What `person`'s message sent with `key` came to, when one was: how the turn that read
    /// it ended, or that it has not. Read from the log as the thread was, so under the
    /// thread's lease it answers for every server.
    pub(crate) fn sent_before(&self, person: Option<PrincipalId>, key: &str) -> Option<SentBefore> {
        let author = person.filter(|person| Some(*person) != self.for_);
        self.keys
            .recent
            .iter()
            .rev()
            .find(|keyed| keyed.key == key && keyed.author == author)
            .map(|keyed| match &keyed.ended {
                Some(ended) => SentBefore::Ended(ended.clone()),
                None => SentBefore::Open,
            })
    }
}

impl Thread {
    /// Whether `queued` was sent with a key a message of the log already holds: a retry of a
    /// message recorded before, on this server or another, which is never recorded again.
    pub(crate) fn sent_already(&self, queued: &Queued) -> bool {
        queued
            .key
            .as_deref()
            .is_some_and(|key| self.sent_before(queued.caller.person, key).is_some())
    }
}

#[cfg(test)]
mod tests {
    use glave_event::Outcome;

    use super::*;

    #[test]
    fn a_key_names_the_turn_that_read_it_and_only_the_most_recent_are_kept() {
        let mut keys = Keys::default();
        keys.sent(None, "first");
        keys.ended(&TurnResult::begun());
        keys.sent(None, "second");
        assert!(keys.recent.iter().any(|keyed| keyed.ended.is_none()));
        let completed = TurnResult {
            outcome: Outcome::Completed,
            ..TurnResult::begun()
        };
        keys.ended(&completed);
        let ends: Vec<_> = keys
            .recent
            .iter()
            .map(|keyed| keyed.ended.clone())
            .collect();
        assert_eq!(ends, [Some(TurnResult::begun()), Some(completed)]);
        for n in 0..KEPT {
            keys.sent(None, &n.to_string());
        }
        assert_eq!(keys.recent.len(), KEPT);
        assert!(keys.recent.iter().all(|keyed| keyed.key != "first"));
    }
}
