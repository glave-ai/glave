//! Watching the threads list (`thread/list/watch`): each thread's summary, as `thread/list`
//! shows it to the caller, whenever it changes, in one project or in every project the caller
//! works in. The watch subscribes before it reads. With `after`, the threads the threads view
//! says were updated since come first, oldest first; a watcher that falls behind its backlog is
//! caught up from the view the same way, from the latest update it was sent. An event's time is
//! taken as it is made, a moment before it is recorded, so a catch-up reads from a minute
//! earlier: a thread sent twice is harmless, one missed is not. Changes that come together are
//! summarised once each, and a subagent's change is its parent's too, whose summary says when a
//! child waits. A project is read only once the caller is found to work in it, checked as
//! `thread/list` checks it, the first time the watch hears of it.

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

use crate::hub::Changed;
use crate::protocol::{ApiError, Notification, ThreadSummary, WatchEnd, code};
use crate::server::Server;
use crate::watch::Watched;

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
        Err(ApiError::new(code::INTERNAL, "the watch already ended"))
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
    only: Option<Scope>,
    /// The projects heard of, with the caller's scope in each they work in.
    scopes: BTreeMap<ProjectId, Option<Scope>>,
    /// The threads to summarise next, in order, each with how many generations above it are
    /// still to be told.
    queue: VecDeque<(ProjectId, ThreadId, u8)>,
    queued: BTreeSet<ThreadId>,
    /// The latest update the watch was sent, or where it began: where a catch-up starts.
    seen: At,
    out: mpsc::Sender<Watched>,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Follows the threads `caller` sees in `project`, or in every project they work in: the
    /// summary of each thread updated since `after` from the threads view first, then each
    /// thread's as it changes, until the watch is dropped.
    pub async fn watch_list(
        &self,
        caller: &Caller,
        project: Option<ProjectId>,
        after: Option<At>,
    ) -> Result<ListWatch, ApiError> {
        let only = match project {
            Some(project) => Some(self.authorized(caller, project).await?.1),
            // Whom the caller is, once, before anything is followed.
            None => self.projects_of(caller).await.map(|_| None)?,
        };
        // Subscribed before anything is read, so nothing recorded meanwhile falls between.
        let changes = self.inner.hub.changes();
        let now = At::from(self.inner.harness.clock().now());
        let (out, items) = mpsc::channel(1);
        let follower = Follower {
            server: self.clone(),
            caller: caller.clone(),
            only,
            scopes: BTreeMap::new(),
            queue: VecDeque::new(),
            queued: BTreeSet::new(),
            seen: after.unwrap_or(now),
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
    async fn run(mut self, mut changes: broadcast::Receiver<Changed>, after: Option<At>) {
        if let Err(failed) = self.follow(&mut changes, after).await {
            let _ = self.out.send(Watched::Ended(Err(failed))).await;
        }
    }

    /// Sends what changed since `after`, then each change as it comes; returns once the client
    /// has gone, or with why the watch cannot go on.
    async fn follow(
        &mut self,
        changes: &mut broadcast::Receiver<Changed>,
        after: Option<At>,
    ) -> Result<(), ApiError> {
        if let Some(after) = after {
            self.catch_up(after).await?;
        }
        loop {
            while let Some((project, thread, above)) = self.queue.pop_front() {
                self.queued.remove(&thread);
                let Some(summary) = self.summary(project, thread).await? else {
                    continue;
                };
                self.seen = self.seen.max(summary.updated_at);
                // A parent's summary says when a child of its waits.
                if let (Some(parent), Some(above)) = (summary.parent, above.checked_sub(1)) {
                    self.push(project, parent, above);
                }
                let summary = Box::new(summary);
                let note = Arc::new(Notification::Changed { thread, summary });
                if self.out.send(Watched::Note(note)).await.is_err() {
                    return Ok(());
                }
            }
            let behind = match changes.recv().await {
                Ok(change) => {
                    self.take(change);
                    self.take_waiting(changes)
                }
                Err(RecvError::Lagged(_)) => true,
                Err(RecvError::Closed) => {
                    return Err(ApiError::new(code::INTERNAL, "the server stopped"));
                }
            };
            if behind {
                self.catch_up(self.seen).await?;
            }
        }
    }

    /// Takes every change already waiting, so that each thread is summarised once: whether the
    /// watch fell behind meanwhile.
    fn take_waiting(&mut self, changes: &mut broadcast::Receiver<Changed>) -> bool {
        loop {
            match changes.try_recv() {
                Ok(change) => self.take(change),
                Err(TryRecvError::Lagged(_)) => return true,
                Err(TryRecvError::Empty | TryRecvError::Closed) => return false,
            }
        }
    }

    fn take(&mut self, change: Changed) {
        let followed = self
            .only
            .is_none_or(|only| only.project() == change.project);
        if followed {
            self.push(change.project, change.thread, MOST_GENERATIONS);
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

    /// The scopes of the projects followed: the one named, or every one the caller works in.
    async fn followed(&mut self) -> Result<Vec<Scope>, ApiError> {
        if let Some(only) = self.only {
            return Ok(vec![only]);
        }
        let mut scopes = Vec::new();
        for project in self.server.projects_of(&self.caller).await? {
            if let Some(scope) = self.scope_of(project).await? {
                scopes.push(scope);
            }
        }
        Ok(scopes)
    }

    /// The caller's scope in `project`, when they work in it: asked the first time, then
    /// remembered for the watch.
    async fn scope_of(&mut self, project: ProjectId) -> Result<Option<Scope>, ApiError> {
        if let Some(only) = self.only {
            return Ok((only.project() == project).then_some(only));
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
            .trust_on_page(&self.caller, project, std::iter::once(thread))
            .await;
        Ok(Some(self.server.asking(summary, ask.as_ref(), false)))
    }
}
