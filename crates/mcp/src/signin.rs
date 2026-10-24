//! Signing a person in to a remote MCP server as MCP's authorization says, through rmcp's
//! own: the server's protected resource metadata (RFC 9728), found from its `401` or at its
//! well-known place, names its authorization server, whose metadata (RFC 8414, or OpenID
//! Connect discovery) gives its endpoints. glave registers itself there as a public client
//! (dynamic client registration, RFC 7591), sends the person to authorize with PKCE (S256)
//! and the resource the token is for (RFC 8707), trades the code their browser brings back
//! for a token, and renews it with its refresh token. A server that publishes no metadata
//! does not sign anyone in this way.

use std::time::Duration;

use glave_event::{Secret, SignInClient};
use rmcp::transport::auth::{
    AuthError, AuthorizationManager, AuthorizationRequest, AuthorizationSession,
    CredentialStore as _, InMemoryCredentialStore, OAuthTokenResponse, StoredCredentials,
};
use serde_json::{Value, json};
use thiserror::Error;

/// Why a sign-in could not start, finish or be renewed, in rmcp's words, for the log. Never
/// holds a token.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SignInError {
    /// The server publishes nothing that says how to sign in to it.
    #[error("{url} does not ask anyone to sign in, as an MCP server that needs it does")]
    NotOffered { url: String },
    /// The server or its authorization server could not be reached, or what it publishes
    /// could not be used: asking again may work, and a refresh token is unspent.
    #[error("could not reach the sign-in for {url}: {message}")]
    Unreachable { url: String, message: String },
    /// The authorization server refused: the registration, the code, or the refresh token;
    /// or it is no longer the one that signed the person in.
    #[error("the sign-in for {url} was refused: {message}")]
    Refused { url: String, message: String },
    /// The authorization server takes no registration, which glave signs in with. Asking
    /// again does not help.
    #[error("glave cannot sign in to {url} yet: {message}")]
    Unregistered { url: String, message: String },
}

impl SignInError {
    fn from_auth(url: &str, error: &AuthError) -> Self {
        let message = error.to_string();
        let url = url.to_owned();
        match error {
            AuthError::HttpError(_)
            | AuthError::TokenRefreshFailed(_)
            | AuthError::MetadataError(_) => Self::Unreachable { url, message },
            _ => Self::Refused { url, message },
        }
    }
}

/// What the authorization server granted: a token, how long it lasts, what renews it, and
/// how it was signed in. Its `Debug` shows neither token.
#[derive(Debug)]
pub struct Granted {
    pub token: Secret,
    /// How long the token lasts from now, when the server said.
    pub lasts: Option<Duration>,
    pub refresh: Option<Secret>,
    pub client: SignInClient,
}

/// A sign-in waiting for the person to authorize glave in their browser.
pub struct SignIn {
    url: String,
    session: AuthorizationSession,
    issuer: Option<String>,
    state: String,
}

impl SignIn {
    /// Starts signing in to the MCP server at `url`, which the person calls `name`, the
    /// browser to come back to `redirect`: the sign-in, and the authorization server's page
    /// the person opens. glave registers there as `glave (<name>)`, so that page says which
    /// server it is for.
    pub async fn start(
        url: &str,
        name: &str,
        redirect: &str,
    ) -> Result<(Self, String), SignInError> {
        let failed = |error: AuthError| SignInError::from_auth(url, &error);
        let mut manager = AuthorizationManager::new(url).await.map_err(failed)?;
        let resolution = manager.resolve_metadata().await.map_err(failed)?;
        if !resolution.source.is_discovered() {
            return Err(SignInError::NotOffered {
                url: url.to_owned(),
            });
        }
        // Client ID Metadata Documents, its alternative, need an address of glave's own.
        if resolution.metadata.registration_endpoint.is_none() {
            return Err(SignInError::Unregistered {
                url: url.to_owned(),
                message: "its authorization server takes no registration".to_owned(),
            });
        }
        let issuer = resolution.metadata.issuer.clone();
        manager.set_metadata(resolution.metadata);
        let client = format!("glave ({name})");
        let request = AuthorizationRequest::new(redirect).with_client_name(client);
        let session = AuthorizationSession::new(manager, request)
            .await
            .map_err(|(_, error)| failed(error))?;
        let page = session.get_authorization_url().to_owned();
        let state = reqwest::Url::parse(&page)
            .ok()
            .and_then(|page| {
                page.query_pairs()
                    .find(|(name, _)| name == "state")
                    .map(|(_, state)| state.into_owned())
            })
            .ok_or_else(|| SignInError::Refused {
                url: url.to_owned(),
                message: "its authorization page could not be made".to_owned(),
            })?;
        let signin = Self {
            url: url.to_owned(),
            session,
            issuer,
            state,
        };
        Ok((signin, page))
    }

    /// The `state` the browser comes back with, which names this sign-in.
    pub fn state(&self) -> &str {
        &self.state
    }

    /// Trades the `code` the browser came back with, from the authorization server `issuer`
    /// when it named itself (RFC 9207), for a token.
    pub async fn finish(self, code: &str, issuer: Option<&str>) -> Result<Granted, SignInError> {
        let failed = |error: AuthError| SignInError::from_auth(&self.url, &error);
        let response = self
            .session
            .handle_callback_with_issuer(code, &self.state, issuer)
            .await
            .map_err(failed)?;
        let (client_id, _) = self.session.get_credentials().await.map_err(failed)?;
        let client = SignInClient {
            issuer: self.issuer.clone(),
            client_id,
            scopes: self.session.auth_manager.get_current_scopes().await,
        };
        granted(&self.url, &response, client)
    }
}

/// Whether the MCP server at `url` asks people to sign in, as MCP's authorization says: it
/// publishes where, which [`SignIn::start`] finds first.
pub async fn asks_sign_in(url: &str) -> Result<bool, SignInError> {
    let failed = |error: AuthError| SignInError::from_auth(url, &error);
    let manager = AuthorizationManager::new(url).await.map_err(failed)?;
    let resolution = manager.resolve_metadata().await.map_err(failed)?;
    Ok(resolution.source.is_discovered())
}

/// Renews the sign-in to the MCP server at `url`, made through `client`, with `refresh`. The
/// server's metadata is found again first, so the token is renewed for the resource it was
/// granted for; the refresh token goes only to the authorization server that signed the person
/// in, as the server still publishes it. A server that no longer says where its sign-in is gets
/// nothing, and is asked again later; one that names another authorization server is a refusal.
pub async fn renew(
    url: &str,
    client: &SignInClient,
    refresh: &Secret,
) -> Result<Granted, SignInError> {
    let failed = |error: AuthError| SignInError::from_auth(url, &error);
    let mut manager = AuthorizationManager::new(url).await.map_err(failed)?;
    let resolution = manager.resolve_metadata().await.map_err(failed)?;
    if !resolution.source.is_discovered() {
        return Err(SignInError::Unreachable {
            url: url.to_owned(),
            message: "it no longer says where its sign-in is".to_owned(),
        });
    }
    let issuer = resolution.metadata.issuer.as_deref();
    if issuer.is_none() || issuer != client.issuer.as_deref() {
        return Err(SignInError::Refused {
            url: url.to_owned(),
            message: "its authorization server is not the one that signed you in".to_owned(),
        });
    }
    // Set, so rmcp's own start from stored credentials looks for nothing again.
    manager.set_metadata(resolution.metadata);
    // What rmcp renews from: a token response holding only the refresh token.
    let held = json!({"access_token": "", "token_type": "bearer",
                      "refresh_token": refresh.expose()});
    let held: OAuthTokenResponse =
        serde_json::from_value(held).map_err(|error| SignInError::Refused {
            url: url.to_owned(),
            message: format!("its refresh token could not be read: {error}"),
        })?;
    let stored = StoredCredentials::new(
        client.client_id.clone(),
        Some(held),
        client.scopes.clone(),
        None,
    )
    .with_issuer(client.issuer.clone());
    let store = InMemoryCredentialStore::new();
    store.save(stored).await.map_err(failed)?;
    manager.set_credential_store(store);
    if !manager.initialize_from_store().await.map_err(failed)? {
        return Err(SignInError::Refused {
            url: url.to_owned(),
            message: "its authorization server is not the one that signed you in".to_owned(),
        });
    }
    let response = manager.refresh_token().await.map_err(failed)?;
    let renewed = SignInClient {
        scopes: manager.get_current_scopes().await,
        ..client.clone()
    };
    granted(url, &response, renewed)
}

/// What `response` grants, signed in through `client`. A renewal that sent no new refresh
/// token holds the one it was renewed with: rmcp keeps it.
fn granted(
    url: &str,
    response: &OAuthTokenResponse,
    client: SignInClient,
) -> Result<Granted, SignInError> {
    // Read by RFC 6749's own names (section 5.1), which rmcp's type serializes to.
    let fields = serde_json::to_value(response).unwrap_or(Value::Null);
    let text = |name: &str| fields.get(name).and_then(Value::as_str).map(Secret::new);
    let token = text("access_token")
        .filter(|token| !token.expose().is_empty())
        .ok_or_else(|| SignInError::Refused {
            url: url.to_owned(),
            message: "it granted no token".to_owned(),
        })?;
    let lasts = fields
        .get("expires_in")
        .and_then(Value::as_u64)
        .map(Duration::from_secs);
    Ok(Granted {
        token,
        lasts,
        refresh: text("refresh_token"),
        client,
    })
}

/// `url` as glave keeps an MCP server's address, so the person's settings and their sign-in
/// name a server alike: an origin [`glave_ports::host::origin`] takes (`https://`, or
/// `http://` only to this computer), in lower case, then its path and query as given,
/// without a final `/`. None for anything else, or an address with a fragment.
pub fn address(url: &str) -> Option<String> {
    let url = url.trim();
    if url.contains(char::is_whitespace) || url.contains('#') {
        return None;
    }
    let (scheme, rest) = url.split_once("://")?;
    let (host, path) = rest.split_at(rest.find(['/', '?']).unwrap_or(rest.len()));
    let origin = format!("{scheme}://{host}").to_ascii_lowercase();
    glave_ports::host::origin(&origin)?;
    let path = path.strip_suffix('/').unwrap_or(path);
    Some(format!("{origin}{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_an_origin_then_its_path() {
        assert_eq!(
            address(" HTTPS://MCP.Example.com/Mcp/ ").as_deref(),
            Some("https://mcp.example.com/Mcp")
        );
        assert_eq!(
            address("http://127.0.0.1:8080/mcp?team=a").as_deref(),
            Some("http://127.0.0.1:8080/mcp?team=a")
        );
        assert_eq!(
            address("https://mcp.example.com").as_deref(),
            Some("https://mcp.example.com")
        );
        for refused in [
            "http://mcp.example.com/mcp",
            "https://me@mcp.example.com/mcp",
            "mcp.example.com/mcp",
            "https://mcp.example.com/mcp#part",
            "https://mcp example.com",
            "ftp://mcp.example.com",
        ] {
            assert_eq!(address(refused), None, "{refused}");
        }
    }
}
