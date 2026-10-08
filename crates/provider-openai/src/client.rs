//! The HTTP client: posts a streaming chat completion and feeds the response through the
//! event-stream decoder and the assembler.

use std::time::Duration;

use glave_ports::{Completion, Delta, ModelProvider, ModelRequest, ProviderError};
use reqwest::header::CONTENT_TYPE;

use crate::assemble::Assembler;
use crate::sse::SseDecoder;
use crate::wire::{caches_automatically, chat_request};

/// How much of an error response body to keep in the error.
const ERROR_BODY_CHARS: usize = 2_000;

/// Where and how to reach an OpenAI-compatible endpoint.
#[derive(Clone)]
pub struct Config {
    /// The API base, such as `https://openrouter.ai/api/v1` or `http://localhost:11434/v1`.
    pub base_url: String,
    /// Sent as a bearer token when set. Never logged.
    pub api_key: Option<String>,
    /// A short name recorded with every response, such as `openrouter` or `ollama`.
    pub provider: String,
}

/// A model provider for any API that speaks OpenAI's chat completions protocol.
pub struct OpenAiCompatible {
    http: reqwest::Client,
    endpoint: String,
    api_key: Option<String>,
    provider: String,
}

impl OpenAiCompatible {
    pub fn new(config: Config) -> Result<Self, ProviderError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        Ok(Self {
            http,
            endpoint: format!("{}/chat/completions", config.base_url.trim_end_matches('/')),
            api_key: config.api_key,
            provider: config.provider,
        })
    }
}

impl ModelProvider for OpenAiCompatible {
    async fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<Completion, ProviderError> {
        let cache = caches_automatically(&self.provider, &request.model);
        let body = serde_json::to_vec(&chat_request(request, cache))
            .map_err(|e| ProviderError::Protocol(format!("could not encode the request: {e}")))?;
        let mut call = self
            .http
            .post(&self.endpoint)
            .header(CONTENT_TYPE, "application/json")
            .body(body);
        if let Some(key) = &self.api_key {
            call = call.bearer_auth(key);
        }
        let mut response = call.send().await.map_err(transport)?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::Http {
                status: status.as_u16(),
                body: body.chars().take(ERROR_BODY_CHARS).collect(),
            });
        }
        let mut decoder = SseDecoder::default();
        let mut assembler = Assembler::new(&self.provider, &request.model);
        'stream: while let Some(bytes) = response.chunk().await.map_err(transport)? {
            for data in decoder.push(&bytes)? {
                if data == "[DONE]" {
                    break 'stream;
                }
                assembler.apply(&data, on_delta)?;
            }
        }
        if let Some(data) = decoder.finish().filter(|data| data != "[DONE]") {
            assembler.apply(&data, on_delta)?;
        }
        assembler.finish()
    }
}

fn transport(error: reqwest::Error) -> ProviderError {
    ProviderError::Transport(error.without_url().to_string())
}
