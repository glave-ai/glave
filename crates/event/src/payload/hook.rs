//! `hook.ran`: a hook, a command the person or a trusted project configured, ran at one point
//! of a turn, and what came of it.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::id::CallId;

pub(crate) const TYPE: &str = "hook.ran";

/// A hook ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookRan {
    /// The point of the turn it ran at.
    pub point: HookPoint,
    /// The command, as configured.
    pub command: String,
    /// The tool call it ran for: set for `pre_tool_use` and `post_tool_use`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<CallId>,
    pub result: HookResult,
    /// The command's exit code, when it exited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// For `blocked`, the reason, as the model or the person was shown it; for `failed`, why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Where in a turn a hook runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookPoint {
    /// Before a tool call runs, and before anyone is asked about it.
    PreToolUse,
    /// After a tool call ran, before the model sees its result.
    PostToolUse,
    /// When the person sends a prompt, before the model sees it.
    UserPromptSubmit,
    /// When a turn ends.
    Stop,
}

/// What came of a hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookResult {
    /// It let things go on.
    Passed,
    /// It stopped the tool call or the prompt, or (after a tool) sent the model a reason.
    Blocked,
    /// It could not run, ran out of time, or exited with a code that means neither: things
    /// went on as if it had passed.
    Failed,
}

impl Payload for HookRan {
    const TYPE: &'static str = TYPE;
    const VERSION: u32 = 1;
}

pub(crate) fn decode(version: u32, data: &RawValue) -> Option<serde_json::Result<HookRan>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}
