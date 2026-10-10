//! Retrying `turn/start` safely: a turn started with an idempotency key is remembered, so the
//! same key for the same thread returns that turn instead of starting another. The memory is
//! bounded: the most recent keys are kept, and a key older than that starts a new turn.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::ThreadId;
use tokio::sync::watch;

use crate::protocol::IdempotencyKey;
use crate::server::Ended;

/// Keys remembered, across all threads.
const KEPT: usize = 1_024;

/// How a turn ended, once it has.
pub(crate) type Done = watch::Receiver<Option<Arc<Ended>>>;

/// The turns started with a key, most recent last.
#[derive(Default)]
pub(crate) struct Keyed {
    recent: Mutex<VecDeque<(ThreadId, IdempotencyKey, Done)>>,
}

impl Keyed {
    /// The turn already started in `thread` with `key`, if there is one. Otherwise `done`
    /// becomes that turn, in the same step, so two retries racing each other start one turn.
    pub(crate) fn claim(
        &self,
        thread: ThreadId,
        key: &IdempotencyKey,
        done: &Done,
    ) -> Option<Done> {
        let mut recent = self.recent.lock().unwrap_or_else(PoisonError::into_inner);
        let started = recent
            .iter()
            .find(|(of, kept, _)| *of == thread && kept == key)
            .map(|(_, _, started)| started.clone());
        if started.is_none() {
            if recent.len() >= KEPT {
                recent.pop_front();
            }
            recent.push_back((thread, key.clone(), done.clone()));
        }
        started
    }

    /// Forgets `key` when the turn it named could not start, so a retry may start it.
    pub(crate) fn release(&self, thread: ThreadId, key: &IdempotencyKey, done: &Done) {
        self.recent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(of, kept, started)| {
                !(*of == thread && kept == key && started.same_channel(done))
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_names_one_turn_per_thread_and_memory_is_bounded() {
        let keyed = Keyed::default();
        let thread = |n: u8| ThreadId::new(jiff::Timestamp::UNIX_EPOCH, [n; 10]).unwrap();
        let key = |text: &str| IdempotencyKey::new(text).unwrap();
        let (_first, done) = watch::channel(None);
        assert!(keyed.claim(thread(1), &key("a"), &done).is_none());
        let (_second, other) = watch::channel(None);
        let again = keyed.claim(thread(1), &key("a"), &other).unwrap();
        assert!(again.same_channel(&done));
        // The same key in another thread is another turn.
        assert!(keyed.claim(thread(2), &key("a"), &other).is_none());
        keyed.release(thread(1), &key("a"), &done);
        assert!(keyed.claim(thread(1), &key("a"), &other).is_none());
        for n in 0..KEPT {
            let _ = keyed.claim(thread(3), &key(&n.to_string()), &other);
        }
        assert_eq!(keyed.recent.lock().unwrap().len(), KEPT);
        assert!(
            keyed.claim(thread(2), &key("a"), &done).is_none(),
            "the oldest was let go"
        );
    }
}
