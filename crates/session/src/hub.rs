//! Where a thread's notifications go: one bounded broadcast per watched thread, fed by the log as
//! events are appended and by running turns as they make progress. A watcher that falls too far
//! behind is told so, and catches up from the log by `seq`; nothing is buffered without bound.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::{RawEvent, Seq, ThreadId};
use glave_ports::{EventLog, LogError, Scope};
use tokio::sync::broadcast;

use crate::protocol::Notification;

/// Notifications held for a slow watcher before it is told it fell behind.
const BACKLOG: usize = 1_024;

/// The broadcasts of every watched thread.
#[derive(Default)]
pub(crate) struct Hub {
    threads: Mutex<HashMap<ThreadId, broadcast::Sender<Arc<Notification>>>>,
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

    /// Sends `notification` to everyone watching `thread`, forgetting threads no one watches.
    pub(crate) fn publish(&self, thread: ThreadId, notification: Notification) {
        let mut threads = self.threads.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(sender) = threads.get(&thread) {
            if sender.receiver_count() == 0 {
                threads.remove(&thread);
            } else {
                // Only fails when no one is listening, which is fine.
                let _ = sender.send(Arc::new(notification));
            }
        }
    }
}

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
            let notification = Notification::Event {
                thread: event.thread,
                event: event.clone(),
            };
            self.hub.publish(event.thread, notification);
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
}
