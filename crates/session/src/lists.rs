//! Watching the threads list (`thread/list/watch`): each thread's summary, as `thread/list`
//! shows it to the caller, whenever it changes, in one project or in every project the caller
//! owns. The watch subscribes before it reads. With `after`, the threads the threads view
//! says were updated since come first, oldest first; a watcher that falls behind its backlog is
//! caught up from the view the same way, from before the first change it may have missed. An
//! event's time is taken as it is made, a moment before it is recorded, so a catch-up reads from
//! a minute earlier: a thread sent twice is harmless, one missed is not. Changes that come
//! together are summarised once each, and a subagent's change of status is its parent's too,
//! whose summary says when a child waits. A project is read only once the caller is found to
//! own it, checked as `thread/list` checks it, and the answer kept, since a project's owner never
//! changes.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use glave_event::{At, ThreadId};
use glave_ports::{
    BlobStore, Caller, EventLog, ModelProvider, ProjectId, Scope, Standing, ThreadQuery,
};
use jiff::SignedDuration;
use tokio::sync::broadcast::{self, error::RecvError, error::TryRecvError};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::access::Access;
use crate::hub::{Change, Changed};
use crate::protocol::{ApiError, Notification, Setting, Status, ThreadSummary, WatchEnd, code};
use crate::server::Server;
use crate::watch::Watched;
use crate::words::Sentence;

/// Views read from the store at a time while a watch catches up.
const PAGE: usize = 256;

/// How long before it is recorded an event's time may have been taken: a catch-up reads from
/// this much before where it starts.
const RECORDING: SignedDuration = SignedDuration::from_secs(60);

/// The most threads one catch-up sends: those updated most recently. A client away longer reads
/// the list again.
const MOST_CAUGHT_UP: usize = 5_000;

/// The projects whose answer a watch remembers; past it, it forgets them and asks again.
const MOST_PROJECTS: usize = 1_024;

/// The threads whose status a watch remembers, to tell their parents only when it changes;
/// past it, it forgets them all.
const MOST_STATUSES: usize = 4_096;

/// How far up a subagent's change travels: its parent, and theirs.
const MOST_GENERATIONS: u8 = 8;

/// A watch of the caller's threads list: each thread's summary as it changes, until it is
/// dropped. A task of its own reads the summaries, one ahead of the client; dropping the watch
/// stops it.
pub struct ListWatch {
    items: mpsc::Receiver<Watched>,
    following: JoinHandle<()>,
}

impl ListWatch {
    /// The next item, or `None` after the ending.
    pub async fn next(&mut self) -> Option<Watched> {
        self.items.recv().await
    }

    /// Follows the watch to its end, passing its notifications over: how it ended, which is
    /// only ever why it could not go on, since a client ends a list's watch by dropping it.
    pub async fn finish(&mut self) -> Result<WatchEnd, ApiError> {
        while let Some(item) = self.next().await {
            if let Watched::Ended(ended) = item {
                return ended;
            }
        }
        Err(ApiError::new(code::INTERNAL, Sentence::WatchEnded))
    }
}

impl Drop for ListWatch {
    fn drop(&mut self) {
        self.following.abort();
    }
}

/// What a list's watch reads and sends, in its caller's name.
struct Follower<P, L, B> {
    server: Server<P, L, B>,
    caller: Caller,
    /// The one project followed, when the watch names one.
    only: Option<ProjectId>,
    /// The projects heard of, with the caller's scope in each they own.
    scopes: BTreeMap<ProjectId, Option<Scope>>,
    /// The status each thread was last sent with.
    statuses: BTreeMap<ThreadId, Status>,
    /// The threads to summarise next, in order, each with how many generations above it are
    /// still to be told.
    queue: VecDeque<(ProjectId, ThreadId, u8)>,
    queued: BTreeSet<ThreadId>,
    /// The caller's own settings that changed, to tell them of next.
    settings: BTreeSet<Setting>,
    /// The caller's sign-in, when something here can end it: the watch ends with it.
    access: Option<Access>,
    /// The latest update the watch was sent, or where it began.
    seen: At,
    /// When the watch had last taken every change told before then: anything it missed was
    /// told after it.
    tail: At,
    out: mpsc::Sender<Watched>,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Follows the threads `caller` sees in `project`, or in every project they own: the
    /// summary of each thread updated since `after` from the threads view first, then each
    /// thread's as it changes, until the caller's sign-in ends or the watch is dropped.
    pub async fn watch_list(
        &self,
        caller: &Caller,
        project: Option<ProjectId>,
        after: Option<At>,
    ) -> Result<ListWatch, ApiError> {
        let now = At::from(self.inner.harness.clock().now());
        let mut scopes = BTreeMap::new();
        match project {
            Some(project) => {
                let (_, scope) = self.authorized(caller, project).await?;
                scopes.insert(project, Some(scope));
            }
            // Whom the caller is, once, before anything is followed.
            None => {
                self.projects_of(caller).await?;
            }
        }
        // Subscribed before anything is read, so nothing recorded meanwhile falls between.
        let changes = self.inner.hub.changes();
        let access = self.access(caller).await?;
        let (out, items) = mpsc::channel(1);
        let follower = Follower {
            server: self.clone(),
            caller: caller.clone(),
            only: project,
            scopes,
            statuses: BTreeMap::new(),
            queue: VecDeque::new(),
            queued: BTreeSet::new(),
            settings: BTreeSet::new(),
            access,
            seen: after.unwrap_or(now),
            tail: now,
            out,
        };
        let following = tokio::spawn(follower.run(changes, after));
        Ok(ListWatch { items, following })
    }
}

impl<P, L, B> Follower<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Follows until the client goes, or says why it cannot go on.
    async fn run(mut self, mut changes: broadcast::Receiver<Change>, after: Option<At>) {
        if let Err(failed) = self.follow(&mut changes, after).await {
            let _ = self.out.send(Watched::Ended(Err(failed))).await;
        }
    }

    /// Sends what changed since `after`, then each change as it comes; returns once the client
    /// has gone, or with why the watch cannot go on.
    async fn follow(
        &mut self,
        changes: &mut broadcast::Receiver<Change>,
        after: Option<At>,
    ) -> Result<(), ApiError> {
        if let Some(after) = after {
            self.catch_up(after).await?;
        }
        loop {
            if let Some(refused) = self.access.as_mut().and_then(Access::ended) {
                return Err(refused);
            }
            // The caller's settings first: a few words each, and nothing to read.
            while let Some(setting) = self.settings.pop_first() {
                let note = Arc::new(Notification::SettingsChanged { setting });
                if self.out.send(Watched::Note(note)).await.is_err() {
                    return Ok(());
                }
            }
            while let Some((project, thread, above)) = self.queue.pop_front() {
                self.queued.remove(&thread);
                let Some(summary) = self.summary(project, thread).await? else {
                    continue;
                };
                self.seen = self.seen.max(summary.updated_at);
                // A parent's summary says when a child of its waits.
                if self.status_changed(thread, summary.status)
                    && let (Some(parent), Some(above)) = (summary.parent, above.checked_sub(1))
                {
                    self.push(project, parent, above);
                }
                let summary = Box::new(summary);
                let note = Arc::new(Notification::Changed { thread, summary });
                if self.out.send(Watched::Note(note)).await.is_err() {
                    return Ok(());
                }
            }
            let received = tokio::select! {
                received = changes.recv() => received,
                // Looked at once the loop comes round.
                () = Access::changed(&mut self.access) => continue,
            };
            let behind = match received {
                Ok(change) => {
                    self.take(change);
                    self.take_waiting(changes)
                }
                Err(RecvError::Lagged(_)) => true,
                // Not while the watch holds the server, whose hub tells it; kept so a hub that
                // went ends the watch rather than spinning.
                Err(RecvError::Closed) => {
                    return Err(ApiError::new(code::INTERNAL, Sentence::ServerStopped));
                }
            };
            if behind {
                // What it missed was told after it last took everything, and recorded since
                // its time, or since the latest update it sent, whichever is earlier; of the
                // caller's settings, any may have changed.
                self.settings.extend(Setting::ALL);
                self.catch_up(self.seen.min(self.tail)).await?;
            }
        }
    }

    /// Takes every change already waiting, so that each thread is summarised once: whether the
    /// watch fell behind meanwhile.
    fn take_waiting(&mut self, changes: &mut broadcast::Receiver<Change>) -> bool {
        loop {
            match changes.try_recv() {
                Ok(change) => self.take(change),
                Err(TryRecvError::Lagged(_)) => return true,
                Err(TryRecvError::Empty) => {
                    self.tail = self.now();
                    return false;
                }
                Err(TryRecvError::Closed) => return false,
            }
        }
    }

    fn now(&self) -> At {
        At::from(self.server.inner.harness.clock().now())
    }

    /// Notes `thread`'s status as sent: whether it changed since it was last sent.
    fn status_changed(&mut self, thread: ThreadId, status: Status) -> bool {
        if self.statuses.len() >= MOST_STATUSES {
            self.statuses.clear();
        }
        self.statuses.insert(thread, status) != Some(status)
    }

    fn take(&mut self, change: Change) {
        match change {
            Change::Thread(Changed { project, thread }) => {
                if self.only.is_none_or(|only| only == project) {
                    self.push(project, thread, MOST_GENERATIONS);
                }
            }
            Change::Setting { person, setting } => {
                // Whose it is, the local person named or not.
                let local = self.server.person();
                if person.or(local) == self.caller.person.or(local) {
                    self.settings.insert(setting);
                }
            }
        }
    }

    /// Queues `thread` unless it is queued already: its summary is read when it is sent.
    fn push(&mut self, project: ProjectId, thread: ThreadId, above: u8) {
        if self.queued.insert(thread) {
            self.queue.push_back((project, thread, above));
        }
    }

    /// Queues every thread followed that the threads view says was updated since `after`,
    /// oldest first: the most recent [`MOST_CAUGHT_UP`] of them.
    async fn catch_up(&mut self, after: At) -> Result<(), ApiError> {
        let from = after
            .timestamp()
            .checked_sub(RECORDING)
            .map_or(after, At::from);
        let mut found = BTreeSet::new();
        for scope in self.followed().await? {
            for standing in [Standing::Active, Standing::Archived, Standing::Deleted] {
                self.updated_since(scope, standing, from, &mut found)
                    .await?;
            }
        }
        for (_, project, thread) in found {
            self.push(project, thread, MOST_GENERATIONS);
        }
        Ok(())
    }

    /// Adds to `found` the threads of `standing` in `scope` updated at or after `from`, keeping
    /// only the most recent.
    async fn updated_since(
        &self,
        scope: Scope,
        standing: Standing,
        from: At,
        found: &mut BTreeSet<(At, ProjectId, ThreadId)>,
    ) -> Result<(), ApiError> {
        let log = self.server.inner.harness.log();
        let mut query = ThreadQuery {
            standing,
            limit: PAGE,
            after: None,
            open: false,
            parent: None,
        };
        loop {
            let views = log.thread_views(&scope, &query).await?;
            for view in &views {
                // Newest first: the rest are older still.
                let older = found.len() >= MOST_CAUGHT_UP
                    && found.first().is_some_and(|(at, ..)| view.updated_at <= *at);
                if view.updated_at < from || older {
                    return Ok(());
                }
                found.insert((view.updated_at, scope.project(), view.thread));
                if found.len() > MOST_CAUGHT_UP {
                    found.pop_first();
                }
            }
            match views.last() {
                Some(last) if views.len() == PAGE => query.after = Some(last.cursor()),
                _ => return Ok(()),
            }
        }
    }

    /// The scopes of the projects followed: the one named, or every one the caller owns.
    async fn followed(&mut self) -> Result<Vec<Scope>, ApiError> {
        let projects = if let Some(only) = self.only {
            vec![only]
        } else {
            let owned = self.server.projects_of(&self.caller).await?;
            owned.into_iter().map(|at| at.project).collect()
        };
        let mut scopes = Vec::new();
        for project in projects {
            if let Some(scope) = self.scope_of(project).await? {
                scopes.push(scope);
            }
        }
        Ok(scopes)
    }

    /// The caller's scope in `project`, when they own it: asked the first time, then
    /// remembered, since a project's owner never changes.
    async fn scope_of(&mut self, project: ProjectId) -> Result<Option<Scope>, ApiError> {
        if self.only.is_some_and(|only| only != project) {
            return Ok(None);
        }
        if let Some(known) = self.scopes.get(&project) {
            return Ok(*known);
        }
        let scope = match self.server.authorized(&self.caller, project).await {
            Ok((_, scope)) => Some(scope),
            Err(refused) if refused.code == code::NO_PROJECT => None,
            Err(failed) => return Err(failed),
        };
        if self.scopes.len() >= MOST_PROJECTS {
            self.scopes.clear();
        }
        self.scopes.insert(project, scope);
        Ok(scope)
    }

    /// `thread`'s summary as `thread/list` shows it to the caller, its project's ask with it;
    /// none when it is not one the watch follows, or not a thread.
    async fn summary(
        &mut self,
        project: ProjectId,
        thread: ThreadId,
    ) -> Result<Option<ThreadSummary>, ApiError> {
        let Some(scope) = self.scope_of(project).await? else {
            return Ok(None);
        };
        let summary = match self.server.summary(scope, thread).await {
            Ok(summary) => summary,
            Err(refused) if refused.code == code::NO_THREAD => return Ok(None),
            Err(failed) => return Err(failed),
        };
        let ask = self
            .server
            .trust_on_page(&self.caller, scope.located(), std::iter::once(thread))
            .await;
        Ok(Some(self.server.asking(summary, ask.as_ref(), false)))
    }
}
