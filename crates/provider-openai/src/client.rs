//! The HTTP client: posts a streaming chat completion and feeds the response through the
//! event-stream decoder and the assembler.

use std::time::Duration;

use glave_ports::auth::{Auth, Outgoing};
use glave_ports::{Completion, Delta, ModelProvider, ModelRequest, ProviderError};
use reqwest::header::{CONTENT_TYPE, HeaderName, HeaderValue};

use crate::assemble::Assembler;
use crate::sse::SseDecoder;
use crate::wire::chat_request;

/// How much of an error message to keep.
const ERROR_CHARS: usize = 500;

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
            "localhost" | "127.0.0.1" | "[::1]" if base_url.contains(":11434") => {
                "ollama".to_owned()
            }
            "localhost" | "127.0.0.1" | "[::1]" => "local".to_owned(),
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
        host_of(&self.base_url)
            .is_some_and(|host| matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]"))
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
    endpoint: String,
    auth: Auth,
    provider: String,
    idle_timeout: Duration,
}

impl OpenAiCompatible {
    pub fn new(config: Config) -> Result<Self, ProviderError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // Each read may wait this long: for the first token, and between the rest.
            .read_timeout(config.idle_timeout)
            .build()
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        Ok(Self {
            http,
            endpoint: format!("{}/chat/completions", config.base_url.trim_end_matches('/')),
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
        for header in self.auth.headers(&outgoing).await? {
            let (name, value) = header_pair(&header.name, header.value.expose())?;
            call = call.header(name, value);
        }
        let mut response = call.body(body).send().await.map_err(|e| self.failed(e))?;
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
        'stream: while let Some(bytes) = response.chunk().await.map_err(|e| self.failed(e))? {
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

impl OpenAiCompatible {
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

/// The message in an error body: `error.message` as OpenAI, OpenRouter and Ollama send it,
/// or the body itself, on one line and cut short, rather than raw JSON.
fn message_of(body: &str) -> String {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let message = parsed.as_ref().and_then(|value| {
        [
            value.pointer("/error/message"),
            value.get("error"),
            value.get("message"),
            value.get("detail"),
        ]
        .into_iter()
        .flatten()
        .find_map(serde_json::Value::as_str)
    });
    let text = message.unwrap_or(body);
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    line.chars().take(ERROR_CHARS).collect()
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
    fn error_bodies_become_one_readable_line() {
        assert_eq!(
            message_of(
                r#"{"error":{"message":"nosuch/x is not a valid model ID","code":400},"user_id":"u_1"}"#
            ),
            "nosuch/x is not a valid model ID"
        );
        assert_eq!(
            message_of(r#"{"error":"model 'gemma9' not found"}"#),
            "model 'gemma9' not found"
        );
        assert_eq!(message_of(r#"{"detail":"Not Found"}"#), "Not Found");
        assert_eq!(
            message_of("<html>\n  <body>Bad Gateway</body>\n</html>"),
            "<html> <body>Bad Gateway</body> </html>"
        );
        assert_eq!(message_of(&"x".repeat(900)).len(), ERROR_CHARS);
    }

    #[test]
    fn a_password_in_the_url_never_shows() {
        let config = Config::for_url("https://me:hunter2@models.example.com/v1", None).unwrap();
        assert_eq!(config.host().as_deref(), Some("models.example.com"));
        assert_eq!(config.provider, "models.example.com");
    }

    #[test]
    fn a_key_that_cannot_be_a_header_is_refused_without_showing_it() {
        let error = header_pair("api-key", "sk-\nsecret").unwrap_err();
        assert!(!error.to_string().contains("secret"), "{error}");
        assert!(error.to_string().contains("api-key"), "{error}");
        let (_, value) = header_pair("authorization", "Bearer sk-1").unwrap();
        assert!(value.is_sensitive());
        let config = Config::for_url("https://api.openai.com/v1", Some("hunter2".into())).unwrap();
        assert!(!format!("{:?}", config.auth).contains("hunter2"));
    }

    #[test]
    fn only_this_computer_is_local() {
        let local = |url| Config::for_url(url, None).is_some_and(|config| config.is_local());
        assert!(local("http://localhost:11434/v1"));
        assert!(local("http://[::1]:8080/v1"));
        assert!(!local("https://openrouter.ai/api/v1"));
    }
}
