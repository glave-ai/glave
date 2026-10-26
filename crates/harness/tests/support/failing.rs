//! A log in memory that fails when told, as a store that blips does: the next write or read
//! holding an event of a given kind fails, writing or reading nothing.

use std::future::Future;
use std::sync::{Arc, Mutex};

use glave_event::{RawEvent, Seq, ThreadId};
use glave_ports::search::Found;
use glave_ports::{EventLog, InMemoryLog, Located, LogError, Scope, ThreadQuery, ThreadView};

/// A log in memory, failing once each time it is told to.
#[derive(Clone, Default)]
pub(crate) struct FailingLog {
    log: Arc<InMemoryLog>,
    /// The kind of event whose next write or read fails, until one does.
    fails: Arc<Mutex<Option<&'static str>>>,
}

impl FailingLog {
    /// Fails the next write or read that holds an event of `kind`.
    pub(crate) fn fail_with(&self, kind: &'static str) {
        *self.fails.lock().unwrap() = Some(kind);
    }
}

/// Whether `events` hold an event of the kind `fails` names, which no longer fails if so.
fn strikes(fails: &Mutex<Option<&'static str>>, events: &[RawEvent]) -> bool {
    let holds = |kind: &mut &'static str| events.iter().any(|event| event.kind.as_str() == *kind);
    fails.lock().unwrap().take_if(holds).is_some()
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
        let appended = (!strikes(&self.fails, events)).then(|| self.log.append(scope, events));
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
        let read = self.log.read(scope, thread, after, limit);
        let fails = Arc::clone(&self.fails);
        async move {
            let events = read.await?;
            if strikes(&fails, &events) {
                return Err(blipped());
            }
            Ok(events)
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
