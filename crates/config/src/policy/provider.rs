//! `[provider]` as written: a cloud contract's models (`specs/gateway.md`, "Cloud contracts"),
//! and which API each model is asked through.

use serde::Deserialize;

/// `[provider]`: a cloud contract's models. The key, when there is one, comes from the
/// environment; without one, the workload's own identity.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "name", rename_all = "lowercase", deny_unknown_fields)]
pub enum ProviderFile {
    /// Azure OpenAI's v1 API; `model` is a deployment's name.
    Azure {
        /// The resource's name, such as `acme-openai`, or its `https://` endpoint.
        resource: String,
        /// The Microsoft Entra scope tokens are asked for, when not Microsoft's default.
        scope: Option<String>,
    },
    /// Amazon Bedrock: Claude through Anthropic's Messages API, other models through its
    /// OpenAI-compatible Chat Completions; `model` is a Bedrock model or inference profile id.
    Bedrock {
        /// The AWS region, such as `us-east-1`.
        region: String,
        /// `runtime` (the default, which AWS recommends) or `mantle`.
        endpoint: Option<BedrockEndpointChoice>,
        /// The API every model is asked through, when not chosen by its family.
        api: Option<ApiChoice>,
    },
    /// Google Vertex AI: Claude through Anthropic's Messages API, Gemini and the rest through
    /// its OpenAI-compatible Chat Completions; `model` as Vertex names it, such as
    /// `claude-opus-5-5` or `google/gemini-3.5-flash`.
    Vertex {
        /// The Google Cloud project's id.
        project: String,
        /// `global` (the default), a multi-region (`us`, `eu`) or a region such as `us-east5`.
        location: Option<String>,
        /// The API every model is asked through, when not chosen by its family.
        api: Option<ApiChoice>,
    },
}

impl ProviderFile {
    /// The provider's name, as `[models]` rules see it: `azure`, `bedrock` or `vertex`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Azure { .. } => "azure",
            Self::Bedrock { .. } => "bedrock",
            Self::Vertex { .. } => "vertex",
        }
    }

    /// The provider and where it is, to show: `azure (acme-openai)`,
    /// `vertex (acme-ai, global, API by model family)`.
    pub fn describe(&self) -> String {
        match self {
            Self::Azure { resource, .. } => format!("azure ({})", resource.trim()),
            Self::Bedrock {
                region,
                endpoint,
                api,
            } => format!(
                "bedrock ({}, {}, {})",
                region.trim(),
                endpoint.unwrap_or_default().name(),
                ApiChoice::describe(*api)
            ),
            Self::Vertex {
                project,
                location,
                api,
            } => format!(
                "vertex ({}, {}, {})",
                project.trim(),
                location.as_deref().map_or("global", str::trim),
                ApiChoice::describe(*api)
            ),
        }
    }
}

/// Which of Bedrock's endpoints.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BedrockEndpointChoice {
    #[default]
    Runtime,
    Mantle,
}

impl BedrockEndpointChoice {
    pub fn name(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Mantle => "mantle",
        }
    }
}

/// The API every model of a cloud is asked through, in place of choosing by model family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiChoice {
    /// Anthropic's Messages API.
    Anthropic,
    /// OpenAI's Chat Completions.
    Openai,
}

impl ApiChoice {
    /// Which API models are asked through, to show.
    fn describe(choice: Option<Self>) -> &'static str {
        match choice {
            None => "API by model family",
            Some(Self::Anthropic) => "api = anthropic",
            Some(Self::Openai) => "api = openai",
        }
    }
}
