//! The threads a server holds: each idle and ready, or busy with one turn or compaction, and
//! work on a thread run in a task of its own so that a client that goes away never strands it.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError};

use glave_event::ThreadId;
use glave_harness::{Cancel, HarnessError, Thread};
use glave_ports::{BlobStore, EventLog, ModelProvider};
use tokio::sync::{oneshot, watch};

use crate::protocol::{ApiError, Status, ThreadSummary, code};
use crate::server::{Inner, Server, api, no_thread};

/// Threads kept in memory while idle; more are rebuilt from the log when asked for.
const IDLE_THREADS: usize = 64;

/// A thread the server holds: idle and ready, or busy with a turn or a compaction.
pub(crate) enum Slot {
    Idle(Box<Thread>),
    Busy {
        cancel: Cancel,
        finished: watch::Receiver<bool>,
    },
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Loads `thread` from the log unless it is already held.
    pub(crate) async fn load(&self, thread: ThreadId) -> Result<(), ApiError> {
        if self.lock().contains_key(&thread) {
            return Ok(());
        }
        let resumed = self
            .inner
            .harness
            .resume_thread(self.inner.scope, thread)
            .await
            .map_err(|error| api(&error))?;
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

    pub(crate) fn summary(&self, id: ThreadId) -> ThreadSummary {
        let status = match self.lock().get(&id) {
            Some(Slot::Busy { .. }) if self.inner.approvals.waiting(id) => Status::Waiting,
            Some(Slot::Busy { .. }) => Status::Running,
            _ => Status::Idle,
        };
        ThreadSummary { id, status }
    }

    /// Runs `work` on `thread` in a task of its own, holding the thread busy until it ends, so
    /// a client that goes away never strands it. The result comes back on the receiver.
    pub(crate) async fn occupy<W, F, R>(
        &self,
        thread: ThreadId,
        work: W,
    ) -> Result<oneshot::Receiver<Result<R, HarnessError>>, ApiError>
    where
        W: FnOnce(Arc<Inner<P, L, B>>, Thread, Cancel) -> F + Send + 'static,
        F: std::future::Future<Output = (Thread, Result<R, HarnessError>)> + Send + 'static,
        R: Send + 'static,
    {
        self.load(thread).await?;
        let cancel = Cancel::new();
        let (finish, finished) = watch::channel(false);
        let held = {
            let mut threads = self.lock();
            let busy = Slot::Busy {
                cancel: cancel.clone(),
                finished,
            };
            match threads.insert(thread, busy) {
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
        let (send, receive) = oneshot::channel();
        let inner = Arc::clone(&self.inner);
        let task = tokio::spawn(async move {
            let (held, result) = work(Arc::clone(&inner), held, cancel).await;
            inner.approvals.forget(thread);
            inner
                .threads
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(thread, Slot::Idle(Box::new(held)));
            let _ = finish.send(true);
            // The asker may have gone; the work is done either way.
            let _ = send.send(result);
        });
        let mut tasks = self
            .inner
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        tasks.retain(|task| !task.is_finished());
        tasks.push(task);
        Ok(receive)
    }
}
