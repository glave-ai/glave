//! What an error the API reports mid-stream means: the status the same error has as a
//! response, so it is classified the same. A response's own error is
//! `ProviderError::from_response`'s, as every provider's is.

use glave_ports::ProviderError;
use glave_ports::auth::error_line;

use crate::events::WireError;

/// An `error` event that arrived after the response began. A known error type becomes the
/// status Anthropic answers it with, so an overload mid-stream is retried like a 529.
pub(crate) fn mid_stream(error: &WireError) -> ProviderError {
    match status_of(&error.kind) {
        Some(status) => ProviderError::Http {
            status,
            body: error_line(&error.message),
        },
        None => ProviderError::Protocol(error_line(&format!(
            "error mid-stream: {}: {}",
            error.kind, error.message
        ))),
    }
}

/// The HTTP status Anthropic documents for each error type.
fn status_of(kind: &str) -> Option<u16> {
    Some(match kind {
        "invalid_request_error" => 400,
        "authentication_error" => 401,
        "billing_error" => 402,
        "permission_error" => 403,
        "not_found_error" => 404,
        "request_too_large" => 413,
        "rate_limit_error" => 429,
        "api_error" => 500,
        "timeout_error" => 504,
        "overloaded_error" => 529,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use glave_ports::Failure;

    use super::*;

    fn wire(kind: &str, message: &str) -> WireError {
        WireError {
            kind: kind.into(),
            message: message.into(),
        }
    }

    #[test]
    fn errors_mid_stream_are_classified_like_responses() {
        let overloaded = mid_stream(&wire("overloaded_error", "Overloaded"));
        assert_eq!(
            overloaded,
            ProviderError::Http {
                status: 529,
                body: "Overloaded".into()
            }
        );
        assert!(
            overloaded.is_transient(),
            "an overload is worth another try"
        );
        assert_eq!(
            mid_stream(&wire("rate_limit_error", "slow down")).failure(),
            Failure::RateLimited
        );
        assert_eq!(
            mid_stream(&wire(
                "invalid_request_error",
                "prompt is too long: 210000 tokens > 200000 maximum"
            ))
            .failure(),
            Failure::TooLong
        );
        assert!(matches!(
            mid_stream(&wire("brand_new_error", "?")),
            ProviderError::Protocol(_)
        ));
    }

    #[test]
    fn response_statuses_are_classified_as_the_port_says() {
        let http = |status, body: &str| ProviderError::from_response(status, body.as_bytes(), &[]);
        let too_long = r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 201000 tokens > 200000 maximum"}}"#;
        assert_eq!(http(400, too_long).failure(), Failure::TooLong);
        assert_eq!(http(413, "").failure(), Failure::TooLong);
        assert_eq!(http(429, "").failure(), Failure::RateLimited);
        assert!(http(529, "").is_transient());
        assert_eq!(http(401, "").failure(), Failure::Refused);
    }
}
