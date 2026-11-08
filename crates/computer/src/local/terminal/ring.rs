//! A terminal's output as its computer keeps it: the latest bytes, each at its offset since the
//! terminal opened, so a read from any offset answers what is still kept from there.

use std::collections::VecDeque;

use glave_ports::computer::{Bytes, Screenful};

/// The latest `room` bytes a terminal printed, where they end, and whether its shell ended.
#[derive(Debug)]
pub(super) struct Ring {
    kept: VecDeque<u8>,
    room: usize,
    /// The offset after the last byte printed.
    end: u64,
    ended: bool,
}

impl Ring {
    pub(super) fn new(room: usize) -> Self {
        Self {
            kept: VecDeque::new(),
            room,
            end: 0,
            ended: false,
        }
    }

    /// Keeps `bytes`, dropping the oldest past its room.
    pub(super) fn push(&mut self, bytes: &[u8]) {
        let tail = bytes.get(bytes.len().saturating_sub(self.room)..).unwrap_or(bytes);
        self.kept.extend(tail);
        let over = self.kept.len().saturating_sub(self.room);
        self.kept.drain(..over);
        let printed = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        self.end = self.end.saturating_add(printed);
    }

    /// Notes that its shell ended: nothing more will be printed.
    pub(super) fn end(&mut self) {
        self.ended = true;
    }

    /// Whether there is something to read from `from`, or nothing ever will be.
    pub(super) fn ready(&self, from: u64) -> bool {
        self.ended || from < self.end
    }

    /// The offset of the oldest byte kept.
    fn start(&self) -> u64 {
        let kept = u64::try_from(self.kept.len()).unwrap_or(u64::MAX);
        self.end.saturating_sub(kept)
    }

    /// At most `most` bytes from `from`: from the oldest kept when `from` is older, from its end
    /// when past it.
    pub(super) fn read(&self, from: u64, most: usize) -> Screenful {
        let from = from.clamp(self.start(), self.end);
        let skip = usize::try_from(from.saturating_sub(self.start())).unwrap_or(usize::MAX);
        let output: Vec<u8> = self.kept.iter().skip(skip).take(most).copied().collect();
        let taken = u64::try_from(output.len()).unwrap_or(u64::MAX);
        let next = from.saturating_add(taken);
        Screenful {
            from,
            output: Bytes::new(output),
            next,
            ended: self.ended && next == self.end,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_answers_from_its_offset_and_says_where_to_read_next() {
        let mut ring = Ring::new(8);
        ring.push(b"hello");
        let read = ring.read(1, 3);
        assert_eq!((read.from, read.output.as_slice(), read.next), (1, &b"ell"[..], 4));
        assert!(!read.ended);
        assert!(ring.ready(4));
        assert!(!ring.ready(5), "nothing past what was printed");
        let past = ring.read(99, 3);
        assert_eq!((past.from, past.output.as_slice(), past.next), (5, &b""[..], 5));
    }

    #[test]
    fn the_oldest_bytes_go_past_its_room_and_a_read_from_before_says_so() {
        let mut ring = Ring::new(4);
        ring.push(b"abc");
        ring.push(b"defgh");
        let read = ring.read(0, 10);
        assert_eq!((read.from, read.output.as_slice(), read.next), (4, &b"efgh"[..], 8));
        ring.push(b"0123456789");
        let read = ring.read(8, 10);
        assert_eq!((read.from, read.output.as_slice()), (14, &b"6789"[..]));
    }

    #[test]
    fn it_has_ended_once_its_shell_ended_and_all_was_read() {
        let mut ring = Ring::new(8);
        ring.push(b"bye");
        ring.end();
        assert!(ring.ready(3), "its end is there to read");
        assert!(!ring.read(0, 2).ended, "one byte is left");
        assert!(ring.read(0, 3).ended);
    }
}
