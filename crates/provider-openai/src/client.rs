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

impl Config {
    /// The endpoint at `base_url`, named after its host: `openrouter`, `openai`, `ollama`,
    /// `local` for other servers on this computer, otherwise the host itself. `None` when
    /// `base_url` is not an http(s) URL.
    pub fn for_url(base_url: impl Into<String>, api_key: Option<String>) -> Option<Self> {
        let base_url = base_url.into();
        let host = host_of(&base_url)?;
        let provider = match host.as_str() {
            "openrouter.ai" => "openrouter".to_owned(),
            "api.openai.com" => "openai".to_owned(),
            "localhost" | "127.0.0.1" | "[::1]" if base_url.contains(":11434") => {
                "ollama".to_owned()
            }
            "localhost" | "127.0.0.1" | "[::1]" => "local".to_owned(),
            _ => host,
        };
        Some(Self {
            base_url,
            api_key,
            provider,
        })
    }

    /// Whether the model runs on this computer, where a missing cost means $0.
    pub fn is_local(&self) -> bool {
        host_of(&self.base_url)
            .is_some_and(|host| matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]"))
    }
}

/// The host part of an http(s) URL.
fn host_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split('/').next()?;
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|b| b.is_ascii_digit()) => host,
        _ => authority,
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn name(url: &str) -> Option<String> {
        Config::for_url(url, None).map(|config| config.provider)
    }

    #[test]
    fn endpoints_are_named_after_their_host() {
        assert_eq!(
            name("https://openrouter.ai/api/v1").as_deref(),
            Some("openrouter")
        );
        assert_eq!(name("https://api.openai.com/v1").as_deref(), Some("openai"));
        assert_eq!(name("http://localhost:11434/v1").as_deref(), Some("ollama"));
        assert_eq!(name("http://127.0.0.1:8080/v1").as_deref(), Some("local"));
        assert_eq!(
            name("https://Models.Example.com:443/v1").as_deref(),
            Some("models.example.com")
        );
        assert_eq!(name("ftp://x"), None);
        assert_eq!(name("https://"), None);
    }

    #[test]
    fn only_this_computer_is_local() {
        let local = |url| Config::for_url(url, None).is_some_and(|config| config.is_local());
        assert!(local("http://localhost:11434/v1"));
        assert!(local("http://[::1]:8080/v1"));
        assert!(!local("https://openrouter.ai/api/v1"));
    }
}
