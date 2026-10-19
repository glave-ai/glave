//! The settings config and policy files fill and the domain crates read: hooks, the judge, a
//! project's protections, what the web tools may reach, and MCP servers as `.mcp.json` names
//! them. They live here, below both, so reading a file never depends on what runs it.

mod hooks;
mod judge;
pub mod mcp;
mod protections;
mod web;

pub use hooks::{HOOK_TIMEOUT_SECS, Hook, HookError, Hooks, point_name};
pub use judge::Judge;
pub use protections::Protections;
pub use web::{BadDomain, Domain, HostRule, WebPolicy};
