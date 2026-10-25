//! The model provider for the endpoint: Anthropic's own Messages API at api.anthropic.com,
//! the OpenAI-compatible one anywhere else, and at a cloud that serves both, each model by the
//! API its family speaks. The harness sees one provider either way.

use glave_event::Reasoning;
use glave_ports::auth::KeyHeader;
use glave_ports::{Completion, Delta, ModelProvider, ModelRequest, ProviderError};
use glave_provider_anthropic::{Anthropic, EFFORTS};
use glave_provider_openai::{Config, OpenAiCompatible};

use crate::config::Endpoint;

/// The host whose API is Anthropic's own rather than OpenAI-compatible.
pub(crate) const ANTHROPIC_HOST: &str = "api.anthropic.com";

/// Claude's Messages API at a cloud, beside its Chat Completions, and which models go there.
#[derive(Clone)]
pub(crate) struct Claude {
    /// Where and how; its idle timeout is the endpoint's.
    pub config: glave_provider_anthropic::Config,
    pub models: Family,
}

/// Which models are asked through Anthropic's Messages API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    /// Every model (`api = "anthropic"`).
    All,
    /// Claude on Vertex AI: ids that start with `claude`, such as `claude-opus-5-5`.
    VertexClaude,
    /// Claude on Bedrock: ids and inference profiles with an `anthropic.` part, such as
    /// `anthropic.claude-opus-5-5` or `us.anthropic.claude-sonnet-5`.
    BedrockClaude,
}

impl Family {
    /// Whether `model` is asked through the Messages API.
    pub(crate) fn takes(self, model: &str) -> bool {
        let model = model.trim().to_ascii_lowercase();
        match self {
            Self::All => true,
            Self::VertexClaude => model.starts_with("claude"),
            Self::BedrockClaude => model.contains("anthropic."),
        }
    }
}

/// The adapter that speaks the endpoint's API.
pub(crate) enum Model {
    OpenAi(OpenAiCompatible),
    Anthropic(Anthropic),
    /// A cloud that serves some models by each API.
    Both {
        openai: OpenAiCompatible,
        claude: Anthropic,
        family: Family,
    },
}

impl Model {
    /// The adapter for `endpoint`.
    pub(crate) fn for_endpoint(endpoint: &Endpoint) -> Result<Self, ProviderError> {
        let config = &endpoint.provider;
        let Some(claude) = &endpoint.claude else {
            return if speaks_anthropic(config) {
                Ok(Self::Anthropic(Anthropic::new(
                    glave_provider_anthropic::Config {
                        base_url: config.base_url.clone(),
                        api: glave_provider_anthropic::Api::Messages,
                        auth: config.auth.clone().in_header(KeyHeader::XApiKey),
                        provider: config.provider.clone(),
                        idle_timeout: config.idle_timeout,
                    },
                )?))
            } else {
                Ok(Self::OpenAi(OpenAiCompatible::new(config.clone())?))
            };
        };
        let anthropic = Anthropic::new(glave_provider_anthropic::Config {
            idle_timeout: config.idle_timeout,
            ..claude.config.clone()
        })?;
        Ok(match claude.models {
            Family::All => Self::Anthropic(anthropic),
            family => Self::Both {
                openai: OpenAiCompatible::new(config.clone())?,
                claude: anthropic,
                family,
            },
        })
    }
}

impl Model {
    /// A `GET` of `path` under the endpoint's API, with its credentials, such as `models`:
    /// what it lists beside its answers.
    pub(crate) async fn get(&self, path: &str) -> Result<serde_json::Value, ProviderError> {
        match self {
            Self::OpenAi(provider)
            | Self::Both {
                openai: provider, ..
            } => provider.get(path).await,
            Self::Anthropic(provider) => provider.get(path).await,
        }
    }
}

/// Whether the endpoint is Anthropic's own API.
pub(crate) fn speaks_anthropic(config: &Config) -> bool {
    config.host().as_deref() == Some(ANTHROPIC_HOST)
}

/// How Claude reasons through `endpoint`'s Messages API, when calls for `model` go there and
/// nothing lists its levels, as at a cloud contract: Anthropic's documented levels, none sent
/// unasked, so a level the model does not take is refused by the API with its own message.
pub(crate) fn documented_reasoning(endpoint: &Endpoint, model: &str) -> Option<Reasoning> {
    let messages = match &endpoint.claude {
        Some(claude) => claude.models.takes(model),
        None => speaks_anthropic(&endpoint.provider),
    };
    messages.then(|| Reasoning {
        efforts: EFFORTS.map(str::to_owned).to_vec(),
        default: None,
        mandatory: false,
    })
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
            Self::Both {
                openai,
                claude,
                family,
            } => {
                if family.takes(&request.model) {
                    claude.complete(request, on_delta).await
                } else {
                    openai.complete(request, on_delta).await
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use glave_ports::testing::FAKE_SECRET;

    use super::*;

    fn model(url: &str) -> Model {
        let provider = Config::for_url(url, Some(FAKE_SECRET.into())).unwrap();
        let endpoint = Endpoint {
            local: false,
            provider,
            claude: None,
            model: "m".into(),
            gateway: None,
            at: "GLAVE_BASE_URL".into(),
            key_from: "GLAVE_API_KEY",
        };
        Model::for_endpoint(&endpoint).unwrap()
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

    #[test]
    fn claude_by_the_messages_api_with_no_listing_is_offered_anthropic_s_documented_levels() {
        let endpoint = |url: &str, models: Option<Family>| Endpoint {
            local: false,
            provider: Config::for_url(url, None).unwrap(),
            claude: models.map(|models| Claude {
                config: glave_provider_anthropic::Config::new(None),
                models,
            }),
            model: "m".into(),
            gateway: None,
            at: "[provider]".into(),
            key_from: "GLAVE_API_KEY",
        };
        let levels = |endpoint: &Endpoint, model: &str| {
            documented_reasoning(endpoint, model).map(|reasoning| reasoning.efforts)
        };
        let documented = Some(EFFORTS.map(str::to_owned).to_vec());
        let own = endpoint("https://api.anthropic.com", None);
        assert_eq!(levels(&own, "claude-opus-5-5"), documented);
        let bedrock = endpoint(
            "https://bedrock-mantle.us-east-1.api.aws/v1",
            Some(Family::BedrockClaude),
        );
        assert_eq!(levels(&bedrock, "us.anthropic.claude-opus-5-5"), documented);
        assert_eq!(
            levels(&bedrock, "openai.gpt-oss-120b-1:0"),
            None,
            "Chat Completions"
        );
        let vertex = endpoint(
            "https://aiplatform.googleapis.com/v1",
            Some(Family::VertexClaude),
        );
        assert_eq!(levels(&vertex, "claude-haiku-5-5"), documented);
        let router = endpoint("https://openrouter.ai/api/v1", None);
        assert_eq!(levels(&router, "anthropic/claude-haiku-5.5"), None);
    }

    #[test]
    fn claude_is_told_apart_by_its_family_at_each_cloud() {
        let vertex = Family::VertexClaude;
        assert!(vertex.takes("claude-opus-5-5"));
        assert!(vertex.takes("claude-haiku-4-5@20251001"));
        assert!(!vertex.takes("google/gemini-3.5-flash"));
        assert!(!vertex.takes("meta/llama-4-maverick"));
        let bedrock = Family::BedrockClaude;
        assert!(bedrock.takes("anthropic.claude-opus-5-5"));
        assert!(bedrock.takes("us.anthropic.claude-sonnet-5"));
        assert!(bedrock.takes(
            "arn:aws:bedrock:us-east-1:123456789012:inference-profile/global.anthropic.claude-opus-5-5"
        ));
        assert!(!bedrock.takes("openai.gpt-oss-120b-1:0"));
        assert!(!bedrock.takes("claude-like-but-not-bedrock"));
        assert!(Family::All.takes("google/gemini-3.5-flash"));
    }

    #[test]
    fn at_a_cloud_claude_goes_by_messages_and_the_rest_by_chat_completions() {
        use glave_event::Part;
        use glave_ports::{Message, ToolChoice};
        use glave_provider_anthropic::fake as messages;
        use glave_provider_openai::fake as chat;

        let gemini = chat::FakeServer::start(vec![chat::Reply::text("from gemini")]).unwrap();
        let claude =
            messages::FakeServer::start(vec![messages::Reply::text("from claude")]).unwrap();
        let mut provider = Config::for_url(gemini.url(), Some("ya29.t".into())).unwrap();
        "vertex".clone_into(&mut provider.provider);
        let endpoint = Endpoint {
            local: false,
            claude: Some(Claude {
                config: glave_provider_anthropic::Config {
                    base_url: format!("{}/v1/projects/p/locations/global", claude.url()),
                    api: glave_provider_anthropic::Api::Vertex,
                    auth: provider.auth.clone(),
                    provider: "vertex".into(),
                    idle_timeout: std::time::Duration::from_secs(1),
                },
                models: Family::VertexClaude,
            }),
            provider,
            model: "google/gemini-3.5-flash".into(),
            gateway: None,
            at: "[provider]".into(),
            key_from: "GLAVE_API_KEY",
        };
        let both = Model::for_endpoint(&endpoint).unwrap();
        let ask = |model: &str| {
            let request = ModelRequest {
                model: model.into(),
                system: String::new(),
                messages: vec![Message::User {
                    parts: vec![Part::Text { text: "hi".into() }],
                }],
                tools: Vec::new(),
                tool_choice: ToolChoice::Auto,
                max_output_tokens: None,
                purpose: glave_ports::Purpose::Main,
                threads: Vec::new(),
                files: Vec::new(),
                effort: None,
            };
            let completion = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(both.complete(&request, &mut |_| {}))
                .unwrap();
            completion.parts
        };
        let text = |text: &str| vec![Part::Text { text: text.into() }];
        assert_eq!(ask("claude-opus-5-5"), text("from claude"));
        assert_eq!(ask("google/gemini-3.5-flash"), text("from gemini"));
        assert!(
            claude.received()[0]
                .path
                .ends_with("/models/claude-opus-5-5:streamRawPredict")
        );
        assert_eq!(
            gemini.received()[0].body["model"],
            "google/gemini-3.5-flash"
        );
    }
}
