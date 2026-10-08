//! The line that counts the seconds while a model has not answered.

use std::io::IsTerminal as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use crate::terminal::{DIM, flush, paint};

/// The "waiting for the model · 12 s" line, shown only on a terminal, from the moment a model
/// is asked until anything comes back. Clones share it.
#[derive(Clone)]
pub struct Waiting(Arc<Mutex<WaitingLine>>);

struct WaitingLine {
    /// Off when the output is not a terminal, where a line cannot be rewritten.
    enabled: bool,
    color: bool,
    speaker: String,
    since: Option<Instant>,
    shown: bool,
}

impl Waiting {
    pub fn new(color: bool, speaker: &str) -> Self {
        Self(Arc::new(Mutex::new(WaitingLine {
            enabled: std::io::stdout().is_terminal(),
            color,
            speaker: speaker.to_owned(),
            since: None,
            shown: false,
        })))
    }

    fn update(&self, change: impl FnOnce(&mut WaitingLine)) {
        let mut line = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        change(&mut line);
    }

    pub fn start(&self) {
        self.update(|line| line.since = Some(Instant::now()));
    }

    /// Clears the line, if shown, before anything else is printed.
    pub fn stop(&self) {
        self.update(|line| {
            line.since = None;
            if line.shown {
                print!("\r\x1b[2K");
                line.shown = false;
            }
        });
    }

    /// Rewrites the line with the seconds waited so far, after the first.
    pub fn tick(&self) {
        self.update(|line| {
            let Some(since) = line.since.filter(|_| line.enabled) else {
                return;
            };
            let seconds = since.elapsed().as_secs();
            if seconds == 0 {
                return;
            }
            let text = format!("waiting for {} · {seconds} s", line.speaker);
            print!("\r\x1b[2K  {}", paint(line.color, DIM, &text));
            line.shown = true;
            flush();
        });
    }
}
