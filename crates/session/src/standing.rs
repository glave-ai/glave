//! Whether a thread is shown: `thread/archive`, `thread/unarchive`, `thread/delete`, and the
//! filtering of `thread/list`. A change is the thread's next event, appended by the server
//! that holds the thread, so it never conflicts with a thread open in that server.

use glave_event::ThreadId;
use glave_harness::{Caller, Change, Standing, standing_of};
use glave_ports::{BlobStore, EventLog, ModelProvider};
use tokio::sync::watch;

use crate::held::{Slot, Use, finished};
use crate::protocol::{ApiError, Reply, Show, code};
use crate::server::{MAX_PAGE, Server, api, no_thread};

/// The most threads a listing looks at while filling a page.
const MOST_SCANNED: usize = 10_000;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Archives, unarchives or deletes `thread` for `caller`.
    pub(crate) async fn change(
        &self,
        caller: &Caller,
        thread: ThreadId,
        change: Change,
    ) -> Result<Reply, ApiError> {
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

    /// The threads of `show`, most recently written first.
    pub(crate) async fn list(&self, limit: Option<usize>, show: Show) -> Result<Reply, ApiError> {
        let limit = limit.unwrap_or(20).clamp(1, MAX_PAGE);
        let wanted = match show {
            Show::Active => Standing::Active,
            Show::Archived => Standing::Archived,
            Show::Deleted => Standing::Deleted,
        };
        let mut found = Vec::new();
        let mut window = limit;
        let mut scanned = 0;
        loop {
            let ids = self
                .inner
                .harness
                .log()
                .threads(&self.inner.scope, window)
                .await
                .map_err(|err| ApiError::new(code::INTERNAL, err.to_string()))?;
            let fetched = ids.len();
            for id in ids.into_iter().skip(scanned) {
                if self.standing(id).await? == wanted {
                    found.push(self.summary(id));
                    if found.len() == limit {
                        return Ok(Reply::Threads { threads: found });
                    }
                }
            }
            if fetched < window || window >= MOST_SCANNED {
                return Ok(Reply::Threads { threads: found });
            }
            scanned = fetched;
            window = window.saturating_mul(2).min(MOST_SCANNED);
        }
    }

    /// Whether `thread` is shown, from memory when the server holds it, else from the log.
    pub(crate) async fn standing(&self, thread: ThreadId) -> Result<Standing, ApiError> {
        match self.lock().get(&thread) {
            Some(Slot::Idle(held)) => return Ok(held.standing()),
            // Nothing runs in an archived or deleted thread.
            Some(Slot::Busy { .. }) => return Ok(Standing::Active),
            None => {}
        }
        standing_of(self.inner.harness.log(), &self.inner.scope, thread)
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
