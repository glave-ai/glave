//! A request that failed, as JSON-RPC carries it.

use glave_ports::Failure;
use serde::{Deserialize, Serialize};

use super::{Told, code};
use crate::words::Sentence;

/// A request that failed, as JSON-RPC carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ApiError {
    pub code: i64,
    /// What a person is told, in their language where it is the server's own sentence.
    pub message: Told,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}

/// Why a model call failed, for a client to say what to fix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ErrorData {
    pub failure: Failure,
}

impl ApiError {
    pub fn new(code: i64, message: impl Into<Told>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// For a client that follows only turns it starts: its message was queued to a turn
    /// already running, which it cannot follow.
    pub fn queued_elsewhere() -> Self {
        Self::new(code::TURN_RUNNING, Sentence::QueuedElsewhere)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}
