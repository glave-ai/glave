//! The HTTP client: posts a streaming chat completion and feeds the response through the
//! event-stream decoder and the assembler.

use std::time::Duration;

use glave_ports::auth::{Auth, ERROR_BODY_BYTES, Outgoing};
use glave_ports::host::is_this_computer;
use glave_ports::{Completion, Delta, ModelProvider, ModelRequest, ProviderError, spoken};
use reqwest::header::{CONTENT_TYPE, HeaderName, HeaderValue};

use crate::assemble::Assembler;
use crate::wire::chat_request;
use glave_ports::SseDecoder;

/// The largest answer [`OpenAiCompatible::get`] reads.
const MOST_LISTED: usize = 8 << 20;

/// Where and how to reach an OpenAI-compatible endpoint.
#[derive(Clone)]
pub struct Config {
    /// The API base, such as `https://openrouter.ai/api/v1` or `http://localhost:11434/v1`.
    pub base_url: String,
    /// How each request proves who is calling: a key, usually as a bearer token, or
    /// credentials that change. Never logged.
    pub auth: Auth,
    /// A short name recorded with every response, such as `openrouter` or `ollama`.
    pub provider: String,
    /// How long the server may send nothing, before or during its answer, before the request
    /// fails as timed out.
    pub idle_timeout: Duration,
}

/// The default idle timeout, as Codex sets it: long enough for a local model to read a long
/// prompt before its first token.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// The longest a connection to the provider may take to open, its TLS handshake included.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

impl Config {
    /// The endpoint at `base_url`, with `api_key` as a bearer token when there is one, named
    /// after its host: `openrouter`, `openai`, `ollama`, `local` for other servers on this
    /// computer, otherwise the host itself. `None` when `base_url` is not an http(s) URL.
    pub fn for_url(base_url: impl Into<String>, api_key: Option<String>) -> Option<Self> {
        let base_url = base_url.into();
        let host = host_of(&base_url)?;
        let provider = match host.as_str() {
            "openrouter.ai" => "openrouter".to_owned(),
            "api.openai.com" => "openai".to_owned(),
            here if is_this_computer(here) && base_url.contains(":11434") => "ollama".to_owned(),
            here if is_this_computer(here) => "local".to_owned(),
            _ => host,
        };
        Some(Self {
            base_url,
            auth: Auth::bearer(api_key),
            provider,
            idle_timeout: IDLE_TIMEOUT,
        })
    }

    /// The server's host name, safe to show: never a user name or password from the URL.
    pub fn host(&self) -> Option<String> {
        host_of(&self.base_url)
    }

    /// Whether the model runs on this computer, where a missing cost means $0.
    pub fn is_local(&self) -> bool {
        host_of(&self.base_url).is_some_and(|host| is_this_computer(&host))
    }
}

/// The host part of an http(s) URL, without any user name or password in it.
fn host_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split('/').next()?.rsplit('@').next()?;
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|b| b.is_ascii_digit()) => host,
        _ => authority,
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// A model provider for any API that speaks OpenAI's chat completions protocol.
pub struct OpenAiCompatible {
    http: reqwest::Client,
    /// The API base, without a trailing slash.
    base: String,
    endpoint: String,
    auth: Auth,
    provider: String,
    idle_timeout: Duration,
}

impl OpenAiCompatible {
    pub fn new(config: Config) -> Result<Self, ProviderError> {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            // Each read may wait this long: for the first token, and between the rest.
            .read_timeout(config.idle_timeout)
            .build()
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let base = config.base_url.trim_end_matches('/').to_owned();
        Ok(Self {
            http,
            endpoint: format!("{base}/chat/completions"),
            base,
            auth: config.auth,
            provider: config.provider,
            idle_timeout: config.idle_timeout,
        })
    }
}

impl ModelProvider for OpenAiCompatible {
    async fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<Completion, ProviderError> {
        let body = serde_json::to_vec(&chat_request(request, &self.provider))
            .map_err(|e| ProviderError::Protocol(format!("could not encode the request: {e}")))?;
        let outgoing = Outgoing {
            method: "POST",
            url: &self.endpoint,
            headers: &[("content-type", "application/json")],
            body: &body,
        };
        let mut call = self
            .http
            .post(&self.endpoint)
            .header(CONTENT_TYPE, "application/json");
        let sent = self.auth.headers(&outgoing).await?;
        for header in &sent {
            let (name, value) = header_pair(&header.name, header.value.expose())?;
            call = call.header(name, value);
        }
        let response = call.body(body).send().await.map_err(|e| self.failed(e))?;
        let status = response.status();
        if !status.is_success() {
            let body = error_body(response).await;
            return Err(ProviderError::from_response(status.as_u16(), &body, &sent));
        }
        let mut assembler = Assembler::new(&self.provider, &request.model);
        match self.stream(response, &mut assembler, on_delta).await {
            Ok(()) => assembler.finish(),
            Err(error) if assembler.has_output() => {
                Err(ProviderError::AfterOutput(Box::new(error)))
            }
            Err(error) => Err(error),
        }
    }
}

impl OpenAiCompatible {
    /// Reads the response's events into `assembler`, until `[DONE]` or the end of the body.
    async fn stream(
        &self,
        mut response: reqwest::Response,
        assembler: &mut Assembler,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<(), ProviderError> {
        let mut decoder = SseDecoder::default();
        while let Some(bytes) = response.chunk().await.map_err(|e| self.failed(e))? {
            for data in decoder.push(&bytes)? {
                if data == "[DONE]" {
                    return Ok(());
                }
                assembler.apply(&data, on_delta)?;
            }
        }
        if let Some(data) = decoder.finish().filter(|data| data != "[DONE]") {
            assembler.apply(&data, on_delta)?;
        }
        Ok(())
    }

    /// A `GET` of `path` under the API, asked with the endpoint's credentials, its JSON
    /// answer: what the API lists beside chat completions, such as its models (`models`), so
    /// a key can be checked when a person adds it. An answer over 8 MiB is refused.
    pub async fn get(&self, path: &str) -> Result<serde_json::Value, ProviderError> {
        let url = format!("{}/{}", self.base, path.trim_start_matches('/'));
        let outgoing = Outgoing {
            method: "GET",
            url: &url,
            headers: &[],
            body: &[],
        };
        let mut call = self.http.get(&url);
        let sent = self.auth.headers(&outgoing).await?;
        for header in &sent {
            let (name, value) = header_pair(&header.name, header.value.expose())?;
            call = call.header(name, value);
        }
        let mut response = call.send().await.map_err(|e| self.failed(e))?;
        let status = response.status();
        if !status.is_success() {
            let body = error_body(response).await;
            return Err(ProviderError::from_response(status.as_u16(), &body, &sent));
        }
        let mut body = Vec::new();
        while let Some(bytes) = response.chunk().await.map_err(|e| self.failed(e))? {
            if body.len().saturating_add(bytes.len()) > MOST_LISTED {
                return Err(ProviderError::Protocol(format!(
                    "{url} answered with more than {MOST_LISTED} bytes"
                )));
            }
            body.extend_from_slice(&bytes);
        }
        serde_json::from_slice(&body)
            .map_err(|e| ProviderError::Protocol(format!("{url} did not answer with JSON: {e}")))
    }

    /// What a failed request means: no connection in time, a silence past the idle timeout, or a
    /// broken connection. A connection that never opened sent nothing, so it is the provider
    /// unreached, not silent.
    fn failed(&self, error: reqwest::Error) -> ProviderError {
        if error.is_connect() && error.is_timeout() {
            let host = error
                .url()
                .and_then(reqwest::Url::host_str)
                .unwrap_or("the provider");
            ProviderError::Transport(format!(
                "no connection to {host} within {}",
                spoken(CONNECT_TIMEOUT)
            ))
        } else if error.is_timeout() {
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

/// The start of an error response's body: as much as [`ProviderError::from_response`] reads,
/// and no more.
async fn error_body(mut response: reqwest::Response) -> Vec<u8> {
    let mut body = Vec::new();
    while body.len() < ERROR_BODY_BYTES
        && let Ok(Some(bytes)) = response.chunk().await
    {
        body.extend_from_slice(&bytes);
    }
    body
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
    use glave_ports::testing::any_secret;

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
    fn a_password_in_the_url_never_shows() {
        let url = format!("https://me:{}@models.example.com/v1", any_secret());
        let config = Config::for_url(&url, None).unwrap();
        assert_eq!(config.host().as_deref(), Some("models.example.com"));
        assert_eq!(config.provider, "models.example.com");
    }

    #[test]
    fn a_key_that_cannot_be_a_header_is_refused_without_showing_it() {
        let key = any_secret();
        let error = header_pair("api-key", &format!("sk-\n{key}")).unwrap_err();
        assert!(!error.to_string().contains(&key), "{error}");
        assert!(error.to_string().contains("api-key"), "{error}");
        let (_, value) = header_pair("authorization", &format!("Bearer {key}")).unwrap();
        assert!(value.is_sensitive());
        let config = Config::for_url("https://api.openai.com/v1", Some(key.clone())).unwrap();
        assert!(!format!("{:?}", config.auth).contains(&key));
    }

    #[test]
    fn only_this_computer_is_local() {
        let local = |url| Config::for_url(url, None).is_some_and(|config| config.is_local());
        assert!(local("http://localhost:11434/v1"));
        assert!(local("http://[::1]:8080/v1"));
        assert!(!local("https://openrouter.ai/api/v1"));
    }
}
