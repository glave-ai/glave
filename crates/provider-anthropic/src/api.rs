//! Which Messages API a server speaks: Anthropic's own, or Vertex AI's rawPredict, which takes
//! the same request with the model in the URL and the version in the body.

use glave_ports::ProviderError;

/// The API version this provider speaks on Anthropic's own API and those that copy it.
const VERSION: &str = "2023-06-01";

/// The API version Vertex AI wants in the body.
const VERTEX_VERSION: &str = "vertex-2023-10-16";

/// Where each request goes, and how its version and model are sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Api {
    /// `<base>/v1/messages`, the version in `anthropic-version`, the model in the body:
    /// Anthropic's own API, Amazon Bedrock's `/anthropic` route, and gateways that copy it.
    #[default]
    Messages,
    /// Vertex AI: `<base>/publishers/anthropic/models/<model>:streamRawPredict`, where the
    /// base is a project's location (`https://<host>/v1/projects/<p>/locations/<l>`); the
    /// version (`vertex-2023-10-16`) in the body and no model there.
    Vertex,
}

impl Api {
    /// Where a request for `model` goes, under `base`.
    pub(crate) fn url(self, base: &str, model: &str) -> Result<String, ProviderError> {
        let base = base.trim_end_matches('/');
        match self {
            Self::Messages => {
                let base = base.strip_suffix("/v1").unwrap_or(base);
                Ok(format!("{base}/v1/messages"))
            }
            Self::Vertex => {
                let named = !model.is_empty()
                    && model.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'@')
                    });
                if !named {
                    return Err(ProviderError::Http {
                        status: 404,
                        body: format!(
                            "not sent: {model:?} is not a Vertex AI model id, such as claude-opus-5-5"
                        ),
                    });
                }
                Ok(format!(
                    "{base}/publishers/anthropic/models/{model}:streamRawPredict"
                ))
            }
        }
    }

    /// The `anthropic-version` header, for the APIs that take it there.
    pub(crate) fn version_header(self) -> Option<&'static str> {
        match self {
            Self::Messages => Some(VERSION),
            Self::Vertex => None,
        }
    }

    /// The version sent in the body, for the APIs that take it there.
    pub(crate) fn version_in_body(self) -> Option<&'static str> {
        match self {
            Self::Messages => None,
            Self::Vertex => Some(VERTEX_VERSION),
        }
    }

    /// Whether the body names the model.
    pub(crate) fn model_in_body(self) -> bool {
        matches!(self, Self::Messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_messages_endpoint_is_found_with_or_without_v1() {
        for base in [
            "https://api.anthropic.com",
            "https://api.anthropic.com/",
            "https://api.anthropic.com/v1",
            "https://api.anthropic.com/v1/",
        ] {
            assert_eq!(
                Api::Messages.url(base, "claude-haiku-5-5").unwrap(),
                "https://api.anthropic.com/v1/messages"
            );
        }
    }

    #[test]
    fn vertex_names_the_model_in_the_url() {
        let base = "https://aiplatform.googleapis.com/v1/projects/acme-ai/locations/global";
        assert_eq!(
            Api::Vertex.url(base, "claude-opus-5-5").unwrap(),
            "https://aiplatform.googleapis.com/v1/projects/acme-ai/locations/global/publishers/anthropic/models/claude-opus-5-5:streamRawPredict"
        );
        assert!(Api::Vertex.url(base, "claude-haiku-4-5@20251001").is_ok());
        for model in ["", "claude/../x", "claude opus", "a?b", "a:rawPredict"] {
            let error = Api::Vertex.url(base, model).unwrap_err();
            assert_eq!(
                error.failure(),
                glave_ports::Failure::UnknownModel,
                "{model}"
            );
        }
    }
}
