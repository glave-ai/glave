//! Chat-completions `ModelProvider` adapter — the only crate that touches
//! HTTP/TLS. Owns reasoning control (enabled + measured), cost metering, and
//! the network reliability layer: connect/read timeouts plus bounded retry
//! with exponential backoff that honors `Retry-After`.
//!
//! One transport, two [`Dialect`]s of the same OpenAI chat API:
//! - **OpenRouter** — the multi-vendor broker; requests carry broker-only
//!   extensions (metered cost, reasoning effort, provider pinning).
//! - **OpenAiCompat** — the first-party rail: a model named `glave/<served>`
//!   routes to the self-hosted endpoint at `GLAVE_BASE_URL` (vLLM / RunPod)
//!   with `GLAVE_API_KEY`, speaking pure-spec requests.
//!
//! Only the explicitly-prefixed model reroutes — a frontier baseline in the
//! same run still goes to OpenRouter, so a scorecard's two legs can't cross
//! wires.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use glave_core::{tool_specs, Completion, Message, ModelProvider, ProviderFactory, ToolCallRequest};

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

/// Which dialect of the OpenAI chat-completions API the endpoint speaks.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dialect {
    /// openrouter.ai — the multi-vendor broker. Accepts broker-only request
    /// extensions: `usage.include` (metered cost), `reasoning` effort,
    /// provider/quant pinning.
    OpenRouter,
    /// A plain OpenAI-compatible server (vLLM behind `GLAVE_BASE_URL`):
    /// pure-spec requests, plus vLLM's own `chat_template_kwargs`.
    OpenAiCompat,
}

pub struct ChatProvider {
    agent: ureq::Agent,
    api_key: String,
    base_url: String,
    dialect: Dialect,
    /// The model id as the rest of the system knows it — traces, labels,
    /// cache keys: e.g. `glave/qwen3.6-27b` or `anthropic/claude-opus-4.8`.
    model: String,
    /// The name sent on the wire. Same as `model` for OpenRouter; for the
    /// glave rail it's the server's `--served-model-name` (prefix stripped).
    wire_model: String,
    temperature: f64,
    max_tokens: u64,
    reasoning: String,
    max_retries: u32,
    base_backoff: Duration,
}

impl ChatProvider {
    #[must_use]
    pub fn openrouter(api_key: String, model: String) -> Self {
        let wire_model = model.clone();
        let base_url = "https://openrouter.ai/api/v1".to_string();
        Self::build(api_key, model, wire_model, base_url, Dialect::OpenRouter)
    }

    /// The first-party rail: a self-hosted OpenAI-compatible endpoint
    /// (vLLM / RunPod). `model` is the public `glave/…` id; `wire_model` is
    /// the server's served name.
    #[must_use]
    pub fn self_hosted(api_key: String, model: String, wire_model: String, base_url: String) -> Self {
        Self::build(api_key, model, wire_model, base_url, Dialect::OpenAiCompat)
    }

    fn build(
        api_key: String,
        model: String,
        wire_model: String,
        base_url: String,
        dialect: Dialect,
    ) -> Self {
        let connect = Duration::from_secs(env_u64("GLAVE_HTTP_CONNECT_SECS", 10));
        let read = Duration::from_secs(env_u64("GLAVE_HTTP_READ_SECS", 300));
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(connect)
            .timeout_read(read)
            .build();
        let reasoning = reasoning_effort();
        let reasoning_on = !matches!(reasoning.as_str(), "off" | "none" | "");
        Self {
            agent,
            api_key,
            base_url,
            dialect,
            model,
            wire_model,
            // Reasoning REQUIRES temperature 1.0 for Anthropic (extended thinking
            // is silently disabled otherwise) and is recommended for reasoning
            // generally; both legs use the same value so the comparison is fair,
            // with run-to-run variance absorbed by --trials. Without reasoning,
            // pin 0.0 for determinism.
            temperature: if reasoning_on { 1.0 } else { 0.0 },
            // GLAVE_MAX_TOKENS = the PER-REQUEST (per-turn) output cap sent to
            // the API. Distinct from GLAVE_TOKEN_BUDGET (agent.rs), which bounds
            // a whole run's TOTAL output across all turns — don't confuse them:
            // setting this expecting a run-wide budget will surprise you.
            // Generous because, for Anthropic, thinking tokens count against this
            // — too low silently throttles high-effort reasoning. Env-tunable.
            max_tokens: env_u64("GLAVE_MAX_TOKENS", 32000),
            reasoning,
            max_retries: u32::try_from(env_u64("GLAVE_HTTP_MAX_RETRIES", 3)).unwrap_or(3),
            base_backoff: Duration::from_secs(1),
        }
    }

    pub fn from_env(model: String) -> Result<Self> {
        Ok(Self::openrouter(api_key_from_env()?, model))
    }

    fn request_body(&self, messages: &[Message]) -> Value {
        let msgs: Vec<Value> = messages.iter().map(message_json).collect();
        let tools: Vec<Value> = tool_specs()
            .into_iter()
            .map(|s| {
                json!({
                    "type": "function",
                    "function": {
                        "name": s.name,
                        "description": s.description,
                        "parameters": s.parameters,
                    }
                })
            })
            .collect();
        let mut body = json!({
            "model": self.wire_model,
            "temperature": self.temperature,
            "max_tokens": self.max_tokens,
            "messages": msgs,
            "tools": tools,
            "tool_choice": "auto",
        });
        let Some(obj) = body.as_object_mut() else { return body };
        let reasoning_on = !matches!(self.reasoning.as_str(), "off" | "none" | "");

        match self.dialect {
            // Self-hosted reasoning models (qwen3.6) think by default and have
            // no effort knob; "off" maps to the chat template's escape hatch.
            Dialect::OpenAiCompat => {
                if !reasoning_on {
                    obj.insert(
                        "chat_template_kwargs".to_string(),
                        json!({ "enable_thinking": false }),
                    );
                }
            }
            Dialect::OpenRouter => {
                obj.insert("usage".to_string(), json!({ "include": true }));
                if reasoning_on {
                    obj.insert("reasoning".to_string(), json!({ "effort": self.reasoning }));
                }
                // Pin provider + quantization for reproducible, like-for-like
                // runs on open models (e.g. DeepInfra / fp8 — the quant we'd
                // actually serve). Skipped for single-source closed models
                // (Claude/GPT/Gemini): no provider/quant choice, and
                // allow_fallbacks:false would wrongly fail them.
                if let Ok(provider) = std::env::var("GLAVE_PROVIDER") {
                    if !is_single_provider(&self.model) {
                        let mut pin = json!({ "only": [provider], "allow_fallbacks": false });
                        if let (Some(p), Ok(quant)) =
                            (pin.as_object_mut(), std::env::var("GLAVE_QUANT"))
                        {
                            p.insert("quantizations".to_string(), json!([quant]));
                        }
                        obj.insert("provider".to_string(), pin);
                    }
                }
            }
        }
        body
    }
}

/// Closed, single-source models — no provider/quant choice to pin.
/// (`google/gemma*` is open and serve-able by many providers, so not matched.)
fn is_single_provider(model: &str) -> bool {
    model.starts_with("anthropic/")
        || model.starts_with("openai/")
        || model.starts_with("google/gemini")
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

/// Serialize a `Message` into the OpenAI chat shape, carrying the native
/// tool-calling envelope: an assistant turn with `tool_calls`, or a `tool`-role
/// result keyed by `tool_call_id`, or a plain message.
fn message_json(m: &Message) -> Value {
    if !m.tool_calls.is_empty() {
        let calls: Vec<Value> = m
            .tool_calls
            .iter()
            .map(|tc| {
                json!({
                    "id": tc.id,
                    "type": "function",
                    "function": {"name": tc.name, "arguments": tc.arguments}
                })
            })
            .collect();
        return json!({"role": m.role.as_str(), "content": m.content, "tool_calls": calls});
    }
    if let Some(id) = &m.tool_call_id {
        return json!({"role": m.role.as_str(), "content": m.content, "tool_call_id": id});
    }
    json!({"role": m.role.as_str(), "content": m.content})
}

/// Extract the structured `tool_calls` from a response message.
fn parse_tool_calls(v: &Value) -> Vec<ToolCallRequest> {
    let Some(arr) = v.pointer("/choices/0/message/tool_calls").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|tc| {
            let id = tc.get("id").and_then(Value::as_str)?;
            let name = tc.pointer("/function/name").and_then(Value::as_str)?;
            let args = tc
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            Some(ToolCallRequest {
                id: id.to_string(),
                name: name.to_string(),
                arguments: args.to_string(),
            })
        })
        .collect()
}

/// Parse an OpenRouter chat-completion response into a `Completion`. Infallible:
/// an empty/odd message becomes an empty `Completion` the agent loop nudges.
fn parse_completion(v: &Value, fallback_model: &str) -> Completion {
    let str_at = |ptr: &str| v.pointer(ptr).and_then(Value::as_str);
    let u64_at = |ptr: &str| v.pointer(ptr).and_then(Value::as_u64).unwrap_or(0);

    let tool_calls = parse_tool_calls(v);
    let content = str_at("/choices/0/message/content").unwrap_or("");
    // OpenRouter surfaces thinking as `reasoning`; vLLM's reasoning parsers
    // (e.g. qwen3) as `reasoning_content`.
    let reasoning_text = str_at("/choices/0/message/reasoning")
        .or_else(|| str_at("/choices/0/message/reasoning_content"))
        .unwrap_or("");
    let content = if content.trim().is_empty() {
        reasoning_text
    } else {
        content
    };
    // An empty turn (no content, no tool calls — the model just "stopped") is a
    // no-op the agent loop nudges, not a fatal error. Return it as-is.

    Completion {
        content: content.to_string(),
        tool_calls,
        prompt_tokens: u64_at("/usage/prompt_tokens"),
        completion_tokens: u64_at("/usage/completion_tokens"),
        reasoning_tokens: u64_at("/usage/completion_tokens_details/reasoning_tokens"),
        cost_usd: v.pointer("/usage/cost").and_then(Value::as_f64).unwrap_or(0.0),
        model: str_at("/model").unwrap_or(fallback_model).to_string(),
        provider: str_at("/provider").map(String::from),
    }
}

/// A self-hosted endpoint reports no `usage.cost` (there's no per-token bill —
/// we rent the GPU). So that $/correct still computes, impute cost from env
/// prices in USD per **million** tokens: `GLAVE_PRICE_IN` / `GLAVE_PRICE_OUT`,
/// derived offline from rental $/s ÷ measured throughput. Never overrides a
/// real metered cost.
fn impute_cost(c: &mut Completion) {
    if c.cost_usd != 0.0 {
        return;
    }
    let price = |key| std::env::var(key).ok().and_then(|v: String| v.parse::<f64>().ok());
    let (Some(input), Some(output)) = (price("GLAVE_PRICE_IN"), price("GLAVE_PRICE_OUT")) else {
        return;
    };
    c.cost_usd = per_mtok(c.prompt_tokens, input) + per_mtok(c.completion_tokens, output);
}

#[allow(clippy::cast_precision_loss)] // token counts are far below 2^52
fn per_mtok(tokens: u64, usd_per_mtok: f64) -> f64 {
    usd_per_mtok * (tokens as f64) / 1_000_000.0
}

impl ModelProvider for ChatProvider {
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
                    let v: Value = r.into_json().context("parsing chat-completion response")?;
                    let mut c = parse_completion(&v, &self.model);
                    if self.dialect == Dialect::OpenAiCompat {
                        impute_cost(&mut c);
                    }
                    return Ok(c);
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
                    bail!("{} HTTP {code}: {detail}", self.base_url);
                }
                // Transport errors (timeout, connection reset): back off and retry.
                Err(e) => {
                    if attempt < self.max_retries {
                        let delay = backoff_delay(attempt, None, self.base_backoff, cap);
                        std::thread::sleep(delay);
                        attempt += 1;
                        continue;
                    }
                    bail!("request to {} failed after {} attempts: {e}", self.base_url, attempt + 1);
                }
            }
        }
    }

    fn model_id(&self) -> &str {
        &self.model
    }
}

/// Mints a provider per model id — the single place a model string becomes a
/// live connection. The OpenRouter API key is read from the env at
/// construction if present; a missing key only surfaces as an error when a
/// model is actually requested — so stub-agent runs (e.g. `validate`) need no
/// key. A server should still verify the key at startup before accepting
/// traffic.
pub struct ChatProviderFactory {
    api_key: Option<String>,
}

impl ChatProviderFactory {
    #[must_use]
    pub fn from_env() -> Self {
        Self { api_key: std::env::var("OPENROUTER_API_KEY").ok() }
    }
}

impl ProviderFactory for ChatProviderFactory {
    fn create(&self, model: &str) -> Result<Box<dyn ModelProvider>> {
        // `glave/<served-name>` → the first-party rail. Only this prefix
        // reroutes; every other model (e.g. the frontier baseline leg of the
        // same scorecard run) still goes to OpenRouter. The full `glave/…` id
        // stays the provider's public identity (traces, labels); only the
        // wire name is stripped to match the server's `--served-model-name`.
        if let Some(served) = model.strip_prefix("glave/") {
            let base_url = std::env::var("GLAVE_BASE_URL").context(
                "GLAVE_BASE_URL not set (required for glave/* models; e.g. \
                 https://api.runpod.ai/v2/<endpoint-id>/openai/v1)",
            )?;
            // vLLM itself doesn't authenticate by default; RunPod does.
            let key = std::env::var("GLAVE_API_KEY").unwrap_or_default();
            return Ok(Box::new(ChatProvider::self_hosted(
                key,
                model.to_string(),
                served.to_string(),
                base_url,
            )));
        }
        let key = self
            .api_key
            .clone()
            .context("OPENROUTER_API_KEY not set (expected in .env)")?;
        Ok(Box::new(ChatProvider::openrouter(key, model.to_string())))
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
    fn parse_completion_reads_content_and_usage() {
        let v = json!({
            "model": "qwen/qwen3.6-27b",
            "provider": "Together",
            "choices": [{"message": {"content": "hello"}}],
            "usage": {
                "prompt_tokens": 10, "completion_tokens": 5, "cost": 0.0012,
                "completion_tokens_details": {"reasoning_tokens": 3}
            }
        });
        let c = parse_completion(&v, "fallback");
        assert_eq!(c.content, "hello");
        assert_eq!(c.prompt_tokens, 10);
        assert_eq!(c.completion_tokens, 5);
        assert_eq!(c.reasoning_tokens, 3);
        assert!((c.cost_usd - 0.0012).abs() < 1e-9);
        assert_eq!(c.model, "qwen/qwen3.6-27b");
        assert_eq!(c.provider.as_deref(), Some("Together"));
    }

    #[test]
    fn parse_completion_falls_back_to_reasoning() {
        let v = json!({"choices": [{"message": {"content": "", "reasoning": "thinking"}}]});
        let c = parse_completion(&v, "m");
        assert_eq!(c.content, "thinking");
    }

    #[test]
    fn parse_completion_reads_vllm_reasoning_content() {
        // vLLM's reasoning parsers emit `reasoning_content`, not `reasoning`.
        let v = json!({"choices": [{"message": {"content": "", "reasoning_content": "vllm-think"}}]});
        let c = parse_completion(&v, "m");
        assert_eq!(c.content, "vllm-think");
    }

    fn glave_rail(model: &str, wire: &str) -> ChatProvider {
        ChatProvider::self_hosted(String::new(), model.into(), wire.into(), "http://h/v1".into())
    }

    #[test]
    fn self_hosted_body_omits_openrouter_only_fields() {
        let p = glave_rail("glave/m", "m");
        let body = p.request_body(&[Message::user("hi")]);
        assert!(body.get("usage").is_none(), "usage.include is OpenRouter-only");
        assert!(body.get("reasoning").is_none(), "effort knob is OpenRouter-only");
        assert!(body.get("provider").is_none(), "provider pin is OpenRouter-only");
        // Core OpenAI shape intact.
        assert!(body.get("tools").is_some());
        assert!(body.get("temperature").is_some());
    }

    #[test]
    fn glave_rail_keeps_public_id_but_strips_wire_name() {
        let p = glave_rail("glave/qwen3.6-27b", "qwen3.6-27b");
        // Traces and labels see the full first-party id…
        assert_eq!(p.model_id(), "glave/qwen3.6-27b");
        // …while the wire carries the server's --served-model-name.
        let body = p.request_body(&[Message::user("hi")]);
        assert_eq!(body.get("model"), Some(&json!("qwen3.6-27b")));
    }

    #[test]
    fn openrouter_body_keeps_usage_include_and_full_model_id() {
        let p = ChatProvider::openrouter(String::new(), "anthropic/claude-opus-4.8".into());
        let body = p.request_body(&[Message::user("hi")]);
        assert_eq!(body.pointer("/usage/include"), Some(&json!(true)));
        assert_eq!(body.get("model"), Some(&json!("anthropic/claude-opus-4.8")));
    }

    #[test]
    fn self_hosted_reasoning_off_disables_thinking_via_template() {
        let mut p = glave_rail("glave/m", "m");
        p.reasoning = "off".to_string();
        let body = p.request_body(&[Message::user("hi")]);
        assert_eq!(
            body.pointer("/chat_template_kwargs/enable_thinking"),
            Some(&json!(false))
        );
    }

    #[test]
    fn impute_cost_prices_tokens_and_never_overrides_metered() {
        let mtok = per_mtok(2_000_000, 0.5);
        assert!((mtok - 1.0).abs() < 1e-9);
        // A metered cost is left alone regardless of env.
        let mut c = parse_completion(
            &json!({
                "choices": [{"message": {"content": "x"}}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "cost": 0.7}
            }),
            "m",
        );
        impute_cost(&mut c);
        assert!((c.cost_usd - 0.7).abs() < 1e-9);
    }

    #[test]
    fn parse_completion_tolerates_empty_turn() {
        // A model can return an empty message (content null, no tool calls — it
        // just "stopped"); that's a no-op the loop nudges, not a fatal error.
        let empty = json!({"choices": [{"message": {"content": null}}]});
        let c = parse_completion(&empty, "m");
        assert!(c.content.is_empty());
        assert!(c.tool_calls.is_empty());
    }

    #[test]
    fn message_json_serializes_the_three_shapes() {
        let asst = Message::assistant_with_tool_calls(
            "",
            vec![ToolCallRequest {
                id: "call_1".to_string(),
                name: "bash".to_string(),
                arguments: r#"{"cmd":"ls"}"#.to_string(),
            }],
        );
        let at = |v: &Value, p: &str| v.pointer(p).and_then(Value::as_str).map(String::from);
        let v = message_json(&asst);
        assert_eq!(at(&v, "/role").as_deref(), Some("assistant"));
        assert_eq!(at(&v, "/tool_calls/0/id").as_deref(), Some("call_1"));
        assert_eq!(at(&v, "/tool_calls/0/type").as_deref(), Some("function"));
        assert_eq!(at(&v, "/tool_calls/0/function/name").as_deref(), Some("bash"));

        let res = message_json(&Message::tool_result("call_1", "files..."));
        assert_eq!(at(&res, "/role").as_deref(), Some("tool"));
        assert_eq!(at(&res, "/tool_call_id").as_deref(), Some("call_1"));
        assert_eq!(at(&res, "/content").as_deref(), Some("files..."));

        let plain = message_json(&Message::user("hi"));
        assert_eq!(at(&plain, "/role").as_deref(), Some("user"));
        assert!(plain.get("tool_calls").is_none());
        assert!(plain.get("tool_call_id").is_none());
    }

    #[test]
    fn parse_tool_calls_reads_structured_calls() {
        let v = json!({
            "choices": [{"message": {"content": null, "tool_calls": [
                {"id": "call_9", "type": "function",
                 "function": {"name": "read", "arguments": "{\"path\":\"a.rs\"}"}}
            ]}}]
        });
        let calls = parse_tool_calls(&v);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls.first().map(|c| c.id.as_str()), Some("call_9"));
        assert_eq!(calls.first().map(|c| c.name.as_str()), Some("read"));
        assert_eq!(calls.first().map(|c| c.arguments.as_str()), Some(r#"{"path":"a.rs"}"#));
    }

    #[test]
    fn parse_completion_accepts_tool_calls_with_empty_content() {
        let v = json!({
            "model": "m",
            "choices": [{"message": {"content": "", "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "submit", "arguments": "{}"}}
            ]}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}
        });
        let c = parse_completion(&v, "m");
        assert_eq!(c.tool_calls.len(), 1); // empty content OK when tool_calls present
    }
}
