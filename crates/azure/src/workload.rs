//! Microsoft Entra workload identity, as AKS sets it up: the cluster projects a short-lived
//! service account token into a file, and Entra exchanges it (a federated client assertion,
//! OAuth 2.0 client credentials) for an access token to the scope asked for. The file is read
//! again for every exchange, since Kubernetes rewrites it before it expires; the Entra token is
//! kept until five minutes before it expires.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use glave_ports::BoxFuture;
use glave_ports::Clock;
use glave_ports::auth::{AuthError, Credentials, Fresh, Header, Outgoing, Refreshing, Secret};
use jiff::SignedDuration;
use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;

/// The scope Microsoft's current Azure OpenAI v1 examples ask for.
pub const DEFAULT_SCOPE: &str = "https://ai.azure.com/.default";

/// Entra's public cloud, when `AZURE_AUTHORITY_HOST` names none.
const DEFAULT_AUTHORITY: &str = "https://login.microsoftonline.com/";

/// What the client assertion is: a JWT from another identity provider, the cluster.
const ASSERTION_TYPE: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

/// How long before a token expires it is replaced, as Azure's own libraries do.
const EARLY: SignedDuration = SignedDuration::from_secs(300);

/// How much of Entra's error description to keep.
const ERROR_CHARS: usize = 300;

/// Microsoft Entra tokens from AKS workload identity, refreshed before they expire.
pub struct WorkloadIdentity {
    http: reqwest::Client,
    token_url: String,
    client_id: String,
    token_file: PathBuf,
    scope: String,
    clock: Arc<dyn Clock>,
    tokens: Refreshing<Secret>,
}

impl WorkloadIdentity {
    /// Workload identity from the variables AKS's webhook sets (`AZURE_FEDERATED_TOKEN_FILE`,
    /// `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, optionally `AZURE_AUTHORITY_HOST`), read through
    /// `env`, for tokens to `scope`. `None` when there is no federated token file: not a pod
    /// with workload identity. An error names a variable that is missing or wrong.
    pub fn from_env(
        env: impl Fn(&str) -> Option<String>,
        scope: &str,
        clock: Arc<dyn Clock>,
    ) -> Result<Option<Self>, AuthError> {
        let set = |name: &str| env(name).filter(|value| !value.trim().is_empty());
        let Some(token_file) = set("AZURE_FEDERATED_TOKEN_FILE") else {
            return Ok(None);
        };
        let needed = |name: &str| {
            set(name).ok_or_else(|| {
                AuthError::refused(format!(
                    "AZURE_FEDERATED_TOKEN_FILE is set but {name} is not: is the pod's \
                     service account annotated for workload identity?"
                ))
            })
        };
        let client_id = needed("AZURE_CLIENT_ID")?;
        let tenant = needed("AZURE_TENANT_ID")?;
        let authority = set("AZURE_AUTHORITY_HOST").unwrap_or_else(|| DEFAULT_AUTHORITY.into());
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| AuthError::unreachable(format!("could not set up HTTP: {e}")))?;
        Ok(Some(Self {
            http,
            token_url: token_url(&authority, &tenant)?,
            client_id: client_id.trim().to_owned(),
            token_file: PathBuf::from(token_file.trim()),
            scope: scope.to_owned(),
            tokens: Refreshing::new(Arc::clone(&clock), EARLY),
            clock,
        }))
    }

    /// Exchanges the cluster's token for an Entra token.
    async fn exchange(&self) -> Result<Fresh<Secret>, AuthError> {
        let assertion = tokio::fs::read_to_string(&self.token_file)
            .await
            .map_err(|error| {
                AuthError::refused(format!(
                    "could not read the federated token file {} (AZURE_FEDERATED_TOKEN_FILE): \
                     {error}",
                    self.token_file.display()
                ))
            })?;
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", &self.client_id)
            .append_pair("scope", &self.scope)
            .append_pair("client_assertion_type", ASSERTION_TYPE)
            .append_pair("client_assertion", assertion.trim())
            .append_pair("grant_type", "client_credentials")
            .finish();
        let asked = self.clock.now();
        let response = self
            .http
            .post(&self.token_url)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(form)
            .send()
            .await
            .map_err(|error| {
                AuthError::unreachable(format!(
                    "could not reach Microsoft Entra at {}: {}",
                    self.token_url,
                    error.without_url()
                ))
            })?;
        let status = response.status();
        let body = response.text().await.map_err(|error| {
            AuthError::unreachable(format!(
                "Microsoft Entra's answer broke off: {}",
                error.without_url()
            ))
        })?;
        if !status.is_success() {
            return Err(refusal(status.as_u16(), &body));
        }
        let token: TokenResponse = serde_json::from_str(&body).map_err(|_| {
            AuthError::unreachable("Microsoft Entra sent a token response glave could not read")
        })?;
        if !token.token_type.eq_ignore_ascii_case("bearer") {
            return Err(AuthError::refused(format!(
                "Microsoft Entra sent a {} token, not a bearer token",
                token.token_type
            )));
        }
        let lifetime = token.expires_in.seconds().ok_or_else(|| {
            AuthError::unreachable("Microsoft Entra's token response has no expires_in")
        })?;
        let expires = asked
            .saturating_add(SignedDuration::from_secs(lifetime))
            .map_err(|_| AuthError::unreachable("Microsoft Entra's token outlives time"))?;
        Ok(Fresh {
            value: Secret::new(token.access_token),
            expires,
        })
    }
}

impl Credentials for WorkloadIdentity {
    fn authorize<'a>(
        &'a self,
        _request: &'a Outgoing<'a>,
    ) -> BoxFuture<'a, Result<Vec<Header>, AuthError>> {
        Box::pin(async move {
            let token = self.tokens.get(|| self.exchange()).await?;
            Ok(vec![Header {
                name: "authorization".to_owned(),
                value: Secret::new(format!("Bearer {}", token.expose())),
            }])
        })
    }

    fn describe(&self) -> String {
        format!(
            "Microsoft Entra workload identity (client {}, scope {})",
            self.client_id, self.scope
        )
    }
}

/// Entra's token endpoint for `tenant` at `authority`: https, or http only on this computer.
fn token_url(authority: &str, tenant: &str) -> Result<String, AuthError> {
    let authority = authority.trim().trim_end_matches('/');
    let local = ["http://localhost:", "http://127.0.0.1:", "http://[::1]:"]
        .iter()
        .any(|start| authority.starts_with(start));
    if !(authority.starts_with("https://") || local) || authority.contains('@') {
        return Err(AuthError::refused(format!(
            "AZURE_AUTHORITY_HOST = {authority:?} is not an https:// address"
        )));
    }
    let tenant = tenant.trim();
    let valid = !tenant.is_empty()
        && tenant
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.');
    if !valid {
        return Err(AuthError::refused(format!(
            "AZURE_TENANT_ID = {tenant:?} is not a tenant id or domain"
        )));
    }
    Ok(format!("{authority}/{tenant}/oauth2/v2.0/token"))
}

/// What a refused exchange means, from Entra's error body: its code and the first line of its
/// description, such as `AADSTS700016: Application with identifier … was not found`.
fn refusal(status: u16, body: &str) -> AuthError {
    let parsed: Option<EntraError> = serde_json::from_str(body).ok();
    let detail = parsed.map_or_else(
        || format!("HTTP {status}"),
        |error| {
            let description = error.error_description.unwrap_or_default();
            let first = description.lines().next().unwrap_or_default().trim();
            format!("{}: {first}", error.error)
        },
    );
    let detail: String = detail.chars().take(ERROR_CHARS).collect();
    let message = format!("Microsoft Entra refused the workload identity token exchange: {detail}");
    if status == 429 || status >= 500 {
        AuthError::unreachable(message)
    } else {
        AuthError::refused(message)
    }
}

/// Entra's answer to a token request. Never printed: it holds the token.
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: Seconds,
}

/// A number of seconds, which Entra sends as a number and some of its endpoints as a string.
#[derive(Deserialize)]
#[serde(untagged)]
enum Seconds {
    Number(i64),
    Text(String),
}

impl Seconds {
    fn seconds(&self) -> Option<i64> {
        match self {
            Self::Number(seconds) => Some(*seconds),
            Self::Text(text) => text.trim().parse().ok(),
        }
        .filter(|seconds| *seconds > 0)
    }
}

/// Entra's error body.
#[derive(Deserialize)]
struct EntraError {
    error: String,
    error_description: Option<String>,
}

#[cfg(test)]
mod tests;
