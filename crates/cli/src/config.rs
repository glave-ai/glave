//! Where the model is, from the environment. Keys come only from the environment, never files.

use std::time::Duration;

use anyhow::{Context as _, bail};
use glave_ports::Failure;
use glave_provider_openai::Config;

use crate::model;

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
    } else if model::speaks_anthropic(&provider) {
        "anthropic".clone_into(&mut provider.provider);
    }
    Ok(Endpoint {
        local: provider.is_local(),
        provider,
        model,
    })
}

/// What to change when a model call fails in a way the person can fix, naming the setting.
pub fn fix(failure: Failure, endpoint: &Endpoint) -> Option<String> {
    let host = endpoint
        .provider
        .host()
        .unwrap_or_else(|| "the server".to_owned());
    let model = &endpoint.model;
    Some(match failure {
        Failure::Refused if endpoint.provider.api_key.is_none() => {
            format!("{host} wants a key: set GLAVE_API_KEY")
        }
        Failure::Refused => format!("{host} did not accept the key: check GLAVE_API_KEY"),
        Failure::NoCredit => {
            format!("the account behind GLAVE_API_KEY has no credit left at {host}")
        }
        Failure::UnknownModel => format!(
            "check the model id {model} (--model or GLAVE_MODEL), and that GLAVE_BASE_URL points \
             at the right server ({host})"
        ),
        Failure::TooLong => "the conversation is longer than the model's context window: run \
             /compact, start a new thread, or set GLAVE_CONTEXT_WINDOW to the model's real window"
            .to_owned(),
        Failure::RateLimited => format!(
            "{host} is limiting requests even after 4 retries: wait a minute, or try another model"
        ),
        Failure::Unavailable => format!(
            "{host} is failing on its side even after 4 retries: try again shortly, or another model"
        ),
        Failure::Unreachable if endpoint.local => {
            format!("is the model server running on this computer? GLAVE_BASE_URL points at {host}")
        }
        Failure::Unreachable => {
            format!(
                "check the network, and that GLAVE_BASE_URL points at the right server ({host})"
            )
        }
        Failure::Silent => format!(
            "{model} sent nothing for {} s: a slow local model may need a larger GLAVE_IDLE_TIMEOUT",
            endpoint.provider.idle_timeout.as_secs()
        ),
        Failure::Malformed if model::speaks_anthropic(&endpoint.provider) => {
            format!("the server at {host} sent a response glave could not read")
        }
        Failure::Malformed => format!(
            "the server at {host} may not speak OpenAI's chat completions API: check GLAVE_BASE_URL"
        ),
        Failure::Rejected | Failure::OverBudget => return None,
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

/// The model that judges flagged actions in auto mode, from `GLAVE_JUDGE_MODEL`: the main
/// model when it is not set, and no judge at all when it is `off`.
pub fn judge_model(main: &str) -> Option<String> {
    judge_from(std::env::var("GLAVE_JUDGE_MODEL").ok().as_deref(), main)
}

fn judge_from(setting: Option<&str>, main: &str) -> Option<String> {
    match setting.map(str::trim) {
        None | Some("") => Some(main.to_owned()),
        Some("off") => None,
        Some(model) => Some(model.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(url: &str, key: Option<&str>) -> Endpoint {
        let provider = Config::for_url(url, key.map(str::to_owned)).unwrap();
        Endpoint {
            local: provider.is_local(),
            provider,
            model: "anthropic/claude-haiku-5.5".into(),
        }
    }

    #[test]
    fn each_fix_names_the_setting_to_change() {
        let remote = endpoint("https://openrouter.ai/api/v1", Some("sk-1"));
        let fix = |failure| fix(failure, &remote).unwrap_or_default();
        assert!(fix(Failure::Refused).contains("check GLAVE_API_KEY"));
        assert!(fix(Failure::NoCredit).contains("no credit"));
        assert!(fix(Failure::UnknownModel).contains("anthropic/claude-haiku-5.5"));
        assert!(fix(Failure::UnknownModel).contains("GLAVE_MODEL"));
        assert!(fix(Failure::TooLong).contains("/compact"));
        assert!(fix(Failure::Unreachable).contains("GLAVE_BASE_URL"));
        assert!(fix(Failure::Silent).contains("GLAVE_IDLE_TIMEOUT"));
        assert!(fix(Failure::Silent).contains("300 s"));
        assert!(fix(Failure::Malformed).contains("openrouter.ai"));
        assert_eq!(
            super::fix(Failure::Rejected, &remote),
            None,
            "the message says it all"
        );
        let keyless = endpoint("https://openrouter.ai/api/v1", None);
        assert!(
            super::fix(Failure::Refused, &keyless)
                .unwrap()
                .contains("set GLAVE_API_KEY")
        );
        let local = endpoint("http://localhost:11434/v1", None);
        assert!(
            super::fix(Failure::Unreachable, &local)
                .unwrap()
                .contains("running on this computer")
        );
    }

    #[test]
    fn the_judge_is_the_main_model_unless_named_or_off() {
        assert_eq!(judge_from(None, "m").as_deref(), Some("m"));
        assert_eq!(judge_from(Some(" "), "m").as_deref(), Some("m"));
        assert_eq!(judge_from(Some("small"), "m").as_deref(), Some("small"));
        assert_eq!(judge_from(Some("off"), "m"), None);
    }
}
