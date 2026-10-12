//! The metadata server's token: on GKE with workload identity, the token of the Google service
//! account the pod's Kubernetes service account is bound to (or the workload's own federated
//! identity); on Compute Engine, Cloud Run and GKE without it, the instance's service account.

use glave_ports::auth::{AuthError, Fresh, Secret};
use jiff::Timestamp;

use crate::token::{TokenResponse, parse, send};

/// The metadata server, unless `GCE_METADATA_HOST` names another, as Google's libraries read it.
pub(crate) const HOST: &str = "metadata.google.internal";

/// The path of the default service account's access token.
const PATH: &str = "/computeMetadata/v1/instance/service-accounts/default/token";

/// The name the server answers to.
const FROM: &str = "the Google metadata server";

/// The token endpoint of the metadata server at `host` (a host, with a port if not 80).
pub(crate) fn token_url(host: &str) -> Result<String, AuthError> {
    let host = host.trim();
    let valid = !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']'));
    if valid {
        Ok(format!("http://{host}{PATH}"))
    } else {
        Err(AuthError::refused(format!(
            "GCE_METADATA_HOST = {host:?} is not a host, such as metadata.google.internal"
        )))
    }
}

/// The default service account's token, from the metadata server at `url`.
pub(crate) async fn token(
    http: &reqwest::Client,
    url: &str,
    asked: Timestamp,
) -> Result<Fresh<Secret>, AuthError> {
    let request = http.get(url).header("Metadata-Flavor", "Google");
    let body = send(request, FROM).await.map_err(|error| {
        if error.is_transient() {
            AuthError::unreachable(format!(
                "{error} (not on Google Cloud? Set GOOGLE_APPLICATION_CREDENTIALS to a \
                 credentials file)"
            ))
        } else {
            error
        }
    })?;
    parse::<TokenResponse>(&body, FROM)?.fresh(asked, FROM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_can_be_moved_but_only_to_a_host() {
        assert_eq!(
            token_url(HOST).unwrap(),
            "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token"
        );
        assert!(token_url("127.0.0.1:8080").is_ok());
        for host in ["", "evil.com/x?", "user@evil.com", "a b"] {
            assert!(token_url(host).is_err(), "{host}");
        }
    }
}
