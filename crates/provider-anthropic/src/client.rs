//! The HTTP client: posts a streaming Messages request and feeds the response through the
//! event-stream decoder and the assembler.

use std::time::Duration;

use glave_ports::auth::{
    Auth, CONNECT_TIMEOUT, KeyHeader, Outgoing, credential_header, not_json, request_failed,
    send_signed,
};
use glave_ports::{Completion, Delta, EndUser, ModelProvider, ModelRequest, ProviderError};
use reqwest::header::CONTENT_TYPE;

use crate::api::Api;
use crate::assemble::Assembler;
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
            .connect_timeout(CONNECT_TIMEOUT)
            // Each read may wait this long: for the first token, and between the rest.
            .read_timeout(config.idle_timeout)
            .build()
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        if let Some(key) = config.auth.fixed_key() {
            credential_header("x-api-key", key.expose())?;
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

impl Anthropic {
    /// Whether it calls Anthropic's own API, the one of those it speaks to that documents an
    /// end user's id: Bedrock's and Vertex AI's routes list no `metadata` among what they take.
    fn own_api(&self) -> bool {
        let base = self.base_url.trim_end_matches('/');
        base.strip_suffix("/v1").unwrap_or(base) == DEFAULT_BASE_URL
    }
}

impl ModelProvider for Anthropic {
    async fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<Completion, ProviderError> {
        let end_user = request.end_user.as_ref().filter(|_| self.own_api());
        let body = messages_request(request, &self.provider, self.api)
            .for_end_user(end_user.map(EndUser::as_str));
        let body = serde_json::to_vec(&body)
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
        let response = send_signed(call.body(body), &sent, self.idle_timeout).await?;
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

impl Anthropic {
    /// Reads the response's events into `assembler`, until `message_stop` or the end of the
    /// body.
    async fn stream(
        &self,
        mut response: reqwest::Response,
        assembler: &mut Assembler,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<(), ProviderError> {
        let mut decoder = SseDecoder::default();
        while let Some(bytes) = response.chunk().await.map_err(|e| self.failed(e))? {
            for data in decoder.push(&bytes)? {
                assembler.apply(&data, on_delta)?;
            }
            if assembler.stopped() {
                return Ok(());
            }
        }
        if let Some(data) = decoder.finish() {
            assembler.apply(&data, on_delta)?;
        }
        Ok(())
    }

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
        let call = self.http.get(&url).header("anthropic-version", version);
        let sent = self.auth.headers(&outgoing).await?;
        let mut response = send_signed(call, &sent, self.idle_timeout).await?;
        let mut body = Vec::new();
        while let Some(bytes) = response.chunk().await.map_err(|e| self.failed(e))? {
            if body.len().saturating_add(bytes.len()) > MOST_LISTED {
                return Err(ProviderError::Protocol(format!(
                    "{} answered with more than {MOST_LISTED} bytes",
                    glave_ports::host::shown(&url)
                )));
            }
            body.extend_from_slice(&bytes);
        }
        serde_json::from_slice(&body).map_err(|e| not_json(&url, &e))
    }

    /// What a failed request means: no connection in time, a silence past the idle timeout, or a
    /// broken connection. A connection that never opened sent nothing, so it is the provider
    /// unreached, not silent.
    fn failed(&self, error: reqwest::Error) -> ProviderError {
        request_failed(error, self.idle_timeout)
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

    #[test]
    fn only_anthropic_s_own_api_is_sent_whose_call_it_is() {
        let at = |base_url: &str, api| {
            let config = Config {
                base_url: base_url.into(),
                api,
                ..Config::new(None)
            };
            Anthropic::new(config).unwrap().own_api()
        };
        assert!(at(DEFAULT_BASE_URL, Api::Messages));
        assert!(at("https://api.anthropic.com/v1/", Api::Messages));
        let bedrock = "https://bedrock-runtime.us-east-1.amazonaws.com/anthropic";
        assert!(!at(bedrock, Api::Messages));
        let vertex = "https://aiplatform.googleapis.com/v1/projects/p/locations/global";
        assert!(!at(vertex, Api::Vertex));
    }
}
