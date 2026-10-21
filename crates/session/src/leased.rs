//! Work on a thread under its lease (`specs/invariants.md`, 3): a turn, a compaction or a change
//! to the thread runs only while this server holds the thread's lease in the store, renewed as it
//! runs and let go when it ends, so another server asked to work on the thread meanwhile is
//! refused (`-32011`). Reading a thread takes no lease and records nothing; work that takes the
//! lease reads the thread again when another glave wrote to it since, and closes what a turn left
//! open before it runs (`Harness::repair`). As the person's server starts, it closes every turn
//! left open whose lease lapsed or that has none (`specs/one-server.md`, "After a restart").

use std::convert::Infallible;
use std::future::Future;
use std::pin::pin;
use std::time::Duration;

use glave_event::ThreadId;
use glave_harness::Cancel;
use glave_ports::{
    BlobStore, EventLog, Holder, Lease, Leased, Leases, ModelProvider, Scope, Standing,
    ThreadQuery, ThreadView,
};

use crate::held::Slot;
use crate::protocol::{ApiError, code};
use crate::server::Server;
use crate::words::Sentence;

/// How long a lease lasts unless it is renewed: a server that stops without letting go keeps
/// its threads from others this long at most.
pub(crate) const LEASE: Duration = Duration::from_secs(30);

/// How often work renews its lease, well within [`LEASE`].
const RENEW: Duration = Duration::from_secs(10);

/// Views of open turns read at a time as the server starts.
const PAGE: usize = 500;

/// What a server found left open as it started: turns ([`Server::repair_left_open`]), or tasks
/// ([`Server::recover_tasks`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Repaired {
    /// Those it closed, `interrupted`.
    pub closed: usize,
    /// Those another server's lease still holds: running there, or left by a server that
    /// stopped less than a lease ago. Look again once it may have lapsed.
    pub held: usize,
}

/// Refused: another server holds the thread's lease.
pub(crate) fn elsewhere(thread: ThreadId) -> ApiError {
    ApiError::new(
        code::RUNNING_ELSEWHERE,
        Sentence::RunningElsewhere {
            thread: thread.to_string(),
        },
    )
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The id this server holds leases by, made the first time it takes one.
    pub(crate) fn holder(&self) -> Result<Holder, ApiError> {
        if let Some(holder) = self.inner.holder.get() {
            return Ok(*holder);
        }
        let harness = &self.inner.harness;
        let made = Holder::new(harness.clock(), harness.entropy())?;
        Ok(*self.inner.holder.get_or_init(|| made))
    }

    /// Takes `thread`'s lease for this server: `None` while another server holds it.
    pub(crate) async fn take(&self, thread: ThreadId) -> Result<Option<Lease>, ApiError> {
        let on = Leased::Thread(thread);
        let leases = &self.inner.leases;
        leases
            .acquire(self.inner.org, on, self.holder()?, LEASE)
            .await
            .map_err(|error| ApiError::internal(&error))
    }

    /// Takes `thread`'s lease for work on it, refused with `-32011` while another server holds
    /// it.
    pub(crate) async fn lease(&self, thread: ThreadId) -> Result<Lease, ApiError> {
        self.take(thread).await?.ok_or_else(|| elsewhere(thread))
    }

    /// Lets go of `lease`; one that cannot be let go lapses on its own.
    pub(crate) async fn let_go(&self, lease: &Lease) {
        if let Err(error) = self.inner.leases.release(lease).await {
            tracing::warn!(%error, "a thread's lease could not be let go; it lapses on its own");
        }
    }

    /// Closes every turn left open in the projects whose tasks this server runs, whose lease
    /// lapsed or that has none: for the person's server as it starts, before it serves. A turn
    /// another server's lease holds is left to it, or to this server once the lease lapses.
    pub async fn repair_left_open(&self) -> Result<Repaired, ApiError> {
        let mut repaired = Repaired::default();
        for project in self.task_projects().await? {
            // Glave's own: recovery, which no one asks for.
            let scope = Scope::glaves_own(self.inner.org, project);
            for view in self.open_turns(&scope).await? {
                // A subagent's turn runs, and goes on, under its parent's lease.
                let root = view.parent.unwrap_or(view.thread);
                self.repair(scope, view.thread, root, &mut repaired).await?;
            }
        }
        Ok(repaired)
    }

    /// Every thread in `scope` with a turn open, whatever its standing.
    async fn open_turns(&self, scope: &Scope) -> Result<Vec<ThreadView>, ApiError> {
        let log = self.inner.harness.log();
        let mut open = Vec::new();
        for standing in [Standing::Active, Standing::Archived, Standing::Deleted] {
            let mut after = None;
            loop {
                let query = ThreadQuery {
                    standing,
                    limit: PAGE,
                    after,
                    open: true,
                    parent: None,
                };
                let page = log.thread_views(scope, &query).await?;
                after = page.last().map(ThreadView::cursor);
                let full = page.len() == PAGE;
                open.extend(page);
                if !full {
                    break;
                }
            }
        }
        Ok(open)
    }

    /// Closes the turn left open in `thread`, whose turn runs under `root`'s lease, unless work
    /// on `root` runs here or another server holds its lease. One that waits stays waiting.
    async fn repair(
        &self,
        scope: Scope,
        thread: ThreadId,
        root: ThreadId,
        repaired: &mut Repaired,
    ) -> Result<(), ApiError> {
        if matches!(self.lock().get(&root), Some(Slot::Busy { .. })) {
            return Ok(());
        }
        let Some(lease) = self.take(root).await? else {
            repaired.held = repaired.held.saturating_add(1);
            return Ok(());
        };
        let closed = self.inner.harness.repair(scope, thread).await;
        self.let_go(&lease).await;
        if closed.map_err(|error| ApiError::from(&error))? {
            repaired.closed = repaired.closed.saturating_add(1);
        }
        Ok(())
    }
}

/// Runs `work` while renewing `lease` in `leases`, every [`RENEW`]: once the lease is lost to
/// another server, `cancel` interrupts the work, which then ends `interrupted` (the log's
/// `(thread, seq)` check keeps the two from both writing). A renewal that fails is tried again.
pub(crate) async fn renewing<F: Future>(
    leases: &dyn Leases,
    lease: &Lease,
    cancel: &Cancel,
    work: F,
) -> F::Output {
    let work = pin!(work);
    let keep = pin!(keep(leases, lease, cancel));
    tokio::select! {
        biased;
        done = work => done,
        never = keep => match never {},
    }
}

/// Renews `lease` every [`RENEW`] until dropped, interrupting `cancel` once it is lost.
#[expect(
    clippy::infinite_loop,
    reason = "it runs until the work it keeps the lease for ends; an async fn cannot return `!`"
)]
async fn keep(leases: &dyn Leases, lease: &Lease, cancel: &Cancel) -> Infallible {
    let mut held = *lease;
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + RENEW, RENEW);
    loop {
        tick.tick().await;
        match leases.renew(&held, LEASE).await {
            Ok(Some(renewed)) => held = renewed,
            Ok(None) => {
                tracing::warn!(lease = %lease.on.key(), "a thread's lease was lost");
                cancel.interrupt();
            }
            Err(error) => tracing::warn!(%error, "a thread's lease could not be renewed"),
        }
    }
}
