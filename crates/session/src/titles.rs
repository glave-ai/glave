//! Titling a thread after its first exchange (`specs/session.md`, "Titles"), in a task of its
//! own: the turn's answer never waits for it. The model call runs while the thread is idle;
//! its usage and title are then recorded through the thread, after any turn running by then.

use glave_event::ThreadId;
use glave_harness::{TitleCase, Titled};
use glave_ports::{BlobStore, EventLog, ModelProvider};
use tokio::sync::watch;

use crate::held::{Slot, Use, finished};
use crate::protocol::code;
use crate::server::Server;

/// How many times recording a title waits for work on the thread to end before giving up.
const TRIES: usize = 3;

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

    /// Records `titled` in `thread` once nothing else works on it. On a best effort: a thread
    /// that is gone, or busy every time, keeps the title its first message gives it.
    async fn record_title(&self, thread: ThreadId, titled: Titled) {
        for _ in 0..TRIES {
            let (finish, mut done) = watch::channel(None);
            let titled = titled.clone();
            let occupied = self
                .occupy_for(
                    thread,
                    finish,
                    Use::Manage,
                    move |inner, mut held, _| async move {
                        let recorded = inner.harness.record_title(&mut held, &titled).await;
                        (held, recorded)
                    },
                )
                .await;
            match occupied {
                Ok(()) => {
                    let _ = finished(&mut done).await;
                    return;
                }
                Err(error) if error.code == code::TURN_RUNNING => self.until_idle(thread).await,
                Err(_) => return,
            }
        }
    }

    /// Waits until the work holding `thread` busy, if any, has ended.
    async fn until_idle(&self, thread: ThreadId) {
        let finished = match self.lock().get(&thread) {
            Some(Slot::Busy { finished, .. }) => finished.clone(),
            _ => return,
        };
        let mut finished = finished;
        let _ = finished.wait_for(|done| *done).await;
    }
}
