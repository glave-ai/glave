//! The settings config and policy files fill and the domain crates read: hooks, the judge, a
//! project's protections, what the web tools may reach, MCP servers as `.mcp.json` names
//! them, and prices as a price list writes them. They live here, below both, so reading a file
//! never depends on what runs it.

mod brings;
mod hooks;
mod judge;
pub mod mcp;
mod prices;
mod protections;
mod unquoted;
mod web;

pub use brings::Brings;
pub use hooks::{HOOK_TIMEOUT_SECS, Hook, HookError, Hooks, point_name};
pub use judge::Judge;
pub use prices::{LongPrompt, PriceEntry, PriceError};
pub use protections::Protections;
pub use unquoted::unquoted;
pub use web::{HostRule, WebPolicy};
