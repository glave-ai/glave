//! Commands kept running in the background (`specs/computers.md`, "Background commands"):
//! what a computer answers about them, and the bounds every computer keeps, the same on this
//! machine and across the wire.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::types::{CommandId, Ending, Finished};

/// The most commands one workspace keeps running in the background at once: a dev server, a
/// watcher and a database beside a build, with room to spare, and few enough that a model
/// starting them in a loop cannot fill the computer. One more is refused, saying so.
pub const MAX_BACKGROUND: usize = 8;

/// The most of a command's unread output a computer keeps, in bytes: the most recent, as a
/// terminal's scrollback keeps it (Codex keeps the same per session). Older output is dropped,
/// and the next read says how much.
pub const KEPT_OUTPUT: usize = 1024 * 1024;

/// How long a command runs in the background with nothing reading its output before its
/// computer stops it: past a person's break between turns, short of a server a thread forgot,
/// or one whose harness went away for good, running for days. Each read starts it again.
pub const IDLE: Duration = Duration::from_hours(1);

/// The longest one read waits for a command to end: a foreground command's longest timeout.
pub const MAX_WAIT: Duration = Duration::from_mins(10);

/// What a command in the background printed since the last read, standard output and error
/// together, and how it ended once it has.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Printed {
    pub text: String,
    /// Bytes it printed since the last read that were dropped before this one, to keep within
    /// [`KEPT_OUTPUT`].
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dropped: u64,
    /// How it ended and how long it ran; `None` while it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<Finished>,
}

/// A command in the background, as a listing shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Background {
    pub id: CommandId,
    pub script: String,
    /// How long it has run, or ran, in milliseconds.
    pub millis: u64,
    /// How it ended; `None` while it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<Ending>,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if"
)]
fn is_zero(value: &u64) -> bool {
    *value == 0
}
