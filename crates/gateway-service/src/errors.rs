//! Why a call was refused or failed, as an HTTP status and a message, in either surface's error
//! format. Messages never carry a key: every one passes through the service's redaction first.

use glave_ports::{Failure, ProviderError};
use hyper::StatusCode;
use serde_json::json;

/// What went wrong, in the terms both surfaces have a word for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The request asked for something the gateway cannot do, or is malformed.
    Invalid,
    /// No sign-in, or not a valid one.
    SignIn,
    /// The caller's policy does not allow it.
    Forbidden,
    /// No such model or path.
    NotFound,
    /// The call could exceed a budget.
    Budget,
    /// The model's provider is limiting requests.
    RateLimited,
    /// The model's provider failed, or failed the gateway.
    Upstream,
    /// The model's provider sent nothing for too long.
    Timeout,
    /// The gateway itself failed, such as its computer's random source.
    Internal,
}

/// A refusal or failure, ready to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refusal {
    pub(crate) kind: Kind,
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}

impl Refusal {
    pub(crate) fn new(kind: Kind, message: impl Into<String>) -> Self {
        let status = match kind {
            Kind::Invalid => StatusCode::BAD_REQUEST,
            Kind::SignIn => StatusCode::UNAUTHORIZED,
            Kind::Forbidden => StatusCode::FORBIDDEN,
            Kind::NotFound => StatusCode::NOT_FOUND,
            Kind::Budget => StatusCode::PAYMENT_REQUIRED,
            Kind::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Kind::Upstream => StatusCode::BAD_GATEWAY,
            Kind::Timeout => StatusCode::GATEWAY_TIMEOUT,
            Kind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            kind,
            status,
            message: message.into(),
        }
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::new(Kind::Invalid, message)
    }

    /// A field the gateway does not pass on, refused rather than dropped.
    pub(crate) fn unsupported(field: &str, why: &str) -> Self {
        Self::invalid(format!(
            "{field} is not supported by this gateway: {why} (specs/gateway.md, \"Surfaces\")"
        ))
    }

    /// What a failed model call means to the caller. A key the provider refused, or an
    /// account out of credit, is the gateway's own trouble, not the caller's sign-in.
    pub(crate) fn of(error: &ProviderError) -> Self {
        let message = error.to_string();
        match error {
            ProviderError::Http { status, .. } => match error.failure() {
                Failure::Refused | Failure::NoCredit => Self::new(
                    Kind::Upstream,
                    format!("the model provider refused the gateway's own account ({status})"),
                ),
                Failure::RateLimited => Self::new(Kind::RateLimited, message),
                Failure::UnknownModel => Self::new(Kind::NotFound, message),
                Failure::TooLong | Failure::Rejected => Self::invalid(message),
                Failure::OverBudget => Self::new(Kind::Budget, message),
                Failure::Unavailable
                | Failure::Unreachable
                | Failure::Silent
                | Failure::Malformed => Self::new(Kind::Upstream, message),
            },
            ProviderError::Credentials(_) => Self::new(
                Kind::Upstream,
                "the gateway could not get its own credentials for the model provider",
            ),
            ProviderError::Timeout { .. } => Self::new(Kind::Timeout, message),
            ProviderError::Unserved { failure, .. } => match failure {
                Failure::UnknownModel => Self::new(Kind::NotFound, message),
                Failure::OverBudget => Self::new(Kind::Budget, message),
                Failure::Refused
                | Failure::NoCredit
                | Failure::TooLong
                | Failure::RateLimited
                | Failure::Unavailable
                | Failure::Unreachable
                | Failure::Silent
                | Failure::Malformed
                | Failure::Rejected => Self::invalid(message),
            },
            ProviderError::OverBudget { .. } => Self::new(Kind::Budget, message),
            ProviderError::Transport(_)
            | ProviderError::Protocol(_)
            | ProviderError::NoOutput(_) => Self::new(Kind::Upstream, message),
        }
    }

    /// The body OpenAI's API sends for an error.
    pub(crate) fn openai(&self) -> serde_json::Value {
        let kind = match self.kind {
            Kind::Invalid => "invalid_request_error",
            Kind::SignIn => "authentication_error",
            Kind::Forbidden => "permission_error",
            Kind::NotFound => "not_found_error",
            Kind::Budget => "insufficient_quota",
            Kind::RateLimited => "rate_limit_error",
            Kind::Upstream | Kind::Timeout | Kind::Internal => "api_error",
        };
        json!({"error": {"message": self.message, "type": kind, "code": null}})
    }

    /// The body Anthropic's API sends for an error.
    pub(crate) fn anthropic(&self) -> serde_json::Value {
        let kind = match self.kind {
            Kind::Invalid | Kind::Budget => "invalid_request_error",
            Kind::SignIn => "authentication_error",
            Kind::Forbidden => "permission_error",
            Kind::NotFound => "not_found_error",
            Kind::RateLimited => "rate_limit_error",
            Kind::Upstream | Kind::Internal => "api_error",
            Kind::Timeout => "overloaded_error",
        };
        json!({"type": "error", "error": {"type": kind, "message": self.message}})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gateway_s_own_account_troubles_are_not_the_caller_s_sign_in() {
        let refused = Refusal::of(&ProviderError::Http {
            status: 401,
            body: "bad key".into(),
        });
        assert_eq!(refused.status, StatusCode::BAD_GATEWAY);
        let limited = Refusal::of(&ProviderError::Http {
            status: 429,
            body: "slow down".into(),
        });
        assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
        let budget = Refusal::of(&ProviderError::OverBudget {
            budget: "person".into(),
            detail: "x".into(),
        });
        assert_eq!(budget.status, StatusCode::PAYMENT_REQUIRED);
        let long = Refusal::of(&ProviderError::Unserved {
            failure: Failure::TooLong,
            reason: "the conversation is longer than m's context window of 10 tokens".into(),
        });
        assert_eq!(long.status, StatusCode::BAD_REQUEST);
        assert!(long.message.contains("context window"));
    }
}
