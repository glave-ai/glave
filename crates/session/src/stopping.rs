//! Stopping a server (`specs/one-server.md`, "Stopping"): it starts no new work, gives what
//! runs a grace to end on its own, then interrupts the rest, which end `interrupted`, not
//! `cancelled`, since the person did not stop them.

use std::sync::PoisonError;
use std::sync::atomic::Ordering;
use std::time::Duration;

use glave_harness::Cancel;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::held::Slot;
use crate::protocol::{ApiError, code};
use crate::server::Server;

impl<P, L, B> Server<P, L, B> {
    /// Stops the server: from now on it starts no new work; what runs has up to `grace` to
    /// end on its own, and the rest is interrupted. Waits for every turn and every task to
    /// end. Call before dropping the server.
    pub async fn stop(&self, grace: Duration) {
        let running: Vec<(Cancel, watch::Receiver<bool>)> = {
            let threads = self
                .inner
                .threads
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            // Set under the threads' lock, so no work starts after the running are listed.
            self.inner.stopping.store(true, Ordering::SeqCst);
            threads
                .values()
                .filter_map(|slot| match slot {
                    Slot::Busy {
                        cancel, finished, ..
                    } => Some((cancel.clone(), finished.clone())),
                    Slot::Idle { .. } => None,
                })
                .collect()
        };
        if !grace.is_zero() {
            let _ = tokio::time::timeout(grace, ended(&running)).await;
        }
        for (cancel, _) in &running {
            cancel.interrupt();
        }
        ended(&running).await;
        let tasks: Vec<JoinHandle<()>> = std::mem::take(
            &mut *self
                .inner
                .tasks
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        for task in tasks {
            let _ = task.await;
        }
    }

    /// Stops the server with no grace: whatever runs is interrupted at once.
    pub async fn shutdown(&self) {
        self.stop(Duration::ZERO).await;
    }

    /// Whether the server is stopping.
    pub(crate) fn is_stopping(&self) -> bool {
        self.inner.stopping.load(Ordering::SeqCst)
    }
}

/// Resolves once every one of `running` has ended.
async fn ended(running: &[(Cancel, watch::Receiver<bool>)]) {
    for (_, finished) in running {
        let mut finished = finished.clone();
        let _ = finished.wait_for(|done| *done).await;
    }
}

/// What work asked for while the server stops is refused with.
pub(crate) fn stopping() -> ApiError {
    ApiError::new(
        code::NOT_AVAILABLE,
        "glave is stopping, so it starts nothing new; try again once it runs again",
    )
}
