//! How glave's own errors reach a client: one `From` for each, every one with a message a person
//! can act on. A failure of glave's records says so plainly; what the store said goes to the
//! server's log, not to the client.

use std::fmt::Display;

use glave_event::{DecodeError, EncodeError, InvalidValue};
use glave_harness::HarnessError;
use glave_identity::signin::BrowserError;
use glave_ports::memories::MemoryError;
use glave_ports::stream::StreamError;
use glave_ports::{EntropyError, LogError, ProjectError};

use crate::accounts::AccountsError;
use crate::protocol::{ApiError, ErrorData, code};
use crate::server::no_thread;

/// What a person is told when glave's records fail them.
const RECORDS_FAILED: &str =
    "glave could not read or save its records: try again, and restart glave if it keeps happening";

impl ApiError {
    /// A failure of glave's own records, `error`, which goes to the server's log; the client
    /// is told what to do.
    pub fn internal(error: &dyn Display) -> Self {
        tracing::warn!(%error, "glave's records failed");
        Self::new(code::INTERNAL, RECORDS_FAILED)
    }
}

/// Another writer took the place glave was writing to: trying again reads what it wrote.
const WRITTEN_ELSEWHERE: &str =
    "this changed elsewhere while glave was saving it: look again, then try again";

impl From<LogError> for ApiError {
    fn from(error: LogError) -> Self {
        match error {
            LogError::Conflict { .. } | LogError::Gap { .. } => {
                Self::new(code::INTERNAL, WRITTEN_ELSEWHERE)
            }
            _ => Self::internal(&error),
        }
    }
}

impl From<StreamError> for ApiError {
    fn from(error: StreamError) -> Self {
        match error {
            StreamError::Log(error) => error.into(),
            busy @ StreamError::Busy { .. } => Self::new(code::INTERNAL, busy.to_string()),
        }
    }
}

impl From<InvalidValue> for ApiError {
    fn from(error: InvalidValue) -> Self {
        Self::internal(&error)
    }
}

impl From<EncodeError> for ApiError {
    fn from(error: EncodeError) -> Self {
        Self::internal(&error)
    }
}

impl From<DecodeError> for ApiError {
    fn from(error: DecodeError) -> Self {
        Self::internal(&error)
    }
}

/// The operating system's random source failed: its message already says what to do.
impl From<EntropyError> for ApiError {
    fn from(error: EntropyError) -> Self {
        Self::new(code::INTERNAL, error.to_string())
    }
}

impl From<AccountsError> for ApiError {
    fn from(error: AccountsError) -> Self {
        match error {
            AccountsError::Stream(error) => error.into(),
            AccountsError::Entropy(error) => error.into(),
            AccountsError::Erase(_) | AccountsError::Event(_) => Self::internal(&error),
        }
    }
}

impl From<ProjectError> for ApiError {
    fn from(error: ProjectError) -> Self {
        match error {
            ProjectError::NotFound(project) => crate::projects::no_project(project),
            ProjectError::Conflict(_) | ProjectError::Invalid(_) => {
                Self::new(code::INVALID_PARAMS, error.to_string())
            }
            ProjectError::Changed(_) | ProjectError::Storage(_) => Self::internal(&error),
        }
    }
}

impl From<MemoryError> for ApiError {
    fn from(error: MemoryError) -> Self {
        let code = match error {
            MemoryError::NotFound(_) => code::NO_MEMORY,
            MemoryError::Refused(_) => code::NOT_ALLOWED,
            MemoryError::Invalid(_) => code::INVALID_PARAMS,
            MemoryError::Off(_) => code::MEMORY_OFF,
            MemoryError::Storage(_) => return Self::internal(&error),
        };
        Self::new(code, error.to_string())
    }
}

impl From<BrowserError> for ApiError {
    fn from(error: BrowserError) -> Self {
        match error {
            BrowserError::Code
            | BrowserError::Ended
            | BrowserError::Setup
            | BrowserError::TooMany
            | BrowserError::Ceremony
            | BrowserError::Passkey(_) => Self::new(code::NO_SIGN_IN, error.to_string()),
            BrowserError::NoPasskeys | BrowserError::NoPasskey => {
                Self::new(code::NOT_AVAILABLE, error.to_string())
            }
            // Where and why go to the server's log; the person is told what to do.
            BrowserError::Keep { .. } | BrowserError::Random(_) => {
                tracing::warn!(%error, "could not sign a browser in");
                Self::new(
                    code::INTERNAL,
                    "glave could not keep this browser's sign-in: try again",
                )
            }
        }
    }
}

/// A harness failure as the API reports it.
impl From<&HarnessError> for ApiError {
    fn from(error: &HarnessError) -> Self {
        match error {
            HarnessError::Provider(failed) => Self {
                code: code::PROVIDER,
                message: error.to_string(),
                data: Some(ErrorData {
                    failure: failed.failure(),
                }),
            },
            HarnessError::NotFound(thread) => no_thread(*thread),
            HarnessError::Archived(_) => Self::new(code::ARCHIVED, error.to_string()),
            HarnessError::Cancelled => Self::new(code::CANCELLED, "cancelled"),
            HarnessError::NoGoal(_) => Self::new(code::NO_GOAL, error.to_string()),
            HarnessError::Workspace(_) => Self::new(code::WORKSPACE, error.to_string()),
            HarnessError::PromptBlocked(_) => Self::new(code::PROMPT_BLOCKED, error.to_string()),
            HarnessError::Invalid(_) => Self::new(code::INVALID_PARAMS, error.to_string()),
            HarnessError::Entropy(_) => Self::new(code::INTERNAL, error.to_string()),
            HarnessError::Log(log) => log.clone().into(),
            HarnessError::Blob(_) | HarnessError::Event(_) => Self::internal(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use glave_ports::ProjectId;

    use super::*;

    #[test]
    fn a_storage_failure_tells_the_person_what_to_do_not_what_the_store_said() {
        let said = "disk I/O error at /var/lib/glave/glave.db";
        let errors = [
            ApiError::from(LogError::Storage(said.into())),
            ApiError::from(ProjectError::Storage(said.into())),
            ApiError::from(MemoryError::Storage(said.into())),
            ApiError::from(&HarnessError::Event(said.into())),
        ];
        for error in errors {
            assert_eq!(error.code, code::INTERNAL);
            assert_eq!(error.message, RECORDS_FAILED);
        }
    }

    #[test]
    fn what_the_person_can_fix_keeps_its_own_words() {
        let invalid = ApiError::from(ProjectError::Invalid("the name is empty".into()));
        assert_eq!(
            (invalid.code, invalid.message.as_str()),
            (code::INVALID_PARAMS, "the name is empty")
        );
        let project = ProjectId::from_uuid(uuid::Uuid::nil());
        assert_eq!(
            ApiError::from(ProjectError::NotFound(project)).code,
            code::NO_PROJECT
        );
    }
}
