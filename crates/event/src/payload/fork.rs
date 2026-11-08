//! `thread.forked`: where a thread made from another's history begins its own
//! (`specs/session.md`, "Forking a thread"): the point of the other it continues from, and how
//! it stands as to files, which a fork never rewinds, said in a sentence.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::envelope::Seq;
use crate::id::ThreadId;

pub(crate) const TYPE: &str = "thread.forked";

/// A thread was forked from another: what comes before this event, but its own start and where
/// it works, is a copy of the other's events up to and including `from`'s `seq`, and what comes
/// after is its own. The thread's own receipts start here: what the copies used is the other's.
/// Glave's own record, which call ids do not count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadForked {
    /// The thread it was forked from, and the last of its events it holds.
    pub from: ForkPoint,
    /// How it stands as to the other's files.
    pub files: ForkFiles,
    /// What the person is told of it, recorded in English and written again in the reader's
    /// language from `files`.
    pub told: String,
    /// How many commands in the background the copies started: the fork's own are numbered
    /// after them, so a number in its history never names one of the fork's. Absent is none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub commands: u32,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if"
)]
fn is_zero(count: &u32) -> bool {
    *count == 0
}

/// The point in another thread that a fork continues from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForkPoint {
    pub thread: ThreadId,
    pub seq: Seq,
}

/// Where a fork works, as to the files of the thread it was forked from, which are never
/// rewound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForkFiles {
    /// In the same folder, with its files as they are now.
    Same,
    /// In another folder, such as a worktree or a scratch folder of its own, without what the
    /// other thread changed in its own.
    Other,
}

impl Payload for ThreadForked {
    const TYPE: &'static str = TYPE;
    const VERSION: u32 = 1;
}

pub(crate) fn decode(version: u32, data: &RawValue) -> Option<serde_json::Result<ThreadForked>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}
