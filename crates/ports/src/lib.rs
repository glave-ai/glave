//! glave's ports: the interfaces the harness depends on, so where data lives and which model
//! answers are configuration.
//!
//! Each port ships an in-memory implementation and a conformance suite every implementation
//! must pass. Ports are shaped as if they crossed a network: coarse calls, batched writes,
//! paged reads, serializable types. Every storage call takes a [`Scope`], and no call can
//! cross scopes. The computer port's in-process implementation runs real processes, so it
//! lives in `glave-computer` (this machine) rather than here; the memory port's is glave's
//! memory server over any event log, in `glave-memory`.

pub mod auth;
mod blob;
mod caller;
mod computer;
#[cfg(any(test, feature = "conformance"))]
pub mod conformance;
mod env;
mod failure;
mod log;
pub mod memories;
mod memory;
mod model;
mod scope;
pub mod search;
pub mod secrets;
mod spaces;
mod sse;
#[cfg(any(test, feature = "conformance"))]
pub mod testing;
mod tools;
mod view;

pub use blob::{BlobError, BlobStore, digest};
pub use caller::Caller;
pub use computer::{
    Background, Bytes, Command, CommandId, ComputerError, ComputerId, ComputerInfo, Computers,
    CredentialAsk, CredentialHost, Credentials, Ending, Entry, EntryKind, Environment, FileWrite,
    Finished, Forge, ForgeTool, GitCredential, Glob, Globbed, IDLE, Image, KEPT_OUTPUT, Listed,
    Listing, MAX_BACKGROUND, MAX_WAIT, Match, Network, OnTimeout, Output, PROTECTED, Printed,
    Sandbox, SandboxLevel, Save, Saved, Search, Searched, Size, Sources, Stat, State, WorkspaceId,
    WorkspaceInfo, is_protected, is_protected_refusal, looks_binary, refuse_protected,
};
pub use env::{Clock, Entropy, OsEntropy, SystemClock};
pub use failure::Failure;
pub use log::{EventLog, LogError, check_append};
pub use memory::{MemoryBlobs, MemoryLog, MemoryTools};
pub use model::{
    Completion, Delta, Message, ModelProvider, ModelRequest, ProviderError, Purpose, RequestedCall,
    Served, ToolCall, ToolChoice, ToolSpec,
};
pub use scope::{OrgId, Scope, SpaceId};
pub use spaces::{
    FolderKey, MAX_NAME, MemorySpaces, Space, SpaceError, SpaceKind, Spaces, origin_remote,
};
pub use sse::{SseDecoder, SseError};
pub use tools::{
    Assessment, BoxFuture, Cancellation, Effect, NeverCancelled, ToolError, ToolInfo, ToolOutput,
    ToolSource, describe_call, unless_cancelled,
};
pub use view::{
    Standing, THREAD_VIEW_VERSION, TITLE_CHARS, ThreadCursor, ThreadQuery, ThreadView,
    fold_from_log, title_from, title_of,
};
