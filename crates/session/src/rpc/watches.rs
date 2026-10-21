//! What one connection of lines follows: the threads and lists it watches, each once, at most
//! [`MOST_WATCHES`] at a time, until the client unwatches it or the connection's input ends;
//! and the turns it follows. A thread's notifications are written to the connection once: a
//! turn started in a thread the connection watches writes nothing of its own (its watch
//! writes them, and its answer waits for the watch to have written its last event), and a
//! thread whose turn the connection follows is watched once that turn has answered.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use glave_event::{Seq, ThreadId};
use glave_ports::ProjectId;
use tokio::sync::{Notify, watch};

use crate::protocol::{ApiError, code};

/// The most threads and lists one connection watches at once.
const MOST_WATCHES: usize = 64;

/// What a connection watches: a thread, or the list of a project's threads (every project's,
/// when none).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Key {
    Thread(ThreadId),
    List(Option<ProjectId>),
}

/// What a connection follows.
#[derive(Default)]
pub(super) struct Watches(Mutex<Held>);

#[derive(Default)]
struct Held {
    /// Each thread and list watched.
    threads: BTreeMap<Key, Watched>,
    /// How many turns the connection follows in each thread.
    turns: BTreeMap<ThreadId, usize>,
    /// Set once the connection's input ended: it watches nothing more.
    closed: bool,
}

/// One thread, or list, the connection watches.
struct Watched {
    /// What ends the watch.
    end: Arc<Notify>,
    /// How far it has written.
    seen: watch::Receiver<Progress>,
    /// Unwatched while turns it writes ran: it ends once they have answered.
    unwatched: bool,
}

impl Watched {
    fn end(&self) {
        self.end.notify_one();
    }
}

/// How far a watch has come.
#[derive(Debug, Clone, Copy, Default)]
struct Progress {
    /// Whether it follows its thread yet, which it does once the server took it.
    following: bool,
    /// The `seq` of its thread's last event it wrote.
    sent: Option<Seq>,
}

/// One watch held for its request, let go when it drops.
pub(super) struct Claim {
    watches: Arc<Watches>,
    key: Key,
    end: Arc<Notify>,
    progress: watch::Sender<Progress>,
}

/// A turn the connection follows, until it drops: quiet when the connection watches its
/// thread, and how far that watch has come.
pub(super) struct Turn {
    watches: Arc<Watches>,
    thread: ThreadId,
    quiet: Option<watch::Receiver<Progress>>,
}

impl Watches {
    fn held(&self) -> MutexGuard<'_, Held> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Holds a watch of `key` for a request, its client having a thread's events up to
    /// `after`; refused when the connection watches it already, follows a turn in the thread,
    /// watches as many as it may, or has closed.
    pub(super) fn claim(self: &Arc<Self>, key: Key, after: Option<Seq>) -> Result<Claim, ApiError> {
        let mut held = self.held();
        if held.closed {
            return Err(ApiError::new(code::INTERNAL, "the connection closed"));
        }
        if let Key::Thread(thread) = key
            && held.turns.contains_key(&thread)
        {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                format!(
                    "this connection follows a turn in thread {thread}: watch it once the turn has \
                     answered, or from another connection"
                ),
            ));
        }
        if held.threads.contains_key(&key) {
            let watched = match key {
                Key::Thread(thread) => format!("thread {thread}"),
                Key::List(Some(project)) => format!("the threads of project {project}"),
                Key::List(None) => "your threads".to_owned(),
            };
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                format!("this connection watches {watched} already"),
            ));
        }
        if held.threads.len() >= MOST_WATCHES {
            return Err(ApiError::new(
                code::LIMIT,
                format!(
                    "a connection watches at most {MOST_WATCHES} threads and lists at once: \
                     unwatch one first"
                ),
            ));
        }
        let end = Arc::new(Notify::new());
        let (progress, seen) = watch::channel(Progress {
            following: false,
            sent: after,
        });
        let watched = Watched {
            end: Arc::clone(&end),
            seen,
            unwatched: false,
        };
        held.threads.insert(key, watched);
        Ok(Claim {
            watches: Arc::clone(self),
            key,
            end,
            progress,
        })
    }

    /// Follows a turn in `thread`, quiet when the connection watches it.
    pub(super) fn turn(self: &Arc<Self>, thread: ThreadId) -> Turn {
        let mut held = self.held();
        let quiet = held
            .threads
            .get(&Key::Thread(thread))
            .map(|watched| watched.seen.clone());
        *held.turns.entry(thread).or_default() += 1;
        Turn {
            watches: Arc::clone(self),
            thread,
            quiet,
        }
    }

    /// Ends the connection's watch of `key`, if it has one, from now on free to be watched
    /// again; a thread's while turns it writes run, once they have answered, so none goes
    /// silent.
    pub(super) fn end(&self, key: Key) {
        let mut held = self.held();
        let writes_turns = match key {
            Key::Thread(thread) => held.turns.contains_key(&thread),
            Key::List(_) => false,
        };
        match held.threads.get_mut(&key) {
            Some(watched) if writes_turns => watched.unwatched = true,
            Some(_) => {
                if let Some(watched) = held.threads.remove(&key) {
                    watched.end();
                }
            }
            None => {}
        }
    }

    /// Ends every watch, and takes no more: the connection's input ended.
    pub(super) fn close(&self) {
        let mut held = self.held();
        held.closed = true;
        for watched in held.threads.values() {
            watched.end();
        }
    }
}

impl Claim {
    /// Done once the watch is ended: unwatched, or its connection closed.
    pub(super) async fn ended(&self) {
        self.end.notified().await;
    }

    /// The watch follows its thread now.
    pub(super) fn following(&self) {
        self.progress
            .send_modify(|progress| progress.following = true);
    }

    /// The watch wrote its thread's event `seq`.
    pub(super) fn sent(&self, seq: Seq) {
        self.progress
            .send_modify(|progress| progress.sent = progress.sent.max(Some(seq)));
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let mut held = self.watches.held();
        let ours = held
            .threads
            .get(&self.key)
            .is_some_and(|watched| Arc::ptr_eq(&watched.end, &self.end));
        if ours {
            held.threads.remove(&self.key);
        }
    }
}

impl Turn {
    /// Waits until the watch that keeps this turn quiet follows its thread; a watch that ended
    /// first leaves the turn to write its own.
    pub(super) async fn wait_for_watch(&mut self) {
        if let Some(seen) = &mut self.quiet
            && seen.wait_for(|progress| progress.following).await.is_err()
        {
            self.quiet = None;
        }
    }

    /// Whether the connection's watch of the thread writes the turn's notifications.
    pub(super) fn quiet(&self) -> bool {
        self.quiet.is_some()
    }

    /// Waits until the watch has written the thread's event `seq`, or ended.
    pub(super) async fn written(&mut self, seq: Seq) {
        if let Some(seen) = &mut self.quiet {
            let _ = seen
                .wait_for(|progress| progress.sent.is_some_and(|sent| sent >= seq))
                .await;
        }
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        let mut held = self.watches.held();
        if let Some(count) = held.turns.get_mut(&self.thread) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                held.turns.remove(&self.thread);
                // A watch unwatched while it wrote this thread's turns ends now.
                let key = Key::Thread(self.thread);
                if held
                    .threads
                    .get(&key)
                    .is_some_and(|watched| watched.unwatched)
                    && let Some(watched) = held.threads.remove(&key)
                {
                    watched.end();
                }
            }
        }
    }
}
