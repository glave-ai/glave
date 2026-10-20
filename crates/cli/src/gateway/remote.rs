//! Client mode (`specs/gateway.md`, "Client mode"): with `[gateway] url` in the person's
//! `config.toml`, glave sends its model calls to the organization's gateway, signed in as the
//! person, and takes the gateway's tool catalog as one more MCP source. Keys stay with the
//! gateway; the person holds only their sign-in. Without it nothing changes.

use std::collections::BTreeMap;

use anyhow::{Context as _, bail};
use glave_mcp::{Limits, McpServer};
use glave_provider_openai::Config;

use crate::config::Endpoint;
use glave_config::policy::Chosen;

/// The variable that holds the person's sign-in for the gateway: an OIDC access token.
pub(crate) const TOKEN: &str = "GLAVE_GATEWAY_TOKEN";

/// What glave calls the gateway: the provider its answers are recorded under, and the name of
/// its catalog among the MCP servers.
pub(crate) const NAME: &str = "gateway";

/// The model asked for when the person chose none: the gateway's main route.
const MAIN: &str = "main";

/// A gateway elsewhere, and the person's sign-in for it.
pub(crate) struct Remote {
    url: String,
    token: String,
}

impl Remote {
    /// The gateway `chosen` names, signed in with the token in `GLAVE_GATEWAY_TOKEN`.
    pub(crate) fn from_choice(chosen: &Chosen<String>) -> anyhow::Result<Self> {
        let token = std::env::var(TOKEN)
            .ok()
            .map(|token| token.trim().to_owned())
            .filter(|token| !token.is_empty());
        let Some(token) = token else {
            bail!(
                "[gateway] url (from {}) sends model calls to {}, signed in as you: set {TOKEN} \
                 to your sign-in, an OIDC access token for the gateway",
                chosen.from,
                chosen.value
            );
        };
        Ok(Self {
            url: chosen.value.trim_end_matches('/').to_owned(),
            token,
        })
    }

    /// The endpoint for `model` at the gateway, or its main route when none is chosen. The
    /// gateway speaks OpenAI's Chat Completions, so the same adapter reaches it.
    pub(crate) fn endpoint(&self, model: Option<String>) -> anyhow::Result<Endpoint> {
        let base_url = format!("{}/v1", self.url);
        let mut provider = Config::for_url(base_url.clone(), Some(self.token.clone()))
            .with_context(|| format!("[gateway] url is not a URL: {}", self.url))?;
        NAME.clone_into(&mut provider.provider);
        Ok(Endpoint {
            local: false,
            provider,
            claude: None,
            model: model.unwrap_or_else(|| MAIN.to_owned()),
            gateway: Some(self.url.clone()),
            at: "[gateway] url".to_owned(),
            key_from: TOKEN,
        })
    }

    /// The gateway's tool catalog, as one MCP server whose tools reach the model named as
    /// if each catalog server were the person's own: `mcp__<server>__<tool>`.
    pub(crate) fn catalog(&self) -> McpServer {
        let headers =
            BTreeMap::from([("Authorization".to_owned(), format!("Bearer {}", self.token))]);
        McpServer::catalog(
            NAME,
            &format!("{}/mcp", self.url),
            &headers,
            Limits::default(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_config::policy::Source;

    #[test]
    fn the_gateway_is_reached_at_its_v1_with_the_sign_in_as_the_key() {
        let remote = Remote {
            url: "https://gw.example.com".into(),
            token: "t0k".into(),
        };
        let endpoint = remote.endpoint(None).unwrap();
        assert_eq!(endpoint.provider.base_url, "https://gw.example.com/v1");
        assert_eq!(
            endpoint
                .provider
                .auth
                .fixed_key()
                .map(glave_ports::auth::Secret::expose),
            Some("t0k")
        );
        assert_eq!(endpoint.provider.provider, "gateway");
        assert_eq!(endpoint.model, "main", "the gateway's main route");
        assert_eq!(remote.endpoint(Some("big".into())).unwrap().model, "big");
        let chosen = Chosen {
            value: "https://gw.example.com".to_owned(),
            from: Source::Default,
        };
        if std::env::var_os(TOKEN).is_none() {
            let refused = Remote::from_choice(&chosen).err().unwrap().to_string();
            assert!(refused.contains(TOKEN), "{refused}");
        }
    }
}
