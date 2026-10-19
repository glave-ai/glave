//! The threads a server holds: each idle and ready, or busy with one turn or compaction, and
//! work on a thread run in a task of its own so that a client that goes away never strands it.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError};

use glave_event::ThreadId;
use glave_harness::{Cancel, Commands, HarnessError, Inbox, Live, Thread};
use glave_ports::{BlobStore, EventLog, ModelProvider, Standing};
use jiff::Timestamp;
use tokio::sync::watch;

use crate::protocol::{ApiError, Status, code};
use crate::server::{Inner, Server, no_thread};
use crate::turn::publish;

/// Threads kept in memory while idle, the least recently used let go first; more are rebuilt
/// from the log when asked for.
const IDLE_THREADS: usize = 256;

/// How long a thread with no work stays in memory, as Codex unloads one: 30 minutes.
const IDLE_MILLIS: i64 = 30 * 60 * 1_000;

/// A thread the server holds: idle and ready, or busy with a turn or a compaction.
pub(crate) enum Slot {
    Idle {
        thread: Box<Thread>,
        /// When it was last used: loaded, read, or left by its last work.
        since: Timestamp,
    },
    Busy {
        cancel: Cancel,
        finished: watch::Receiver<bool>,
        /// Where a message sent meanwhile waits for the model's next call, when the work is a
        /// turn or a goal's turns (`steer.rs`).
        inbox: Option<Inbox>,
        /// Its commands in the background, listed and stopped while the work holds it.
        commands: Arc<Commands>,
    },
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Loads `thread` from the log unless it is already held, where its project's folder is
    /// now: refused with `-32029` when that isn't there anymore. A deleted thread is not
    /// there.
    pub(crate) async fn load(&self, thread: ThreadId) -> Result<(), ApiError> {
        if glave_ports::stream::Reserved::of(thread).is_some() {
            return Err(no_thread(thread));
        }
        let now = self.inner.harness.clock().now();
        match self.lock().get_mut(&thread) {
            Some(Slot::Idle { thread: held, .. }) if held.standing() == Standing::Deleted => {
                return Err(no_thread(thread));
            }
            Some(Slot::Idle { since, .. }) => {
                *since = now;
                return Ok(());
            }
            Some(Slot::Busy { .. }) => return Ok(()),
            None => {}
        }
        let scope = self.scope_of(thread).await?;
        let folder = self.folder_now(scope.project()).await?;
        let resumed = self
            .inner
            .harness
            .resume_thread_in(scope, thread, folder.as_deref())
            .await
            .map_err(|error| ApiError::from(&error))?;
        if resumed.standing() == Standing::Deleted {
            return Err(no_thread(thread));
        }
        if !self.lock().contains_key(&thread) {
            self.keep(thread, resumed);
        }
        Ok(())
    }

    /// The folder of the project `project` on this computer, where its threads work: refused
    /// with `-32029` when it isn't there anymore; none when it has none here.
    async fn folder_now(
        &self,
        project: glave_ports::ProjectId,
    ) -> Result<Option<String>, ApiError> {
        match self.folder_found(project).await {
            Err(refused) if crate::gone::is_gone(&refused) => Err(refused),
            found => Ok(found.ok().flatten()),
        }
    }

    /// Holds `thread` idle, letting go of others idle too long or beyond the cap.
    pub(crate) fn keep(&self, id: ThreadId, thread: Thread) {
        let now = self.inner.harness.clock().now();
        hold_idle(&mut self.lock(), id, Box::new(thread), now);
    }

    /// Lets go of the threads idle for 30 minutes, for a tick to call.
    pub(crate) fn let_go_stale(&self) {
        let now = self.inner.harness.clock().now();
        let_go_idle(&mut self.lock(), IDLE_THREADS, now);
    }

    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ThreadId, Slot>> {
        self.inner
            .threads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// What `id` is doing now.
    pub(crate) fn status(&self, id: ThreadId) -> Status {
        match self.lock().get(&id) {
            Some(Slot::Busy { .. }) if self.inner.approvals.waiting(id) => Status::Waiting,
            Some(Slot::Busy { .. }) => Status::Running,
            _ => Status::Idle,
        }
    }

    /// Runs `work` on `thread` in a task of its own, holding the thread busy until it ends, so
    /// a client that goes away never strands it. The result is sent on `done` once the thread
    /// is ready for the next turn.
    pub(crate) async fn occupy<W, F, R>(
        &self,
        thread: ThreadId,
        done: Finish<R>,
        work: W,
    ) -> Result<(), ApiError>
    where
        W: FnOnce(Arc<Inner<P, L, B>>, Thread, Cancel) -> F + Send + 'static,
        F: std::future::Future<Output = (Thread, Result<R, HarnessError>)> + Send + 'static,
        R: Send + Sync + 'static,
    {
        self.occupy_for(thread, done, Use::Work, work).await
    }

    /// Like [`Server::occupy`], for work of the kind `purpose` says.
    pub(crate) async fn occupy_for<W, F, R>(
        &self,
        thread: ThreadId,
        done: Finish<R>,
        purpose: Use,
        work: W,
    ) -> Result<(), ApiError>
    where
        W: FnOnce(Arc<Inner<P, L, B>>, Thread, Cancel) -> F + Send + 'static,
        F: std::future::Future<Output = (Thread, Result<R, HarnessError>)> + Send + 'static,
        R: Send + Sync + 'static,
    {
        // Nothing but changing whether it is shown runs where its project's folder was.
        if purpose != Use::Manage {
            let scope = self.scope_of(thread).await?;
            self.folder_now(scope.project()).await?;
        }
        self.load(thread).await?;
        let cancel = Cancel::new();
        let (finish, finished) = watch::channel(false);
        let held = {
            let mut threads = self.lock();
            // Checked under the threads' lock, which a stop sets it under.
            if self.is_stopping() {
                return Err(crate::stopping::stopping());
            }
            match threads.remove(&thread) {
                // Turns and compactions do not run in an archived thread.
                Some(slot)
                    if purpose != Use::Manage
                        && matches!(&slot, Slot::Idle { thread: held, .. }
                            if held.standing() == Standing::Archived) =>
                {
                    threads.insert(thread, slot);
                    return Err(ApiError::from(&HarnessError::Archived(thread)));
                }
                Some(Slot::Idle { thread: held, .. }) => {
                    // A turn takes messages from now until it has made its last call.
                    let inbox = (purpose == Use::Steered).then(|| held.inbox().clone());
                    if let Some(inbox) = &inbox {
                        inbox.open();
                    }
                    let slot = Slot::Busy {
                        cancel: cancel.clone(),
                        finished,
                        inbox,
                        commands: held.commands(),
                    };
                    threads.insert(thread, slot);
                    *held
                }
                Some(running) => {
                    threads.insert(thread, running);
                    return Err(busy(thread));
                }
                None => return Err(no_thread(thread)),
            }
        };
        let inner = Arc::clone(&self.inner);
        // The work stays in the span of the request that started it.
        let span = tracing::Span::current();
        let task = tokio::spawn(async move {
            let work = work(Arc::clone(&inner), held, cancel);
            let (held, result) = tracing::Instrument::instrument(work, span).await;
            inner.approvals.forget(thread);
            let mut threads = inner.threads.lock().unwrap_or_else(PoisonError::into_inner);
            // Ended with the thread made idle, so a message sent from now on starts a turn:
            // anything still waiting, which only work that ended early leaves, is given back.
            let unread = if purpose == Use::Steered {
                held.inbox().end()
            } else {
                Vec::new()
            };
            let now = inner.harness.clock().now();
            leave(&mut threads, thread, held, &result, now);
            drop(threads);
            for queued in unread {
                let id = queued.id;
                publish(&inner.hub, thread, Live::Unread { id, reason: None });
            }
            let _ = finish.send(true);
            // Kept even when no one is following any more: a retry may still ask for it.
            done.send_replace(Some(Arc::new(result)));
        });
        let mut tasks = self
            .inner
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        tasks.retain(|task| !task.is_finished());
        tasks.push(task);
        Ok(())
    }
}

/// After work on `id` ends at `now` with `result`: held idle again, or, archived or deleted
/// from outside while held, let go, since the work stopped where it was and the thread is read
/// again from the log next time, which closes what it left open.
fn leave<R>(
    threads: &mut HashMap<ThreadId, Slot>,
    id: ThreadId,
    held: Thread,
    result: &Result<R, HarnessError>,
    now: Timestamp,
) {
    if matches!(
        result,
        Err(HarnessError::Archived(_) | HarnessError::NotFound(_))
    ) {
        threads.remove(&id);
    } else {
        hold_idle(threads, id, Box::new(held), now);
    }
}

/// Holds `thread` idle in `threads` as used at `now`, letting go of others idle too long or
/// beyond the cap.
fn hold_idle(
    threads: &mut HashMap<ThreadId, Slot>,
    id: ThreadId,
    thread: Box<Thread>,
    now: Timestamp,
) {
    threads.remove(&id);
    let_go_idle(threads, IDLE_THREADS.saturating_sub(1), now);
    threads.insert(id, Slot::Idle { thread, since: now });
}

/// Lets go of every idle thread in `threads` unused for 30 minutes at `now`, and of the least
/// recently used beyond `room`. Busy threads stay.
fn let_go_idle(threads: &mut HashMap<ThreadId, Slot>, room: usize, now: Timestamp) {
    let idle = threads.iter().map(|(id, slot)| {
        let since = match slot {
            Slot::Idle { since, .. } => Some(*since),
            Slot::Busy { .. } => None,
        };
        (*id, since)
    });
    for id in to_let_go(idle, room, now) {
        threads.remove(&id);
    }
}

/// Of the threads `held`, each with when it was last used if it is idle, those to let go at
/// `now` to keep at most `room` idle: every one unused for 30 minutes, then the least
/// recently used. A busy thread (no time) is never let go.
fn to_let_go(
    held: impl Iterator<Item = (ThreadId, Option<Timestamp>)>,
    room: usize,
    now: Timestamp,
) -> Vec<ThreadId> {
    let mut idle: Vec<(ThreadId, Timestamp)> = held
        .filter_map(|(id, since)| since.map(|since| (id, since)))
        .collect();
    // The most recently used first.
    idle.sort_by_key(|(_, since)| std::cmp::Reverse(*since));
    let stale = |since: Timestamp| {
        now.as_millisecond().saturating_sub(since.as_millisecond()) >= IDLE_MILLIS
    };
    idle.into_iter()
        .enumerate()
        .filter(|(kept, (_, since))| *kept >= room || stale(*since))
        .map(|(_, (id, _))| id)
        .collect()
}

/// What work on a thread is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Use {
    /// Other work on the conversation, a compaction or recording a goal's end: refused in an
    /// archived thread.
    Work,
    /// A turn, or a goal's turns: refused in an archived thread, and messages sent while it
    /// runs are queued to it.
    Steered,
    /// Changing whether the thread is shown.
    Manage,
}

/// The refusal for work on `thread` while other work runs there that takes no messages.
pub(crate) fn busy(thread: ThreadId) -> ApiError {
    ApiError::new(
        code::TURN_RUNNING,
        format!("a turn is already running in thread {thread}"),
    )
}

/// Where work on a thread sends its result: nothing until it ends, then how it ended. Shared,
/// so anyone may follow it, however late.
pub(crate) type Finish<R> = watch::Sender<Option<Arc<Result<R, HarnessError>>>>;

/// How work on a thread ended, once it has; `None` if it stopped without saying.
pub(crate) async fn finished<R>(done: &mut watch::Receiver<Option<Arc<R>>>) -> Option<Arc<R>> {
    loop {
        let current = done.borrow_and_update().clone();
        if current.is_some() {
            return current;
        }
        if done.changed().await.is_err() {
            return done.borrow().clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes: i64) -> Timestamp {
        Timestamp::from_millisecond(1_791_000_000_000 + minutes * 60_000).unwrap()
    }

    fn id(n: u8) -> ThreadId {
        ThreadId::new(Timestamp::UNIX_EPOCH, [n; 10]).unwrap()
    }

    #[test]
    fn the_least_recently_used_idle_thread_goes_first_and_a_busy_one_never() {
        let held = [
            (id(1), Some(at(3))),
            (id(2), Some(at(1))),
            (id(3), None),
            (id(4), Some(at(2))),
        ];
        assert_eq!(to_let_go(held.into_iter(), 2, at(4)), [id(2)]);
        assert_eq!(
            to_let_go(held.into_iter(), 0, at(4)),
            [id(1), id(4), id(2)],
            "every idle one may go, the busy one never"
        );
        assert_eq!(to_let_go(held.into_iter(), 3, at(4)), []);
    }

    #[test]
    fn a_thread_unused_for_30_minutes_is_let_go_and_a_busy_one_never() {
        let held = [(id(1), Some(at(0))), (id(2), Some(at(1))), (id(3), None)];
        assert_eq!(to_let_go(held.into_iter(), IDLE_THREADS, at(29)), []);
        assert_eq!(to_let_go(held.into_iter(), IDLE_THREADS, at(30)), [id(1)]);
        assert_eq!(
            to_let_go(held.into_iter(), IDLE_THREADS, at(600)),
            [id(2), id(1)]
        );
    }
}
