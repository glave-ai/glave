//! Where a thread's notifications go: one bounded broadcast per watched thread, fed by the log as
//! events are appended and by running turns as they make progress. A watcher that falls too far
//! behind is told so, and catches up from the log by `seq`; nothing is buffered without bound.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::{Body, Event, RawEvent, Seq, ThreadId, decode};
use glave_ports::{EventLog, LogError, Scope};
use tokio::sync::broadcast;

use crate::protocol::Notification;

/// Notifications held for a slow watcher before it is told it fell behind.
const BACKLOG: usize = 1_024;

/// The broadcasts of every watched thread.
#[derive(Default)]
pub(crate) struct Hub {
    threads: Mutex<HashMap<ThreadId, broadcast::Sender<Arc<Notification>>>>,
    /// Each running subagent's parent: its notifications go to the parent's watchers too.
    parents: Mutex<HashMap<ThreadId, ThreadId>>,
}

impl Hub {
    /// Starts watching `thread`: everything published from now on.
    pub(crate) fn watch(&self, thread: ThreadId) -> broadcast::Receiver<Arc<Notification>> {
        let mut threads = self.threads.lock().unwrap_or_else(PoisonError::into_inner);
        threads
            .entry(thread)
            .or_insert_with(|| broadcast::channel(BACKLOG).0)
            .subscribe()
    }

    /// Sends `notification` to everyone watching `thread` or a thread it is a child of,
    /// forgetting threads no one watches.
    pub(crate) fn publish(&self, thread: ThreadId, notification: Notification) {
        let notification = Arc::new(notification);
        let mut to = Some(thread);
        // A child's parent may itself be a child; a cycle could never be recorded, but the walk
        // is bounded all the same.
        for _ in 0..MOST_GENERATIONS {
            let Some(watched) = to else {
                break;
            };
            self.send(watched, &notification);
            to = self
                .parents
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&watched)
                .copied();
        }
    }

    fn send(&self, thread: ThreadId, notification: &Arc<Notification>) {
        let mut threads = self.threads.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(sender) = threads.get(&thread) {
            if sender.receiver_count() == 0 {
                threads.remove(&thread);
            } else {
                // Only fails when no one is listening, which is fine.
                let _ = sender.send(Arc::clone(notification));
            }
        }
    }

    /// Notes that `child` was started by a call in `parent`, until `child`'s turn ends.
    fn adopt(&self, child: ThreadId, parent: ThreadId) {
        self.parents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(child, parent);
    }

    /// `thread`, then the threads that started it, nearest first.
    pub(crate) fn lineage(&self, thread: ThreadId) -> Vec<ThreadId> {
        let parents = self.parents.lock().unwrap_or_else(PoisonError::into_inner);
        let mut lineage = vec![thread];
        let mut at = thread;
        for _ in 0..MOST_GENERATIONS {
            match parents.get(&at) {
                Some(parent) => {
                    lineage.push(*parent);
                    at = *parent;
                }
                None => break,
            }
        }
        lineage
    }

    fn release(&self, child: ThreadId) {
        self.parents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&child);
    }
}

/// How far up a child's notifications travel.
const MOST_GENERATIONS: usize = 8;

/// An event log that publishes every event it appends.
pub(crate) struct Observed<L> {
    inner: L,
    hub: Arc<Hub>,
}

impl<L> Observed<L> {
    pub(crate) fn new(inner: L, hub: Arc<Hub>) -> Self {
        Self { inner, hub }
    }
}

impl<L: EventLog> EventLog for Observed<L> {
    async fn append(&self, scope: &Scope, events: &[RawEvent]) -> Result<(), LogError> {
        self.inner.append(scope, events).await?;
        for event in events {
            if event.kind.as_str() == "thread.started"
                && let Ok(Event {
                    body: Body::ThreadStarted(started),
                    ..
                }) = decode(event.clone())
                && let Some(parent) = started.parent
            {
                self.hub.adopt(event.thread, parent.thread);
            }
            let notification = Notification::Event {
                thread: event.thread,
                event: event.clone(),
            };
            self.hub.publish(event.thread, notification);
            if event.kind.as_str() == "turn.ended" {
                self.hub.release(event.thread);
            }
        }
        Ok(())
    }

    fn read(
        &self,
        scope: &Scope,
        thread: ThreadId,
        after: Option<Seq>,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<RawEvent>, LogError>> + Send {
        self.inner.read(scope, thread, after, limit)
    }

    fn threads(
        &self,
        scope: &Scope,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<ThreadId>, LogError>> + Send {
        self.inner.threads(scope, limit)
    }

    fn search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<glave_ports::search::Found>, LogError>> + Send {
        self.inner.search(scope, query, limit)
    }

    fn latest_of(
        &self,
        scope: &Scope,
        thread: ThreadId,
        kinds: &[&str],
    ) -> impl Future<Output = Result<Option<RawEvent>, LogError>> + Send {
        self.inner.latest_of(scope, thread, kinds)
    }

    fn locate(
        &self,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<Scope>, LogError>> + Send {
        self.inner.locate(thread)
    }

    fn thread_views(
        &self,
        scope: &Scope,
        query: &glave_ports::ThreadQuery,
    ) -> impl Future<Output = Result<Vec<glave_ports::ThreadView>, LogError>> + Send {
        self.inner.thread_views(scope, query)
    }

    fn thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<glave_ports::ThreadView>, LogError>> + Send {
        self.inner.thread_view(scope, thread)
    }

    fn rebuild_thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<glave_ports::ThreadView>, LogError>> + Send {
        self.inner.rebuild_thread_view(scope, thread)
    }
}
