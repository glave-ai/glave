//! Whether a thread is shown, and how lists show it: `thread/archive`, `thread/unarchive`,
//! `thread/delete` and `thread/rename`, and `thread/list` and the summaries it returns, read
//! from the threads view a page at a time. A change is the thread's next event, appended by the
//! server that holds the thread, so it never conflicts with a thread open in that server.

use glave_event::{Seq, ThreadId};
use glave_harness::{Change, Protections, standing_of};
use glave_ports::{
    BlobStore, Caller, EventLog, ModelProvider, ProjectId, Scope, Standing, ThreadCursor,
    ThreadQuery, ThreadView, title_of,
};
use tokio::sync::watch;

use crate::held::{Slot, Use, finished};
use crate::modes::Choice;
use crate::protocol::{ApiError, Reply, Show, ThreadSummary, code};
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
    /// Answers `thread/start`: a thread for `caller` in `project`, as [`Server::new_thread`]
    /// places it.
    pub(crate) async fn start_thread(
        &self,
        caller: &Caller,
        project: Option<ProjectId>,
        worktree: Option<bool>,
        background: bool,
        choice: &Choice,
    ) -> Result<Reply, ApiError> {
        let project = self.project_for(caller, project).await?.id;
        let thread = self
            .new_thread(caller, project, worktree, background, choice)
            .await?;
        let scope = Scope {
            org: self.inner.home.org,
            project,
        };
        self.opened(caller, scope, thread).await
    }

    /// Answers `thread/get`: also for a thread whose project's folder isn't there anymore,
    /// which is read from the log rather than opened.
    pub(crate) async fn get_thread(
        &self,
        caller: &Caller,
        thread: ThreadId,
    ) -> Result<Reply, ApiError> {
        match self.load(thread).await {
            Err(refused) if crate::gone::is_gone(&refused) => {
                self.ensure_not_deleted(thread).await?;
            }
            loaded => loaded?,
        }
        let scope = self.scope_of(thread).await?;
        self.opened(caller, scope, thread).await
    }

    /// The summary of `thread`, opened or started by `caller`: it raises what its project asks
    /// them to trust, when it asks anything (`trust.rs`).
    async fn opened(
        &self,
        caller: &Caller,
        scope: Scope,
        thread: ThreadId,
    ) -> Result<Reply, ApiError> {
        let summary = self.summary(scope, thread).await?;
        let ask = self.trust_in(caller, scope.project).await;
        Ok(Reply::Thread {
            thread: self.asking(summary, ask.as_ref(), true),
        })
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
        let by = caller.clone();
        let occupied = self
            .occupy_for(
                thread,
                finish,
                Use::Manage,
                move |inner, mut held, _| async move {
                    let changed = inner.harness.change_standing(&mut held, &by, change).await;
                    (held, changed)
                },
            )
            .await;
        match occupied {
            Err(refused) if crate::gone::is_gone(&refused) => {
                return self.change_gone(caller, thread, change).await;
            }
            occupied => occupied?,
        }
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
        let summary = self.summary(scope, thread).await?;
        let ask = self
            .trust_on_page(caller, scope.project, std::iter::once(thread))
            .await;
        Ok(Reply::Thread {
            thread: self.asking(summary, ask.as_ref(), false),
        })
    }

    /// Answers `thread/list`: a page of the threads of `show` in `project`, as `thread/start`
    /// picks it, most recently updated first, from the threads view: its cost is the page's,
    /// however many threads the project has.
    pub(crate) async fn list_threads(
        &self,
        caller: &Caller,
        project: Option<ProjectId>,
        limit: Option<usize>,
        show: Option<Show>,
        cursor: Option<String>,
    ) -> Result<Reply, ApiError> {
        let project = self.project_for(caller, project).await?.id;
        let scope = Scope {
            org: self.inner.home.org,
            project,
        };
        let limit = limit.unwrap_or(20).clamp(1, MAX_PAGE);
        let after = cursor
            .map(|cursor| cursor.parse::<ThreadCursor>())
            .transpose()
            .map_err(|error| ApiError::new(code::INVALID_PARAMS, error))?;
        let query = ThreadQuery {
            standing: match show.unwrap_or_default() {
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
        let protections = self.project_settled(project).await.protections;
        let ask = self
            .trust_on_page(caller, project, views.iter().map(|view| view.thread))
            .await;
        let mut threads = Vec::with_capacity(views.len());
        for view in views {
            if crate::ledger::is_ledger(view.thread) {
                continue;
            }
            self.place(view.thread, scope);
            let summary = self.summarize(scope, view, protections).await?;
            threads.push(self.asking(summary, ask.as_ref(), false));
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
        let protections = self.project_settled(scope.project).await.protections;
        self.summarize(scope, view, protections).await
    }

    /// A view as the API shows it: what the thread is doing now, its title, read from the one
    /// event that gives it, and the mode it runs in, as its project changes it (`protections`)
    /// when it is the person on this computer's.
    async fn summarize(
        &self,
        scope: Scope,
        view: ThreadView,
        protections: Protections,
    ) -> Result<ThreadSummary, ApiError> {
        let theirs = view.for_.as_ref().and_then(|whose| whose.id);
        let protections = if theirs.is_none_or(|person| Some(person) == self.person()) {
            protections
        } else {
            Protections::default()
        };
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
            project: scope.project,
            status: self.status(view.thread),
            standing: view.standing,
            title,
            approval: self.approval_now(&scope, view.thread, protections).await?,
            trust: None,
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
            Some(Slot::Idle { thread: held, .. }) => return Ok(held.standing()),
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
