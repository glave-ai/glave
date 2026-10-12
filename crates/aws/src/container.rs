//! A container credentials endpoint: EKS Pod Identity's agent on the node
//! (`AWS_CONTAINER_CREDENTIALS_FULL_URI`, with a token file to prove the pod), or ECS's
//! (`AWS_CONTAINER_CREDENTIALS_RELATIVE_URI`). It answers temporary keys as JSON.

use std::path::PathBuf;

use glave_ports::auth::{AuthError, Fresh, Secret, safe_to_send};
use reqwest::header::{AUTHORIZATION, HeaderValue};
use serde::Deserialize;

use crate::keys::{Keys, expiration};

/// ECS's credentials endpoint, under which a relative URI is.
const ECS: &str = "http://169.254.170.2";

/// Hosts besides this computer a plain-http credentials endpoint may be on, as AWS's libraries
/// allow: ECS's endpoint, and EKS Pod Identity's agent.
const NODE_HOSTS: &[&str] = &["169.254.170.2", "169.254.170.23", "[fd00:ec2::23]"];

/// A container credentials endpoint and how the pod proves itself to it.
pub struct Container {
    uri: String,
    token: Option<Token>,
}

/// The `Authorization` the endpoint wants.
enum Token {
    /// `AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE`, read again for every fetch: EKS rewrites it.
    File(PathBuf),
    /// `AWS_CONTAINER_AUTHORIZATION_TOKEN`.
    Fixed(Secret),
}

impl Container {
    /// The endpoint the environment names, if any.
    pub(crate) fn from_env(
        set: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Option<Self>, AuthError> {
        let uri = if let Some(relative) = set("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI") {
            format!("{ECS}{}", relative.trim())
        } else if let Some(full) = set("AWS_CONTAINER_CREDENTIALS_FULL_URI") {
            checked(full.trim())?
        } else {
            return Ok(None);
        };
        let token = set("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE")
            .map(|file| Token::File(PathBuf::from(file.trim())))
            .or_else(|| {
                set("AWS_CONTAINER_AUTHORIZATION_TOKEN")
                    .map(|token| Token::Fixed(Secret::new(token.trim())))
            });
        Ok(Some(Self { uri, token }))
    }

    /// Temporary keys from the endpoint.
    pub(crate) async fn fetch(&self, http: &reqwest::Client) -> Result<Fresh<Keys>, AuthError> {
        let mut call = http.get(&self.uri);
        if let Some(token) = self.authorization().await? {
            call = call.header(AUTHORIZATION, token);
        }
        let response = call.send().await.map_err(|error| {
            AuthError::unreachable(format!(
                "could not reach the container credentials endpoint {}: {}",
                self.uri,
                error.without_url()
            ))
        })?;
        let status = response.status();
        if !status.is_success() {
            let message = format!(
                "the container credentials endpoint {} answered HTTP {}",
                self.uri,
                status.as_u16()
            );
            return Err(if status.is_server_error() || status.as_u16() == 429 {
                AuthError::unreachable(message)
            } else {
                AuthError::refused(message)
            });
        }
        let body = response.bytes().await.map_err(|error| {
            AuthError::unreachable(format!(
                "the container credentials endpoint's answer broke off: {}",
                error.without_url()
            ))
        })?;
        let answer: Answer = serde_json::from_slice(&body).map_err(|_| {
            AuthError::unreachable(format!(
                "the container credentials endpoint {} sent keys glave could not read",
                self.uri
            ))
        })?;
        Ok(Fresh {
            value: Keys {
                access_key_id: answer.access_key_id,
                secret_access_key: Secret::new(answer.secret_access_key),
                session_token: answer.token.map(Secret::new),
            },
            expires: expiration(&answer.expiration, "the container credentials endpoint")?,
        })
    }

    /// The `Authorization` header's value, read from its file now when it has one.
    async fn authorization(&self) -> Result<Option<HeaderValue>, AuthError> {
        let token = match &self.token {
            None => return Ok(None),
            Some(Token::Fixed(token)) => token.expose().to_owned(),
            Some(Token::File(path)) => tokio::fs::read_to_string(path).await.map_err(|error| {
                AuthError::refused(format!(
                    "could not read the container authorization token file {} \
                         (AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE): {error}",
                    path.display()
                ))
            })?,
        };
        let mut value = HeaderValue::from_str(token.trim()).map_err(|_| {
            AuthError::refused("the container authorization token is not a valid header value")
        })?;
        value.set_sensitive(true);
        Ok(Some(value))
    }

    pub(crate) fn describe(&self) -> String {
        format!(
            "the container credentials endpoint {} (EKS Pod Identity or ECS)",
            self.uri
        )
    }
}

/// A full URI the environment gives: https, or http only to the hosts AWS's libraries allow.
fn checked(uri: &str) -> Result<String, AuthError> {
    if safe_to_send(uri, NODE_HOSTS) {
        Ok(uri.to_owned())
    } else {
        Err(AuthError::refused(
            "AWS_CONTAINER_CREDENTIALS_FULL_URI is neither https nor a container credentials \
             endpoint on this node",
        ))
    }
}

/// The endpoint's answer. Never printed: it holds the keys.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Answer {
    access_key_id: String,
    secret_access_key: String,
    token: Option<String>,
    expiration: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_or_the_node_s_endpoints_are_trusted() {
        for uri in [
            "http://169.254.170.23/v1/credentials",
            "http://[fd00:ec2::23]/v1/credentials",
            "http://127.0.0.1:8080/creds",
            "https://creds.example.com/v1",
        ] {
            assert!(checked(uri).is_ok(), "{uri}");
        }
        for uri in [
            "http://creds.example.com/v1",
            "http://169.254.169.254/latest",
            "http://user:pw@169.254.170.23/",
            "file:///etc/passwd",
        ] {
            assert!(checked(uri).is_err(), "{uri}");
        }
    }

    #[test]
    fn a_relative_uri_is_under_ecs_s_endpoint() {
        let env = |name: &str| {
            (name == "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI").then(|| "/v2/creds/1".to_owned())
        };
        let container = Container::from_env(&env).unwrap().unwrap();
        assert_eq!(container.uri, "http://169.254.170.2/v2/creds/1");
        assert!(container.token.is_none());
    }
}
