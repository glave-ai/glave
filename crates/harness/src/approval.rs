//! Asking the person before anything changes: writes, edits and commands.

use std::future::{Future, ready};

use glave_event::Arguments;

use crate::tools::Tool;

/// What the harness wants to do, in words the person can judge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub tool: Tool,
    pub summary: String,
}

/// The person's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    AllowOnce,
    /// Allow this tool for the rest of the thread.
    AllowForThread,
    Deny,
}

/// Decides whether an action may go ahead. The CLI asks in the terminal; tests and `--yes`
/// allow everything.
pub trait Approver: Send + Sync {
    fn decide(&self, request: &ApprovalRequest) -> impl Future<Output = Decision> + Send;
}

/// Allows every action without asking.
#[derive(Debug, Clone, Copy, Default)]
pub struct AllowAll;

impl Approver for AllowAll {
    fn decide(&self, _request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
        ready(Decision::AllowOnce)
    }
}

/// A one-line description of a tool call, such as `edit src/main.rs` or `bash: cargo test`.
pub(crate) fn summarize(tool: Tool, arguments: &Arguments) -> String {
    let field = |name: &str| match arguments {
        Arguments::Json(map) => map.get(name).and_then(|v| v.as_str()).map(str::to_owned),
        Arguments::Malformed(_) => None,
    };
    let path = field("path").unwrap_or_else(|| "?".into());
    match tool {
        Tool::Read => format!("read {path}"),
        Tool::Write => {
            let lines = field("content").map_or(0, |content| content.lines().count());
            format!("write {path} ({lines} lines)")
        }
        Tool::Edit => format!("edit {path}"),
        Tool::Bash => format!("bash: {}", field("command").unwrap_or_else(|| "?".into())),
    }
}
