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
    /// No plan in the thread waits for the person's approval, to edit or build.
    #[error("thread {0} has no plan waiting for approval")]
    NoPlan(ThreadId),
    /// A goal does not run in plan mode, which stops at a plan for the person to build.
    #[error("thread {0} is in plan mode, where a goal does not run")]
    GoalInPlan(ThreadId),
    /// The thread's workspace could not be opened, such as a worktree outside a repository.
    #[error("{}", .0.message())]
    Workspace(#[from] ComputerError),
    /// A `UserPromptSubmit` hook stopped the prompt; the model never saw it.
    #[error("a hook stopped this prompt: {0}")]
    PromptBlocked(String),
    /// What was asked for is not valid, such as an empty title.
    #[error(transparent)]
    Invalid(#[from] Invalid),
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

/// What was asked for that is not valid, by what was wrong with it.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Invalid {
    /// A thread on a computer of the person's own, which cannot dial in yet.
    #[error(
        "a thread on computer {0} is not available yet: computers cannot dial in to a server \
         until a later version"
    )]
    ComputerNotYet(String),
    /// A model's id, at most this many bytes.
    #[error("a model id is 1 to {0} bytes, not only spaces")]
    ModelId(usize),
    /// A reasoning level, at most this many letters.
    #[error("a reasoning level is one word of 1 to {0} letters, such as high")]
    EffortLevel(usize),
    /// A plan to build where only Plan is allowed.
    #[error("only Plan is allowed here, so no plan can be built")]
    OnlyPlan,
    /// Steps a plan cannot hold, saying why as the model is told it.
    #[error("{0}")]
    Steps(String),
    /// A title, at most this many characters.
    #[error("a title is 1 to {0} characters, not only spaces")]
    Title(usize),
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
