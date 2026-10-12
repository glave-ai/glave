//! glave's way into Google Vertex AI, for an organization whose models come through its Google
//! Cloud contract.
//!
//! Vertex serves Gemini through OpenAI's Chat Completions, so `glave-provider-openai` talks to
//! it, and Claude through Anthropic's Messages API, so `glave-provider-anthropic` does. This
//! crate says where (a project's location, global, multi-region or regional) and with what:
//! Application Default Credentials, as Google's own libraries find them, kept as a bearer token
//! until shortly before it expires. Keyless on GKE, that is the metadata server's token for the
//! pod's workload identity.

mod adc;
mod endpoint;
mod external;
mod jwt;
mod metadata;
mod token;

pub use adc::{Adc, SCOPE};
pub use endpoint::{EndpointError, Location, Vertex};
