//! Whether glave keeps its computer awake while it works (`specs/one-server.md`, owner
//! decision 10), as Settings shows it.

use serde::{Deserialize, Serialize};

/// Whether this server keeps its computer from sleeping on its own while a turn, task or
/// schedule runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AwakeInfo {
    /// Never: a server, which does not sleep. There is nothing to choose.
    Never,
    /// On a person's own computer, as they choose: kept awake while glave works when `on`, the
    /// screen still free to sleep and closing the lid still sleeping it.
    Chosen { on: bool },
    /// On a person's own computer that glave has nothing to keep awake with: why, in a
    /// sentence.
    Cannot { reason: String },
}
