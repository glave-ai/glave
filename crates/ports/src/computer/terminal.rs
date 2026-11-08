//! A person's own terminal on a computer (`specs/computers.md`, "Terminals"): their shell in a
//! folder, on a pseudo-terminal, outside the agent's sandbox, typed into and read as any
//! terminal is. Its output is kept in a ring and read from an offset, so a reader that went
//! away reads on from where it was, one that fell behind is told what it missed, and memory
//! stays bounded however much the shell prints.

use serde::{Deserialize, Serialize};

use super::bytes::Bytes;
use super::refusal::Refusal;
use super::types::{ComputerError, TerminalId};

/// The most terminals one computer keeps open at once: a few for its person, as a terminal
/// app's tabs, and few enough that a client opening them in a loop cannot fill the computer.
/// One more is refused, saying so.
pub const MAX_TERMINALS: usize = 8;

/// The most terminals a computer keeps open for one owner (a thread): a few side by side.
pub const MAX_OWNED_TERMINALS: usize = 4;

/// The most of a terminal's output its computer keeps, in bytes: the most recent, as a
/// terminal's scrollback keeps it. Older output is dropped, and a read from before it says so.
pub const TERMINAL_KEPT: usize = 256 * 1024;

/// The most bytes one read answers, and one write takes: a screenful many times over, small
/// enough that one call never holds the wire for long.
pub const TERMINAL_CHUNK: usize = 64 * 1024;

/// The most columns or rows a terminal is opened or resized to.
const TERMINAL_MAX_SIDE: u16 = 1_000;

/// How many columns and rows a terminal shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl TerminalSize {
    /// The size, when each side is 1 to 1,000; refused otherwise.
    pub fn checked(self) -> Result<Self, ComputerError> {
        let fits = |side: u16| (1..=TERMINAL_MAX_SIDE).contains(&side);
        if fits(self.cols) && fits(self.rows) {
            Ok(self)
        } else {
            Err(ComputerError::Refused(
                Refusal::TerminalSize {
                    most: TERMINAL_MAX_SIDE.to_string(),
                    cols: self.cols.to_string(),
                    rows: self.rows.to_string(),
                }
                .into(),
            ))
        }
    }
}

/// A terminal open on a computer, as its listing shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TerminalInfo {
    pub id: TerminalId,
    /// The folder its shell started in, as its computer names it.
    pub folder: String,
    /// Whether its shell has ended; it stays until it is closed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ended: bool,
}

/// What a read of a terminal answers: what it printed from `from`, the offset of the first
/// byte, up to `next`, the offset to read from next. A `from` past the offset asked for says
/// the bytes between were dropped, the computer keeping only the latest [`TERMINAL_KEPT`].
/// `ended` once its shell has ended and all it printed has been read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Screenful {
    pub from: u64,
    #[schemars(with = "String")]
    pub output: Bytes,
    pub next: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ended: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_is_one_to_the_most_on_each_side() {
        let size = |cols, rows| TerminalSize { cols, rows }.checked();
        assert!(size(80, 24).is_ok());
        assert!(size(1, TERMINAL_MAX_SIDE).is_ok());
        assert!(size(0, 24).is_err());
        assert!(size(80, TERMINAL_MAX_SIDE + 1).is_err());
    }

    #[test]
    fn a_screenful_carries_its_bytes_as_base64() {
        let screen = Screenful {
            from: 3,
            output: Bytes::new(b"hi\r\n".to_vec()),
            next: 7,
            ended: false,
        };
        let json = serde_json::to_string(&screen).unwrap();
        assert_eq!(json, r#"{"from":3,"output":"aGkNCg==","next":7}"#);
        assert_eq!(serde_json::from_str::<Screenful>(&json).unwrap(), screen);
    }
}
