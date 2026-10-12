//! The model provider for the endpoint: Anthropic's own Messages API at api.anthropic.com,
//! the OpenAI-compatible one anywhere else. The harness sees one provider either way.

use glave_ports::auth::KeyHeader;
use glave_ports::{Completion, Delta, ModelProvider, ModelRequest, ProviderError};
use glave_provider_anthropic::Anthropic;
use glave_provider_openai::{Config, OpenAiCompatible};

/// The host whose API is Anthropic's own rather than OpenAI-compatible.
pub const ANTHROPIC_HOST: &str = "api.anthropic.com";

/// The adapter that speaks the endpoint's API.
pub enum Model {
    OpenAi(OpenAiCompatible),
    Anthropic(Anthropic),
}

impl Model {
    /// The adapter for the endpoint `config` describes.
    pub fn for_endpoint(config: &Config) -> Result<Self, ProviderError> {
        if speaks_anthropic(config) {
            Ok(Self::Anthropic(Anthropic::new(
                glave_provider_anthropic::Config {
                    base_url: config.base_url.clone(),
                    auth: config.auth.clone().in_header(KeyHeader::XApiKey),
                    provider: config.provider.clone(),
                    idle_timeout: config.idle_timeout,
                },
            )?))
        } else {
            Ok(Self::OpenAi(OpenAiCompatible::new(config.clone())?))
        }
    }
}

/// Whether the endpoint is Anthropic's own API.
pub fn speaks_anthropic(config: &Config) -> bool {
    config.host().as_deref() == Some(ANTHROPIC_HOST)
}

impl ModelProvider for Model {
    async fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<Completion, ProviderError> {
        match self {
            Self::OpenAi(provider) => provider.complete(request, on_delta).await,
            Self::Anthropic(provider) => provider.complete(request, on_delta).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(url: &str) -> Model {
        let config = Config::for_url(url, Some("sk-test".into())).unwrap();
        Model::for_endpoint(&config).unwrap()
    }

    #[test]
    fn only_anthropic_s_own_host_gets_the_messages_api() {
        assert!(matches!(
            model("https://api.anthropic.com"),
            Model::Anthropic(_)
        ));
        assert!(matches!(
            model("https://API.anthropic.com/v1"),
            Model::Anthropic(_)
        ));
        assert!(matches!(
            model("https://openrouter.ai/api/v1"),
            Model::OpenAi(_)
        ));
        assert!(matches!(
            model("http://localhost:11434/v1"),
            Model::OpenAi(_)
        ));
    }
}
