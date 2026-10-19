//! The memory port: what glave remembers from one thread for the next (`specs/memory.md`), as
//! the harness and the session API see it through [`Memory`]: the model's tools (`remember`,
//! `edit`, `forget`, served by an MCP server), the current memories for a thread, and the
//! person's own controls. Scope comes from the thread, never from the model, and the store
//! enforces it on every read.

use glave_event::{Arguments, At, MemoryId, MemoryScope, MemorySource, PrincipalId, Seq, ThreadId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::scope::Scope;
use crate::tools::{BoxFuture, ToolError, ToolInfo, ToolOutput};

/// The memory server's name, so its tools reach the model as `mcp__memory__<tool>`.
pub const MEMORY_SERVER: &str = "memory";
/// The tool that saves a memory.
pub const REMEMBER: &str = "mcp__memory__remember";
/// The tool that changes a memory's text.
pub const EDIT_MEMORY: &str = "mcp__memory__edit";
/// The tool that forgets a memory.
pub const FORGET: &str = "mcp__memory__forget";
/// The tool that searches memories and past conversations by keyword.
pub const RECALL: &str = "mcp__memory__recall";

/// Who and where a memory request is for: always from the session or the thread, never from
/// the model. A memory server receives it in each request's `_meta`, flat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryContext {
    /// The person the thread works for; personal memory is theirs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<PrincipalId>,
    /// The agent that runs the thread: who a memory the model saves is `by`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<PrincipalId>,
    /// The thread's organization and project.
    #[serde(flatten)]
    pub scope: Scope,
    /// The thread asking, and the `seq` a save in it is recorded at.
    pub thread: ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<Seq>,
    /// Whether the thread has read untrusted content or a secrets file: a memory saved now
    /// records `untrusted_read`.
    #[serde(default)]
    pub untrusted_read: bool,
}

/// One memory as it is shown: to the model, and to the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Remembered {
    pub memory: MemoryId,
    /// 1 when saved, one more with each edit.
    pub version: u32,
    pub scope: MemoryScope,
    pub text: String,
    pub saved_at: At,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<At>,
    /// Saved in a thread that had read untrusted content or a secrets file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub untrusted_read: bool,
    /// The thread and `seq` it was saved in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<MemorySource>,
}

/// A memory at one version: what a turn records it was shown, never the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Shown {
    pub memory: MemoryId,
    pub version: u32,
}

impl Remembered {
    pub fn shown(&self) -> Shown {
        Shown {
            memory: self.memory,
            version: self.version,
        }
    }
}

/// What a call of a memory tool did, beyond what the model reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryChange {
    /// A memory was saved, or replaced one.
    Saved {
        memory: Remembered,
        replaced: Option<MemoryId>,
    },
    /// A memory's text changed.
    Edited(Remembered),
    /// A memory was forgotten.
    Forgotten(MemoryId),
}

/// A memory tool's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryOutput {
    pub output: ToolOutput,
    pub change: Option<MemoryChange>,
}

impl MemoryOutput {
    /// A failure the model reads, which changed nothing.
    pub fn failed(text: impl Into<String>) -> Self {
        Self {
            output: ToolOutput::text(true, text),
            change: None,
        }
    }
}

/// Why a memory request was refused or failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MemoryError {
    /// No such memory in the scopes the caller may read.
    #[error("{0}")]
    NotFound(String),
    /// The caller may not do this: another person's memory, or a scope the project does not allow.
    #[error("{0}")]
    Refused(String),
    /// The request itself is wrong: too long, a credential, over the budget.
    #[error("{0}")]
    Invalid(String),
    /// Memory is off here.
    #[error("{0}")]
    Off(String),
    /// The store failed.
    #[error("memory storage failed: {0}")]
    Storage(String),
}

impl From<crate::stream::StreamError> for MemoryError {
    fn from(error: crate::stream::StreamError) -> Self {
        Self::Storage(error.to_string())
    }
}

/// The person's own controls, from any client (`specs/memory.md`, "Seeing, editing and
/// forgetting"). They are the person's actions, not the model's: no tool call, no judge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    /// The memories of `scope`, or of both scopes the project allows.
    List {
        scope: Option<MemoryScope>,
    },
    Save {
        text: String,
        scope: MemoryScope,
    },
    Edit {
        memory: MemoryId,
        text: String,
    },
    /// Saved in the new scope, forgotten in the old.
    Move {
        memory: MemoryId,
        scope: MemoryScope,
    },
    Forget {
        memory: MemoryId,
    },
    /// Every memory of `scope`.
    ForgetAll {
        scope: MemoryScope,
    },
    /// Undoes glave's latest change to it, within 30 days.
    Restore {
        memory: MemoryId,
    },
}

/// Who asks for a control, and in which project: the verified caller, never a parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access {
    pub person: Option<PrincipalId>,
    pub scope: Scope,
}

/// Memory, as the harness and the session API use it. glave's own is an in-process MCP server
/// over the data store (`glave-memory`); any implementation must enforce scope itself, since
/// the model's arguments never name a project.
pub trait Memory: Send + Sync {
    /// Whether memory is read and written at all in threads of `on`: the project's setting.
    /// When it is not, the model is sent exactly what it would be without memory.
    fn active(&self, on: &MemoryContext) -> bool;

    /// The tools on offer to the model.
    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>>;

    /// Runs one call of a memory tool for the thread `on` describes.
    fn call<'a>(
        &'a self,
        on: &'a MemoryContext,
        name: &'a str,
        arguments: &'a Arguments,
    ) -> BoxFuture<'a, MemoryOutput>;

    /// The memories a thread of `on` reads now, oldest first.
    fn list<'a>(
        &'a self,
        on: &'a MemoryContext,
    ) -> BoxFuture<'a, Result<Vec<Remembered>, MemoryError>>;

    /// The memories in `wanted` at those versions, for rebuilding what a thread was shown.
    /// Forgotten ones, whose keys are destroyed, are left out.
    fn versions<'a>(
        &'a self,
        on: &'a MemoryContext,
        wanted: &'a [Shown],
    ) -> BoxFuture<'a, Result<Vec<Remembered>, MemoryError>>;

    /// Pairs of the memories a thread of `on` reads that tidy-up could not reconcile: both are
    /// kept until the person says which holds. None by default.
    fn questions<'a>(
        &'a self,
        _on: &'a MemoryContext,
    ) -> BoxFuture<'a, Result<Vec<[MemoryId; 2]>, MemoryError>> {
        Box::pin(std::future::ready(Ok(Vec::new())))
    }

    /// One of the person's controls, by `by`. Returns the memories it lists or changed.
    fn control<'a>(
        &'a self,
        by: &'a Access,
        control: Control,
    ) -> BoxFuture<'a, Result<Vec<Remembered>, MemoryError>>;
}
