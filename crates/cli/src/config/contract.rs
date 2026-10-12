//! A cloud contract's models, as `[provider]` names them: Azure OpenAI, Amazon Bedrock or
//! Google Vertex AI, each through its OpenAI-compatible API and, for Claude at Bedrock and
//! Vertex, Anthropic's Messages API; with a key from the environment or, keyless, the
//! workload's own identity (`specs/gateway.md`, "Cloud contracts").

use std::sync::Arc;

use anyhow::{Context as _, bail};
use glave_aws::{Endpoint as BedrockEndpoint, Region, SigV4, Source as AwsSource};
use glave_azure::{DEFAULT_SCOPE, WorkloadIdentity};
use glave_config::policy::{ApiChoice, BedrockEndpointChoice, Chosen, ProviderFile, Source};
use glave_google::{Adc, Vertex};
use glave_ports::SystemClock;
use glave_ports::auth::{Auth, KeyHeader};
use glave_provider_anthropic::Api;
use glave_provider_openai::{Config, IDLE_TIMEOUT};

use crate::model::{Claude, Family};

/// Variables that hold an Azure OpenAI key, in the order they are read.
const AZURE_KEYS: &[&str] = &["GLAVE_API_KEY", "AZURE_OPENAI_API_KEY"];

/// Variables that hold a Bedrock API key, in the order they are read.
const BEDROCK_KEYS: &[&str] = &["GLAVE_API_KEY", "AWS_BEARER_TOKEN_BEDROCK"];

/// How a cloud's models are reached.
pub struct Reached {
    /// Its OpenAI-compatible Chat Completions.
    pub provider: Config,
    /// Anthropic's Messages API there, for the models whose family speaks it.
    pub claude: Option<Claude>,
    /// The variable the key came from, if a key did.
    pub key_from: Option<&'static str>,
}

/// The provider `[provider]` names, with credentials from `env`.
pub fn provider(
    chosen: &Chosen<ProviderFile>,
    env: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<Reached> {
    let set = |name: &str| env(name).filter(|value| !value.trim().is_empty());
    let name = chosen.value.name();
    let from = &chosen.from;
    match &chosen.value {
        ProviderFile::Azure { resource, scope } => azure(resource, scope.as_deref(), from, &set),
        ProviderFile::Bedrock {
            region,
            endpoint,
            api,
        } => bedrock(region, endpoint.unwrap_or_default(), *api, from, &set),
        ProviderFile::Vertex {
            project,
            location,
            api,
        } => vertex(project, location.as_deref(), *api, from, &set),
    }
    .map(|mut reached| {
        name.clone_into(&mut reached.provider.provider);
        if let Some(claude) = &mut reached.claude {
            name.clone_into(&mut claude.config.provider);
        }
        reached
    })
}

/// Chat Completions at `base_url`.
fn chat(base_url: String, auth: Auth) -> Config {
    Config {
        base_url,
        auth,
        provider: String::new(),
        idle_timeout: IDLE_TIMEOUT,
    }
}

/// Claude's Messages API at `base_url`, for the models `family` names, unless `api` sends
/// every model through Chat Completions.
fn claude(
    base_url: String,
    api: Api,
    auth: Auth,
    family: Family,
    choice: Option<ApiChoice>,
) -> Option<Claude> {
    let models = match choice {
        None => family,
        Some(ApiChoice::Anthropic) => Family::All,
        Some(ApiChoice::Openai) => return None,
    };
    let config = glave_provider_anthropic::Config {
        base_url,
        api,
        auth,
        ..glave_provider_anthropic::Config::new(None)
    };
    Some(Claude { config, models })
}

/// The first of `names` that holds a key, and the key.
fn key(
    names: &[&'static str],
    set: &dyn Fn(&str) -> Option<String>,
) -> Option<(&'static str, String)> {
    names
        .iter()
        .find_map(|name| set(name).map(|key| (*name, key.trim().to_owned())))
}

/// Azure OpenAI's v1 API at `resource`: an `api-key`, else Entra workload identity.
fn azure(
    resource: &str,
    scope: Option<&str>,
    from: &Source,
    set: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<Reached> {
    let base_url = glave_azure::base_url(resource)
        .with_context(|| format!("[provider] resource (from {from})"))?;
    if let Some((name, key)) = key(AZURE_KEYS, set) {
        return Ok(Reached {
            provider: chat(base_url, Auth::key(KeyHeader::ApiKey, key)),
            claude: None,
            key_from: Some(name),
        });
    }
    let scope = scope.unwrap_or(DEFAULT_SCOPE);
    let Some(identity) = WorkloadIdentity::from_env(set, scope, Arc::new(SystemClock))? else {
        bail!(
            "[provider] name = \"azure\" (from {from}) needs a key or a workload identity: set \
             AZURE_OPENAI_API_KEY, or run on AKS with workload identity \
             (AZURE_FEDERATED_TOKEN_FILE)"
        );
    };
    Ok(Reached {
        provider: chat(base_url, Auth::Changing(Arc::new(identity))),
        claude: None,
        key_from: None,
    })
}

/// Bedrock in `region`: Chat Completions, and Claude at the `/anthropic` route; a Bedrock API
/// key, else SigV4 with keys from the environment, IRSA or Pod Identity.
fn bedrock(
    region: &str,
    endpoint: BedrockEndpointChoice,
    api: Option<ApiChoice>,
    from: &Source,
    set: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<Reached> {
    let region = Region::new(region).with_context(|| format!("[provider] region (from {from})"))?;
    let endpoint = match endpoint {
        BedrockEndpointChoice::Runtime => BedrockEndpoint::Runtime,
        BedrockEndpointChoice::Mantle => BedrockEndpoint::Mantle,
    };
    let (base_url, messages) = (
        endpoint.base_url(&region),
        endpoint.anthropic_base_url(&region),
    );
    if let Some((name, key)) = key(BEDROCK_KEYS, set) {
        let auth = Auth::bearer(Some(key));
        let claude_auth = auth.clone().in_header(KeyHeader::XApiKey);
        return Ok(Reached {
            provider: chat(base_url, auth),
            claude: claude(
                messages,
                Api::Messages,
                claude_auth,
                Family::BedrockClaude,
                api,
            ),
            key_from: Some(name),
        });
    }
    let Some(source) = AwsSource::from_env(set, &region)? else {
        bail!(
            "[provider] name = \"bedrock\" (from {from}) needs a Bedrock API key or AWS \
             credentials: set AWS_BEARER_TOKEN_BEDROCK, or run with IRSA, EKS Pod Identity or \
             AWS_ACCESS_KEY_ID"
        );
    };
    let signer = SigV4::new(source, region, endpoint, Arc::new(SystemClock))?;
    let auth = Auth::Changing(Arc::new(signer));
    Ok(Reached {
        provider: chat(base_url, auth.clone()),
        claude: claude(messages, Api::Messages, auth, Family::BedrockClaude, api),
        key_from: None,
    })
}

/// Vertex AI's `project` at `location`: Chat Completions, and Claude through rawPredict, with
/// Application Default Credentials (on GKE, the pod's workload identity).
fn vertex(
    project: &str,
    location: Option<&str>,
    api: Option<ApiChoice>,
    from: &Source,
    set: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<Reached> {
    let vertex = Vertex::new(project, location.unwrap_or("global"))
        .with_context(|| format!("[provider] project or location (from {from})"))?;
    let adc = Adc::from_env(set, glave_google::SCOPE, Arc::new(SystemClock))?;
    let auth = Auth::Changing(Arc::new(adc));
    Ok(Reached {
        provider: chat(vertex.openai_base_url(), auth.clone()),
        claude: claude(vertex.base(), Api::Vertex, auth, Family::VertexClaude, api),
        key_from: None,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn chosen(text: &str) -> Chosen<ProviderFile> {
        let path = std::path::Path::new("/srv/glave/config.toml");
        Chosen {
            value: glave_config::policy::config(text, path)
                .unwrap()
                .provider
                .unwrap(),
            from: Source::File(path.into()),
        }
    }

    fn reached(text: &str, vars: &[(&str, &str)]) -> anyhow::Result<Reached> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        provider(&chosen(text), &|name| vars.get(name).cloned())
    }

    fn reach(text: &str, vars: &[(&str, &str)]) -> anyhow::Result<(Config, Option<&'static str>)> {
        reached(text, vars).map(|reached| (reached.provider, reached.key_from))
    }

    const AZURE: &str = "[provider]\nname = \"azure\"\nresource = \"acme-openai\"\n";
    const BEDROCK: &str = "[provider]\nname = \"bedrock\"\nregion = \"eu-west-1\"\n";

    #[test]
    fn azure_takes_a_key_in_api_key_or_its_workload_identity() {
        let (config, from) = reach(AZURE, &[("AZURE_OPENAI_API_KEY", "az-1")]).unwrap();
        assert_eq!(
            config.base_url,
            "https://acme-openai.openai.azure.com/openai/v1"
        );
        assert_eq!(config.provider, "azure");
        assert_eq!(from, Some("AZURE_OPENAI_API_KEY"));
        assert!(matches!(
            config.auth,
            Auth::Key {
                header: KeyHeader::ApiKey,
                ..
            }
        ));
        let workload = [
            ("AZURE_FEDERATED_TOKEN_FILE", "/var/run/secrets/azure/token"),
            ("AZURE_CLIENT_ID", "client-1"),
            ("AZURE_TENANT_ID", "tenant-1"),
        ];
        let (config, from) = reach(AZURE, &workload).unwrap();
        assert_eq!(from, None);
        assert!(
            config.auth.describe().contains(
                "workload identity (client client-1, scope https://ai.azure.com/.default)"
            )
        );
        let scoped = format!("{AZURE}scope = \"https://cognitiveservices.azure.com/.default\"\n");
        let (config, _) = reach(&scoped, &workload).unwrap();
        assert!(config.auth.describe().contains("cognitiveservices"));
        let neither = reach(AZURE, &[]).err().unwrap().to_string();
        assert!(
            neither.contains("AZURE_OPENAI_API_KEY") && neither.contains("/srv/glave/config.toml"),
            "{neither}"
        );
    }

    #[test]
    fn bedrock_takes_an_api_key_as_a_bearer_or_signs_with_aws_keys() {
        let (config, from) = reach(BEDROCK, &[("AWS_BEARER_TOKEN_BEDROCK", "bedrock-1")]).unwrap();
        assert_eq!(
            config.base_url,
            "https://bedrock-runtime.eu-west-1.amazonaws.com/openai/v1"
        );
        assert_eq!(config.provider, "bedrock");
        assert_eq!(from, Some("AWS_BEARER_TOKEN_BEDROCK"));
        assert!(matches!(
            config.auth,
            Auth::Key {
                header: KeyHeader::Bearer,
                ..
            }
        ));
        let irsa = [
            ("AWS_WEB_IDENTITY_TOKEN_FILE", "/var/run/irsa/token"),
            ("AWS_ROLE_ARN", "arn:aws:iam::123456789012:role/glave"),
        ];
        let mantle = format!("{BEDROCK}endpoint = \"mantle\"\n");
        let (config, from) = reach(&mantle, &irsa).unwrap();
        assert_eq!(from, None);
        assert_eq!(
            config.base_url,
            "https://bedrock-mantle.eu-west-1.api.aws/v1"
        );
        let described = config.auth.describe();
        assert!(
            described.contains("SigV4 for bedrock-mantle in eu-west-1")
                && described.contains("role/glave"),
            "{described}"
        );
        let neither = reach(BEDROCK, &[]).err().unwrap().to_string();
        assert!(neither.contains("AWS_BEARER_TOKEN_BEDROCK"), "{neither}");
        let wrong = "[provider]\nname = \"bedrock\"\nregion = \"Frankfurt\"\n";
        let refused = format!("{:#}", reach(wrong, &[]).err().unwrap());
        assert!(refused.contains("[provider] region"), "{refused}");
    }

    #[test]
    fn glave_s_own_key_variable_comes_first() {
        let both = [("GLAVE_API_KEY", "g-1"), ("AZURE_OPENAI_API_KEY", "az-1")];
        let (config, from) = reach(AZURE, &both).unwrap();
        assert_eq!(from, Some("GLAVE_API_KEY"));
        assert_eq!(
            config
                .auth
                .fixed_key()
                .map(glave_ports::auth::Secret::expose),
            Some("g-1")
        );
    }

    const VERTEX: &str = "[provider]\nname = \"vertex\"\nproject = \"acme-ai\"\n";

    #[test]
    fn vertex_reaches_gemini_and_claude_with_application_default_credentials() {
        let gke = [("GCE_METADATA_HOST", "metadata.google.internal")];
        let both = reached(VERTEX, &gke).unwrap();
        assert_eq!(
            both.provider.base_url,
            "https://aiplatform.googleapis.com/v1/projects/acme-ai/locations/global/endpoints/openapi"
        );
        assert_eq!(both.provider.provider, "vertex");
        assert_eq!(both.key_from, None);
        assert!(
            both.provider
                .auth
                .describe()
                .contains("Application Default Credentials"),
            "{}",
            both.provider.auth.describe()
        );
        let claude = both.claude.unwrap();
        assert_eq!(
            claude.config.base_url,
            "https://aiplatform.googleapis.com/v1/projects/acme-ai/locations/global"
        );
        assert_eq!(claude.config.api, Api::Vertex);
        assert_eq!(claude.config.provider, "vertex");
        assert_eq!(claude.models, Family::VertexClaude);
        let regional = format!("{VERTEX}location = \"us-east5\"\napi = \"anthropic\"\n");
        let claude = reached(&regional, &gke).unwrap().claude.unwrap();
        assert!(
            claude
                .config
                .base_url
                .starts_with("https://us-east5-aiplatform.googleapis.com/")
        );
        assert_eq!(claude.models, Family::All);
        let openai = format!("{VERTEX}api = \"openai\"\n");
        assert!(reached(&openai, &gke).unwrap().claude.is_none());
        let file = [("GOOGLE_APPLICATION_CREDENTIALS", "/var/run/google/sa.json")];
        let named = reached(VERTEX, &file).unwrap();
        assert!(
            named
                .provider
                .auth
                .describe()
                .contains("/var/run/google/sa.json")
        );
        let wrong = "[provider]\nname = \"vertex\"\nproject = \"Acme AI\"\n";
        let refused = format!("{:#}", reached(wrong, &gke).err().unwrap());
        assert!(
            refused.contains("[provider] project or location"),
            "{refused}"
        );
    }

    #[test]
    fn claude_on_bedrock_shares_the_signer_or_takes_its_key_in_x_api_key() {
        let keyed = reached(BEDROCK, &[("AWS_BEARER_TOKEN_BEDROCK", "bedrock-1")]).unwrap();
        let claude = keyed.claude.unwrap();
        assert_eq!(
            claude.config.base_url,
            "https://bedrock-runtime.eu-west-1.amazonaws.com/anthropic"
        );
        assert_eq!(claude.config.api, Api::Messages);
        assert_eq!(claude.models, Family::BedrockClaude);
        assert_eq!(claude.config.provider, "bedrock");
        assert!(matches!(
            claude.config.auth,
            Auth::Key {
                header: KeyHeader::XApiKey,
                ..
            }
        ));
        let keys = [
            ("AWS_ACCESS_KEY_ID", "AKIDEXAMPLE"),
            ("AWS_SECRET_ACCESS_KEY", "wJalrXUtnFEMI"),
        ];
        let mantle = format!("{BEDROCK}endpoint = \"mantle\"\napi = \"anthropic\"\n");
        let claude = reached(&mantle, &keys).unwrap().claude.unwrap();
        assert_eq!(
            claude.config.base_url,
            "https://bedrock-mantle.eu-west-1.api.aws/anthropic"
        );
        assert_eq!(claude.models, Family::All);
        assert!(
            claude
                .config
                .auth
                .describe()
                .contains("SigV4 for bedrock-mantle"),
            "{}",
            claude.config.auth.describe()
        );
        let openai = format!("{BEDROCK}api = \"openai\"\n");
        assert!(reached(&openai, &keys).unwrap().claude.is_none());
    }

    #[test]
    fn azure_has_no_claude() {
        let azure = reached(AZURE, &[("AZURE_OPENAI_API_KEY", "az-1")]).unwrap();
        assert!(azure.claude.is_none());
    }
}
