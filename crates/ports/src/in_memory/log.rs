//! An event log held in memory, for tests and for runs that keep nothing.

use std::collections::{BTreeSet, HashMap};
use std::future::{Future, ready};
use std::sync::Mutex;

use glave_event::{RawEvent, Seq, ThreadId};

use crate::log::{EventLog, LogError, check_append};
use crate::scope::{Located, Scope};
use crate::search::{Found, indexed, query_terms};
use crate::view::{ThreadQuery, ThreadView};

/// One searchable event and its words.
type Indexed = (Found, BTreeSet<String>);

/// An event log held in memory. Nothing survives the process.
#[derive(Debug, Default)]
pub struct InMemoryLog {
    threads: Mutex<HashMap<(Scope, ThreadId), Thread>>,
    /// Each scope's searchable events and their words, oldest first.
    index: Mutex<HashMap<Scope, Vec<Indexed>>>,
}

#[derive(Debug, Default)]
struct Thread {
    events: Vec<RawEvent>,
    /// When the thread was last written, as a count of appends across the log.
    written: u64,
    /// Its row of the threads view, folded as events are appended.
    view: Option<ThreadView>,
}

impl InMemoryLog {
    pub fn new() -> Self {
        Self::default()
    }

    fn append_now(&self, scope: &Scope, events: &[RawEvent]) -> Result<(), LogError> {
        let Some(first) = events.first() else {
            return Ok(());
        };
        let mut threads = self
            .threads
            .lock()
            .map_err(|_| LogError::Storage("the in-memory log's lock is poisoned".into()))?;
        let clock = threads
            .values()
            .map(|thread| thread.written)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let thread = threads.entry((*scope, first.thread)).or_default();
        check_append(thread.events.last().map(|event| event.seq), events)?;
        thread.events.extend(events.iter().cloned());
        thread.written = clock;
        thread.view = ThreadView::fold(thread.view.take(), events).map(|mut view| {
            view.position = i64::try_from(clock).unwrap_or(i64::MAX);
            view
        });
        drop(threads);
        let mut index = self
            .index
            .lock()
            .map_err(|_| LogError::Storage("the in-memory index's lock is poisoned".into()))?;
        let entries = index.entry(*scope).or_default();
        for event in events {
            let words = indexed(event);
            if !words.is_empty() {
                let found = Found {
                    thread: event.thread,
                    seq: event.seq,
                };
                entries.push((found, words.into_iter().collect()));
            }
        }
        Ok(())
    }

    fn search_now(&self, scope: &Scope, query: &str, limit: usize) -> Result<Vec<Found>, LogError> {
        let wanted = query_terms(query);
        let index = self
            .index
            .lock()
            .map_err(|_| LogError::Storage("the in-memory index's lock is poisoned".into()))?;
        let Some(entries) = index.get(scope) else {
            return Ok(Vec::new());
        };
        // Most query words matched first; among equals, the newest first.
        let mut found: Vec<(usize, usize, Found)> = entries
            .iter()
            .enumerate()
            .filter_map(|(order, (found, words))| {
                let matched = wanted.iter().filter(|word| words.contains(*word)).count();
                (matched > 0).then_some((matched, order, *found))
            })
            .collect();
        found.sort_by_key(|(matched, order, _)| std::cmp::Reverse((*matched, *order)));
        Ok(found
            .into_iter()
            .take(limit)
            .map(|(_, _, found)| found)
            .collect())
    }

    fn read_now(
        &self,
        scope: &Scope,
        thread: ThreadId,
        after: Option<Seq>,
        limit: usize,
    ) -> Result<Vec<RawEvent>, LogError> {
        let threads = self
            .threads
            .lock()
            .map_err(|_| LogError::Storage("the in-memory log's lock is poisoned".into()))?;
        let Some(stored) = threads.get(&(*scope, thread)) else {
            return Ok(Vec::new());
        };
        Ok(stored
            .events
            .iter()
            .filter(|event| after.is_none_or(|after| event.seq > after))
            .take(limit)
            .cloned()
            .collect())
    }
}

/// Every thread the in-memory log holds, by scope and id.
type Threads = HashMap<(Scope, ThreadId), Thread>;

impl InMemoryLog {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Threads>, LogError> {
        self.threads
            .lock()
            .map_err(|_| LogError::Storage("the in-memory log's lock is poisoned".into()))
    }

    fn views_now(&self, scope: &Scope, query: &ThreadQuery) -> Result<Vec<ThreadView>, LogError> {
        let threads = self.lock()?;
        let mut found: Vec<ThreadView> = threads
            .iter()
            .filter(|((owner, _), _)| owner == scope)
            .filter_map(|(_, thread)| thread.view.clone())
            .filter(|view| query.admits(view))
            .collect();
        found.sort_by_key(|view| std::cmp::Reverse(view.cursor()));
        found.truncate(query.limit);
        Ok(found)
    }

    fn view_now(&self, scope: &Scope, thread: ThreadId) -> Result<Option<ThreadView>, LogError> {
        Ok(self
            .lock()?
            .get(&(*scope, thread))
            .and_then(|thread| thread.view.clone()))
    }

    fn rebuild_now(&self, scope: &Scope, thread: ThreadId) -> Result<Option<ThreadView>, LogError> {
        let mut threads = self.lock()?;
        let Some(stored) = threads.get_mut(&(*scope, thread)) else {
            return Ok(None);
        };
        let position = i64::try_from(stored.written).unwrap_or(i64::MAX);
        stored.view = ThreadView::fold(None, &stored.events).map(|mut view| {
            view.position = position;
            view
        });
        Ok(stored.view.clone())
    }
}

impl EventLog for InMemoryLog {
    fn thread_views(
        &self,
        scope: &Scope,
        query: &ThreadQuery,
    ) -> impl Future<Output = Result<Vec<ThreadView>, LogError>> + Send {
        ready(self.views_now(scope, query))
    }

    fn thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<ThreadView>, LogError>> + Send {
        ready(self.view_now(scope, thread))
    }

    fn rebuild_thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<ThreadView>, LogError>> + Send {
        ready(self.rebuild_now(scope, thread))
    }

    fn append(
        &self,
        scope: &Scope,
        events: &[RawEvent],
    ) -> impl Future<Output = Result<(), LogError>> + Send {
        ready(self.append_now(scope, events))
    }

    fn read(
        &self,
        scope: &Scope,
        thread: ThreadId,
        after: Option<Seq>,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<RawEvent>, LogError>> + Send {
        ready(self.read_now(scope, thread, after, limit))
    }

    fn search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Found>, LogError>> + Send {
        ready(self.search_now(scope, query, limit))
    }

    fn locate(
        &self,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<Located>, LogError>> + Send {
        let found = self
            .threads
            .lock()
            .map_err(|_| LogError::Storage("the in-memory log's lock is poisoned".into()))
            .map(|threads| {
                threads
                    .keys()
                    .filter(|(_, id)| *id == thread)
                    .map(|(scope, _)| *scope)
                    .min()
                    .map(|scope| Located {
                        org: scope.org(),
                        project: scope.project(),
                    })
            });
        ready(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{block_on, conformance};

    #[test]
    fn in_memory_log_passes_the_event_log_conformance_suite() {
        assert_eq!(
            block_on!(conformance::event_log(&InMemoryLog::new())),
            Ok(())
        );
    }

    #[test]
    fn in_memory_log_passes_the_search_conformance_suite() {
        assert_eq!(block_on!(conformance::search(&InMemoryLog::new())), Ok(()));
    }

    #[test]
    fn in_memory_log_passes_the_threads_view_conformance_suite() {
        assert_eq!(
            block_on!(conformance::thread_views(&InMemoryLog::new())),
            Ok(())
        );
    }
}
