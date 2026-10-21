//! Watching a thread (`thread/watch`): its events after a `seq`, read from the log, then as
//! they are recorded, with its live updates and its running subagents'. The watch subscribes
//! before it reads, and sends each thread's events once and in order: an event that arrives
//! ahead of what was sent, and everything a watcher missed by falling behind its bounded
//! backlog, is read from the log instead, so a watcher never sees a gap in what was recorded.
//! Live updates it missed are gone: they are never replayed.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use glave_event::{RawEvent, Seq, ThreadId};
use glave_ports::{BlobStore, BoxFuture, Caller, EventLog, ModelProvider, Scope};
use tokio::sync::broadcast::{self, error::RecvError, error::TryRecvError};

use crate::protocol::{ApiError, Notification, WatchEnd, code};
use crate::server::Server;

/// Events read from the log at a time while a watch catches up.
const PAGE: usize = 256;

/// The subagents' threads whose place a watch keeps; past it the oldest is forgotten, and read
/// from its start should it be seen again.
const MOST_CHILDREN: usize = 256;

/// Reads a page of a thread's events after a `seq`, from its start when there is none.
pub(crate) type Reader = Arc<
    dyn Fn(ThreadId, Option<Seq>) -> BoxFuture<'static, Result<Vec<RawEvent>, ApiError>>
        + Send
        + Sync,
>;

/// What a watch sends next.
#[derive(Debug, Clone)]
pub enum Watched {
    Note(Arc<Notification>),
    /// How the watch ended: the thread was deleted, or its log could not be read. Always the
    /// last item.
    Ended(Result<WatchEnd, ApiError>),
}

/// The first reading of a watched thread from the log.
enum Seam {
    /// Still to end: the `seq` of the thread's latest event published before the watch began,
    /// when the hub knew it.
    Reading(Option<Seq>),
    Read,
}

/// A thread followed as it is recorded: its events from the log after a `seq`, then each as it
/// is recorded, with its live updates, until the thread is deleted or the watch is dropped.
pub struct Watch {
    thread: ThreadId,
    read: Reader,
    notes: broadcast::Receiver<Arc<Notification>>,
    /// The last `seq` sent of each thread whose events were sent: the watched one, and its
    /// subagents'.
    sent: BTreeMap<ThreadId, Seq>,
    /// Threads whose events are read from the log before more are taken as they come.
    behind: BTreeSet<ThreadId>,
    /// Events read from the log and not sent yet, oldest first: at most a page.
    read_ahead: VecDeque<RawEvent>,
    /// After reading from the log, live updates may be older than the events read: they are
    /// dropped until an event comes that is no older than what was sent.
    stale: bool,
    /// Where the first reading of the watched thread is: live updates are stale after it
    /// unless every event it read had been published before the watch began.
    seam: Seam,
    /// How the watch ends, once the event that ends it was sent.
    ending: Option<Result<WatchEnd, ApiError>>,
    ended: bool,
}

impl Watch {
    /// A watch of `thread` taking what `notes` brings, its events after `after` read first
    /// with `read`. `notes` must have been subscribed before anything is read; `published` is
    /// the `seq` of the thread's latest event published before, when known.
    pub(crate) fn new(
        thread: ThreadId,
        read: Reader,
        (notes, published): (broadcast::Receiver<Arc<Notification>>, Option<Seq>),
        after: Option<Seq>,
    ) -> Self {
        Self {
            thread,
            read,
            notes,
            sent: after.map(|after| (thread, after)).into_iter().collect(),
            behind: BTreeSet::from([thread]),
            read_ahead: VecDeque::new(),
            stale: false,
            seam: Seam::Reading(published),
            ending: None,
            ended: false,
        }
    }

    /// The thread watched.
    pub fn thread(&self) -> ThreadId {
        self.thread
    }

    /// The next item, or `None` after the ending.
    pub async fn next(&mut self) -> Option<Watched> {
        self.item(true).await
    }

    /// The next item, waiting for more to be recorded when `wait`; else `None` once nothing
    /// more has come. `None` after the ending.
    async fn item(&mut self, wait: bool) -> Option<Watched> {
        if self.ended {
            return None;
        }
        let item = self.step(wait).await;
        self.ended = matches!(item, Some(Watched::Ended(_)));
        item
    }

    /// Follows the watch to its end, passing its notifications over: how it ended.
    pub async fn finish(&mut self) -> Result<WatchEnd, ApiError> {
        while let Some(item) = self.next().await {
            if let Watched::Ended(ended) = item {
                return ended;
            }
        }
        Err(ApiError::new(code::INTERNAL, "the watch already ended"))
    }

    /// The next item: from the log while the watch catches up, else what comes, waiting for it
    /// when `wait`. `None` only when not waiting and nothing more has come.
    async fn step(&mut self, wait: bool) -> Option<Watched> {
        loop {
            if let Some(ending) = self.ending.take() {
                return Some(Watched::Ended(ending));
            }
            if let Some(event) = self.read_ahead.pop_front() {
                let thread = event.thread;
                let note = Arc::new(Notification::Event { thread, event });
                return Some(self.send(note));
            }
            if let Some(thread) = self.next_behind() {
                if let Err(failed) = self.catch_up(thread).await {
                    return Some(Watched::Ended(Err(failed)));
                }
                continue;
            }
            let received = if wait {
                self.notes.recv().await
            } else {
                match self.notes.try_recv() {
                    Ok(note) => Ok(note),
                    Err(TryRecvError::Empty) => return None,
                    Err(TryRecvError::Lagged(missed)) => Err(RecvError::Lagged(missed)),
                    Err(TryRecvError::Closed) => Err(RecvError::Closed),
                }
            };
            match received {
                Ok(note) => {
                    if let Some(item) = self.take(note) {
                        return Some(item);
                    }
                }
                Err(RecvError::Lagged(_)) => self.fell_behind(),
                Err(RecvError::Closed) => {
                    return Some(Watched::Ended(Err(ApiError::new(
                        code::INTERNAL,
                        "the server stopped",
                    ))));
                }
            }
        }
    }

    /// The thread to read from the log next: the watched one first.
    fn next_behind(&self) -> Option<ThreadId> {
        if self.behind.contains(&self.thread) {
            Some(self.thread)
        } else {
            self.behind.first().copied()
        }
    }

    /// Reads the next page of `thread`'s events after what was sent; once a page is short, the
    /// thread has caught up.
    async fn catch_up(&mut self, thread: ThreadId) -> Result<(), ApiError> {
        let after = self.sent.get(&thread).copied();
        let page = (self.read)(thread, after).await?;
        if page.len() < PAGE {
            self.behind.remove(&thread);
            if thread == self.thread
                && let Seam::Reading(published) = std::mem::replace(&mut self.seam, Seam::Read)
            {
                // Unless the latest event read was published before the watch began, its
                // copy is still to come, and live updates before it are older than it.
                let read = page.last().map(|event| event.seq).or(after);
                self.stale = read.is_some_and(|read| published.is_none_or(|p| p < read));
            }
        }
        self.read_ahead.extend(page);
        Ok(())
    }

    /// Takes one notification as it came: an event in order is sent, a copy of one sent is
    /// dropped, and one ahead of what was sent has its thread read from the log, itself with it.
    fn take(&mut self, note: Arc<Notification>) -> Option<Watched> {
        let Notification::Event { thread, event } = note.as_ref() else {
            let live = matches!(note.as_ref(), Notification::Live { .. });
            return (!(live && self.stale)).then_some(Watched::Note(note));
        };
        let (thread, seq) = (*thread, event.seq);
        let last = self.sent.get(&thread).copied();
        if last.is_none_or(|last| seq >= last) {
            self.stale = false;
        }
        if last.is_some_and(|last| seq <= last) {
            return None;
        }
        if Some(seq) == last.map_or(Some(Seq::FIRST), Seq::next) {
            return Some(self.send(note));
        }
        self.behind.insert(thread);
        None
    }

    /// Sends `note`, an event: its thread's place moves on to it, and the thread's deletion
    /// ends the watch once it is sent.
    fn send(&mut self, note: Arc<Notification>) -> Watched {
        if let Notification::Event { thread, event } = note.as_ref() {
            if *thread == self.thread && event.kind.as_str() == "thread.deleted" {
                self.ending = Some(Ok(WatchEnd::Deleted));
            }
            self.keep_place(*thread, event.seq);
        }
        Watched::Note(note)
    }

    /// Notes that `thread`'s events were sent up to `seq`, forgetting the oldest subagent's
    /// place past the most kept.
    fn keep_place(&mut self, thread: ThreadId, seq: Seq) {
        self.sent.insert(thread, seq);
        if self.sent.len() > MOST_CHILDREN.saturating_add(1) {
            let oldest = self.sent.keys().copied().find(|kept| *kept != self.thread);
            if let Some(oldest) = oldest {
                self.sent.remove(&oldest);
            }
        }
    }

    /// The watcher missed notifications: every thread it sent events of is read from the log
    /// again from where it was, and live updates wait for the next event.
    fn fell_behind(&mut self) {
        self.stale = true;
        self.behind.insert(self.thread);
        self.behind.extend(self.sent.keys().copied());
    }
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Follows `thread` for `caller`, who must work in its project: its events after `after`
    /// (every one when there is none) from the log, then each as it is recorded, with its live
    /// updates and its running subagents', until it is deleted or the watch is dropped.
    pub async fn watch(
        &self,
        caller: &Caller,
        thread: ThreadId,
        after: Option<Seq>,
    ) -> Result<Watch, ApiError> {
        let scope = self.authorize(caller, thread).await?;
        self.ensure_not_deleted(thread).await?;
        // Subscribed before anything is read, so nothing recorded meanwhile falls between.
        let notes = self.inner.hub.watch(thread);
        Ok(Watch::new(thread, self.reader(scope), notes, after))
    }

    /// What reads a page of a thread's events in `scope`, as a watch catches up.
    fn reader(&self, scope: Scope) -> Reader {
        let server = self.clone();
        Arc::new(move |thread, after| {
            let server = server.clone();
            Box::pin(async move {
                let log = server.inner.harness.log();
                Ok(log.read(&scope, thread, after, PAGE).await?)
            })
        })
    }
}

#[cfg(test)]
pub(crate) mod tests;
