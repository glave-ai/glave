//! Application Default Credentials, found where Google's own libraries look: the file
//! `GOOGLE_APPLICATION_CREDENTIALS` names, else `gcloud`'s application default credentials
//! file, else the metadata server (GKE workload identity, or the instance's service account).
//! The access token is kept until five minutes before it expires; files are read again for
//! every new token, so a rotated key or a rewritten token file is picked up.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use glave_ports::auth::{AuthError, Credentials, Fresh, Header, Outgoing, Refreshing, Secret};
use glave_ports::{BoxFuture, Clock};
use jiff::{SignedDuration, Timestamp};
use serde::Deserialize;

use crate::external::{ExternalAccount, https};
use crate::jwt::{Claims, SigningKey};
use crate::token::{exchange, parse};
use crate::{jwt, metadata};

/// The scope tokens are asked for: all of Google Cloud, narrowed by the account's IAM roles.
pub const SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

/// Google's OAuth 2.0 token endpoint, when a file names none.
const TOKEN_URI: &str = "https://oauth2.googleapis.com/token";

/// How long before a token expires it is replaced.
const EARLY: SignedDuration = SignedDuration::from_secs(300);

/// How long a service account's assertion is good for: Google's maximum, one hour.
const ASSERTION_SECONDS: i64 = 3_600;

/// Google access tokens from Application Default Credentials, refreshed before they expire.
pub struct Adc {
    source: Source,
    scope: String,
    http: reqwest::Client,
    clock: Arc<dyn Clock>,
    tokens: Refreshing<Secret>,
}

/// Where tokens come from.
enum Source {
    /// The credentials file `GOOGLE_APPLICATION_CREDENTIALS` names.
    Named(PathBuf),
    /// `gcloud`'s file when there is one, else the metadata server at this URL.
    Found {
        gcloud: Option<PathBuf>,
        metadata: String,
    },
}

impl Adc {
    /// Application Default Credentials from the environment, read through `env`, for tokens
    /// to `scope` ([`SCOPE`] unless an organization narrows it). An error names a variable
    /// that is wrong.
    pub fn from_env(
        env: impl Fn(&str) -> Option<String>,
        scope: &str,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, AuthError> {
        let set = |name: &str| env(name).filter(|value| !value.trim().is_empty());
        let source = if let Some(file) = set("GOOGLE_APPLICATION_CREDENTIALS") {
            Source::Named(PathBuf::from(file.trim()))
        } else {
            let folder = set("CLOUDSDK_CONFIG").map(PathBuf::from).or_else(|| {
                if cfg!(windows) {
                    set("APPDATA").map(|data| PathBuf::from(data).join("gcloud"))
                } else {
                    set("HOME").map(|home| PathBuf::from(home).join(".config").join("gcloud"))
                }
            });
            let host = set("GCE_METADATA_HOST").unwrap_or_else(|| metadata::HOST.to_owned());
            Source::Found {
                gcloud: folder.map(|folder| folder.join("application_default_credentials.json")),
                metadata: metadata::token_url(&host)?,
            }
        };
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| AuthError::unreachable(format!("could not set up HTTP: {e}")))?;
        Ok(Self {
            source,
            scope: scope.to_owned(),
            http,
            tokens: Refreshing::new(Arc::clone(&clock), EARLY),
            clock,
        })
    }

    /// A new access token from the source.
    async fn fetch(&self) -> Result<Fresh<Secret>, AuthError> {
        let now = self.clock.now();
        match &self.source {
            Source::Named(path) => self.read_file(path, now).await,
            Source::Found { gcloud, metadata } => {
                if let Some(path) = gcloud
                    && tokio::fs::try_exists(path).await.unwrap_or(false)
                {
                    self.read_file(path, now).await
                } else {
                    metadata::token(&self.http, metadata, now).await
                }
            }
        }
    }

    /// A token from the credentials file at `path`.
    async fn read_file(&self, path: &Path, now: Timestamp) -> Result<Fresh<Secret>, AuthError> {
        let shown = path.display();
        let text = tokio::fs::read_to_string(path).await.map_err(|error| {
            AuthError::refused(format!(
                "could not read the Google credentials file {shown}: {error}"
            ))
        })?;
        let what = format!("the Google credentials file {shown}");
        let kind: Kind = parse(&text, &what)?;
        match kind.kind.as_str() {
            "service_account" => {
                parse::<ServiceAccount>(&text, &what)?
                    .token(self, now)
                    .await
            }
            "authorized_user" => {
                parse::<AuthorizedUser>(&text, &what)?
                    .token(self, now)
                    .await
            }
            "external_account" => {
                parse::<ExternalAccount>(&text, &what)?
                    .token(&self.http, &self.scope, now)
                    .await
            }
            other => Err(AuthError::refused(format!(
                "{what} is of type {other:?}; glave reads service_account, authorized_user and \
                 external_account"
            ))),
        }
    }
}

impl Credentials for Adc {
    fn authorize<'a>(
        &'a self,
        _request: &'a Outgoing<'a>,
    ) -> BoxFuture<'a, Result<Vec<Header>, AuthError>> {
        Box::pin(async move {
            let token = self.tokens.get(|| self.fetch()).await?;
            Ok(vec![Header {
                name: "authorization".to_owned(),
                value: Secret::new(format!("Bearer {}", token.expose())),
            }])
        })
    }

    fn describe(&self) -> String {
        match &self.source {
            Source::Named(path) => format!(
                "Google credentials in {} (GOOGLE_APPLICATION_CREDENTIALS)",
                path.display()
            ),
            Source::Found { gcloud, .. } => match gcloud {
                Some(path) => format!(
                    "Google Application Default Credentials ({} if present, else the metadata \
                     server)",
                    path.display()
                ),
                None => "Google Application Default Credentials (the metadata server)".to_owned(),
            },
        }
    }
}

/// A credentials file's type, read before the rest.
#[derive(Deserialize)]
struct Kind {
    #[serde(rename = "type")]
    kind: String,
}

/// A service account key file. Never printed: it holds the private key.
#[derive(Deserialize)]
struct ServiceAccount {
    client_email: String,
    private_key: String,
    private_key_id: Option<String>,
    token_uri: Option<String>,
}

impl ServiceAccount {
    /// A token for an assertion signed with the account's key (RFC 7523).
    async fn token(&self, adc: &Adc, now: Timestamp) -> Result<Fresh<Secret>, AuthError> {
        let token_uri = https("token_uri", self.token_uri.as_deref().unwrap_or(TOKEN_URI))?;
        let iat = now.as_second();
        let assertion = SigningKey::from_pem(&self.private_key)?.sign(
            &jwt::Header {
                alg: "RS256",
                typ: "JWT",
                kid: self.private_key_id.as_deref(),
            },
            &Claims {
                iss: &self.client_email,
                scope: &adc.scope,
                aud: token_uri,
                iat,
                exp: iat.saturating_add(ASSERTION_SECONDS),
            },
        )?;
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer")
            .append_pair("assertion", &assertion)
            .finish();
        let from = format!(
            "Google's token endpoint (service account {})",
            self.client_email
        );
        exchange(&adc.http, token_uri, form, &from, now).await
    }
}

/// `gcloud auth application-default login`'s file: a person's refresh token. Never printed.
#[derive(Deserialize)]
struct AuthorizedUser {
    client_id: String,
    client_secret: String,
    refresh_token: String,
    token_uri: Option<String>,
}

impl AuthorizedUser {
    /// A token for the person's refresh token.
    async fn token(&self, adc: &Adc, now: Timestamp) -> Result<Fresh<Secret>, AuthError> {
        let token_uri = https("token_uri", self.token_uri.as_deref().unwrap_or(TOKEN_URI))?;
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "refresh_token")
            .append_pair("client_id", &self.client_id)
            .append_pair("client_secret", &self.client_secret)
            .append_pair("refresh_token", &self.refresh_token)
            .finish();
        let from = "Google's token endpoint (gcloud's user credentials)";
        exchange(&adc.http, token_uri, form, from, now).await
    }
}

#[cfg(test)]
mod tests;
