//! What the person types: lines, read by one thread so that a question abandoned by a cancel
//! never swallows the next line; and Ctrl-C, which cancels whatever is waiting.

use std::io::BufRead as _;
use std::sync::Arc;

use glave_harness::Cancel;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;

/// Lines read ahead of whoever asks for them.
const READ_AHEAD: usize = 16;

/// The person's input, one line at a time. Clones share it.
#[derive(Clone)]
pub(crate) struct Input {
    lines: Arc<Mutex<mpsc::Receiver<String>>>,
}

impl Input {
    /// Starts reading standard input. The reader is a plain thread, since a blocking read of a
    /// terminal cannot be cancelled; it ends at the end of input, or at the first line after
    /// everyone has stopped listening.
    pub(crate) fn start() -> Self {
        let (sender, receiver) = mpsc::channel(READ_AHEAD);
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if sender.blocking_send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            lines: Arc::new(Mutex::new(receiver)),
        }
    }

    /// The next line, without its line ending; `None` at the end of input.
    pub(crate) async fn line(&self) -> Option<String> {
        self.lines.lock().await.recv().await
    }
}

/// Cancels a [`Cancel`] when the person presses Ctrl-C, for as long as this lives.
pub(crate) struct CtrlC(JoinHandle<()>);

impl CtrlC {
    /// Listens from the moment it returns, so a Ctrl-C right after is never missed.
    #[cfg(unix)]
    pub(crate) fn cancels(cancel: &Cancel) -> Self {
        use tokio::signal::unix::{SignalKind, signal};
        let cancel = cancel.clone();
        let interrupts = signal(SignalKind::interrupt()).ok();
        Self(tokio::spawn(async move {
            if let Some(mut interrupts) = interrupts
                && interrupts.recv().await.is_some()
            {
                cancel.cancel();
            }
        }))
    }

    #[cfg(not(unix))]
    pub fn cancels(cancel: &Cancel) -> Self {
        let cancel = cancel.clone();
        Self(tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                cancel.cancel();
            }
        }))
    }
}

impl Drop for CtrlC {
    fn drop(&mut self) {
        self.0.abort();
    }
}
