//! glave's MCP client: each Model Context Protocol server becomes a tool source, through the
//! official rmcp SDK. Its tools reach the model as `mcp__<server>__<tool>`; every call still goes
//! through the harness, which asks the person by what the tool says it does. A remote server
//! that wants the person signed in gets them signed in as MCP's authorization says
//! (`signin.rs`), and their token, asked for on every request (`signed.rs`).

mod call;
mod client;
mod connection;
mod error;
mod names;
mod output;
mod probe;
mod prompts;
mod reason;
mod resources;
mod server;
mod signed;
mod signin;

pub use error::McpError;
pub use names::{catalog, exposed};
pub use output::output as tool_output;
pub use resources::Resources;
pub use server::{Limits, McpServer};
pub use signin::{Granted, SignIn, SignInError, address, offers_sign_in, renew};
