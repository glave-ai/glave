//! A glave model provider for Anthropic's native Messages API, spoken over plain HTTP and
//! server-sent events.
//!
//! It translates glave's neutral request into the Messages format, streams the response,
//! forwards live text and readable thinking, and returns one finished answer: tool calls
//! assembled, thinking kept with its signature so it can be sent back, usage normalized to
//! inclusive totals, and stop reasons mapped. It speaks Anthropic's own API (and Amazon
//! Bedrock's `/anthropic` route, which copies it) or Vertex AI's rawPredict, which takes the
//! same request with the model in the URL and the version in the body.

mod api;
mod assemble;
mod client;
mod error;
mod events;
#[cfg(feature = "fake-server")]
pub mod fake;
mod sse;
mod wire;

pub use api::Api;
pub use client::{Anthropic, Config, DEFAULT_BASE_URL, IDLE_TIMEOUT};
