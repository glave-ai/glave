//! Who a process glave started belongs to, as its ledger entry records it.

use serde::{Deserialize, Serialize};

/// What a process was started for: it goes when its owner does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Owner {
    /// glave itself: the person's own MCP servers, and anything glave runs for its own work.
    Server,
    /// A project, by its folder: the MCP servers its `.mcp.json` brings.
    Project(String),
    /// A workspace of a computer on this machine, by its id: a thread's commands.
    Workspace(String),
}
