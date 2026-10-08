//! glave's MCP client: each Model Context Protocol server becomes a tool source, through the
//! official rmcp SDK. Its tools reach the model as `mcp__<server>__<tool>`; every call still goes
//! through the harness, which asks the person by what the tool says it does.

mod config;
mod connection;
mod error;
mod names;
mod output;
mod reason;
mod server;

pub use config::{ServerConfig, parse};
pub use error::McpError;
pub use names::exposed;
pub use server::{Limits, McpServer};
