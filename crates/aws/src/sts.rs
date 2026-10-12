//! IRSA: EKS projects a service account token into a file, and STS's
//! `AssumeRoleWithWebIdentity` exchanges it for temporary keys to the role the service account
//! is annotated with. The call needs no signature; the token is the proof. The file is read
//! again for every exchange, since Kubernetes rewrites it before it expires.

use std::path::PathBuf;

use glave_ports::auth::{AuthError, Fresh, Secret};
use reqwest::header::{ACCEPT, CONTENT_TYPE};

use crate::Region;
use crate::keys::{Keys, expiration};

/// STS's API version.
const VERSION: &str = "2011-06-15";

/// The session name when `AWS_ROLE_SESSION_NAME` gives none: it shows in CloudTrail.
const SESSION_NAME: &str = "glave";

/// How much of an STS error message to keep.
const ERROR_CHARS: usize = 300;

/// A role assumed with the cluster's web identity token.
pub struct WebIdentity {
    sts: String,
    role_arn: String,
    token_file: PathBuf,
    session_name: String,
}

impl WebIdentity {
    /// IRSA from `AWS_WEB_IDENTITY_TOKEN_FILE`, `AWS_ROLE_ARN` and optionally
    /// `AWS_ROLE_SESSION_NAME` and `AWS_ENDPOINT_URL_STS`; `None` without a token file.
    pub(crate) fn from_env(
        set: &dyn Fn(&str) -> Option<String>,
        region: &Region,
    ) -> Result<Option<Self>, AuthError> {
        let Some(token_file) = set("AWS_WEB_IDENTITY_TOKEN_FILE") else {
            return Ok(None);
        };
        let role_arn = set("AWS_ROLE_ARN").ok_or_else(|| {
            AuthError::refused(
                "AWS_WEB_IDENTITY_TOKEN_FILE is set but AWS_ROLE_ARN is not: is the pod's \
                 service account annotated with eks.amazonaws.com/role-arn?",
            )
        })?;
        let sts = match set("AWS_ENDPOINT_URL_STS") {
            Some(url) => checked(&url)?,
            None => format!("https://sts.{}.amazonaws.com/", region.as_str()),
        };
        Ok(Some(Self {
            sts,
            role_arn: role_arn.trim().to_owned(),
            token_file: PathBuf::from(token_file.trim()),
            session_name: set("AWS_ROLE_SESSION_NAME").unwrap_or_else(|| SESSION_NAME.into()),
        }))
    }

    /// Temporary keys for the role.
    pub(crate) async fn assume(&self, http: &reqwest::Client) -> Result<Fresh<Keys>, AuthError> {
        let token = tokio::fs::read_to_string(&self.token_file)
            .await
            .map_err(|error| {
                AuthError::refused(format!(
                    "could not read the web identity token file {} \
                     (AWS_WEB_IDENTITY_TOKEN_FILE): {error}",
                    self.token_file.display()
                ))
            })?;
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("Action", "AssumeRoleWithWebIdentity")
            .append_pair("Version", VERSION)
            .append_pair("RoleArn", &self.role_arn)
            .append_pair("RoleSessionName", &self.session_name)
            .append_pair("WebIdentityToken", token.trim())
            .finish();
        let response = http
            .post(&self.sts)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(ACCEPT, "text/xml")
            .body(form)
            .send()
            .await
            .map_err(|error| {
                AuthError::unreachable(format!(
                    "could not reach AWS STS at {}: {}",
                    self.sts,
                    error.without_url()
                ))
            })?;
        let status = response.status().as_u16();
        let body = response.text().await.map_err(|error| {
            AuthError::unreachable(format!(
                "AWS STS's answer broke off: {}",
                error.without_url()
            ))
        })?;
        if !(200..300).contains(&status) {
            return Err(refusal(status, &body));
        }
        let field = |name: &str| {
            tag(&body, name)
                .ok_or_else(|| AuthError::unreachable(format!("AWS STS's answer has no {name}")))
        };
        Ok(Fresh {
            value: Keys {
                access_key_id: field("AccessKeyId")?,
                secret_access_key: Secret::new(field("SecretAccessKey")?),
                session_token: Some(Secret::new(field("SessionToken")?)),
            },
            expires: expiration(&field("Expiration")?, "AWS STS")?,
        })
    }

    pub(crate) fn describe(&self) -> String {
        format!("IRSA: the role {} through AWS STS", self.role_arn)
    }
}

/// An STS endpoint given in the environment: https, or http only on this computer.
fn checked(url: &str) -> Result<String, AuthError> {
    let url = url.trim();
    let local = ["http://localhost:", "http://127.0.0.1:", "http://[::1]:"]
        .iter()
        .any(|start| url.starts_with(start));
    if (url.starts_with("https://") || local) && !url.contains('@') {
        Ok(url.to_owned())
    } else {
        Err(AuthError::refused(format!(
            "AWS_ENDPOINT_URL_STS = {url:?} is not an https:// address"
        )))
    }
}

/// What a refused call means, from STS's error: its code and message.
fn refusal(status: u16, body: &str) -> AuthError {
    let code = tag(body, "Code").unwrap_or_else(|| format!("HTTP {status}"));
    let message = tag(body, "Message").unwrap_or_default();
    let detail: String = format!("{code}: {message}")
        .chars()
        .take(ERROR_CHARS)
        .collect();
    let message = format!("AWS STS refused AssumeRoleWithWebIdentity: {detail}");
    if status == 429 || status >= 500 || code == "Throttling" || code == "IDPCommunicationError" {
        AuthError::unreachable(message)
    } else {
        AuthError::refused(message)
    }
}

/// The text of the first `<name>` element in `xml`, its entities replaced.
fn tag(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = xml.find(&open)? + open.len();
    let rest = xml.get(start..)?;
    let text = rest.get(..rest.find(&close)?)?;
    Some(
        text.trim()
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&amp;", "&"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elements_are_read_with_their_entities_replaced() {
        let xml = "<R><Credentials><AccessKeyId>ASIA1</AccessKeyId>\
                   <SessionToken> a&amp;b&lt;c </SessionToken></Credentials></R>";
        assert_eq!(tag(xml, "AccessKeyId").as_deref(), Some("ASIA1"));
        assert_eq!(tag(xml, "SessionToken").as_deref(), Some("a&b<c"));
        assert_eq!(tag(xml, "Expiration"), None);
        assert_eq!(tag("<A>open", "A"), None);
    }

    #[test]
    fn sts_errors_say_their_code_and_whether_to_try_again() {
        let body = "<ErrorResponse><Error><Type>Sender</Type><Code>InvalidIdentityToken</Code>\
                    <Message>No OpenIDConnect provider found</Message></Error></ErrorResponse>";
        let refused = refusal(400, body);
        assert!(!refused.is_transient());
        assert!(
            refused
                .to_string()
                .contains("InvalidIdentityToken: No OpenIDConnect provider found"),
            "{refused}"
        );
        assert!(refusal(503, "").is_transient());
        assert!(refusal(400, "<Code>IDPCommunicationError</Code>").is_transient());
    }

    #[test]
    fn only_https_or_this_computer_is_an_sts_endpoint() {
        assert!(checked("https://sts.us-east-1.amazonaws.com").is_ok());
        assert!(checked("http://127.0.0.1:4566").is_ok());
        assert!(checked("http://sts.example.com").is_err());
        assert!(checked("https://user:pw@sts.example.com").is_err());
    }
}
