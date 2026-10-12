//! A glave model provider for any API that speaks OpenAI's chat completions protocol:
//! OpenAI, OpenRouter, Ollama, llama.cpp, vLLM and LM Studio.
//!
//! It translates glave's neutral request into the wire format, streams the response, forwards
//! live text and reasoning, and returns one finished answer with tool calls merged, usage
//! normalized to inclusive totals, stop reasons mapped, and any reported cost kept exactly.

mod assemble;
mod client;
#[cfg(feature = "fake-server")]
pub mod fake;
mod wire;

pub use client::{Config, IDLE_TIMEOUT, OpenAiCompatible};
