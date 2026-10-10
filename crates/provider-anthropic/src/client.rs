//! The HTTP client: posts a streaming Messages request and feeds the response through the
//! event-stream decoder and the assembler.

use std::time::Duration;

use glave_ports::{Completion, Delta, ModelProvider, ModelRequest, ProviderError};
use reqwest::header::{CONTENT_TYPE, HeaderValue};

use crate::assemble::Assembler;
use crate::error::message_of;
use crate::sse::SseDecoder;
use crate::wire::messages_request;

/// Anthropic's API.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// The API version this provider speaks.
const VERSION: &str = "2023-06-01";

/// The default idle timeout, the same as the OpenAI-compatible provider's.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// Where and how to reach the Messages API.
#[derive(Clone)]
pub struct Config {
    /// The API base, such as `https://api.anthropic.com`; a trailing `/v1` is accepted too.
    pub base_url: String,
    /// Sent as `x-api-key` when set. Never logged.
    pub api_key: Option<String>,
    /// A short name recorded with every response and with the thinking it returns, such as
    /// `anthropic`.
    pub provider: String,
    /// How long the server may send nothing, before or during its answer, before the request
    /// fails as timed out.
    pub idle_timeout: Duration,
}

impl Config {
    /// Anthropic's own API, with this key.
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            api_key,
            provider: "anthropic".to_owned(),
            idle_timeout: IDLE_TIMEOUT,
        }
    }
}

/// A model provider for Anthropic's Messages API.
pub struct Anthropic {
    http: reqwest::Client,
    endpoint: String,
    api_key: Option<HeaderValue>,
    provider: String,
    idle_timeout: Duration,
}

impl Anthropic {
    pub fn new(config: Config) -> Result<Self, ProviderError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // Each read may wait this long: for the first token, and between the rest.
            .read_timeout(config.idle_timeout)
            .build()
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let api_key = match config.api_key {
            Some(key) => {
                let mut value = HeaderValue::from_str(&key).map_err(|_| {
                    ProviderError::Transport("the API key is not a valid header value".into())
                })?;
                value.set_sensitive(true);
                Some(value)
            }
            None => None,
        };
        Ok(Self {
            http,
            endpoint: endpoint(&config.base_url),
            api_key,
            provider: config.provider,
            idle_timeout: config.idle_timeout,
        })
    }
}

/// The Messages endpoint under `base_url`, whether or not the base ends in `/v1`.
fn endpoint(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    format!("{base}/v1/messages")
}

impl ModelProvider for Anthropic {
    async fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<Completion, ProviderError> {
        let body = serde_json::to_vec(&messages_request(request, &self.provider))
            .map_err(|e| ProviderError::Protocol(format!("could not encode the request: {e}")))?;
        let mut call = self
            .http
            .post(&self.endpoint)
            .header(CONTENT_TYPE, "application/json")
            .header("anthropic-version", VERSION)
            .body(body);
        if let Some(key) = &self.api_key {
            call = call.header("x-api-key", key.clone());
        }
        let mut response = call.send().await.map_err(|e| self.failed(e))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::Http {
                status: status.as_u16(),
                body: message_of(&body),
            });
        }
        let mut decoder = SseDecoder::default();
        let mut assembler = Assembler::new(&self.provider, &request.model);
        while let Some(bytes) = response.chunk().await.map_err(|e| self.failed(e))? {
            for data in decoder.push(&bytes)? {
                assembler.apply(&data, on_delta)?;
            }
            if assembler.stopped() {
                break;
            }
        }
        if let Some(data) = decoder.finish() {
            assembler.apply(&data, on_delta)?;
        }
        assembler.finish()
    }
}

impl Anthropic {
    /// What a failed request means: a silence past the idle timeout, or a broken connection.
    fn failed(&self, error: reqwest::Error) -> ProviderError {
        if error.is_timeout() {
            ProviderError::Timeout {
                seconds: self.idle_timeout.as_secs(),
            }
        } else {
            ProviderError::Transport(cause_of(error))
        }
    }
}

/// What went wrong underneath a failed request, such as `Connection refused (os error 61)`:
/// the innermost cause, which says more than "error sending request".
fn cause_of(error: reqwest::Error) -> String {
    let mut cause: &dyn std::error::Error = &error;
    while let Some(inner) = cause.source() {
        cause = inner;
    }
    let cause = cause.to_string();
    if error.is_connect() && !cause.is_empty() {
        cause
    } else {
        error.without_url().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_endpoint_is_found_with_or_without_v1() {
        for base in [
            "https://api.anthropic.com",
            "https://api.anthropic.com/",
            "https://api.anthropic.com/v1",
            "https://api.anthropic.com/v1/",
        ] {
            assert_eq!(endpoint(base), "https://api.anthropic.com/v1/messages");
        }
    }

    #[test]
    fn a_key_that_cannot_be_a_header_is_refused_without_showing_it() {
        let mut config = Config::new(Some("sk-ant-\nsecret".into()));
        config.idle_timeout = Duration::from_secs(1);
        let Err(error) = Anthropic::new(config) else {
            panic!("a newline in a key must be refused");
        };
        assert!(!error.to_string().contains("secret"));
    }
}
