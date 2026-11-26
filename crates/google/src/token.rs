//! An access token from one of Google's token endpoints: the request sent, the answer read,
//! and a refusal said in Google's own words, never with a secret in it.

use glave_ports::auth::{AuthError, Fresh, Secret, exchange_token, send_for_body};
use jiff::Timestamp;
use serde::Deserialize;

/// Sends `request` to `from` (a name to show, such as `Google's token endpoint`), and gives
/// the body of a successful answer; a refusal otherwise.
pub(crate) async fn send(
    request: reqwest::RequestBuilder,
    from: &str,
) -> Result<String, AuthError> {
    send_for_body(request, from, |status, body| refusal(from, status, body)).await
}

/// Posts `form` to the OAuth 2.0 or STS endpoint at `url`, for a bearer token.
pub(crate) async fn exchange(
    http: &reqwest::Client,
    url: &str,
    form: String,
    from: &str,
    asked: Timestamp,
) -> Result<Fresh<Secret>, AuthError> {
    exchange_token(http, url, form, from, asked, |status, body| {
        refusal(from, status, body)
    })
    .await
}

/// What a refused request means, from Google's error body: OAuth's
/// `{"error": "invalid_grant", "error_description": "…"}`, or Google APIs'
/// `{"error": {"status": "PERMISSION_DENIED", "message": "…"}}`.
pub(crate) fn refusal(from: &str, status: u16, body: &str) -> AuthError {
    let detail = serde_json::from_str::<ErrorBody>(body)
        .ok()
        .map_or_else(|| format!("HTTP {status}"), |error| error.detail(status));
    AuthError::answered(status, &format!("{from} refused"), &detail)
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
}
