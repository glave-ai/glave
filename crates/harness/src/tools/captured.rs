//! Output as the model is shown it: its start and its end, with a count of what fell in
//! between, so a long command's run, or any tool's output, fits the room it has.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::Mutex;

use glave_ports::BoxFuture;
use glave_ports::computer::Output;

/// The most of a command's output kept, however much room the model has: a quarter from its
/// start and the rest from its end, where errors and results are.
pub(crate) const MOST_BYTES: usize = 32 * 1024;

/// The most the note in place of the middle takes.
const NOTE_BYTES: usize = 64;

/// A command's output as it arrives, keeping its start and its end.
pub(crate) struct Capture(Mutex<Captured>);

impl Capture {
    /// Keeping at most `room` bytes, and never more than [`MOST_BYTES`].
    pub(crate) fn within(room: usize) -> Self {
        Self(Mutex::new(Captured::within(room)))
    }

    pub(crate) fn render(&self) -> String {
        self.0.lock().map(|c| c.render()).unwrap_or_default()
    }
}

impl Output for Capture {
    fn write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, ()> {
        if let Ok(mut captured) = self.0.lock() {
            captured.push(text.as_bytes());
        }
        Box::pin(std::future::ready(()))
    }
}

/// The start and the end of some output, with a count of what fell in between.
#[derive(Debug)]
pub(crate) struct Captured {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: usize,
    head_bytes: usize,
    tail_bytes: usize,
}

impl Captured {
    /// Keeping at most `room` bytes, and never more than [`MOST_BYTES`].
    pub(crate) fn within(room: usize) -> Self {
        Self::fitting(room.min(MOST_BYTES))
    }

    /// Keeping at most `room` bytes: a quarter from the start, the rest from the end.
    fn fitting(room: usize) -> Self {
        let head_bytes = room / 4;
        Self {
            head: Vec::new(),
            tail: VecDeque::new(),
            total: 0,
            head_bytes,
            tail_bytes: room.saturating_sub(head_bytes),
        }
    }

    /// `text`, kept as the model is shown a command's output in `room` bytes.
    pub(crate) fn of(text: &str, room: usize) -> Self {
        let mut captured = Self::within(room);
        captured.push(text.as_bytes());
        captured
    }

    /// `text` as one tool's output in `room` bytes, however long: whole when it fits.
    pub(crate) fn fit(text: &str, room: usize) -> String {
        if text.len() <= room {
            return text.to_owned();
        }
        // The note saying what was left out fits too.
        let mut captured = Self::fitting(room.saturating_sub(NOTE_BYTES));
        captured.push(text.as_bytes());
        captured.render()
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.total = self.total.saturating_add(bytes.len());
        let room = self.head_bytes.saturating_sub(self.head.len());
        let (to_head, to_tail) = bytes.split_at(room.min(bytes.len()));
        self.head.extend_from_slice(to_head);
        self.tail.extend(to_tail);
        while self.tail.len() > self.tail_bytes {
            self.tail.pop_front();
        }
    }

    pub(crate) fn render(&self) -> String {
        let mut text = String::from_utf8_lossy(&self.head).into_owned();
        let kept = self.head.len().saturating_add(self.tail.len());
        if self.total > kept {
            let _ = write!(
                text,
                "\n[… {} bytes of output omitted …]\n",
                self.total.saturating_sub(kept)
            );
        }
        let tail: Vec<u8> = self.tail.iter().copied().collect();
        text.push_str(&String::from_utf8_lossy(&tail));
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_output_keeps_head_and_tail() {
        let mut captured = Captured::within(usize::MAX);
        captured.push(&vec![b'a'; MOST_BYTES / 4 + 100]);
        captured.push(&vec![b'z'; MOST_BYTES * 3 / 4 + 50]);
        let text = captured.render();
        assert!(text.starts_with('a'));
        assert!(text.ends_with('z'));
        assert!(text.contains("[… 150 bytes of output omitted …]"));
    }

    #[test]
    fn short_output_is_complete() {
        let mut captured = Captured::within(MOST_BYTES);
        captured.push(b"hello\n");
        captured.push(b"world\n");
        assert_eq!(captured.render(), "hello\nworld\n");
    }

    #[test]
    fn a_small_room_keeps_less_of_both_ends() {
        let text = format!("{}{}", "a".repeat(5_000), "z".repeat(5_000));
        let fitted = Captured::of(&text, 1_000).render();
        assert!(fitted.starts_with(&"a".repeat(250)), "{fitted}");
        assert!(fitted.ends_with(&"z".repeat(750)), "{fitted}");
        assert!(fitted.contains("[… 9000 bytes of output omitted …]"));
        assert_eq!(Captured::fit("short", 1_000), "short");
        let fit = Captured::fit(&"x".repeat(200_000), 100_000);
        assert!(
            fit.len() < 100_100,
            "a tool's output may be longer than a command's"
        );
    }
}
