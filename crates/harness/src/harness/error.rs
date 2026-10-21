//! Why a turn could not run.

use glave_event::{CallId, EncodeError, InvalidValue, ThreadId};
use glave_ports::computer::ComputerError;
use glave_ports::{BlobError, EntropyError, LogError, ProviderError};
use thiserror::Error;

/// Why a turn could not run. Tool failures are not errors: the model sees them and adapts.
#[derive(Debug, Error)]
pub enum HarnessError {
    #[error(transparent)]
    Log(#[from] LogError),
    #[error(transparent)]
    Blob(#[from] BlobError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("could not build an event: {0}")]
    Event(String),
    #[error(transparent)]
    Entropy(#[from] EntropyError),
    #[error("thread {0} has no events in this project")]
    NotFound(ThreadId),
    /// The thread was archived, from outside this harness, while it held the thread.
    #[error("thread {0} is archived; unarchive it first")]
    Archived(ThreadId),
    #[error("cancelled")]
    Cancelled,
    #[error("thread {0} has no active goal")]
    NoGoal(ThreadId),
    /// The thread's workspace could not be opened, such as a worktree outside a repository.
    #[error("{}", .0.message())]
    Workspace(#[from] ComputerError),
    /// A `UserPromptSubmit` hook stopped the prompt; the model never saw it.
    #[error("a hook stopped this prompt: {0}")]
    PromptBlocked(String),
    /// What was asked for is not valid, such as an empty title.
    #[error("{0}")]
    Invalid(String),
    /// The turn waits for the person, or for a subagent that does: it stopped where its log
    /// shows the wait, and goes on from the log once an answer comes ([`crate::Harness::go_on`]).
    #[error("thread {0} waits for an answer")]
    Waits(ThreadId),
    /// No turn is open in the thread to go on with.
    #[error("no turn is open in thread {0}")]
    NoTurn(ThreadId),
    /// No turn in the thread waits for this approval.
    #[error("no approval {} is pending", .0.as_str())]
    NoApproval(CallId),
    /// No turn in the thread waits for answers to this call's questions.
    #[error("no question {} is waiting", .0.as_str())]
    NoQuestion(CallId),
}

impl From<EncodeError> for HarnessError {
    fn from(error: EncodeError) -> Self {
        Self::Event(error.to_string())
    }
}

impl From<InvalidValue> for HarnessError {
    fn from(error: InvalidValue) -> Self {
        Self::Event(error.to_string())
    }
}

impl From<glave_ports::stream::StreamError> for HarnessError {
    fn from(error: glave_ports::stream::StreamError) -> Self {
        match error {
            glave_ports::stream::StreamError::Log(error) => Self::Log(error),
            busy @ glave_ports::stream::StreamError::Busy { .. } => Self::Event(busy.to_string()),
        }
    }
}
