//! A log in memory that fails when told, as a store that blips does, writing or reading
//! nothing then: the next write holding an event of a given kind, and the next read.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use glave_event::{RawEvent, Seq, ThreadId};
use glave_ports::search::Found;
use glave_ports::{EventLog, InMemoryLog, Located, LogError, Scope, ThreadQuery, ThreadView};

/// A log in memory, failing once each time it is told to.
#[derive(Clone, Default)]
pub(crate) struct FailingLog {
    log: Arc<InMemoryLog>,
    write: Arc<Mutex<Option<&'static str>>>,
    read: Arc<AtomicBool>,
}

impl FailingLog {
    /// Fails the next write that holds an event of `kind`.
    pub(crate) fn fail_write_of(&self, kind: &'static str) {
        *self.write.lock().unwrap() = Some(kind);
    }

    /// Fails the next read.
    pub(crate) fn fail_read(&self) {
        self.read.store(true, Ordering::SeqCst);
    }
}

fn blipped() -> LogError {
    LogError::Storage("the store did not answer".into())
}

#[deny(clippy::missing_trait_methods)]
impl EventLog for FailingLog {
    fn append(
        &self,
        scope: &Scope,
        events: &[RawEvent],
    ) -> impl Future<Output = Result<(), LogError>> + Send {
        let fails = {
            let mut write = self.write.lock().unwrap();
            let holds = |kind: &str| events.iter().any(|event| event.kind.as_str() == kind);
            write.take_if(|kind| holds(kind)).is_some()
        };
        let appended = (!fails).then(|| self.log.append(scope, events));
        async move {
            match appended {
                Some(appended) => appended.await,
                None => Err(blipped()),
            }
        }
    }

    fn read(
        &self,
        scope: &Scope,
        thread: ThreadId,
        after: Option<Seq>,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<RawEvent>, LogError>> + Send {
        let fails = self.read.swap(false, Ordering::SeqCst);
        let read = (!fails).then(|| self.log.read(scope, thread, after, limit));
        async move {
            match read {
                Some(read) => read.await,
                None => Err(blipped()),
            }
        }
    }

    fn search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Found>, LogError>> + Send {
        self.log.search(scope, query, limit)
    }

    fn locate(
        &self,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<Located>, LogError>> + Send {
        self.log.locate(thread)
    }

    fn thread_views(
        &self,
        scope: &Scope,
        query: &ThreadQuery,
    ) -> impl Future<Output = Result<Vec<ThreadView>, LogError>> + Send {
        self.log.thread_views(scope, query)
    }

    fn thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<ThreadView>, LogError>> + Send {
        self.log.thread_view(scope, thread)
    }

    fn rebuild_thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<ThreadView>, LogError>> + Send {
        self.log.rebuild_thread_view(scope, thread)
    }

    fn latest_of(
        &self,
        scope: &Scope,
        thread: ThreadId,
        kinds: &[&str],
    ) -> impl Future<Output = Result<Option<RawEvent>, LogError>> + Send {
        self.log.latest_of(scope, thread, kinds)
    }
}
