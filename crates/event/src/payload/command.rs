//! `command.started` and `command.ended`: a command kept running in the background
//! (`specs/computers.md`, "Background commands"), from the call that started it there, or moved
//! it there at its timeout, to its end as glave learned it. glave's own records, which call ids
//! do not count; resuming a thread reads them to know which of its commands may still run.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::codec::{Payload, parse};
use crate::id::CallId;
use crate::payload::principal::Principal;

pub(crate) const STARTED: &str = "command.started";
pub(crate) const ENDED: &str = "command.ended";

/// A command began running in the background.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandStarted {
    /// Its id, as its computer named it.
    pub command: Uuid,
    /// The workspace it runs in, as its computer named it.
    pub workspace: Uuid,
    /// The tool call that started it.
    pub call_id: CallId,
    pub script: String,
    /// It started in the foreground, and moved to the background at its timeout.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub moved: bool,
}

/// A command in the background ended, as glave learned it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandEnded {
    pub command: Uuid,
    pub ending: CommandEnding,
    /// How long it ran, in milliseconds, when its computer said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub millis: Option<u64>,
}

/// How a command in the background ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandEnding {
    /// It exited by itself.
    Exited { code: i32 },
    /// A signal it did not ask for stopped it.
    Signalled,
    /// Someone stopped it: `by` them, absent when it was the thread's agent, with
    /// `command_output`.
    Stopped {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<Principal>,
    },
    /// glave stopped it: its thread was archived or deleted, or the subagent that started it
    /// finished its work.
    Released,
    /// Its computer stopped it after an hour in which nothing read its output.
    Idle,
    /// Its computer stopped it at its time limit, such as a worker's run time.
    TimedOut,
    /// It was no longer on its computer: it ended, or its computer or glave stopped, while
    /// glave was not watching.
    Gone,
}

impl Payload for CommandStarted {
    const TYPE: &'static str = STARTED;
    const VERSION: u32 = 1;
}

impl Payload for CommandEnded {
    const TYPE: &'static str = ENDED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_started(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<CommandStarted>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_ended(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<CommandEnded>> {
    (version == 1).then(|| parse(data))
}
