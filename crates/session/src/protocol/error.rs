//! A request that failed, as JSON-RPC carries it.

use glave_ports::Failure;
use serde::{Deserialize, Serialize};

use super::code;

/// A request that failed, as JSON-RPC carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ApiError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}

/// Why a model call failed, for a client to say what to fix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ErrorData {
    pub failure: Failure,
}

impl ApiError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// For a client that follows only turns it starts: its message was queued to a turn
    /// already running, which it cannot follow.
    pub fn queued_elsewhere() -> Self {
        Self::new(
            code::TURN_RUNNING,
            "a turn was already running in this thread: your message was queued to it, and the \
             model reads it at its next step",
        )
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}
