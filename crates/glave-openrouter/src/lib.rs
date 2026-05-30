//! OpenRouter `ModelProvider` adapter — the only crate that touches HTTP/TLS.
//!
//! Owns reasoning control (enabled + measured) and cost metering. Repoints to a
//! local vLLM or a frontier leg by swapping this adapter; the core never changes.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use glave_core::{Completion, Message, ModelProvider};

/// Reasoning effort: "high" | "medium" | "low" | "off". Default high.
pub fn reasoning_effort() -> String {
    std::env::var("GLAVE_REASONING").unwrap_or_else(|_| "high".to_string())
}

pub struct OpenRouterProvider {
    api_key: String,
    base_url: String,
    model: String,
    temperature: f64,
    max_tokens: u64,
    reasoning: String,
}

impl OpenRouterProvider {
    pub fn from_env(model: String) -> Result<Self> {
        let api_key = std::env::var("OPENROUTER_API_KEY")
            .context("OPENROUTER_API_KEY not set (expected in .env)")?;
        Ok(Self {
            api_key,
            base_url: "https://openrouter.ai/api/v1".to_string(),
            model,
            temperature: 0.0, // pinned for run-to-run comparability
            max_tokens: 8192, // headroom for reasoning + the action
            reasoning: reasoning_effort(),
        })
    }
}

impl ModelProvider for OpenRouterProvider {
    fn complete(&self, messages: &[Message]) -> Result<Completion> {
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

        let url = format!("{}/chat/completions", self.base_url);
        let resp = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("Content-Type", "application/json")
            .set("HTTP-Referer", "https://glave.dev")
            .set("X-Title", "Glave Harness")
            .send_json(body);

        let v: Value = match resp {
            Ok(r) => r.into_json().context("parsing OpenRouter response")?,
            Err(ureq::Error::Status(code, r)) => {
                let body = r.into_string().unwrap_or_default();
                bail!("OpenRouter HTTP {code}: {body}");
            }
            Err(e) => bail!("OpenRouter request failed: {e}"),
        };

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
            prompt_tokens: u64_at("/usage/prompt_tokens"),
            completion_tokens: u64_at("/usage/completion_tokens"),
            reasoning_tokens: u64_at("/usage/completion_tokens_details/reasoning_tokens"),
            cost_usd: v.pointer("/usage/cost").and_then(Value::as_f64).unwrap_or(0.0),
            model: str_at("/model").unwrap_or(self.model.as_str()).to_string(),
            provider: str_at("/provider").map(String::from),
        })
    }

    fn model_id(&self) -> &str {
        &self.model
    }
}
