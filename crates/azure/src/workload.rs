//! Microsoft Entra workload identity, as AKS sets it up: the cluster projects a short-lived
//! service account token into a file, and Entra exchanges it (a federated client assertion,
//! OAuth 2.0 client credentials) for an access token to the scope asked for. The file is read
//! again for every exchange, since Kubernetes rewrites it before it expires; the Entra token is
//! kept until five minutes before it expires.

use std::path::PathBuf;
use std::sync::Arc;

use glave_ports::BoxFuture;
use glave_ports::Clock;
use glave_ports::auth::{
    AuthError, Fresh, Header, Outgoing, Refreshing, RequestSigner, Secret, credentials_client,
    exchange_token, read_token_file, safe_to_send,
};
use jiff::SignedDuration;
use serde::Deserialize;

/// The scope Microsoft's current Azure OpenAI v1 examples ask for.
pub const DEFAULT_SCOPE: &str = "https://ai.azure.com/.default";

/// Entra's public cloud, when `AZURE_AUTHORITY_HOST` names none.
const DEFAULT_AUTHORITY: &str = "https://login.microsoftonline.com/";

/// What the client assertion is: a JWT from another identity provider, the cluster.
const ASSERTION_TYPE: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

/// How long before a token expires it is replaced, as Azure's own libraries do.
const EARLY: SignedDuration = SignedDuration::from_secs(300);

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
        let http = credentials_client()?;
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
        let assertion = read_token_file(
            &self.token_file,
            "the federated token file",
            "AZURE_FEDERATED_TOKEN_FILE",
        )
        .await?;
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", &self.client_id)
            .append_pair("scope", &self.scope)
            .append_pair("client_assertion_type", ASSERTION_TYPE)
            .append_pair("client_assertion", &assertion)
            .append_pair("grant_type", "client_credentials")
            .finish();
        let asked = self.clock.now();
        exchange_token(
            &self.http,
            &self.token_url,
            form,
            "Microsoft Entra",
            asked,
            refusal,
        )
        .await
    }
}

impl RequestSigner for WorkloadIdentity {
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
    if !safe_to_send(authority, &[]) {
        return Err(AuthError::refused(
            "AZURE_AUTHORITY_HOST is not an https:// address with no name or password in it",
        ));
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
    AuthError::answered(
        status,
        "Microsoft Entra refused the workload identity token exchange",
        &detail,
    )
}

/// Entra's error body.
#[derive(Deserialize)]
struct EntraError {
    error: String,
    error_description: Option<String>,
}

#[cfg(test)]
mod tests;
