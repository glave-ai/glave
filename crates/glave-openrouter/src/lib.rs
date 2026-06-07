//! OpenRouter `ModelProvider` adapter — the only crate that touches HTTP/TLS.
//!
//! Owns reasoning control (enabled + measured), cost metering, and the network
//! reliability layer: connect/read timeouts plus bounded retry with exponential
//! backoff that honors `Retry-After`. Repoint to a local vLLM or a frontier leg
//! by swapping this adapter; the core never changes.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use glave_core::{Completion, Message, ModelProvider, ProviderFactory};

/// Reasoning effort: "high" | "medium" | "low" | "off". Default high.
pub fn reasoning_effort() -> String {
    std::env::var("GLAVE_REASONING").unwrap_or_else(|_| "high".to_string())
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

pub struct OpenRouterProvider {
    agent: ureq::Agent,
    api_key: String,
    base_url: String,
    model: String,
    temperature: f64,
    max_tokens: u64,
    reasoning: String,
    max_retries: u32,
    base_backoff: Duration,
}

impl OpenRouterProvider {
    #[must_use]
    pub fn new(api_key: String, model: String) -> Self {
        let connect = Duration::from_secs(env_u64("GLAVE_HTTP_CONNECT_SECS", 10));
        let read = Duration::from_secs(env_u64("GLAVE_HTTP_READ_SECS", 300));
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(connect)
            .timeout_read(read)
            .build();
        Self {
            agent,
            api_key,
            base_url: "https://openrouter.ai/api/v1".to_string(),
            model,
            temperature: 0.0, // pinned for run-to-run comparability
            max_tokens: 8192, // headroom for reasoning + the action
            reasoning: reasoning_effort(),
            max_retries: u32::try_from(env_u64("GLAVE_HTTP_MAX_RETRIES", 3)).unwrap_or(3),
            base_backoff: Duration::from_secs(1),
        }
    }

    pub fn from_env(model: String) -> Result<Self> {
        Ok(Self::new(api_key_from_env()?, model))
    }

    fn request_body(&self, messages: &[Message]) -> Value {
        let msgs: Vec<Value> = messages
            .iter()
            .map(|m| json!({"role": m.role, "content": m.content}))
            .collect();
        let mut body = json!({
            "model": self.model,
            "temperature": self.temperature,
            "max_tokens": self.max_tokens,
            "messages": msgs,
            "usage": { "include": true },
        });
        let reasoning_on = !matches!(self.reasoning.as_str(), "off" | "none" | "");
        if reasoning_on {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("reasoning".to_string(), json!({ "effort": self.reasoning }));
            }
        }
        body
    }
}

fn api_key_from_env() -> Result<String> {
    std::env::var("OPENROUTER_API_KEY").context("OPENROUTER_API_KEY not set (expected in .env)")
}

/// Whether an HTTP status warrants a retry (transient/overload, not a client error).
fn is_retryable_status(code: u16) -> bool {
    matches!(code, 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

/// Delay before a retry attempt (0-based): honor `Retry-After` if present, else
/// exponential backoff from `base`, capped.
fn backoff_delay(
    attempt: u32,
    retry_after: Option<Duration>,
    base: Duration,
    cap: Duration,
) -> Duration {
    if let Some(ra) = retry_after {
        return ra.min(cap);
    }
    let mult = 2u32.saturating_pow(attempt.min(10));
    base.saturating_mul(mult).min(cap)
}

fn retry_after(resp: &ureq::Response) -> Option<Duration> {
    resp.header("Retry-After")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

/// Parse an OpenRouter chat-completion response into a `Completion`.
fn parse_completion(v: &Value, fallback_model: &str) -> Result<Completion> {
    let str_at = |ptr: &str| v.pointer(ptr).and_then(Value::as_str);
    let u64_at = |ptr: &str| v.pointer(ptr).and_then(Value::as_u64).unwrap_or(0);

    let content = str_at("/choices/0/message/content").unwrap_or("");
    let reasoning_text = str_at("/choices/0/message/reasoning").unwrap_or("");
    let content = if content.trim().is_empty() {
        reasoning_text
    } else {
        content
    };
    if content.trim().is_empty() {
        bail!("empty completion (no content or reasoning): {v}");
    }

    Ok(Completion {
        content: content.to_string(),
        // Parsing of native `tool_calls` from the response is wired in the
        // integration step; for now the harness still drives via text tags.
        tool_calls: Vec::new(),
        prompt_tokens: u64_at("/usage/prompt_tokens"),
        completion_tokens: u64_at("/usage/completion_tokens"),
        reasoning_tokens: u64_at("/usage/completion_tokens_details/reasoning_tokens"),
        cost_usd: v.pointer("/usage/cost").and_then(Value::as_f64).unwrap_or(0.0),
        model: str_at("/model").unwrap_or(fallback_model).to_string(),
        provider: str_at("/provider").map(String::from),
    })
}

impl ModelProvider for OpenRouterProvider {
    fn complete(&self, messages: &[Message]) -> Result<Completion> {
        let body = self.request_body(messages);
        let url = format!("{}/chat/completions", self.base_url);
        let cap = Duration::from_secs(30);
        let mut attempt = 0u32;
        loop {
            let resp = self
                .agent
                .post(&url)
                .set("Authorization", &format!("Bearer {}", self.api_key))
                .set("Content-Type", "application/json")
                .set("HTTP-Referer", "https://glave.dev")
                .set("X-Title", "Glave Harness")
                .send_json(body.clone());

            match resp {
                Ok(r) => {
                    let v: Value = r.into_json().context("parsing OpenRouter response")?;
                    return parse_completion(&v, &self.model);
                }
                // Retryable HTTP statuses (rate-limit / provider overload): back off.
                Err(ureq::Error::Status(code, r)) => {
                    if is_retryable_status(code) && attempt < self.max_retries {
                        let delay = backoff_delay(attempt, retry_after(&r), self.base_backoff, cap);
                        std::thread::sleep(delay);
                        attempt += 1;
                        continue;
                    }
                    let detail = r.into_string().unwrap_or_default();
                    bail!("OpenRouter HTTP {code}: {detail}");
                }
                // Transport errors (timeout, connection reset): back off and retry.
                Err(e) => {
                    if attempt < self.max_retries {
                        let delay = backoff_delay(attempt, None, self.base_backoff, cap);
                        std::thread::sleep(delay);
                        attempt += 1;
                        continue;
                    }
                    bail!("OpenRouter request failed after {} attempts: {e}", attempt + 1);
                }
            }
        }
    }

    fn model_id(&self) -> &str {
        &self.model
    }
}

/// Mints OpenRouter providers per model. The API key is read from the env at
/// construction if present; a missing key only surfaces as an error when a model
/// is actually requested — so stub-agent runs (e.g. `validate`) need no key. A
/// server should still verify the key at startup before accepting traffic.
pub struct OpenRouterFactory {
    api_key: Option<String>,
}

impl OpenRouterFactory {
    #[must_use]
    pub fn from_env() -> Self {
        Self { api_key: std::env::var("OPENROUTER_API_KEY").ok() }
    }
}

impl ProviderFactory for OpenRouterFactory {
    fn create(&self, model: &str) -> Result<Box<dyn ModelProvider>> {
        let key = self
            .api_key
            .clone()
            .context("OPENROUTER_API_KEY not set (expected in .env)")?;
        Ok(Box::new(OpenRouterProvider::new(key, model.to_string())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_statuses_are_transient_only() {
        for c in [408, 425, 429, 500, 502, 503, 504] {
            assert!(is_retryable_status(c), "{c} should be retryable");
        }
        for c in [200, 400, 401, 403, 404, 422] {
            assert!(!is_retryable_status(c), "{c} should not be retryable");
        }
    }

    #[test]
    fn backoff_is_exponential_and_capped() {
        let base = Duration::from_secs(1);
        let cap = Duration::from_secs(30);
        assert_eq!(backoff_delay(0, None, base, cap), Duration::from_secs(1));
        assert_eq!(backoff_delay(1, None, base, cap), Duration::from_secs(2));
        assert_eq!(backoff_delay(2, None, base, cap), Duration::from_secs(4));
        assert_eq!(backoff_delay(3, None, base, cap), Duration::from_secs(8));
        assert_eq!(backoff_delay(20, None, base, cap), cap); // far past cap
    }

    #[test]
    fn backoff_honors_and_caps_retry_after() {
        let base = Duration::from_secs(1);
        let cap = Duration::from_secs(30);
        assert_eq!(
            backoff_delay(0, Some(Duration::from_secs(5)), base, cap),
            Duration::from_secs(5)
        );
        assert_eq!(backoff_delay(0, Some(Duration::from_secs(120)), base, cap), cap);
    }

    #[test]
    fn parse_completion_reads_content_and_usage() -> Result<()> {
        let v = json!({
            "model": "qwen/qwen3.6-27b",
            "provider": "Together",
            "choices": [{"message": {"content": "hello"}}],
            "usage": {
                "prompt_tokens": 10, "completion_tokens": 5, "cost": 0.0012,
                "completion_tokens_details": {"reasoning_tokens": 3}
            }
        });
        let c = parse_completion(&v, "fallback")?;
        assert_eq!(c.content, "hello");
        assert_eq!(c.prompt_tokens, 10);
        assert_eq!(c.completion_tokens, 5);
        assert_eq!(c.reasoning_tokens, 3);
        assert!((c.cost_usd - 0.0012).abs() < 1e-9);
        assert_eq!(c.model, "qwen/qwen3.6-27b");
        assert_eq!(c.provider.as_deref(), Some("Together"));
        Ok(())
    }

    #[test]
    fn parse_completion_falls_back_to_reasoning() -> Result<()> {
        let v = json!({"choices": [{"message": {"content": "", "reasoning": "thinking"}}]});
        let c = parse_completion(&v, "m")?;
        assert_eq!(c.content, "thinking");
        Ok(())
    }

    #[test]
    fn parse_completion_errors_on_empty() {
        let empty = json!({"choices": [{"message": {"content": ""}}]});
        assert!(parse_completion(&empty, "m").is_err());
    }
}
