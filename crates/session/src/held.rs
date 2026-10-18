//! The threads a server holds: each idle and ready, or busy with one turn or compaction, and
//! work on a thread run in a task of its own so that a client that goes away never strands it.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError};

use glave_event::ThreadId;
use glave_harness::{Cancel, Commands, HarnessError, Inbox, Live, Thread};
use glave_ports::{BlobStore, EventLog, ModelProvider, Standing};
use tokio::sync::watch;

use crate::protocol::{ApiError, Status, code};
use crate::server::{Inner, Server, api, no_thread};
use crate::turn::publish;

/// Threads kept in memory while idle; more are rebuilt from the log when asked for.
const IDLE_THREADS: usize = 64;

/// A thread the server holds: idle and ready, or busy with a turn or a compaction.
pub(crate) enum Slot {
    Idle(Box<Thread>),
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
        if crate::ledger::is_ledger(thread) {
            return Err(no_thread(thread));
        }
        match self.lock().get(&thread) {
            Some(Slot::Idle(held)) if held.standing() == Standing::Deleted => {
                return Err(no_thread(thread));
            }
            Some(_) => return Ok(()),
            None => {}
        }
        let scope = self.scope_of(thread).await?;
        let folder = self.folder_now(scope.space).await?;
        let resumed = self
            .inner
            .harness
            .resume_thread_in(scope, thread, folder.as_deref())
            .await
            .map_err(|error| api(&error))?;
        if resumed.standing() == Standing::Deleted {
            return Err(no_thread(thread));
        }
        if !self.lock().contains_key(&thread) {
            self.keep(thread, resumed);
        }
        Ok(())
    }

    /// The folder of the project `space` on this computer, where its threads work: refused
    /// with `-32029` when it isn't there anymore; none when it has none here.
    async fn folder_now(&self, space: glave_ports::SpaceId) -> Result<Option<String>, ApiError> {
        match self.folder_found(space).await {
            Err(refused) if crate::gone::is_gone(&refused) => Err(refused),
            found => Ok(found.ok().flatten()),
        }
    }

    /// Holds `thread` idle, letting go of others beyond the cap.
    pub(crate) fn keep(&self, id: ThreadId, thread: Thread) {
        let mut threads = self.lock();
        let idle: Vec<ThreadId> = threads
            .iter()
            .filter(|(other, slot)| **other != id && matches!(slot, Slot::Idle(_)))
            .map(|(other, _)| *other)
            .collect();
        for other in idle.iter().skip(IDLE_THREADS.saturating_sub(1)) {
            threads.remove(other);
        }
        threads.insert(id, Slot::Idle(Box::new(thread)));
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
            self.folder_now(scope.space).await?;
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
                Some(Slot::Idle(held))
                    if purpose != Use::Manage && held.standing() == Standing::Archived =>
                {
                    threads.insert(thread, Slot::Idle(held));
                    return Err(api(&HarnessError::Archived(thread)));
                }
                Some(Slot::Idle(held)) => {
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
            // Archived or deleted from outside while held: the work stopped where it was, so
            // the thread is read again from the log next time, which closes what it left open.
            if matches!(
                result,
                Err(HarnessError::Archived(_) | HarnessError::NotFound(_))
            ) {
                threads.remove(&thread);
            } else {
                threads.insert(thread, Slot::Idle(Box::new(held)));
            }
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
