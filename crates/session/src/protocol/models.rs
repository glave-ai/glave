//! The models a server can run, as `model/list` shows them, so a client can offer a choice.

use serde::{Deserialize, Serialize};

/// One model a thread may be sent to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelInfo {
    /// What `thread/start` and `turn/start` take as `model`: the name the gateway knows it by.
    pub id: String,
    /// A name to show for it, when one is configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Whether threads that choose no model use this one. Exactly one model is the default.
    pub default: bool,
    /// Whether this is Hope, glave's own model, for clients to show as such.
    pub hope: bool,
}

impl ModelInfo {
    /// `id`, with no name, neither the default nor Hope.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
            default: false,
            hope: false,
        }
    }
}
