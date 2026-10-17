//! The threads a server holds: each idle and ready, or busy with one turn or compaction, and
//! work on a thread run in a task of its own so that a client that goes away never strands it.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError};

use glave_event::ThreadId;
use glave_harness::{Cancel, Commands, HarnessError, Standing, Thread};
use glave_ports::{BlobStore, EventLog, ModelProvider};
use tokio::sync::watch;

use crate::protocol::{ApiError, Status, code};
use crate::server::{Inner, Server, api, no_thread};

/// Threads kept in memory while idle; more are rebuilt from the log when asked for.
const IDLE_THREADS: usize = 64;

/// A thread the server holds: idle and ready, or busy with a turn or a compaction.
pub(crate) enum Slot {
    Idle(Box<Thread>),
    Busy {
        cancel: Cancel,
        finished: watch::Receiver<bool>,
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
    /// Loads `thread` from the log unless it is already held. A deleted thread is not there.
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
        let resumed = self
            .inner
            .harness
            .resume_thread(scope, thread)
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
        self.occupy_for(thread, done, Use::Turn, work).await
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
        self.load(thread).await?;
        let cancel = Cancel::new();
        let (finish, finished) = watch::channel(false);
        let commands = match self.lock().get(&thread) {
            Some(Slot::Idle(held)) => held.commands(),
            _ => Arc::default(),
        };
        let held = {
            let mut threads = self.lock();
            let busy = Slot::Busy {
                cancel: cancel.clone(),
                finished,
                commands,
            };
            match threads.insert(thread, busy) {
                // Turns and compactions do not run in an archived thread.
                Some(Slot::Idle(held))
                    if purpose == Use::Turn && held.standing() == Standing::Archived =>
                {
                    threads.insert(thread, Slot::Idle(held));
                    return Err(api(&HarnessError::Archived(thread)));
                }
                Some(Slot::Idle(held)) => *held,
                Some(running) => {
                    threads.insert(thread, running);
                    return Err(ApiError::new(
                        code::TURN_RUNNING,
                        format!("a turn is already running in thread {thread}"),
                    ));
                }
                None => {
                    threads.remove(&thread);
                    return Err(no_thread(thread));
                }
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
    /// A turn, a goal or a compaction: refused in an archived thread.
    Turn,
    /// Changing whether the thread is shown.
    Manage,
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
