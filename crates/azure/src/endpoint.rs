//! Where a resource's v1 API is: `https://<resource>.openai.azure.com/openai/v1`, or the
//! `/openai/v1` of an endpoint given whole, such as `https://<name>.services.ai.azure.com`.

use thiserror::Error;

/// A resource or endpoint that cannot be an Azure OpenAI base URL.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EndpointError {
    /// An `https://` address with a path, or a user name or password, in it. Not shown: it
    /// may hold a password.
    #[error(
        "the endpoint is not an Azure endpoint: give its address alone, such as \
         https://acme.services.ai.azure.com"
    )]
    Endpoint,
    #[error(
        "{0:?} is not an Azure OpenAI resource name (letters, digits and hyphens) nor an \
         https:// endpoint"
    )]
    Resource(String),
}

/// The v1 API's base URL for `resource`: either a resource's name, such as `acme-openai`, or
/// an `https://` endpoint, with or without `/openai/v1`.
pub fn base_url(resource: &str) -> Result<String, EndpointError> {
    let resource = resource.trim();
    if let Some(rest) = resource.strip_prefix("https://") {
        let rest = rest.trim_end_matches('/');
        let rest = rest.strip_suffix("/openai/v1").unwrap_or(rest);
        if rest.is_empty() || rest.contains(['@', '/', '?', '#']) {
            return Err(EndpointError::Endpoint);
        }
        return Ok(format!("https://{rest}/openai/v1"));
    }
    let named = (2..=64).contains(&resource.len())
        && resource
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !resource.starts_with('-')
        && !resource.ends_with('-');
    if !named {
        return Err(EndpointError::Resource(resource.to_owned()));
    }
    Ok(format!("https://{resource}.openai.azure.com/openai/v1"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resource_name_becomes_its_v1_endpoint() {
        assert_eq!(
            base_url("acme-openai").unwrap(),
            "https://acme-openai.openai.azure.com/openai/v1"
        );
    }

    #[test]
    fn an_endpoint_given_whole_gets_the_v1_path_once() {
        for given in [
            "https://acme.services.ai.azure.com",
            "https://acme.services.ai.azure.com/",
            "https://acme.services.ai.azure.com/openai/v1",
            "https://acme.services.ai.azure.com/openai/v1/",
        ] {
            assert_eq!(
                base_url(given).unwrap(),
                "https://acme.services.ai.azure.com/openai/v1",
                "{given}"
            );
        }
    }

    #[test]
    fn anything_else_is_refused_saying_what_is_wanted() {
        for given in [
            "",
            "a",
            "-acme",
            "acme_openai",
            "acme.openai.azure.com",
            "http://acme.openai.azure.com",
            "https://",
            "https://me:pw@acme.openai.azure.com",
            "https://acme.openai.azure.com/openai/deployments/x",
        ] {
            let error = base_url(given).unwrap_err().to_string();
            assert!(error.contains("not an Azure"), "{given}: {error}");
            assert!(!error.contains("pw@"), "{error}");
        }
    }
}
