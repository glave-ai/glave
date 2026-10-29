//! How glave's own errors reach a client: one `From` for each, every one with a message a person
//! can act on. A failure of glave's records says so plainly; what the store said goes to the
//! server's log, not to the client.

use std::fmt::Display;

use glave_event::{DecodeError, EncodeError, InvalidValue};
use glave_harness::{HarnessError, Invalid, MAX_QUEUED, NotQueued};
use glave_identity::signin::{BrowserError, SignInError};
use glave_ports::computer::ComputerError;
use glave_ports::memories::MemoryError;
use glave_ports::stream::StreamError;
use glave_ports::{EntropyError, LogError, ProjectError};

use crate::accounts::AccountsError;
use crate::protocol::{ApiError, ErrorData, code};
use crate::server::no_thread;
use crate::words::Sentence;

impl ApiError {
    /// A failure of glave's own records, `error`, which goes to the server's log; the client
    /// is told what to do.
    pub fn internal(error: &dyn Display) -> Self {
        tracing::warn!(%error, "glave's records failed");
        Self::new(code::INTERNAL, Sentence::RecordsFailed)
    }
}

impl From<LogError> for ApiError {
    fn from(error: LogError) -> Self {
        match error {
            // Another writer took the place glave was writing to: trying again reads it.
            LogError::Conflict { .. } | LogError::Gap { .. } => {
                Self::new(code::INTERNAL, Sentence::WrittenElsewhere)
            }
            _ => Self::internal(&error),
        }
    }
}

impl From<StreamError> for ApiError {
    fn from(error: StreamError) -> Self {
        match error {
            StreamError::Log(error) => error.into(),
            StreamError::Busy { .. } => Self::new(code::INTERNAL, Sentence::RecordsBusy),
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
            ProjectError::Conflict(refusal) | ProjectError::Invalid(refusal) => {
                Self::new(code::INVALID_PARAMS, refusal)
            }
            ProjectError::Storage(_) => Self::internal(&error),
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
            BrowserError::Code => Self::new(code::NO_SIGN_IN, Sentence::LinkSpent),
            BrowserError::Ended => Self::new(code::NO_SIGN_IN, Sentence::SignInEnded),
            BrowserError::Elsewhere => Self::new(code::NO_SIGN_IN, Sentence::SignedInThere),
            BrowserError::Setup => Self::new(code::NO_SIGN_IN, Sentence::SetupSpent),
            BrowserError::TooMany => Self::new(code::NO_SIGN_IN, Sentence::TooManyLinks),
            BrowserError::Ceremony => Self::new(code::NO_SIGN_IN, Sentence::PasskeyExpired),
            BrowserError::Passkey(reason) => {
                Self::new(code::NO_SIGN_IN, Sentence::PasskeyRefused { reason })
            }
            BrowserError::Unknown(ref unknown) => Self {
                code: code::NO_SIGN_IN,
                message: Sentence::PasskeyUnknown.into(),
                data: Some(ErrorData::UnknownPasskey {
                    unknown_passkey: unknown.clone(),
                }),
            },
            BrowserError::NoPasskeys => Self::new(code::NOT_AVAILABLE, Sentence::NoPasskeysHere),
            BrowserError::NoPasskey => Self::new(code::NOT_AVAILABLE, Sentence::NoPasskeyYet),
            // Where and why go to the server's log; the person is told what to do.
            BrowserError::Keep { .. } | BrowserError::Random(_) => {
                tracing::warn!(%error, "could not sign a browser in");
                Self::new(code::INTERNAL, Sentence::BrowserNotKept)
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
                message: error.to_string().into(),
                data: Some(ErrorData::Failed {
                    failure: failed.failure(),
                }),
            },
            HarnessError::NotFound(thread) => no_thread(*thread),
            HarnessError::Archived(thread) => Self::new(
                code::ARCHIVED,
                Sentence::Archived {
                    thread: thread.to_string(),
                },
            ),
            HarnessError::Cancelled => Self::new(code::CANCELLED, Sentence::Cancelled),
            HarnessError::NoGoal(thread) => Self::new(
                code::NO_GOAL,
                Sentence::NoGoal {
                    thread: thread.to_string(),
                },
            ),
            HarnessError::NoPlan(thread) => Self::new(
                code::NO_PLAN,
                Sentence::NoPlan {
                    thread: thread.to_string(),
                },
            ),
            HarnessError::GoalInPlan(_) => Self::new(code::NOT_ALLOWED, Sentence::GoalInPlan),
            HarnessError::Workspace(ComputerError::Refused(reason)) => {
                Self::new(code::WORKSPACE, reason)
            }
            HarnessError::Workspace(_) => Self::new(code::WORKSPACE, error.to_string()),
            HarnessError::PromptBlocked(reason) => Self::new(
                code::PROMPT_BLOCKED,
                Sentence::PromptBlocked {
                    reason: reason.clone(),
                },
            ),
            HarnessError::Invalid(invalid) => {
                Self::new(code::INVALID_PARAMS, invalid_said(invalid))
            }
            HarnessError::Waits(thread) => crate::held::waiting(*thread),
            HarnessError::NoTurn(thread) => Self::new(
                code::NO_TURN,
                Sentence::NoOpenTurn {
                    thread: thread.to_string(),
                },
            ),
            HarnessError::NoApproval(call) => Self::new(
                code::NO_APPROVAL,
                Sentence::NoApproval {
                    approval: call.as_str().to_owned(),
                },
            ),
            HarnessError::NoQuestion(call) => Self::new(
                code::NO_QUESTION,
                Sentence::NoQuestion {
                    question: call.as_str().to_owned(),
                },
            ),
            HarnessError::Entropy(_) => Self::new(code::INTERNAL, error.to_string()),
            HarnessError::Log(log) => log.clone().into(),
            HarnessError::Blob(_) | HarnessError::Event(_) => Self::internal(error),
        }
    }
}

/// What was not valid, in the server's words.
fn invalid_said(invalid: &Invalid) -> Sentence {
    match invalid {
        Invalid::ComputerNotYet(computer) => Sentence::NoComputerYet {
            computer: computer.clone(),
        },
        Invalid::ModelId(most) => Sentence::ModelIdLength { most: *most },
        Invalid::EffortLevel(most) => Sentence::EffortWord { most: *most },
        Invalid::OnlyPlan => Sentence::NothingBuilt,
        Invalid::Steps(_) => crate::plan::steps_invalid(),
        Invalid::Title(most) => Sentence::TitleLength { most: *most },
    }
}

/// A message refused because as many wait as a turn takes.
impl From<NotQueued> for ApiError {
    fn from(refused: NotQueued) -> Self {
        match refused {
            NotQueued::Full => {
                Self::new(code::QUEUE_FULL, Sentence::QueueFull { most: MAX_QUEUED })
            }
            // Only a turn ending refuses it otherwise, and then it is not asked for.
            NotQueued::Ending => Self::new(code::INTERNAL, Sentence::TurnEnded),
        }
    }
}

/// Why a sign-in was refused, in the server's words: what is wrong with it, never the token.
pub(crate) fn refused(error: &SignInError) -> Sentence {
    match error {
        SignInError::Missing => Sentence::NoSignIn,
        SignInError::Malformed(reason) => Sentence::TokenMalformed {
            reason: reason.clone(),
        },
        SignInError::UnknownIssuer => Sentence::UnknownIssuer,
        SignInError::Keys { issuer, reason } => Sentence::IssuerKeys {
            issuer: issuer.clone(),
            reason: reason.clone(),
        },
        SignInError::UnknownKey { issuer } => Sentence::UnknownKey {
            issuer: issuer.clone(),
        },
        SignInError::Invalid(reason) => Sentence::TokenInvalid {
            reason: reason.clone(),
        },
        SignInError::Expired => Sentence::TokenExpired,
        SignInError::NotYet => Sentence::TokenNotYet,
        SignInError::Insecure(issuer) => Sentence::IssuerInsecure {
            issuer: issuer.clone(),
        },
        SignInError::NoIssuers => Sentence::NoIssuers,
        SignInError::Client(reason) => Sentence::KeysClient {
            reason: reason.clone(),
        },
        SignInError::NotLoopback => Sentence::LocalTokenHereOnly,
        SignInError::WrongLocalToken => Sentence::WrongLocalToken,
        SignInError::EndedSession => Sentence::SignInEnded,
        SignInError::OnlyThere => Sentence::SignedInThereOnly,
    }
}

#[cfg(test)]
mod tests {
    use glave_ports::{ProjectId, ProjectRefusal};

    use crate::protocol::Told;

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
            assert_eq!(error.message, Told::from(Sentence::RecordsFailed));
        }
    }

    #[test]
    fn what_the_person_can_fix_keeps_its_own_words() {
        let invalid = ApiError::from(ProjectError::Invalid(ProjectRefusal::NameLength {
            most: "200".into(),
        }));
        assert_eq!(
            (invalid.code, &*invalid.message),
            (
                code::INVALID_PARAMS,
                "a project's name is 1 to 200 characters, not only spaces"
            )
        );
        let project = ProjectId::from_uuid(uuid::Uuid::nil());
        assert_eq!(
            ApiError::from(ProjectError::NotFound(project)).code,
            code::NO_PROJECT
        );
    }

    #[test]
    fn what_other_crates_refuse_with_reads_as_it_did_and_is_written_again() {
        use glave_event::{CallId, ThreadId};

        use crate::words::{Language, say};

        let thread = ThreadId::new(jiff::Timestamp::UNIX_EPOCH, [0; 10]).unwrap();
        let call = || CallId::new("call_1").unwrap();
        let harness = [
            HarnessError::Archived(thread),
            HarnessError::NoGoal(thread),
            HarnessError::NoTurn(thread),
            HarnessError::NoApproval(call()),
            HarnessError::NoQuestion(call()),
            HarnessError::PromptBlocked("it names a secret".into()),
            HarnessError::Invalid(Invalid::ModelId(255)),
            HarnessError::Invalid(Invalid::EffortLevel(32)),
            HarnessError::Invalid(Invalid::Title(80)),
        ];
        let mut told: Vec<(String, ApiError)> = harness
            .iter()
            .map(|error| (error.to_string(), ApiError::from(error)))
            .collect();
        let browsers = [
            BrowserError::Code,
            BrowserError::Setup,
            BrowserError::TooMany,
            BrowserError::Ended,
            BrowserError::Elsewhere,
            BrowserError::NoPasskeys,
            BrowserError::NoPasskey,
            BrowserError::Ceremony,
            BrowserError::Passkey("the challenge did not match".into()),
        ];
        told.extend(browsers.map(|error| (error.to_string(), ApiError::from(error))));
        told.push((NotQueued::Full.to_string(), ApiError::from(NotQueued::Full)));
        let busy = StreamError::Busy { thread };
        told.push((busy.to_string(), ApiError::from(busy)));
        let projects = ProjectRefusal::every().into_iter().flat_map(|refusal| {
            [
                ProjectError::Invalid(refusal.clone()),
                ProjectError::Conflict(refusal),
            ]
        });
        told.extend(projects.map(|error| (error.to_string(), ApiError::from(error))));
        for (english, error) in told {
            assert_eq!(&*error.message, english.as_str());
            let mut written = error.message.clone();
            written.translate(Language::Accented);
            assert!(
                written.starts_with('['),
                "{english} is not the server's own"
            );
        }
        let signins = [
            SignInError::Missing,
            SignInError::Malformed("no dots".into()),
            SignInError::UnknownIssuer,
            SignInError::Keys {
                issuer: "https://id.example".into(),
                reason: "timed out".into(),
            },
            SignInError::UnknownKey {
                issuer: "https://id.example".into(),
            },
            SignInError::Invalid("bad audience".into()),
            SignInError::Expired,
            SignInError::NotYet,
            SignInError::Insecure("http://id.example".into()),
            SignInError::NoIssuers,
            SignInError::Client("no roots".into()),
            SignInError::NotLoopback,
            SignInError::WrongLocalToken,
            SignInError::EndedSession,
            SignInError::OnlyThere,
        ];
        for error in signins {
            assert_eq!(say(Language::English, &refused(&error)), error.to_string());
        }
    }
}
