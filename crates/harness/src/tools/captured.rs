//! A command's output as the model is shown it: its start and its end, with a count of what
//! fell in between, so a long run fits.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::Mutex;

use glave_ports::BoxFuture;
use glave_ports::computer::Output;

/// Output kept from the start and the end of a long run; the middle is dropped.
const HEAD_BYTES: usize = 8 * 1024;
const TAIL_BYTES: usize = 24 * 1024;

/// A command's output as it arrives, keeping its start and its end.
#[derive(Default)]
pub(crate) struct Capture(Mutex<Captured>);

impl Capture {
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

/// The start and the end of a command's output, with a count of what fell in between.
#[derive(Debug, Default)]
pub(crate) struct Captured {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: usize,
}

impl Captured {
    /// `text`, kept as the model is shown a command's output.
    pub(crate) fn of(text: &str) -> Self {
        let mut captured = Self::default();
        captured.push(text.as_bytes());
        captured
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.total = self.total.saturating_add(bytes.len());
        let room = HEAD_BYTES.saturating_sub(self.head.len());
        let (to_head, to_tail) = bytes.split_at(room.min(bytes.len()));
        self.head.extend_from_slice(to_head);
        self.tail.extend(to_tail);
        while self.tail.len() > TAIL_BYTES {
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
        let mut captured = Captured::default();
        captured.push(&vec![b'a'; HEAD_BYTES + 100]);
        captured.push(&vec![b'z'; TAIL_BYTES + 50]);
        let text = captured.render();
        assert!(text.starts_with('a'));
        assert!(text.ends_with('z'));
        assert!(text.contains("[… 150 bytes of output omitted …]"));
    }

    #[test]
    fn short_output_is_complete() {
        let mut captured = Captured::default();
        captured.push(b"hello\n");
        captured.push(b"world\n");
        assert_eq!(captured.render(), "hello\nworld\n");
    }
}
