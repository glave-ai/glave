//! A cloud contract's models, as `[provider]` names them: Azure OpenAI or Amazon Bedrock, each
//! through its OpenAI-compatible API, with a key from the environment or, keyless, the
//! workload's own identity (`specs/gateway.md`, "Cloud contracts").

use std::sync::Arc;

use anyhow::{Context as _, bail};
use glave_aws::{Endpoint as BedrockEndpoint, Region, SigV4, Source as AwsSource};
use glave_azure::{DEFAULT_SCOPE, WorkloadIdentity};
use glave_config::policy::{BedrockEndpointChoice, Chosen, ProviderFile, Source};
use glave_ports::SystemClock;
use glave_ports::auth::{Auth, KeyHeader};
use glave_provider_openai::{Config, IDLE_TIMEOUT};

/// Variables that hold an Azure OpenAI key, in the order they are read.
const AZURE_KEYS: &[&str] = &["GLAVE_API_KEY", "AZURE_OPENAI_API_KEY"];

/// Variables that hold a Bedrock API key, in the order they are read.
const BEDROCK_KEYS: &[&str] = &["GLAVE_API_KEY", "AWS_BEARER_TOKEN_BEDROCK"];

/// Where a provider is, how it is reached, and the variable its key came from, if a key did.
struct Reached {
    base_url: String,
    auth: Auth,
    key_from: Option<&'static str>,
}

/// The provider `[provider]` names, with credentials from `env`, and the variable its key
/// came from, if a key did.
pub fn provider(
    chosen: &Chosen<ProviderFile>,
    env: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<(Config, Option<&'static str>)> {
    let set = |name: &str| env(name).filter(|value| !value.trim().is_empty());
    let reached = match &chosen.value {
        ProviderFile::Azure { resource, scope } => {
            azure(resource, scope.as_deref(), &chosen.from, &set)?
        }
        ProviderFile::Bedrock { region, endpoint } => {
            bedrock(region, endpoint.unwrap_or_default(), &chosen.from, &set)?
        }
    };
    let config = Config {
        base_url: reached.base_url,
        auth: reached.auth,
        provider: chosen.value.name().to_owned(),
        idle_timeout: IDLE_TIMEOUT,
    };
    Ok((config, reached.key_from))
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
            base_url,
            auth: Auth::key(KeyHeader::ApiKey, key),
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
        base_url,
        auth: Auth::Changing(Arc::new(identity)),
        key_from: None,
    })
}

/// Bedrock's OpenAI-compatible API in `region`: a Bedrock API key, else SigV4 with keys from
/// the environment, IRSA or Pod Identity.
fn bedrock(
    region: &str,
    endpoint: BedrockEndpointChoice,
    from: &Source,
    set: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<Reached> {
    let region = Region::new(region).with_context(|| format!("[provider] region (from {from})"))?;
    let endpoint = match endpoint {
        BedrockEndpointChoice::Runtime => BedrockEndpoint::Runtime,
        BedrockEndpointChoice::Mantle => BedrockEndpoint::Mantle,
    };
    let base_url = endpoint.base_url(&region);
    if let Some((name, key)) = key(BEDROCK_KEYS, set) {
        return Ok(Reached {
            base_url,
            auth: Auth::bearer(Some(key)),
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
    Ok(Reached {
        base_url,
        auth: Auth::Changing(Arc::new(signer)),
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

    fn reach(text: &str, vars: &[(&str, &str)]) -> anyhow::Result<(Config, Option<&'static str>)> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        provider(&chosen(text), &|name| vars.get(name).cloned())
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
}
