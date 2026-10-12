//! An access token from one of Google's token endpoints: the request sent, the answer read,
//! and a refusal said in Google's own words, never with a secret in it.

use glave_ports::auth::{AuthError, Fresh, Secret};
use jiff::{SignedDuration, Timestamp};
use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;

/// How much of an error description to keep.
const ERROR_CHARS: usize = 300;

/// A token answer: OAuth 2.0's, or STS's (RFC 8693), which adds fields glave does not need.
/// Never printed: it holds the token.
#[derive(Deserialize)]
pub(crate) struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
}

impl TokenResponse {
    /// The token and when it expires, counted from `asked`, when it was asked for.
    pub(crate) fn fresh(self, asked: Timestamp, from: &str) -> Result<Fresh<Secret>, AuthError> {
        if let Some(kind) = &self.token_type
            && !kind.eq_ignore_ascii_case("bearer")
        {
            return Err(AuthError::refused(format!(
                "{from} sent a {kind} token, not a bearer token"
            )));
        }
        let lifetime = self
            .expires_in
            .filter(|seconds| *seconds > 0)
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

/// Sends `request` to `from` (a name to show, such as `Google's token endpoint`), and gives
/// the body of a successful answer; a refusal otherwise.
pub(crate) async fn send(
    request: reqwest::RequestBuilder,
    from: &str,
) -> Result<String, AuthError> {
    let response = request.send().await.map_err(|error| {
        AuthError::unreachable(format!("could not reach {from}: {}", error.without_url()))
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
        Err(refusal(from, status.as_u16(), &body))
    }
}

/// Posts `form` to the OAuth 2.0 or STS endpoint at `url`, for a bearer token.
pub(crate) async fn exchange(
    http: &reqwest::Client,
    url: &str,
    form: String,
    from: &str,
    asked: Timestamp,
) -> Result<Fresh<Secret>, AuthError> {
    let request = http
        .post(url)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(form);
    let body = send(request, from).await?;
    parse::<TokenResponse>(&body, from)?.fresh(asked, from)
}

/// `body` as `T`, or an error naming `from`, never showing the body.
pub(crate) fn parse<T: for<'a> Deserialize<'a>>(body: &str, from: &str) -> Result<T, AuthError> {
    serde_json::from_str(body)
        .map_err(|_| AuthError::unreachable(format!("{from} sent an answer glave could not read")))
}

/// What a refused request means, from Google's error body: OAuth's
/// `{"error": "invalid_grant", "error_description": "…"}`, or Google APIs'
/// `{"error": {"status": "PERMISSION_DENIED", "message": "…"}}`.
pub(crate) fn refusal(from: &str, status: u16, body: &str) -> AuthError {
    let detail = serde_json::from_str::<ErrorBody>(body)
        .ok()
        .map_or_else(|| format!("HTTP {status}"), |error| error.detail(status));
    let detail: String = detail.chars().take(ERROR_CHARS).collect();
    let message = format!("{from} refused: {detail}");
    if status == 429 || status >= 500 {
        AuthError::unreachable(message)
    } else {
        AuthError::refused(message)
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ErrorBody {
    OAuth {
        error: String,
        error_description: Option<String>,
    },
    Api {
        error: ApiError,
    },
}

#[derive(Deserialize)]
struct ApiError {
    status: Option<String>,
    message: Option<String>,
}

impl ErrorBody {
    fn detail(self, status: u16) -> String {
        let (code, text) = match self {
            Self::OAuth {
                error,
                error_description,
            } => (error, error_description.unwrap_or_default()),
            Self::Api { error } => (
                error.status.unwrap_or_else(|| format!("HTTP {status}")),
                error.message.unwrap_or_default(),
            ),
        };
        let first = text.lines().next().unwrap_or_default().trim();
        if first.is_empty() {
            code
        } else {
            format!("{code}: {first}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_say_google_s_reason_and_whether_to_try_again() {
        let oauth = refusal(
            "Google's token endpoint",
            400,
            r#"{"error":"invalid_grant","error_description":"Invalid JWT Signature.\nmore"}"#,
        );
        assert_eq!(
            oauth.to_string(),
            "Google's token endpoint refused: invalid_grant: Invalid JWT Signature."
        );
        assert!(!oauth.is_transient());
        let api = refusal(
            "IAM Credentials",
            403,
            r#"{"error":{"code":403,"status":"PERMISSION_DENIED","message":"Permission 'iam.serviceAccounts.getAccessToken' denied"}}"#,
        );
        assert!(
            api.to_string()
                .contains("PERMISSION_DENIED: Permission 'iam.serviceAccounts.getAccessToken'"),
            "{api}"
        );
        assert!(refusal("x", 503, "<html>").is_transient());
        assert_eq!(
            refusal("x", 404, "Not Found").to_string(),
            "x refused: HTTP 404"
        );
    }

    #[test]
    fn a_token_needs_a_lifetime_and_must_be_a_bearer_token() {
        let at = Timestamp::from_second(1_000).unwrap();
        let token = |json: &str| parse::<TokenResponse>(json, "x").unwrap().fresh(at, "x");
        let fresh =
            token(r#"{"access_token":"ya29.a","expires_in":3599,"token_type":"Bearer"}"#).unwrap();
        assert_eq!(fresh.value.expose(), "ya29.a");
        assert_eq!(fresh.expires, Timestamp::from_second(4_599).unwrap());
        assert!(token(r#"{"access_token":"ya29.a","token_type":"Bearer"}"#).is_err());
        let mac = token(r#"{"access_token":"ya29.a","expires_in":1,"token_type":"mac"}"#);
        assert!(mac.unwrap_err().to_string().contains("not a bearer"));
        let unreadable = parse::<TokenResponse>(r#"{"secret":"ya29.b"}"#, "x")
            .err()
            .unwrap();
        assert!(!unreadable.to_string().contains("ya29"), "{unreadable}");
    }
}
