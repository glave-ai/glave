//! Where the model is, from the environment. Keys come only from the environment, never files.

use std::time::Duration;

use anyhow::{Context as _, bail};
use glave_provider_openai::Config;

const DEFAULT_BASE_URL: &str = "http://localhost:11434/v1";

/// The model endpoint and the model to use.
pub struct Endpoint {
    pub provider: Config,
    pub model: String,
    /// True when the model runs on this computer, so a missing cost means $0.
    pub local: bool,
}

pub fn endpoint(model_flag: Option<String>) -> anyhow::Result<Endpoint> {
    let base_url = std::env::var("GLAVE_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned());
    let Some(model) = model_flag.or_else(|| std::env::var("GLAVE_MODEL").ok()) else {
        bail!(
            "no model chosen: set GLAVE_MODEL or pass --model (for example gemma4:12b with \
             Ollama, or an OpenRouter model id with GLAVE_BASE_URL=https://openrouter.ai/api/v1)"
        );
    };
    let api_key = std::env::var("GLAVE_API_KEY")
        .ok()
        .filter(|key| !key.is_empty());
    let mut provider = Config::for_url(base_url.clone(), api_key)
        .with_context(|| format!("GLAVE_BASE_URL is not a URL: {base_url}"))?;
    if let Ok(name) = std::env::var("GLAVE_PROVIDER") {
        provider.provider = name;
    }
    Ok(Endpoint {
        local: provider.is_local(),
        provider,
        model,
    })
}

/// How long the model may send nothing, from `GLAVE_IDLE_TIMEOUT` in seconds, when it is set.
pub fn idle_timeout() -> anyhow::Result<Option<Duration>> {
    match std::env::var("GLAVE_IDLE_TIMEOUT") {
        Ok(text) => {
            let seconds: u64 = text.trim().parse().with_context(|| {
                format!("GLAVE_IDLE_TIMEOUT must be a number of seconds, not {text:?}")
            })?;
            anyhow::ensure!(seconds >= 1, "GLAVE_IDLE_TIMEOUT must be at least 1 second");
            Ok(Some(Duration::from_secs(seconds)))
        }
        Err(_) => Ok(None),
    }
}

/// The model's context window from `GLAVE_CONTEXT_WINDOW`, in tokens, when it is set.
pub fn context_window() -> anyhow::Result<Option<u64>> {
    match std::env::var("GLAVE_CONTEXT_WINDOW") {
        Ok(text) => {
            let tokens: u64 = text.trim().parse().with_context(|| {
                format!("GLAVE_CONTEXT_WINDOW must be a number of tokens, not {text:?}")
            })?;
            anyhow::ensure!(
                tokens >= 1_000,
                "GLAVE_CONTEXT_WINDOW is too small: {tokens} tokens"
            );
            Ok(Some(tokens))
        }
        Err(_) => Ok(None),
    }
}
