//! Where the model is: `GLAVE_BASE_URL`'s server, or the cloud contract `[provider]` names.
//! Keys come only from the environment, never files. Which model is chosen in `policy`,
//! nearest wins.

mod contract;

use std::time::Duration;

use anyhow::{Context as _, bail};
use glave_config::policy::{Chosen, ProviderFile};
use glave_ports::Failure;
use glave_provider_openai::Config;

use crate::model::{self, Claude};

const DEFAULT_BASE_URL: &str = "http://localhost:11434/v1";

/// The variable a key comes from when nothing else says.
const KEY: &str = "GLAVE_API_KEY";

/// The model endpoint and the model to use.
pub struct Endpoint {
    pub provider: Config,
    /// Claude's Messages API at the same cloud, for the models whose family speaks it.
    pub claude: Option<Claude>,
    pub model: String,
    /// True when the model runs on this computer, so a missing cost means $0.
    pub local: bool,
    /// The gateway elsewhere the endpoint is, when `[gateway] url` sends calls to one.
    pub gateway: Option<String>,
    /// The setting that says where the endpoint is: `GLAVE_BASE_URL`, or `[provider]` and
    /// its file.
    pub at: String,
    /// The variable the key came from, or would: `GLAVE_API_KEY` unless a cloud's own.
    pub key_from: &'static str,
}

/// The endpoint for `model`, the model chosen, at the cloud `provider` names, else at
/// `GLAVE_BASE_URL`'s server; an error saying how to choose a model when none is.
pub fn endpoint(
    model: Option<String>,
    provider: Option<&Chosen<ProviderFile>>,
) -> anyhow::Result<Endpoint> {
    let Some(model) = model else {
        bail!(
            "no model chosen: set GLAVE_MODEL, pass --model, or set model in config.toml (for \
             example gemma4:12b with Ollama, or an OpenRouter model id with \
             GLAVE_BASE_URL=https://openrouter.ai/api/v1)"
        );
    };
    if let Some(chosen) = provider {
        if std::env::var_os("GLAVE_BASE_URL").is_some() {
            bail!(
                "[provider] (from {}) and GLAVE_BASE_URL both say where the models are: unset one",
                chosen.from
            );
        }
        let reached = contract::provider(chosen, &|name| std::env::var(name).ok())?;
        return Ok(Endpoint {
            local: false,
            provider: reached.provider,
            claude: reached.claude,
            model,
            gateway: None,
            at: format!("[provider] (from {})", chosen.from),
            key_from: reached.key_from.unwrap_or(KEY),
        });
    }
    let base_url = std::env::var("GLAVE_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned());
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
        claude: None,
        model,
        gateway: None,
        at: "GLAVE_BASE_URL".to_owned(),
        key_from: KEY,
    })
}

/// `[provider]` in the `config.toml` in glave's folder, for a command that reads no project
/// settings, such as `glave gateway serve`: a project never chooses it.
pub async fn provider_here() -> anyhow::Result<Option<Chosen<ProviderFile>>> {
    let path = crate::home::folder()?.join(glave_config::policy::CONFIG);
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", path.display()));
        }
    };
    let file = glave_config::policy::config(&text, &path)?;
    Ok(file.provider.map(|value| Chosen {
        value,
        from: glave_config::policy::Source::File(path),
    }))
}

/// What to change when a model call fails in a way the person can fix, naming the setting.
pub fn fix(failure: Failure, endpoint: &Endpoint) -> Option<String> {
    if let Some(gateway) = &endpoint.gateway {
        return through_gateway(failure, gateway, &endpoint.model);
    }
    let host = endpoint
        .provider
        .host()
        .unwrap_or_else(|| "the server".to_owned());
    let model = &endpoint.model;
    let (at, key) = (&endpoint.at, endpoint.key_from);
    Some(match failure {
        Failure::Refused if endpoint.provider.auth.is_none() => {
            format!("{host} wants a key: set {key}")
        }
        Failure::Refused if endpoint.provider.auth.fixed_key().is_none() => format!(
            "{host} refused, or glave could not get credentials from {}: check its setup",
            endpoint.provider.auth.describe()
        ),
        Failure::Refused => format!("{host} did not accept the key: check {key}"),
        Failure::NoCredit => {
            format!("the account behind {key} has no credit left at {host}")
        }
        Failure::UnknownModel => format!(
            "check the model id {model} (--model or GLAVE_MODEL), and that {at} points at the \
             right server ({host})"
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
            format!("is the model server running on this computer? {at} points at {host}")
        }
        Failure::Unreachable => {
            format!("check the network, and that {at} points at the right server ({host})")
        }
        Failure::Silent => format!(
            "{model} sent nothing for {} s: a slow local model may need a larger GLAVE_IDLE_TIMEOUT",
            endpoint.provider.idle_timeout.as_secs()
        ),
        Failure::Malformed
            if endpoint.claude.is_some() || model::speaks_anthropic(&endpoint.provider) =>
        {
            format!("the server at {host} sent a response glave could not read")
        }
        Failure::Malformed => {
            format!("the server at {host} may not speak OpenAI's chat completions API: check {at}")
        }
        Failure::Rejected | Failure::OverBudget => return None,
    })
}

/// What to change when a call through the gateway at `gateway` fails: the gateway holds the
/// keys and the policy, so the person's part is their sign-in and their choices.
fn through_gateway(failure: Failure, gateway: &str, model: &str) -> Option<String> {
    Some(match failure {
        Failure::Refused => format!(
            "the gateway at {gateway} refused: renew your sign-in in GLAVE_GATEWAY_TOKEN, or ask \
             whether your organization's policy allows {model}"
        ),
        Failure::NoCredit | Failure::OverBudget => {
            "a budget the gateway keeps for you is spent: it says which".to_owned()
        }
        Failure::UnknownModel => format!(
            "the gateway has no model {model}: choose another with --model or model in \
             config.toml, or leave it unset for the gateway's own"
        ),
        Failure::Unreachable => {
            format!("check the network, and [gateway] url in your config.toml ({gateway})")
        }
        Failure::TooLong => "the conversation is longer than the model's context window: run \
             /compact, or start a new thread"
            .to_owned(),
        Failure::RateLimited | Failure::Unavailable | Failure::Silent | Failure::Malformed => {
            format!("the gateway at {gateway} could not get an answer: try again shortly")
        }
        Failure::Rejected => return None,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(url: &str, key: Option<&str>) -> Endpoint {
        let provider = Config::for_url(url, key.map(str::to_owned)).unwrap();
        Endpoint {
            local: provider.is_local(),
            provider,
            claude: None,
            model: "anthropic/claude-haiku-5.5".into(),
            gateway: None,
            at: "GLAVE_BASE_URL".into(),
            key_from: KEY,
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
    fn a_cloud_s_fixes_name_its_own_settings() {
        let mut azure = endpoint("https://acme.openai.azure.com/openai/v1", Some("az-1"));
        azure.at = "[provider] (from /srv/glave/config.toml)".into();
        azure.key_from = "AZURE_OPENAI_API_KEY";
        let fix = |failure| fix(failure, &azure).unwrap_or_default();
        assert!(fix(Failure::Refused).contains("check AZURE_OPENAI_API_KEY"));
        assert!(fix(Failure::Unreachable).contains("[provider] (from /srv/glave/config.toml)"));
        assert!(fix(Failure::UnknownModel).contains("acme.openai.azure.com"));
    }
}
