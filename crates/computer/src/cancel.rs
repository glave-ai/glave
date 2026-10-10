//! A switch that cancels a command: tripped by a `command/cancel` request, or for every
//! command when a connection ends.

use std::sync::Arc;

use glave_ports::{BoxFuture, Cancellation};
use tokio::sync::watch;

/// Cancels whatever runs with it. Clones share one switch.
#[derive(Debug, Clone)]
pub struct Switch {
    sender: Arc<watch::Sender<bool>>,
}

impl Default for Switch {
    fn default() -> Self {
        Self {
            sender: Arc::new(watch::Sender::new(false)),
        }
    }
}

impl Switch {
    pub fn cancel(&self) {
        self.sender.send_replace(true);
    }
}

impl Cancellation for Switch {
    fn is_cancelled(&self) -> bool {
        *self.sender.borrow()
    }

    fn cancelled(&self) -> BoxFuture<'_, ()> {
        let mut receiver = self.sender.subscribe();
        Box::pin(async move {
            // The sender lives as long as `self`, so waiting cannot fail.
            let _ = receiver.wait_for(|cancelled| *cancelled).await;
        })
    }
}
