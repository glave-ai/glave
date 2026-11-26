//! What every cloud's credentials share over HTTP: a request sent for the body of its answer,
//! a refusal said in the source's own words; an OAuth 2.0 token answered for a form; and a
//! token read from the file a cluster keeps it in.

use std::path::Path;
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;

use super::CONNECT_TIMEOUT;
use crate::auth::{AuthError, Fresh, Secret};
use crate::host::shown;

/// The longest a request for credentials may take, from its connection to its answer's end.
const CREDENTIALS_TIMEOUT: Duration = Duration::from_secs(30);

/// The HTTP client a cloud's credentials are asked for with: a connection within
/// [`CONNECT_TIMEOUT`], and the whole answer within 30 seconds.
pub fn credentials_client() -> Result<reqwest::Client, AuthError> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(CREDENTIALS_TIMEOUT)
        .build()
        .map_err(|e| AuthError::unreachable(format!("could not set up HTTP: {e}")))
}

/// Sends `request` to `from` (a name to show, such as `Google's token endpoint`), and gives
/// the body of a successful answer; otherwise what `refused` makes of the answer's status and
/// body.
pub async fn send_for_body(
    request: reqwest::RequestBuilder,
    from: &str,
    refused: impl FnOnce(u16, &str) -> AuthError,
) -> Result<String, AuthError> {
    let response = request.send().await.map_err(|error| {
        let at = error
            .url()
            .map(|url| format!(" at {}", shown(url.as_str())))
            .unwrap_or_default();
        AuthError::unreachable(format!(
            "could not reach {from}{at}: {}",
            error.without_url()
        ))
    })?;
    let status = response.status();
    let body = response.text().await.map_err(|error| {
        AuthError::unreachable(format!(
            "{from}'s answer broke off: {}",
            error.without_url()
        ))
    })?;
    if status.is_success() {
        Ok(body)
    } else {
        Err(refused(status.as_u16(), &body))
    }
}

/// Posts `form` to the OAuth 2.0 or STS token endpoint at `url`, asked at `asked`, for a bearer
/// token; `refused` says what a refusal means.
pub async fn exchange_token(
    http: &reqwest::Client,
    url: &str,
    form: String,
    from: &str,
    asked: Timestamp,
    refused: impl FnOnce(u16, &str) -> AuthError,
) -> Result<Fresh<Secret>, AuthError> {
    let request = http
        .post(url)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(form);
    let body = send_for_body(request, from, refused).await?;
    parse_answer::<TokenResponse>(&body, from)?.fresh(asked, from)
}

/// `body` as `T`, or an error naming `from`, never showing the body.
pub fn parse_answer<T: for<'a> Deserialize<'a>>(body: &str, from: &str) -> Result<T, AuthError> {
    serde_json::from_str(body)
        .map_err(|_| AuthError::unreachable(format!("{from} sent an answer glave could not read")))
}

/// The token in the file at `path`, which `variable` names, trimmed: `what` says which, such as
/// `the federated token file`. Read again for every exchange, since a cluster rewrites it before
/// it expires.
pub async fn read_token_file(path: &Path, what: &str, variable: &str) -> Result<String, AuthError> {
    let token = tokio::fs::read_to_string(path).await.map_err(|error| {
        AuthError::refused(format!(
            "could not read {what} {} ({variable}): {error}",
            path.display()
        ))
    })?;
    Ok(token.trim().to_owned())
}

/// A token answer: OAuth 2.0's, or STS's (RFC 8693), which adds fields glave does not need.
/// Never printed: it holds the token.
#[derive(Deserialize)]
pub struct TokenResponse {
    access_token: String,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    expires_in: Option<Seconds>,
}

/// A number of seconds, which most token endpoints send as a number and some as a string.
#[derive(Deserialize)]
#[serde(untagged)]
enum Seconds {
    Number(i64),
    Text(String),
}

impl Seconds {
    /// The seconds, when they are more than none.
    fn seconds(&self) -> Option<i64> {
        match self {
            Self::Number(seconds) => Some(*seconds),
            Self::Text(text) => text.trim().parse().ok(),
        }
        .filter(|seconds| *seconds > 0)
    }
}

impl TokenResponse {
    /// The token and when it expires, counted from `asked`, when it was asked for.
    pub fn fresh(self, asked: Timestamp, from: &str) -> Result<Fresh<Secret>, AuthError> {
        if let Some(kind) = &self.token_type
            && !kind.eq_ignore_ascii_case("bearer")
        {
            return Err(AuthError::refused(format!(
                "{from} sent a {kind} token, not a bearer token"
            )));
        }
        let lifetime = self
            .expires_in
            .as_ref()
            .and_then(Seconds::seconds)
            .ok_or_else(|| AuthError::unreachable(format!("{from}'s token has no expires_in")))?;
        let expires = asked
            .saturating_add(SignedDuration::from_secs(lifetime))
            .map_err(|_| AuthError::unreachable(format!("{from}'s token outlives time")))?;
        Ok(Fresh {
            value: Secret::new(self.access_token),
            expires,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_needs_a_lifetime_as_a_number_or_a_string_and_must_be_a_bearer_token() {
        let at = Timestamp::from_second(1_000).unwrap();
        let token = |json: &str| {
            parse_answer::<TokenResponse>(json, "x")
                .unwrap()
                .fresh(at, "x")
        };
        let fresh =
            token(r#"{"access_token":"ya29.a","expires_in":3599,"token_type":"Bearer"}"#).unwrap();
        assert_eq!(fresh.value.expose(), "ya29.a");
        assert_eq!(fresh.expires, Timestamp::from_second(4_599).unwrap());
        let text = token(r#"{"access_token":"a","expires_in":" 3599 ","token_type":"bearer"}"#);
        assert_eq!(
            text.unwrap().expires,
            Timestamp::from_second(4_599).unwrap()
        );
        assert!(token(r#"{"access_token":"ya29.a","token_type":"Bearer"}"#).is_err());
        assert!(token(r#"{"access_token":"a","expires_in":"0"}"#).is_err());
        let mac = token(r#"{"access_token":"ya29.a","expires_in":1,"token_type":"mac"}"#);
        assert!(mac.unwrap_err().to_string().contains("not a bearer"));
        let unreadable = parse_answer::<TokenResponse>(r#"{"secret":"ya29.b"}"#, "x")
            .err()
            .unwrap();
        assert!(!unreadable.to_string().contains("ya29"), "{unreadable}");
    }
}
