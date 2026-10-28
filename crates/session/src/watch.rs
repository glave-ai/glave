//! Watching a thread (`thread/watch`): its events after a `seq`, read from the log, then as
//! they are recorded, with its live updates and its running subagents'. The watch subscribes
//! before it reads, and sends each thread's events once and in order: an event that arrives
//! ahead of what was sent, and everything a watcher missed by falling behind its bounded
//! backlog, is read from the log instead, so a watcher never sees a gap in what was recorded.
//! Live updates it missed are gone: they are never replayed.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use glave_event::{RawEvent, Seq, ThreadId};
use glave_ports::{BlobStore, BoxFuture, Caller, EventLog, ModelProvider, Scope};
use tokio::sync::broadcast::{self, error::RecvError, error::TryRecvError};

use crate::access::Access;
use crate::protocol::{ApiError, Notification, WatchEnd, code};
use crate::server::{Server, no_thread};
use crate::words::Sentence;

/// Events read from the log at a time while a watch catches up.
const PAGE: usize = 256;

/// The subagents' threads whose place a watch keeps; past it the one sent to longest ago is
/// forgotten, and read from its start should it be seen again.
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

/// Why a thread is read from the log, which says whether copies of what is read, and live
/// updates older than them, may still come once it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Why {
    /// The watched thread's first reading: the `seq` of its latest event the hub had published
    /// before the watch began, when it knew one. Copies come of what was published after; with
    /// none known (nothing published since the server started, or the thread long forgotten),
    /// none is taken to come.
    Begun(Option<Seq>),
    /// An event came ahead of what was sent, at this `seq`: copies come of what is read past it.
    Ahead(Seq),
    /// The watcher fell behind: copies of anything read may still come.
    Behind,
}

impl Why {
    /// The `seq` live updates of the thread wait for, after a reading up to `read`: none when
    /// no copy of what was read is still to come.
    fn stale_until(self, read: Seq) -> Option<Seq> {
        let waits = match self {
            Self::Begun(published) => published.is_some_and(|published| published < read),
            Self::Ahead(came) => came < read,
            Self::Behind => true,
        };
        waits.then_some(read)
    }

    /// Both reasons for one reading: falling behind outweighs the others.
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Behind, _) | (_, Self::Behind) => Self::Behind,
            (Self::Ahead(one), Self::Ahead(other)) => Self::Ahead(one.min(other)),
            (Self::Begun(published), _) | (_, Self::Begun(published)) => Self::Begun(published),
        }
    }
}

/// Where a thread's events were sent up to, and when it was last sent one.
#[derive(Debug, Clone, Copy)]
struct Place {
    seq: Seq,
    /// How many events the watch had sent then.
    at: u64,
}

/// A thread followed as it is recorded: its events from the log after a `seq`, then each as it
/// is recorded, with its live updates, until the thread is deleted or the watch is dropped.
pub struct Watch {
    thread: ThreadId,
    read: Reader,
    notes: broadcast::Receiver<Arc<Notification>>,
    /// Where each thread's events were sent up to: the watched one's, and its subagents'.
    sent: BTreeMap<ThreadId, Place>,
    /// How many events were sent.
    sends: u64,
    /// Threads whose events are read from the log before more are taken as they come, and why.
    behind: BTreeMap<ThreadId, Why>,
    /// Events read from the log and not sent yet, oldest first: at most a page.
    read_ahead: VecDeque<RawEvent>,
    /// Threads whose live updates may be older than the events read of them from the log:
    /// each thread's are dropped until its event at or past the `seq` here comes.
    stale: BTreeMap<ThreadId, Seq>,
    /// How the watch ends, once the event that ends it was sent.
    ending: Option<Result<WatchEnd, ApiError>>,
    ended: bool,
    /// The caller's sign-in, when something here can end it: the watch ends with it.
    access: Option<Access>,
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
        let mut watch = Self {
            thread,
            read,
            notes,
            sent: BTreeMap::new(),
            sends: 0,
            behind: BTreeMap::from([(thread, Why::Begun(published))]),
            read_ahead: VecDeque::new(),
            stale: BTreeMap::new(),
            ending: None,
            ended: false,
            access: None,
        };
        if let Some(after) = after {
            watch.keep_place(thread, after);
        }
        watch
    }

    /// A watch of `thread` from its event `last` on that takes only what `notes` brings, as a
    /// turn's stream follows what the turn records: nothing is read from the log unless the
    /// watch falls behind, or an event comes ahead of what was sent. Live updates wait for an
    /// event past `last` unless the hub had `published` it before.
    pub(crate) fn from(
        thread: ThreadId,
        read: Reader,
        (notes, published): (broadcast::Receiver<Arc<Notification>>, Option<Seq>),
        last: Seq,
    ) -> Self {
        let mut watch = Self::new(thread, read, (notes, published), Some(last));
        watch.behind.clear();
        if let Some(until) = Why::Begun(published).stale_until(last) {
            watch.stale.insert(thread, until);
        }
        watch
    }

    /// The watch, ending as soon as `access` does.
    pub(crate) fn until(mut self, access: Option<Access>) -> Self {
        self.access = access;
        self
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

    /// What is still to be sent now, without waiting for more to be recorded: what a turn's
    /// stream sends once the turn has ended. `None` once there is nothing more.
    pub(crate) async fn rest(&mut self) -> Option<Watched> {
        self.item(false).await
    }

    /// Follows the watch to its end, passing its notifications over: how it ended.
    pub async fn finish(&mut self) -> Result<WatchEnd, ApiError> {
        while let Some(item) = self.next().await {
            if let Watched::Ended(ended) = item {
                return ended;
            }
        }
        Err(ApiError::new(code::INTERNAL, Sentence::WatchEnded))
    }

    /// The next item: from the log while the watch catches up, else what comes, waiting for it
    /// when `wait`. `None` only when not waiting and nothing more has come.
    async fn step(&mut self, wait: bool) -> Option<Watched> {
        loop {
            if let Some(ending) = self.ending.take() {
                return Some(Watched::Ended(ending));
            }
            if let Some(refused) = self.access.as_mut().and_then(Access::ended) {
                return Some(Watched::Ended(Err(refused)));
            }
            if let Some(event) = self.read_ahead.pop_front() {
                let thread = event.thread;
                let note = Arc::new(Notification::Event { thread, event });
                return Some(self.send(note));
            }
            if let Some((thread, why)) = self.next_behind() {
                if let Err(failed) = self.catch_up(thread, why).await {
                    return Some(Watched::Ended(Err(failed)));
                }
                continue;
            }
            let received = if wait {
                tokio::select! {
                    received = self.notes.recv() => received,
                    // Looked at once the loop comes round.
                    () = Access::changed(&mut self.access) => continue,
                }
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
                        Sentence::ServerStopped,
                    ))));
                }
            }
        }
    }

    /// The thread to read from the log next, and why: the watched one first.
    fn next_behind(&self) -> Option<(ThreadId, Why)> {
        match self.behind.get(&self.thread) {
            Some(why) => Some((self.thread, *why)),
            None => self
                .behind
                .first_key_value()
                .map(|(thread, why)| (*thread, *why)),
        }
    }

    /// Reads the next page of `thread`'s events after what was sent; once a page is short, the
    /// thread has caught up, and its live updates wait for the copy of the last event read
    /// when one is still to come.
    async fn catch_up(&mut self, thread: ThreadId, why: Why) -> Result<(), ApiError> {
        let after = self.sent.get(&thread).map(|place| place.seq);
        let page = (self.read)(thread, after).await?;
        if page.len() < PAGE {
            self.behind.remove(&thread);
            let read = page.last().map(|event| event.seq).or(after);
            if let Some(until) = read.and_then(|read| why.stale_until(read)) {
                let waits = self.stale.entry(thread).or_insert(until);
                *waits = (*waits).max(until);
            }
        }
        self.read_ahead.extend(page);
        Ok(())
    }

    /// Takes one notification as it came: an event in order is sent, a copy of one sent is
    /// dropped, and one ahead of what was sent has its thread read from the log, itself with
    /// it. A thread's live updates wait while they may be older than what was read of it.
    fn take(&mut self, note: Arc<Notification>) -> Option<Watched> {
        let (thread, seq) = match note.as_ref() {
            Notification::Event { thread, event } => (*thread, event.seq),
            Notification::Live { thread, .. } if self.stale.contains_key(thread) => return None,
            Notification::Live { .. }
            | Notification::TaskFinished { .. }
            | Notification::Changed { .. }
            | Notification::SettingsChanged { .. } => {
                return Some(Watched::Note(note));
            }
        };
        if self.stale.get(&thread).is_some_and(|until| seq >= *until) {
            self.stale.remove(&thread);
        }
        let last = self.sent.get(&thread).map(|place| place.seq);
        if last.is_some_and(|last| seq <= last) {
            return None;
        }
        if Some(seq) == last.map_or(Some(Seq::FIRST), Seq::next) {
            return Some(self.send(note));
        }
        self.read_from_log(thread, Why::Ahead(seq));
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

    /// Notes that `thread`'s events were sent up to `seq`. Past the most kept, the subagent
    /// sent to longest ago is forgotten, but never one still to be read from the log.
    fn keep_place(&mut self, thread: ThreadId, seq: Seq) {
        self.sends = self.sends.saturating_add(1);
        let at = self.sends;
        self.sent.insert(thread, Place { seq, at });
        if self.sent.len() <= MOST_CHILDREN.saturating_add(1) {
            return;
        }
        let oldest = self
            .sent
            .iter()
            .filter(|(kept, _)| {
                **kept != self.thread && **kept != thread && !self.behind.contains_key(*kept)
            })
            .min_by_key(|(_, place)| place.at)
            .map(|(kept, _)| *kept);
        if let Some(oldest) = oldest {
            self.sent.remove(&oldest);
            self.stale.remove(&oldest);
        }
    }

    /// Reads `thread`'s events from the log before more are taken as they come.
    fn read_from_log(&mut self, thread: ThreadId, why: Why) {
        let why = match self.behind.get(&thread) {
            Some(already) => already.and(why),
            None => why,
        };
        self.behind.insert(thread, why);
    }

    /// The watcher missed notifications: every thread it sent events of is read from the log
    /// again from where it was, and each one's live updates wait for its next event.
    fn fell_behind(&mut self) {
        let threads: Vec<ThreadId> = self.sent.keys().copied().collect();
        self.read_from_log(self.thread, Why::Behind);
        for thread in threads {
            self.read_from_log(thread, Why::Behind);
        }
    }
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Follows `thread` for `caller`, who must own its project: its events after `after`
    /// (every one when there is none) from the log, then each as it is recorded, with its live
    /// updates and its running subagents', until it is deleted, the caller's sign-in ends, or
    /// the watch is dropped.
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
        let access = self.access(caller)?;
        Ok(Watch::new(thread, self.reader(scope), notes, after).until(access))
    }

    /// A watch of `thread` from its latest event on, for a turn or a goal `caller` is about to
    /// start there: what is recorded from now, and the live updates, until their sign-in ends.
    pub(crate) async fn watch_from_now(
        &self,
        caller: &Caller,
        scope: Scope,
        thread: ThreadId,
    ) -> Result<Watch, ApiError> {
        // Subscribed before the latest event is read, so nothing recorded meanwhile falls
        // between.
        let notes = self.inner.hub.watch(thread);
        let access = self.access(caller)?;
        let view = self.inner.harness.log().thread_view(&scope, thread).await?;
        let last = view
            .map(|view| view.last_seq)
            .ok_or_else(|| no_thread(thread))?;
        Ok(Watch::from(thread, self.reader(scope), notes, last).until(access))
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
