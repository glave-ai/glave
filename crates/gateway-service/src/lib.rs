//! glave's gateway as a service (`specs/gateway.md`, "Surfaces"): the same gateway glave runs
//! in-process, behind HTTP for everyone in an organization.
//!
//! - **Model calls** come in as OpenAI Chat Completions (`/v1/chat/completions`) or Anthropic
//!   Messages (`/v1/messages`), streamed or whole, and go out by the gateway's routes, with keys
//!   only the gateway holds.
//! - **Every call is signed in**: an OIDC access token, checked by the session API's verifier.
//!   The caller's own policy decides which models and tools they may use, and their own budget
//!   what they may spend.
//! - **Metering**: every call is recorded, content-free, in the caller's stream in the
//!   gateway's scope of the event log: `model.responded` for a model call,
//!   `gateway.tool.called` for a tool call.
//! - **The tool catalog** (`/mcp`): the organization's MCP servers, each with credentials only
//!   the gateway holds, served as one MCP server whose tools are named `<server>__<tool>`.

mod anthropic;
mod catalog;
mod errors;
mod fields;
mod http;
mod meter;
mod models;
mod openai;
mod service;
mod stream;

pub use http::serve;
pub use meter::stream_id;
pub use service::{Notices, Policy, Service, Wiring};
