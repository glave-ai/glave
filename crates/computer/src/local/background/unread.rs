//! A command's output that no one has read yet: at most [`KEPT_OUTPUT`] bytes, the most recent,
//! with a count of what was dropped to stay within it.

use std::collections::VecDeque;

use glave_ports::computer::KEPT_OUTPUT;

#[derive(Debug, Default)]
pub(super) struct Unread {
    text: VecDeque<u8>,
    dropped: u64,
}

impl Unread {
    /// Adds `text`, dropping the oldest whole characters beyond the limit.
    pub(super) fn push(&mut self, text: &str) {
        self.text.extend(text.as_bytes());
        let mut over = self.text.len().saturating_sub(KEPT_OUTPUT);
        // Never half a character: what is left starts where one does.
        while self
            .text
            .get(over)
            .is_some_and(|byte| byte & 0b1100_0000 == 0b1000_0000)
        {
            over = over.saturating_add(1);
        }
        if over > 0 {
            self.text.drain(..over);
            self.dropped = self
                .dropped
                .saturating_add(u64::try_from(over).unwrap_or(u64::MAX));
        }
    }

    /// Everything unread, and how much was dropped since the last take.
    pub(super) fn take(&mut self) -> (String, u64) {
        let bytes: Vec<u8> = self.text.drain(..).collect();
        let text = String::from_utf8(bytes)
            .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned());
        (text, std::mem::take(&mut self.dropped))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_most_recent_output_and_counts_the_rest() {
        let mut unread = Unread::default();
        unread.push("start\n");
        unread.push(&"a".repeat(KEPT_OUTPUT));
        unread.push("end\n");
        let (text, dropped) = unread.take();
        assert_eq!(text.len(), KEPT_OUTPUT);
        assert!(text.ends_with("aend\n"));
        assert_eq!(dropped, 10);
        assert_eq!(unread.take(), (String::new(), 0));
    }

    #[test]
    fn never_keeps_half_a_character() {
        let mut unread = Unread::default();
        unread.push(&"é".repeat(KEPT_OUTPUT / 2 + 1));
        let (text, dropped) = unread.take();
        assert!(text.chars().all(|c| c == 'é'));
        assert_eq!(text.len(), KEPT_OUTPUT);
        assert_eq!(dropped, 2);
    }
}
