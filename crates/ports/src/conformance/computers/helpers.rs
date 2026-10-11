//! What the computers suite needs to drive a computer: output it can read, a cancellation it
//! can trip, and short ways to run commands and build writes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

use super::Verdict;
use crate::computer::{
    Bytes, Command, ComputerError, Computers, Ending, FileWrite, Finished, Output, WorkspaceId,
};
use crate::tools::{BoxFuture, Cancellation};

/// Says which step failed, and how.
pub(super) fn failed(step: &'static str) -> impl Fn(ComputerError) -> String {
    move |error| format!("{step}: {error}")
}

pub(super) fn files(pairs: &[(&str, &[u8])]) -> Vec<FileWrite> {
    pairs
        .iter()
        .map(|(path, content)| FileWrite::new((*path).to_owned(), Bytes::new(*content)))
        .collect()
}

/// A cancellation the suite trips by hand.
#[derive(Default)]
pub(super) struct Trip {
    tripped: AtomicBool,
    wakers: Mutex<Vec<Waker>>,
}

impl Trip {
    fn trip(&self) {
        self.tripped.store(true, Ordering::SeqCst);
        if let Ok(mut wakers) = self.wakers.lock() {
            for waker in wakers.drain(..) {
                waker.wake();
            }
        }
    }
}

impl Cancellation for Trip {
    fn is_cancelled(&self) -> bool {
        self.tripped.load(Ordering::SeqCst)
    }

    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(std::future::poll_fn(|cx| {
            if self.is_cancelled() {
                return Poll::Ready(());
            }
            if let Ok(mut wakers) = self.wakers.lock() {
                wakers.push(cx.waker().clone());
            }
            // Tripped between the check and the registration: do not miss it.
            if self.is_cancelled() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }))
    }
}

/// Collects a command's output; trips `trip` once the output holds `marker`.
pub(super) struct Collect<'a> {
    text: Arc<Mutex<String>>,
    trigger: Option<(&'a Trip, &'static str)>,
}

impl<'a> Collect<'a> {
    pub(super) fn new() -> Self {
        Self {
            text: Arc::default(),
            trigger: None,
        }
    }

    pub(super) fn tripping(trip: &'a Trip, marker: &'static str) -> Self {
        Self {
            text: Arc::default(),
            trigger: Some((trip, marker)),
        }
    }

    pub(super) fn text(&self) -> String {
        self.text
            .lock()
            .map(|text| text.clone())
            .unwrap_or_default()
    }
}

impl Output for Collect<'_> {
    fn write<'b>(&'b self, text: &'b str) -> BoxFuture<'b, ()> {
        let seen = self.text.lock().map(|mut all| {
            all.push_str(text);
            all.clone()
        });
        if let (Ok(seen), Some((trip, marker))) = (seen, self.trigger)
            && seen.contains(marker)
        {
            trip.trip();
        }
        Box::pin(std::future::ready(()))
    }
}

/// Runs a command to its end and returns how it ended and everything it printed.
pub(super) async fn run<C: Computers>(
    computers: &C,
    workspace: WorkspaceId,
    command: &Command,
) -> Result<(Finished, String), String> {
    let output = Collect::new();
    let never = Trip::default();
    let finished = computers
        .run(workspace, command, &output, &never)
        .await
        .map_err(|error| format!("run {:?}: {error}", command.script))?;
    Ok((finished, output.text()))
}

/// Runs a setup command that must succeed.
pub(super) async fn run_quietly<C: Computers>(
    computers: &C,
    workspace: WorkspaceId,
    script: &str,
) -> Verdict {
    let (finished, output) = run(computers, workspace, &Command::new(script)).await?;
    if finished.ending == (Ending::Exited { code: 0 }) {
        Ok(())
    } else {
        Err(format!("setting up with {script:?} failed: {output}"))
    }
}
