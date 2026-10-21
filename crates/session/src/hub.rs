//! Where a thread's notifications go: one bounded broadcast per watched thread, fed by the log as
//! events are appended and by running turns as they make progress; and one more saying which
//! thread changed, for the watches of lists (`lists.rs`), fed by the same appends and by work on
//! a thread beginning and ending. A follower that falls too far behind misses what was not held
//! for it: a watch reads it again from the log by `seq` (`watch.rs`), a list's from the threads
//! view (`lists.rs`), so nothing is buffered without bound.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use glave_event::{Body, Event, RawEvent, Seq, ThreadId, decode};
use glave_ports::{EventLog, Located, LogError, ProjectId, Scope};
use tokio::sync::broadcast;

use crate::protocol::Notification;

/// Notifications held for a slow watcher before it is told it fell behind.
const BACKLOG: usize = 1_024;

/// Threads whose latest published event the hub keeps; past it, the one published to least
/// recently is forgotten.
const MOST_PUBLISHED: usize = 4_096;

/// The broadcasts of every watched thread, and of which threads changed.
pub(crate) struct Hub {
    channels: Mutex<Channels>,
    /// Each running subagent's parent: its notifications go to the parent's watchers too.
    parents: Mutex<HashMap<ThreadId, ThreadId>>,
    /// Each thread that changed, in its project, for the watches of lists.
    changes: broadcast::Sender<Changed>,
}

impl Default for Hub {
    fn default() -> Self {
        Self {
            channels: Mutex::default(),
            parents: Mutex::default(),
            changes: broadcast::channel(BACKLOG).0,
        }
    }
}

/// A thread whose summary may have changed: an event was recorded in it here, or work on it
/// began or ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Changed {
    pub(crate) project: ProjectId,
    pub(crate) thread: ThreadId,
}

#[derive(Default)]
struct Channels {
    /// The broadcast of each thread someone watches.
    open: HashMap<ThreadId, broadcast::Sender<Arc<Notification>>>,
    /// The `seq` of each thread's latest event published, watched or not, for the threads
    /// published to most recently, and when, by the count of publishes.
    published: BTreeMap<ThreadId, (Seq, u64)>,
    /// The threads of `published` by when they were last published to, least recently first.
    recency: BTreeMap<u64, ThreadId>,
    publishes: u64,
}

impl Hub {
    /// Starts watching `thread`: everything published from now on, and the `seq` of its latest
    /// event published before, when the hub still knows it.
    pub(crate) fn watch(
        &self,
        thread: ThreadId,
    ) -> (broadcast::Receiver<Arc<Notification>>, Option<Seq>) {
        let mut channels = self.channels();
        let notes = channels
            .open
            .entry(thread)
            .or_insert_with(|| broadcast::channel(BACKLOG).0)
            .subscribe();
        let published = channels.published.get(&thread).map(|(seq, _)| *seq);
        (notes, published)
    }

    /// Starts hearing which threads change, from now on.
    pub(crate) fn changes(&self) -> broadcast::Receiver<Changed> {
        self.changes.subscribe()
    }

    /// Tells the watches of lists that `thread`, in `project`, changed. Glave's own streams
    /// beside threads are never listed, so never told.
    pub(crate) fn changed(&self, project: ProjectId, thread: ThreadId) {
        if glave_ports::stream::Reserved::of(thread).is_none() {
            // Only fails when no list is watched, which is fine.
            let _ = self.changes.send(Changed { project, thread });
        }
    }

    fn channels(&self) -> MutexGuard<'_, Channels> {
        self.channels.lock().unwrap_or_else(PoisonError::into_inner)
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
        let mut channels = self.channels();
        if let Notification::Event { thread: of, event } = notification.as_ref()
            && *of == thread
        {
            channels.published(thread, event.seq);
        }
        if let Some(sender) = channels.open.get(&thread) {
            if sender.receiver_count() == 0 {
                channels.open.remove(&thread);
            } else {
                // Only fails when no one is listening, which is fine.
                let _ = sender.send(Arc::clone(notification));
            }
        }
    }

    /// Notes that `child` was started by a call in `parent`, until `child`'s turn ends.
    pub(crate) fn adopt(&self, child: ThreadId, parent: ThreadId) {
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

impl Channels {
    /// Notes that `thread`'s event `seq` was published, keeping the latest when two writers'
    /// are told out of order, and forgetting the thread published to least recently past the
    /// most kept.
    fn published(&mut self, thread: ThreadId, seq: Seq) {
        self.publishes = self.publishes.saturating_add(1);
        let now = self.publishes;
        let latest = match self.published.insert(thread, (seq, now)) {
            Some((before, then)) => {
                self.recency.remove(&then);
                before.max(seq)
            }
            None => seq,
        };
        self.published.insert(thread, (latest, now));
        self.recency.insert(now, thread);
        if self.published.len() > MOST_PUBLISHED
            && let Some((_, forgotten)) = self.recency.pop_first()
        {
            self.published.remove(&forgotten);
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

#[deny(clippy::missing_trait_methods)]
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
        // Once each thread the batch wrote to, after its events.
        let mut told: Vec<ThreadId> = Vec::new();
        for event in events {
            if !told.contains(&event.thread) {
                told.push(event.thread);
                self.hub.changed(scope.project(), event.thread);
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
    ) -> impl Future<Output = Result<Option<Located>, LogError>> + Send {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(n: u16) -> ThreadId {
        let mut random = [3; 10];
        random[8..].copy_from_slice(&n.to_be_bytes());
        ThreadId::new(jiff::Timestamp::UNIX_EPOCH, random).unwrap()
    }

    #[test]
    fn the_thread_published_to_least_recently_is_forgotten_first_and_the_latest_seq_kept() {
        let mut channels = Channels::default();
        let seq = |n| Seq::new(n).unwrap();
        channels.published(thread(0), seq(5));
        // Told out of order: the latest stays.
        channels.published(thread(0), seq(4));
        for n in 1..u16::try_from(MOST_PUBLISHED).unwrap() {
            channels.published(thread(n), seq(1));
        }
        // The oldest thread is still in use, so another goes when the map is full.
        channels.published(thread(0), seq(6));
        channels.published(thread(9_999), seq(1));
        assert_eq!(channels.published.len(), MOST_PUBLISHED);
        let kept = channels.published.get(&thread(0)).map(|kept| kept.0);
        assert_eq!(kept, Some(seq(6)));
        assert!(!channels.published.contains_key(&thread(1)));
        assert!(channels.published.contains_key(&thread(2)));
        assert_eq!(channels.recency.len(), MOST_PUBLISHED);
    }
}
