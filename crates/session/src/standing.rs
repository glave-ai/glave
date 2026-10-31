//! Whether a thread is shown, and how lists show it: `thread/archive`, `thread/unarchive`,
//! `thread/delete` and `thread/rename`, and `thread/list` and the summaries it returns, read
//! from the threads view a page at a time. A change is the thread's next event, appended by the
//! server that holds the thread, so it never conflicts with a thread open in that server. A
//! thread archived or deleted lets go of its worktrees on the host's computer.

use glave_event::{Body, Event, RawEvent, Seq, ThreadId, decode};
use glave_harness::{Change, standing_of};
use glave_ports::settings::Protections;
use glave_ports::{
    BlobStore, BoxFuture, Caller, EventLog, ModelProvider, ProjectId, Scope, Standing, ThreadQuery,
    ThreadView, title_of,
};
use tokio::sync::watch;

use crate::choice::Choice;
use crate::held::{Slot, Use, finished};
use crate::protocol::{ApiError, ContextUse, Reply, Show, ThreadSummary, code};
use crate::server::{MAX_PAGE, Server, no_thread};
use crate::words::Sentence;

/// The worktrees threads work in on the host's computer, which the host keeps.
pub trait Worktrees: Send + Sync {
    /// Lets go of the worktrees of `thread`, in `scope`, just archived or deleted: each one is
    /// removed when that loses nothing, and kept otherwise, which the host says in its log.
    fn release(&self, scope: Scope, thread: ThreadId) -> BoxFuture<'_, ()>;
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
        let (_, scope) = self.project_for(caller, project).await?;
        let thread = self
            .new_thread(caller, scope, worktree, background, choice)
            .await?;
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
        let scope = self.scope_of(thread)?;
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
        let ask = self.trust_in(caller, scope.located()).await;
        Ok(Reply::Thread {
            thread: self.asking(summary, ask.as_ref(), true),
        })
    }

    /// Archives, unarchives or deletes `thread` for `caller`; archived or deleted, it lets go
    /// of its worktrees.
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
                self.change_gone(caller, thread, change).await?;
            }
            occupied => {
                occupied?;
                let ended = finished(&mut done)
                    .await
                    .ok_or_else(|| ApiError::new(code::INTERNAL, Sentence::ChangeStopped))?;
                ended.as_ref().as_ref().map_err(ApiError::from)?;
            }
        }
        // Its place was found to change it, so it is known by now.
        if change != Change::Unarchive
            && let Some(worktrees) = &self.inner.worktrees
            && let Ok(scope) = self.scope_of(thread)
        {
            worktrees.release(scope, thread).await;
        }
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
            .ok_or_else(|| ApiError::new(code::INTERNAL, Sentence::RenameStopped))?;
        ended.as_ref().as_ref().map_err(ApiError::from)?;
        let scope = self.scope_of(thread)?;
        let summary = self.summary(scope, thread).await?;
        let ask = self
            .trust_on_page(caller, scope.located(), std::iter::once(thread))
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
        let (_, scope) = self.project_for(caller, project).await?;
        let project = scope.project();
        let limit = limit.unwrap_or(20).clamp(1, MAX_PAGE);
        let standing = match show.unwrap_or_default() {
            Show::Active => Standing::Active,
            Show::Archived => Standing::Archived,
            Show::Deleted => Standing::Deleted,
        };
        let after = cursor
            .map(|cursor| crate::cursor::read(&cursor, project, standing))
            .transpose()?;
        let query = ThreadQuery {
            standing,
            // One more than asked shows whether another page follows.
            limit: limit.saturating_add(1),
            after,
            open: false,
            parent: None,
        };
        let mut views = self
            .inner
            .harness
            .log()
            .thread_views(&scope, &query)
            .await?;
        let next = (views.len() > limit)
            .then(|| {
                views.truncate(limit);
                let last = views.last()?;
                Some(crate::cursor::write(project, standing, last.cursor()))
            })
            .flatten();
        let protections = self.project_settled(scope.located()).await.protections;
        let ask = self
            .trust_on_page(
                caller,
                scope.located(),
                views.iter().map(|view| view.thread),
            )
            .await;
        let mut threads = Vec::with_capacity(views.len());
        for view in views {
            if glave_ports::stream::Reserved::of(view.thread).is_some() {
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
        let view = match log.thread_view(&scope, thread).await? {
            Some(view) => view,
            None => log
                .rebuild_thread_view(&scope, thread)
                .await?
                .ok_or_else(|| no_thread(thread))?,
        };
        let protections = self.project_settled(scope.located()).await.protections;
        self.summarize(scope, view, protections).await
    }

    /// A view as the API shows it: what the thread is doing now, its title, read from the one
    /// event that gives it, and the mode it runs with, as its project changes it (`protections`)
    /// when it is the person on this computer's.
    async fn summarize(
        &self,
        scope: Scope,
        view: ThreadView,
        protections: Protections,
    ) -> Result<ThreadSummary, ApiError> {
        let theirs = view.whose();
        let protections = if self.goes_by_project(theirs) {
            protections
        } else {
            Protections::default()
        };
        let title = match self.event_at(&scope, view.thread, view.title_seq).await? {
            Some(event) => title_of(&event),
            None => None,
        };
        let workspace = match self
            .event_at(&scope, view.thread, view.workspace_seq)
            .await?
        {
            Some(event) => match decode(event) {
                Ok(Event {
                    body: Body::WorkspaceBound(bound),
                    ..
                }) => Some(Box::new(bound)),
                _ => None,
            },
            None => None,
        };
        let context = view.context.as_ref().map(|last| {
            ContextUse::of(last.tokens, self.inner.harness.context_window(&last.model))
        });
        Ok(ThreadSummary {
            id: view.thread,
            project: scope.project(),
            status: self.status_of(&scope, &view).await,
            standing: view.standing,
            title,
            permissions: self
                .permissions_now(&scope, view.thread, protections)
                .await?,
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
            estimated: view.estimated,
            workspace,
            context,
        })
    }

    /// `thread`'s event at `seq`, when there is one: the one a summary reads its title, or where
    /// it works, from.
    async fn event_at(
        &self,
        scope: &Scope,
        thread: ThreadId,
        seq: Option<Seq>,
    ) -> Result<Option<RawEvent>, ApiError> {
        let Some(seq) = seq else {
            return Ok(None);
        };
        let before = Seq::new(seq.get().saturating_sub(1)).ok();
        let read = self
            .inner
            .harness
            .log()
            .read(scope, thread, before, 1)
            .await?;
        Ok(read.into_iter().next().filter(|event| event.seq == seq))
    }

    /// Whether `thread` is shown, from memory when the server holds it, else from the log.
    pub(crate) async fn standing(&self, thread: ThreadId) -> Result<Standing, ApiError> {
        match self.lock().get(&thread) {
            Some(Slot::Idle { thread: held, .. }) => return Ok(held.standing()),
            // Nothing runs in an archived or deleted thread.
            Some(Slot::Busy { .. }) => return Ok(Standing::Active),
            None => {}
        }
        let scope = self.scope_of(thread)?;
        standing_of(self.inner.harness.log(), &scope, thread)
            .await
            .map_err(|error| ApiError::from(&error))
    }

    /// Refuses with "no such thread" when `thread` is deleted.
    pub(crate) async fn ensure_not_deleted(&self, thread: ThreadId) -> Result<(), ApiError> {
        if self.standing(thread).await? == Standing::Deleted {
            return Err(no_thread(thread));
        }
        Ok(())
    }
}
