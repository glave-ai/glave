//! The threads one connection of lines watches: each once, at most [`MOST_WATCHES`] at a time,
//! each until the client unwatches it or the connection's input ends.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use glave_event::ThreadId;
use tokio::sync::Notify;

use crate::protocol::{ApiError, code};

/// The most threads one connection watches at once.
const MOST_WATCHES: usize = 64;

/// A connection's watches, by thread: what ends each.
#[derive(Default)]
pub(super) struct Watches(Mutex<Held>);

#[derive(Default)]
struct Held {
    threads: BTreeMap<ThreadId, Arc<Notify>>,
    /// Set once the connection's input ended: it watches nothing more.
    closed: bool,
}

/// One watch held for its request, let go when it drops.
pub(super) struct Claim {
    watches: Arc<Watches>,
    thread: ThreadId,
    end: Arc<Notify>,
}

impl Watches {
    fn held(&self) -> MutexGuard<'_, Held> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Holds a watch of `thread` for a request; refused when the connection watches it
    /// already, or as many threads as it may, or has closed.
    pub(super) fn claim(self: &Arc<Self>, thread: ThreadId) -> Result<Claim, ApiError> {
        let mut held = self.held();
        if held.closed {
            return Err(ApiError::new(code::INTERNAL, "the connection closed"));
        }
        if held.threads.contains_key(&thread) {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                format!("this connection watches thread {thread} already"),
            ));
        }
        if held.threads.len() >= MOST_WATCHES {
            return Err(ApiError::new(
                code::LIMIT,
                format!(
                    "a connection watches at most {MOST_WATCHES} threads at once: unwatch one \
                     first"
                ),
            ));
        }
        let end = Arc::new(Notify::new());
        held.threads.insert(thread, Arc::clone(&end));
        Ok(Claim {
            watches: Arc::clone(self),
            thread,
            end,
        })
    }

    /// Ends the connection's watch of `thread`, if it has one.
    pub(super) fn end(&self, thread: ThreadId) {
        if let Some(end) = self.held().threads.get(&thread) {
            end.notify_one();
        }
    }

    /// Ends every watch, and takes no more: the connection's input ended.
    pub(super) fn close(&self) {
        let mut held = self.held();
        held.closed = true;
        for end in held.threads.values() {
            end.notify_one();
        }
    }
}

impl Claim {
    /// Done once the watch is ended: unwatched, or its connection closed.
    pub(super) async fn ended(&self) {
        self.end.notified().await;
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let mut held = self.watches.held();
        let ours = held
            .threads
            .get(&self.thread)
            .is_some_and(|end| Arc::ptr_eq(end, &self.end));
        if ours {
            held.threads.remove(&self.thread);
        }
    }
}
