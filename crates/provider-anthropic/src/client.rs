//! The HTTP client: posts a streaming Messages request and feeds the response through the
//! event-stream decoder and the assembler.

use std::time::Duration;

use glave_ports::auth::{Auth, KeyHeader, Outgoing, redact_credentials};
use glave_ports::{Completion, Delta, ModelProvider, ModelRequest, ProviderError};
use reqwest::header::{CONTENT_TYPE, HeaderName, HeaderValue};

use crate::api::Api;
use crate::assemble::Assembler;
use crate::error::message_of;
use crate::wire::messages_request;
use glave_ports::SseDecoder;

/// Anthropic's API.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// The largest answer [`Anthropic::get`] reads.
const MOST_LISTED: usize = 8 << 20;

/// The default idle timeout, the same as the OpenAI-compatible provider's.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// Where and how to reach the Messages API.
#[derive(Clone)]
pub struct Config {
    /// The API base, such as `https://api.anthropic.com` (a trailing `/v1` is accepted too),
    /// or, for Vertex AI, a project's location.
    pub base_url: String,
    /// Which Messages API the server speaks: Anthropic's own, or Vertex AI's.
    pub api: Api,
    /// How each request proves who is calling: a key, in `x-api-key` for Anthropic's own
    /// API, or credentials that change. Never logged.
    pub auth: Auth,
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
            api: Api::Messages,
            auth: Auth::bearer(api_key).in_header(KeyHeader::XApiKey),
            provider: "anthropic".to_owned(),
            idle_timeout: IDLE_TIMEOUT,
        }
    }
}

/// A model provider for Anthropic's Messages API.
pub struct Anthropic {
    http: reqwest::Client,
    base_url: String,
    api: Api,
    auth: Auth,
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
        if let Some(key) = config.auth.fixed_key() {
            header_pair("x-api-key", key.expose())?;
        }
        Ok(Self {
            http,
            base_url: config.base_url,
            api: config.api,
            auth: config.auth,
            provider: config.provider,
            idle_timeout: config.idle_timeout,
        })
    }
}

impl ModelProvider for Anthropic {
    async fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<Completion, ProviderError> {
        let body = serde_json::to_vec(&messages_request(request, &self.provider, self.api))
            .map_err(|e| ProviderError::Protocol(format!("could not encode the request: {e}")))?;
        let url = self.api.url(&self.base_url, &request.model)?;
        let version = self.api.version_header();
        let mut sent = vec![("content-type", "application/json")];
        sent.extend(version.map(|version| ("anthropic-version", version)));
        let outgoing = Outgoing {
            method: "POST",
            url: &url,
            headers: &sent,
            body: &body,
        };
        let mut call = self
            .http
            .post(&url)
            .header(CONTENT_TYPE, "application/json");
        if let Some(version) = version {
            call = call.header("anthropic-version", version);
        }
        let sent = self.auth.headers(&outgoing).await?;
        for header in &sent {
            let (name, value) = header_pair(&header.name, header.value.expose())?;
            call = call.header(name, value);
        }
        let mut response = call.body(body).send().await.map_err(|e| self.failed(e))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::Http {
                status: status.as_u16(),
                body: message_of(&redact_credentials(&body, &sent)),
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
    /// A `GET` of `path` under Anthropic's own API (`<base>/v1/<path>`), asked with the
    /// endpoint's credentials, its JSON answer: what the API lists, such as its models
    /// (`models`), so a key can be checked when a person adds it. Vertex AI lists nothing
    /// here. An answer over 8 MiB is refused.
    pub async fn get(&self, path: &str) -> Result<serde_json::Value, ProviderError> {
        let Some(version) = self.api.version_header() else {
            return Err(ProviderError::Protocol(
                "Vertex AI's Messages API lists nothing here".into(),
            ));
        };
        let base = self.base_url.trim_end_matches('/');
        let base = base.strip_suffix("/v1").unwrap_or(base);
        let url = format!("{base}/v1/{}", path.trim_start_matches('/'));
        let sent = [("anthropic-version", version)];
        let outgoing = Outgoing {
            method: "GET",
            url: &url,
            headers: &sent,
            body: &[],
        };
        let mut call = self.http.get(&url).header("anthropic-version", version);
        let sent = self.auth.headers(&outgoing).await?;
        for header in &sent {
            let (name, value) = header_pair(&header.name, header.value.expose())?;
            call = call.header(name, value);
        }
        let mut response = call.send().await.map_err(|e| self.failed(e))?;
        let status = response.status();
        let mut body = Vec::new();
        while let Some(bytes) = response.chunk().await.map_err(|e| self.failed(e))? {
            if body.len().saturating_add(bytes.len()) > MOST_LISTED {
                return Err(ProviderError::Protocol(format!(
                    "{url} answered with more than {MOST_LISTED} bytes"
                )));
            }
            body.extend_from_slice(&bytes);
        }
        if !status.is_success() {
            return Err(ProviderError::Http {
                status: status.as_u16(),
                body: message_of(&redact_credentials(&String::from_utf8_lossy(&body), &sent)),
            });
        }
        serde_json::from_slice(&body)
            .map_err(|e| ProviderError::Protocol(format!("{url} did not answer with JSON: {e}")))
    }

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

/// A header that proves who is calling, its value marked sensitive so it is never shown. An
/// invalid one is refused without showing it.
fn header_pair(name: &str, value: &str) -> Result<(HeaderName, HeaderValue), ProviderError> {
    let name = HeaderName::from_bytes(name.as_bytes())
        .map_err(|_| ProviderError::Credentials(format!("{name:?} is not a header name")))?;
    let mut value = HeaderValue::from_str(value).map_err(|_| {
        ProviderError::Credentials(format!("the value for {name} is not a valid header value"))
    })?;
    value.set_sensitive(true);
    Ok((name, value))
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
    fn a_key_that_cannot_be_a_header_is_refused_without_showing_it() {
        let mut config = Config::new(Some("sk-ant-\nsecret".into()));
        config.idle_timeout = Duration::from_secs(1);
        let Err(error) = Anthropic::new(config) else {
            panic!("a newline in a key must be refused");
        };
        assert!(!error.to_string().contains("secret"));
    }
}
