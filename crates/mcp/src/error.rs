//! What can go wrong before a server's tools are on offer.

use thiserror::Error;

/// Why a server could not be started.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum McpError {
    #[error("MCP server {server:?} did not start: {message}")]
    Start { server: String, message: String },
}

impl McpError {
    pub(crate) fn start(server: &str, message: impl Into<String>) -> Self {
        Self::Start {
            server: server.to_owned(),
            message: message.into(),
        }
    }
}
