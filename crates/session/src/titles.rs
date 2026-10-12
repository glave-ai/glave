//! Titling a thread after its first exchange (`specs/session.md`, "Titles"), in a task of its
//! own: the turn's answer never waits for it, and it never holds the thread, so a turn started
//! meanwhile is never refused. Its usage and title are appended straight to the log.

use glave_event::ThreadId;
use glave_harness::{TitleCase, Titled};
use glave_ports::{BlobStore, EventLog, ModelProvider};

use crate::server::Server;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Titles `thread` from `case` in a task the server owns.
    pub(crate) fn title_later(&self, thread: ThreadId, case: TitleCase) {
        let server = self.clone();
        let task = tokio::spawn(async move {
            // A title someone gave the thread stays: no call is made for one.
            if server.renamed(thread).await {
                return;
            }
            if let Some(titled) = server.inner.harness.write_title(&case).await {
                server.record_title(thread, titled).await;
            }
        });
        self.own(task);
    }

    /// Whether `thread` has a title someone gave it; true when that cannot be read.
    async fn renamed(&self, thread: ThreadId) -> bool {
        let Ok(scope) = self.scope_of(thread).await else {
            return true;
        };
        let log = self.inner.harness.log();
        !matches!(
            log.latest_of(&scope, thread, &["thread.renamed"]).await,
            Ok(None)
        )
    }

    /// Records `titled` in `thread`, on a best effort: a thread that is gone keeps the title
    /// its first message gives it. The thread is never held for it, so a turn started
    /// meanwhile runs: the log orders the writes, and the turn takes the title in.
    async fn record_title(&self, thread: ThreadId, titled: Titled) {
        if let Ok(scope) = self.scope_of(thread).await {
            let _ = self
                .inner
                .harness
                .record_title(&scope, thread, &titled)
                .await;
        }
    }
}
