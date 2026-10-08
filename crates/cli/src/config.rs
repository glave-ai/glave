//! Where the model is, from the environment. Keys come only from the environment, never files.

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
    let host =
        host_of(&base_url).with_context(|| format!("GLAVE_BASE_URL is not a URL: {base_url}"))?;
    let local = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]");
    let Some(model) = model_flag.or_else(|| std::env::var("GLAVE_MODEL").ok()) else {
        bail!(
            "no model chosen: set GLAVE_MODEL or pass --model (for example gemma4:12b with \
             Ollama, or an OpenRouter model id with GLAVE_BASE_URL=https://openrouter.ai/api/v1)"
        );
    };
    let provider =
        std::env::var("GLAVE_PROVIDER").unwrap_or_else(|_| provider_name(&host, &base_url));
    let api_key = std::env::var("GLAVE_API_KEY")
        .ok()
        .filter(|key| !key.is_empty());
    Ok(Endpoint {
        provider: Config {
            base_url,
            api_key,
            provider,
        },
        model,
        local,
    })
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

fn provider_name(host: &str, url: &str) -> String {
    match host {
        "openrouter.ai" => "openrouter".into(),
        "api.openai.com" => "openai".into(),
        "localhost" | "127.0.0.1" | "[::1]" if url.contains(":11434") => "ollama".into(),
        "localhost" | "127.0.0.1" | "[::1]" => "local".into(),
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_and_provider_names() {
        assert_eq!(
            host_of("https://openrouter.ai/api/v1").as_deref(),
            Some("openrouter.ai")
        );
        assert_eq!(
            host_of("http://localhost:11434/v1").as_deref(),
            Some("localhost")
        );
        assert_eq!(host_of("ftp://x"), None);
        assert_eq!(provider_name("openrouter.ai", ""), "openrouter");
        assert_eq!(
            provider_name("localhost", "http://localhost:11434/v1"),
            "ollama"
        );
        assert_eq!(
            provider_name("localhost", "http://localhost:8080/v1"),
            "local"
        );
    }
}
