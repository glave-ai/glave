//! Workload identity federation (`"type": "external_account"`, Google's AIP-4117): a token from
//! another identity provider, such as a Kubernetes cluster's projected service account token,
//! exchanged at Google's STS for a federated access token, and that, when the file names a
//! service account to impersonate, for the service account's token.
//!
//! The subject token comes from a file (read again for every exchange, since Kubernetes
//! rewrites it) or a URL, as text or JSON. AWS and executable sources are not supported.

use std::collections::BTreeMap;

use glave_ports::auth::{AuthError, Fresh, Secret};
use jiff::Timestamp;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;

use crate::token::{exchange, parse, send};

const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const ACCESS_TOKEN: &str = "urn:ietf:params:oauth:token-type:access_token";

/// The lifetime asked of an impersonated token when the file says none: Google's default.
const LIFETIME: u32 = 3_600;

/// An `external_account` credentials file.
#[derive(Deserialize)]
pub(crate) struct ExternalAccount {
    audience: String,
    subject_token_type: String,
    token_url: String,
    service_account_impersonation_url: Option<String>,
    #[serde(default)]
    service_account_impersonation: Impersonation,
    credential_source: CredentialSource,
}

#[derive(Default, Deserialize)]
struct Impersonation {
    token_lifetime_seconds: Option<u32>,
}

#[derive(Deserialize)]
struct CredentialSource {
    file: Option<String>,
    url: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    format: Format,
    environment_id: Option<String>,
    executable: Option<serde_json::Value>,
}

#[derive(Default, Deserialize)]
struct Format {
    #[serde(rename = "type")]
    kind: Option<String>,
    subject_token_field_name: Option<String>,
}

/// The impersonation endpoint's answer. Never printed: it holds the token.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Impersonated {
    access_token: String,
    expire_time: String,
}

impl ExternalAccount {
    /// An access token to `scope`.
    pub(crate) async fn token(
        &self,
        http: &reqwest::Client,
        scope: &str,
        now: Timestamp,
    ) -> Result<Fresh<Secret>, AuthError> {
        let token_url = https("token_url", &self.token_url)?;
        let impersonate = self
            .service_account_impersonation_url
            .as_deref()
            .map(|url| https("service_account_impersonation_url", url))
            .transpose()?;
        let subject = self.subject_token(http).await?;
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", GRANT_TYPE)
            .append_pair("audience", &self.audience)
            .append_pair("scope", scope)
            .append_pair("requested_token_type", ACCESS_TOKEN)
            .append_pair("subject_token", &subject)
            .append_pair("subject_token_type", &self.subject_token_type)
            .finish();
        let federated = exchange(http, token_url, form, "Google STS", now).await?;
        let Some(url) = impersonate else {
            return Ok(federated);
        };
        let lifetime = self
            .service_account_impersonation
            .token_lifetime_seconds
            .unwrap_or(LIFETIME);
        let body = serde_json::json!({"scope": [scope], "lifetime": format!("{lifetime}s")});
        let request = http
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .header(
                AUTHORIZATION,
                format!("Bearer {}", federated.value.expose()),
            )
            .body(body.to_string());
        let from = "Google IAM Credentials (service account impersonation)";
        let answer: Impersonated = parse(&send(request, from).await?, from)?;
        let expires = answer.expire_time.trim().parse().map_err(|_| {
            AuthError::unreachable(format!("{from} sent an expireTime glave could not read"))
        })?;
        Ok(Fresh {
            value: Secret::new(answer.access_token),
            expires,
        })
    }

    /// The token from the other identity provider.
    async fn subject_token(&self, http: &reqwest::Client) -> Result<String, AuthError> {
        let source = &self.credential_source;
        if source.environment_id.is_some() || source.executable.is_some() {
            return Err(AuthError::refused(
                "the external account's credential_source is an AWS or executable source, \
                 which glave does not support: use a file or url source",
            ));
        }
        let text = match (&source.file, &source.url) {
            (Some(file), _) => tokio::fs::read_to_string(file).await.map_err(|error| {
                AuthError::refused(format!(
                    "could not read the external account's subject token file {file}: {error}"
                ))
            })?,
            (None, Some(url)) => {
                let url = plain_url(url)?;
                let mut request = http.get(url);
                for (name, value) in &source.headers {
                    request = request.header(name, value);
                }
                send(request, "the external account's credential_source url").await?
            }
            (None, None) => {
                return Err(AuthError::refused(
                    "the external account's credential_source has neither file nor url",
                ));
            }
        };
        let token = match source.format.kind.as_deref().unwrap_or("text") {
            "text" => text.trim().to_owned(),
            "json" => {
                let field = source
                    .format
                    .subject_token_field_name
                    .as_deref()
                    .ok_or_else(|| {
                        AuthError::refused(
                            "the external account's credential_source format is json but names no \
                         subject_token_field_name",
                        )
                    })?;
                let json: serde_json::Value =
                    parse(&text, "the external account's subject token source")?;
                json.get(field)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        AuthError::refused(format!(
                            "the external account's subject token has no {field:?} field"
                        ))
                    })?
            }
            other => {
                return Err(AuthError::refused(format!(
                    "the external account's credential_source format {other:?} is not text or json"
                )));
            }
        };
        if token.is_empty() {
            return Err(AuthError::refused(
                "the external account's subject token is empty",
            ));
        }
        Ok(token)
    }
}

/// A Google endpoint named in a credentials file: https, or http only on this computer.
pub(crate) fn https<'a>(field: &str, url: &'a str) -> Result<&'a str, AuthError> {
    let url = url.trim();
    let local = ["http://localhost:", "http://127.0.0.1:", "http://[::1]:"]
        .iter()
        .any(|start| url.starts_with(start));
    if (url.starts_with("https://") || local) && !signs_in(url) {
        Ok(url)
    } else {
        Err(AuthError::refused(format!(
            "the credentials file's {field} = {url:?} is not an https:// address"
        )))
    }
}

/// A subject token source's URL: http or https, such as a local metadata server.
fn plain_url(url: &str) -> Result<&str, AuthError> {
    let url = url.trim();
    if (url.starts_with("https://") || url.starts_with("http://")) && !signs_in(url) {
        Ok(url)
    } else {
        Err(AuthError::refused(format!(
            "the external account's credential_source url = {url:?} is not an http(s) address"
        )))
    }
}

/// Whether `url` carries a user name or password before its host, where a mistake could put
/// a secret. An `@` later, in a path such as a service account's email, is fine.
fn signs_in(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#'])
        .next()
        .is_some_and(|authority| authority.contains('@'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_must_be_https_but_sources_may_be_local_http() {
        assert!(https("token_url", "https://sts.googleapis.com/v1/token").is_ok());
        assert!(https("token_url", "http://127.0.0.1:9/token").is_ok());
        assert!(https("token_url", "http://sts.example.com/token").is_err());
        assert!(https("token_url", "https://me:pw@sts.googleapis.com").is_err());
        assert!(plain_url("http://169.254.169.254/metadata/identity/oauth2/token").is_ok());
        assert!(plain_url("file:///etc/passwd").is_err());
        assert!(
            https(
                "service_account_impersonation_url",
                "https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/glave@acme-ai.iam.gserviceaccount.com:generateAccessToken"
            )
            .is_ok()
        );
    }
}
