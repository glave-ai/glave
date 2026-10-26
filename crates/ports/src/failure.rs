//! What kind of failure a provider error is, so that whoever shows it can say what to fix, and
//! the harness knows whether trying again may help.

use serde::{Deserialize, Serialize};

use crate::model::ProviderError;

/// The kind of a failed model request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    /// The key was refused (401, 403), or credentials could not be had.
    Refused,
    /// The account has no credit left (402).
    NoCredit,
    /// The server has no such model, or no chat completions API at that address.
    UnknownModel,
    /// The conversation is longer than the model's context window.
    TooLong,
    /// The provider is limiting requests (429).
    RateLimited,
    /// The provider failed on its side (5xx).
    Unavailable,
    /// The server could not be reached, or the connection broke.
    Unreachable,
    /// The server sent nothing for too long.
    Silent,
    /// The server's answer did not follow the protocol.
    Malformed,
    /// Any other refusal of the request.
    Rejected,
    /// The call could exceed a budget, so it was not sent.
    OverBudget,
}

impl Failure {
    /// Whether trying again may work: a rate limit, a server error, a broken connection or a
    /// silence. A refused key or an unknown model fails the same way again.
    pub fn is_transient(self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::Unavailable | Self::Unreachable | Self::Silent
        )
    }
}

impl ProviderError {
    pub fn failure(&self) -> Failure {
        match self {
            Self::Http { status, body } => from_status(*status, &body.to_ascii_lowercase()),
            Self::Credentials(_) => Failure::Refused,
            Self::Transport(_) => Failure::Unreachable,
            Self::Timeout { .. } => Failure::Silent,
            Self::NoOutput(_) => Failure::Unavailable,
            Self::AfterOutput(error) => error.failure(),
            Self::Protocol(_) => Failure::Malformed,
            Self::Unserved { failure, .. } => *failure,
            Self::OverBudget { .. } => Failure::OverBudget,
        }
    }

    /// Whether trying again may work; see [`Failure::is_transient`]. An answer that broke off
    /// after output never is, whatever broke it: asking again would show that output twice.
    pub fn is_transient(&self) -> bool {
        !matches!(self, Self::AfterOutput(_)) && self.failure().is_transient()
    }
}

/// Phrases servers use when a request is longer than the model's window.
const TOO_LONG: &[&str] = &[
    "context length",
    "context window",
    "context_length",
    "maximum context",
    "too many tokens",
    "prompt is too long",
    "reduce the length",
];

/// Phrases servers use for a model they do not have.
const NO_MODEL: &[&str] = &[
    "not a valid model",
    "model not found",
    "model_not_found",
    "no such model",
    "unknown model",
    "does not exist",
];

fn from_status(status: u16, message: &str) -> Failure {
    let says = |phrases: &[&str]| phrases.iter().any(|phrase| message.contains(phrase));
    match status {
        401 | 403 => Failure::Refused,
        402 => Failure::NoCredit,
        413 => Failure::TooLong,
        429 => Failure::RateLimited,
        500..=599 => Failure::Unavailable,
        _ if says(TOO_LONG) => Failure::TooLong,
        404 => Failure::UnknownModel,
        _ if says(NO_MODEL) => Failure::UnknownModel,
        _ => Failure::Rejected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(status: u16, body: &str) -> Failure {
        ProviderError::Http {
            status,
            body: body.into(),
        }
        .failure()
    }

    #[test]
    fn statuses_and_messages_name_the_failure() {
        assert_eq!(http(401, "User not found."), Failure::Refused);
        assert_eq!(http(403, ""), Failure::Refused);
        assert_eq!(http(402, "Insufficient credits"), Failure::NoCredit);
        assert_eq!(
            http(400, "nosuch/model is not a valid model ID"),
            Failure::UnknownModel
        );
        assert_eq!(
            http(404, "model 'gemma9' not found, try pulling it first"),
            Failure::UnknownModel
        );
        assert_eq!(
            http(400, "This model's maximum context length is 32768 tokens"),
            Failure::TooLong
        );
        assert_eq!(
            http(404, "prompt is too long for this model"),
            Failure::TooLong,
            "the message wins over a vague status"
        );
        assert_eq!(http(413, ""), Failure::TooLong);
        assert_eq!(http(429, "slow down"), Failure::RateLimited);
        assert_eq!(http(503, "overloaded"), Failure::Unavailable);
        assert_eq!(
            http(400, "temperature must be at most 2"),
            Failure::Rejected
        );
        assert_eq!(
            ProviderError::Transport("refused".into()).failure(),
            Failure::Unreachable
        );
        assert_eq!(
            ProviderError::Timeout { seconds: 9 }.failure(),
            Failure::Silent
        );
        assert_eq!(
            ProviderError::Protocol("bad".into()).failure(),
            Failure::Malformed
        );
        assert_eq!(
            ProviderError::NoOutput("finish_reason error".into()).failure(),
            Failure::Unavailable
        );
        let unserved = ProviderError::Unserved {
            failure: Failure::TooLong,
            reason: "longer than every model's context".into(),
        };
        assert_eq!(unserved.failure(), Failure::TooLong);
        let over = ProviderError::OverBudget {
            budget: "person".into(),
            detail: "x".into(),
        };
        assert_eq!(over.failure(), Failure::OverBudget);
    }

    #[test]
    fn an_answer_that_broke_off_says_why_and_is_never_tried_again() {
        let overloaded = ProviderError::Http {
            status: 529,
            body: "Overloaded".into(),
        };
        assert!(overloaded.is_transient());
        let broke = ProviderError::AfterOutput(Box::new(overloaded));
        assert_eq!(broke.failure(), Failure::Unavailable);
        assert!(
            !broke.is_transient(),
            "asking again would repeat what was shown"
        );
    }

    #[test]
    fn only_failures_that_may_pass_are_transient() {
        let transient: Vec<Failure> = [
            Failure::Refused,
            Failure::NoCredit,
            Failure::UnknownModel,
            Failure::TooLong,
            Failure::RateLimited,
            Failure::Unavailable,
            Failure::Unreachable,
            Failure::Silent,
            Failure::Malformed,
            Failure::Rejected,
            Failure::OverBudget,
        ]
        .into_iter()
        .filter(|failure| failure.is_transient())
        .collect();
        assert_eq!(
            transient,
            [
                Failure::RateLimited,
                Failure::Unavailable,
                Failure::Unreachable,
                Failure::Silent
            ]
        );
    }
}
