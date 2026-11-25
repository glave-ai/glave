//! What every model provider's HTTP client makes of a request: the headers that prove who is
//! calling, never shown; what a failed request means; a refusal read from the start of its
//! answer; and an answer that is not the JSON it should be, its address as it may be shown.

use std::time::Duration;

use reqwest::header::{HeaderName, HeaderValue};

use super::{ERROR_BODY_BYTES, Header};
use crate::failure::Failure;
use crate::host::shown;
use crate::model::ProviderError;
use crate::spoken;

/// The longest a connection to the provider may take to open, its TLS handshake included.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// `call` sent with the headers `sent` prove who is calling with, each never shown: its answer,
/// or the provider's refusal read from the start of it with `sent` taken out, or what the
/// failure means, a silence past `idle` among them.
pub async fn send_signed(
    mut call: reqwest::RequestBuilder,
    sent: &[Header],
    idle: Duration,
) -> Result<reqwest::Response, ProviderError> {
    for header in sent {
        let (name, value) = credential_header(&header.name, header.value.expose())?;
        call = call.header(name, value);
    }
    let response = call
        .send()
        .await
        .map_err(|error| request_failed(error, idle))?;
    let status = response.status();
    if !status.is_success() {
        let body = error_body(response).await;
        return Err(ProviderError::from_response(status.as_u16(), &body, sent));
    }
    Ok(response)
}

/// A header that proves who is calling, its value marked sensitive so it is never shown. An
/// invalid one is refused without showing it.
pub fn credential_header(
    name: &str,
    value: &str,
) -> Result<(HeaderName, HeaderValue), ProviderError> {
    let name = HeaderName::from_bytes(name.as_bytes())
        .map_err(|_| ProviderError::Credentials(format!("{name:?} is not a header name")))?;
    let mut value = HeaderValue::from_str(value).map_err(|_| {
        ProviderError::Credentials(format!("the value for {name} is not a valid header value"))
    })?;
    value.set_sensitive(true);
    Ok((name, value))
}

/// What a failed request means: no connection within [`CONNECT_TIMEOUT`] or none at all, a
/// silence past `idle`, or a broken connection. A connection that never opened sent nothing,
/// so it is the provider unreached, neither silent nor billed.
pub fn request_failed(error: reqwest::Error, idle: Duration) -> ProviderError {
    if error.is_connect() && error.is_timeout() {
        let host = error
            .url()
            .and_then(reqwest::Url::host_str)
            .unwrap_or("the provider");
        unreached(&format!(
            "no connection to {host} within {}",
            spoken(CONNECT_TIMEOUT)
        ))
    } else if error.is_connect() {
        unreached(&cause_of(error))
    } else if error.is_timeout() {
        ProviderError::Timeout {
            seconds: idle.as_secs(),
        }
    } else {
        ProviderError::Transport(cause_of(error))
    }
}

/// The start of an error response's body: as much as [`ProviderError::from_response`] reads,
/// and no more.
async fn error_body(mut response: reqwest::Response) -> Vec<u8> {
    let mut body = Vec::new();
    while body.len() < ERROR_BODY_BYTES
        && let Ok(Some(bytes)) = response.chunk().await
    {
        body.extend_from_slice(&bytes);
    }
    body
}

/// The provider never reached, for `reason`: nothing was sent, so the next target may serve
/// and nothing was billed.
fn unreached(reason: &str) -> ProviderError {
    ProviderError::Unserved {
        failure: Failure::Unreachable,
        reason: format!("could not reach the model provider: {reason}"),
    }
}

/// A request that could not be written out, for `error`: never sent, so nothing was billed.
pub fn not_encoded(error: &serde_json::Error) -> ProviderError {
    ProviderError::Unserved {
        failure: Failure::Rejected,
        reason: format!("could not encode the request: {error}"),
    }
}

/// The answer from `url` that is not the JSON it should be: `url` as it may be shown.
pub fn not_json(url: &str, error: &serde_json::Error) -> ProviderError {
    ProviderError::Protocol(format!("{} did not answer with JSON: {error}", shown(url)))
}

/// What went wrong underneath a failed request, such as `Connection refused (os error 61)`:
/// the innermost cause, which says more than "error sending request".
fn cause_of(error: reqwest::Error) -> String {
    let mut cause: &dyn std::error::Error = &error;
    while let Some(inner) = cause.source() {
        cause = inner;
    }
    let cause = cause.to_string();
    if error.is_connect() && !cause.is_empty() {
        cause
    } else {
        error.without_url().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::any_secret;

    #[test]
    fn a_key_that_cannot_be_a_header_is_refused_without_showing_it() {
        let key = any_secret();
        let error = credential_header("api-key", &format!("sk-\n{key}")).unwrap_err();
        assert!(!error.to_string().contains(&key), "{error}");
        assert!(error.to_string().contains("api-key"), "{error}");
        let (_, value) = credential_header("authorization", &format!("Bearer {key}")).unwrap();
        assert!(value.is_sensitive());
    }
}
