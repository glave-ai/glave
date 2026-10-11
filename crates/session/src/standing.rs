//! Whether a thread is shown, and how lists show it: `thread/archive`, `thread/unarchive`,
//! `thread/delete` and `thread/rename`, and `thread/list` and the summaries it returns, read
//! from the threads view a page at a time. A change is the thread's next event, appended by the
//! server that holds the thread, so it never conflicts with a thread open in that server.

use glave_event::{Seq, ThreadId};
use glave_harness::{Caller, Change, Standing, standing_of};
use glave_ports::{
    BlobStore, EventLog, ModelProvider, Scope, SpaceId, ThreadCursor, ThreadQuery, ThreadView,
    title_of,
};
use tokio::sync::watch;

use crate::held::{Slot, Use, finished};
use crate::protocol::{ApiError, Reply, Request, Show, ThreadSummary, code};
use crate::server::{MAX_PAGE, Server, api, no_thread};

fn failed(error: &impl std::fmt::Display) -> ApiError {
    ApiError::new(code::INTERNAL, error.to_string())
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `thread/start`, `thread/list`, `thread/get` and the changes to a thread.
    pub(crate) async fn thread_request(
        &self,
        caller: &Caller,
        request: Request,
    ) -> Result<Reply, ApiError> {
        match request {
            Request::ThreadStart {
                space,
                worktree,
                background,
            } => {
                let space = self.space_for(caller, space).await?.id;
                let thread = self.new_thread(caller, space, worktree, background).await?;
                let scope = Scope {
                    org: self.inner.home.org,
                    space,
                };
                Ok(Reply::Thread {
                    thread: self.summary(scope, thread).await?,
                })
            }
            Request::ThreadList {
                space,
                limit,
                show,
                cursor,
            } => {
                let space = self.space_for(caller, space).await?.id;
                self.list(space, limit, show.unwrap_or_default(), cursor)
                    .await
            }
            Request::ThreadGet { thread } => {
                self.load(thread).await?;
                let scope = self.scope_of(thread).await?;
                Ok(Reply::Thread {
                    thread: self.summary(scope, thread).await?,
                })
            }
            Request::ThreadRename { thread, title } => self.rename(caller, thread, title).await,
            Request::ThreadArchive { thread } => self.change(caller, thread, Change::Archive).await,
            Request::ThreadUnarchive { thread } => {
                self.change(caller, thread, Change::Unarchive).await
            }
            Request::ThreadDelete { thread } => self.change(caller, thread, Change::Delete).await,
            _ => Err(ApiError::new(code::INTERNAL, "not a thread request")),
        }
    }

    /// Archives, unarchives or deletes `thread` for `caller`.
    pub(crate) async fn change(
        &self,
        caller: &Caller,
        thread: ThreadId,
        change: Change,
    ) -> Result<Reply, ApiError> {
        // A task in a thread being put away stops first.
        if change != Change::Unarchive {
            self.stop_task_if_any(caller, thread).await;
        }
        let (finish, mut done) = watch::channel(None);
        let caller = caller.clone();
        self.occupy_for(
            thread,
            finish,
            Use::Manage,
            move |inner, mut held, _| async move {
                let changed = inner
                    .harness
                    .change_standing(&mut held, &caller, change)
                    .await;
                (held, changed)
            },
        )
        .await?;
        let ended = finished(&mut done)
            .await
            .ok_or_else(|| ApiError::new(code::INTERNAL, "the change stopped"))?;
        ended.as_ref().as_ref().map_err(api)?;
        Ok(Reply::Done {})
    }

    /// Gives `thread` the title `title` for `caller`, and answers with its summary.
    pub(crate) async fn rename(
        &self,
        caller: &Caller,
        thread: ThreadId,
        title: String,
    ) -> Result<Reply, ApiError> {
        let (finish, mut done) = watch::channel(None);
        let by = caller.clone();
        self.occupy_for(
            thread,
            finish,
            Use::Manage,
            move |inner, mut held, _| async move {
                let renamed = inner.harness.rename(&mut held, &by, &title).await;
                (held, renamed)
            },
        )
        .await?;
        let ended = finished(&mut done)
            .await
            .ok_or_else(|| ApiError::new(code::INTERNAL, "the rename stopped"))?;
        ended.as_ref().as_ref().map_err(api)?;
        let scope = self.scope_of(thread).await?;
        Ok(Reply::Thread {
            thread: self.summary(scope, thread).await?,
        })
    }

    /// A page of the threads of `show` in `space`, most recently updated first, from the
    /// threads view: its cost is the page's, however many threads the space has.
    pub(crate) async fn list(
        &self,
        space: SpaceId,
        limit: Option<usize>,
        show: Show,
        cursor: Option<String>,
    ) -> Result<Reply, ApiError> {
        let scope = Scope {
            org: self.inner.home.org,
            space,
        };
        let limit = limit.unwrap_or(20).clamp(1, MAX_PAGE);
        let after = cursor
            .map(|cursor| cursor.parse::<ThreadCursor>())
            .transpose()
            .map_err(|error| ApiError::new(code::INVALID_PARAMS, error))?;
        let query = ThreadQuery {
            standing: match show {
                Show::Active => Standing::Active,
                Show::Archived => Standing::Archived,
                Show::Deleted => Standing::Deleted,
            },
            // One more than asked shows whether another page follows.
            limit: limit.saturating_add(1),
            after,
        };
        let mut views = self
            .inner
            .harness
            .log()
            .thread_views(&scope, &query)
            .await
            .map_err(|e| failed(&e))?;
        let next = (views.len() > limit)
            .then(|| {
                views.truncate(limit);
                views.last().map(|view| view.cursor().to_string())
            })
            .flatten();
        let mut threads = Vec::with_capacity(views.len());
        for view in views {
            if crate::ledger::is_ledger(view.thread) {
                continue;
            }
            self.place(view.thread, scope);
            threads.push(self.summarize(scope, view).await?);
        }
        Ok(Reply::Threads { threads, next })
    }

    /// The summary of `thread`, from its view: rebuilt from the log if it has none yet.
    pub(crate) async fn summary(
        &self,
        scope: Scope,
        thread: ThreadId,
    ) -> Result<ThreadSummary, ApiError> {
        let log = self.inner.harness.log();
        let view = match log
            .thread_view(&scope, thread)
            .await
            .map_err(|e| failed(&e))?
        {
            Some(view) => view,
            None => log
                .rebuild_thread_view(&scope, thread)
                .await
                .map_err(|e| failed(&e))?
                .ok_or_else(|| no_thread(thread))?,
        };
        self.summarize(scope, view).await
    }

    /// A view as the API shows it: what the thread is doing now, and its title, read from the
    /// one event that gives it.
    async fn summarize(&self, scope: Scope, view: ThreadView) -> Result<ThreadSummary, ApiError> {
        let title = match view.title_seq {
            Some(seq) => {
                let before = Seq::new(seq.get().saturating_sub(1)).ok();
                let read = self
                    .inner
                    .harness
                    .log()
                    .read(&scope, view.thread, before, 1)
                    .await
                    .map_err(|e| failed(&e))?;
                read.first().and_then(title_of)
            }
            None => None,
        };
        Ok(ThreadSummary {
            id: view.thread,
            space: scope.space,
            status: self.status(view.thread),
            standing: view.standing,
            title,
            for_: view.for_,
            origin: view.origin,
            parent: view.parent,
            created_at: view.created_at,
            updated_at: view.updated_at,
            turns: view.turns,
            outcome: view.outcome,
            tokens: view.tokens,
            cost: if view.unpriced { None } else { view.cost },
        })
    }

    /// Whether `thread` is shown, from memory when the server holds it, else from the log.
    pub(crate) async fn standing(&self, thread: ThreadId) -> Result<Standing, ApiError> {
        match self.lock().get(&thread) {
            Some(Slot::Idle(held)) => return Ok(held.standing()),
            // Nothing runs in an archived or deleted thread.
            Some(Slot::Busy { .. }) => return Ok(Standing::Active),
            None => {}
        }
        let scope = self.scope_of(thread).await?;
        standing_of(self.inner.harness.log(), &scope, thread)
            .await
            .map_err(|error| api(&error))
    }

    /// Refuses with "no such thread" when `thread` is deleted.
    pub(crate) async fn ensure_not_deleted(&self, thread: ThreadId) -> Result<(), ApiError> {
        if self.standing(thread).await? == Standing::Deleted {
            return Err(no_thread(thread));
        }
        Ok(())
    }
}
