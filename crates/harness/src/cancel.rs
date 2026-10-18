//! Cancelling a turn from outside it: a Ctrl-C handler, a session API call, a deadline, or
//! glave stopping, which interrupts it.

use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;

use glave_event::Outcome;
use glave_ports::{BoxFuture, Cancellation};
use tokio::sync::watch;

/// A switch that cancels a turn. Clones share it, so one can go to whoever may cancel while
/// the turn runs with another. Cancelling works from any thread, before or during the turn,
/// and more than once; a turn given a cancelled switch stops at its first step. The first
/// reason given is how the turn ends: cancelled, or interrupted.
#[derive(Debug, Clone)]
pub struct Cancel {
    sender: Arc<watch::Sender<Option<Outcome>>>,
}

impl Default for Cancel {
    fn default() -> Self {
        Self::new()
    }
}

impl Cancel {
    pub fn new() -> Self {
        Self {
            sender: Arc::new(watch::Sender::new(None)),
        }
    }

    /// Cancels the turn. A running command is stopped with everything it started; a model
    /// call or a question to the person is abandoned.
    pub fn cancel(&self) {
        self.stop(Outcome::Cancelled);
    }

    /// Cancels the turn as [`Cancel::cancel`] does, for a reason the person did not choose,
    /// such as glave stopping: the turn ends interrupted, not cancelled.
    pub fn interrupt(&self) {
        self.stop(Outcome::Interrupted);
    }

    fn stop(&self, outcome: Outcome) {
        self.sender.send_if_modified(|now| {
            let first = now.is_none();
            if first {
                *now = Some(outcome);
            }
            first
        });
    }

    pub fn is_cancelled(&self) -> bool {
        self.sender.borrow().is_some()
    }

    /// How a turn this stops ends: interrupted when it was interrupted, else cancelled.
    pub fn outcome(&self) -> Outcome {
        self.sender.borrow().unwrap_or(Outcome::Cancelled)
    }

    /// Resolves once this is cancelled.
    pub async fn cancelled(&self) {
        let mut receiver = self.sender.subscribe();
        // The sender lives as long as `self`, so waiting cannot fail.
        let _ = receiver.wait_for(Option::is_some).await;
    }

    /// Runs `work` until it finishes, or until this is cancelled: `None` then, and `work` is
    /// dropped. Cancellation wins a tie.
    pub async fn run<T>(&self, work: impl Future<Output = T>) -> Option<T> {
        let mut work = pin!(work);
        let mut cancelled = pin!(self.cancelled());
        poll_fn(|cx| {
            if cancelled.as_mut().poll(cx).is_ready() {
                return Poll::Ready(None);
            }
            work.as_mut().poll(cx).map(Some)
        })
        .await
    }
}

impl Cancellation for Cancel {
    fn is_cancelled(&self) -> bool {
        Cancel::is_cancelled(self)
    }

    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(Cancel::cancelled(self))
    }
}

#[cfg(test)]
mod tests {

    use glave_ports::block_on;

    use super::*;

    #[test]
    fn work_finishes_unless_cancelled() {
        let cancel = Cancel::new();
        assert_eq!(block_on!(cancel.run(async { 7 })), Some(7));
        assert!(!cancel.is_cancelled());
        let other = cancel.clone();
        other.cancel();
        assert!(cancel.is_cancelled(), "clones share the switch");
        assert_eq!(
            block_on!(cancel.run(async { 7 })),
            None,
            "cancellation wins a tie"
        );
    }

    #[test]
    fn the_first_reason_is_how_the_turn_ends() {
        let cancel = Cancel::new();
        assert_eq!(cancel.outcome(), Outcome::Cancelled);
        cancel.interrupt();
        cancel.cancel();
        assert!(cancel.is_cancelled());
        assert_eq!(cancel.outcome(), Outcome::Interrupted);
        let other = Cancel::new();
        other.cancel();
        other.interrupt();
        assert_eq!(other.outcome(), Outcome::Cancelled);
    }

    #[test]
    fn cancelling_stops_work_that_would_never_finish() {
        let cancel = Cancel::new();
        let canceller = cancel.clone();
        let result = block_on!(cancel.run(async move {
            canceller.cancel();
            std::future::pending::<()>().await;
        }));
        assert_eq!(result, None);
        block_on!(cancel.cancelled());
    }
}
