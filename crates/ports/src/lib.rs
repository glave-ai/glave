//! glave's ports: the interfaces the harness depends on, so where data lives and which model
//! answers are configuration.
//!
//! Each port ships an in-memory implementation and a conformance suite every implementation
//! must pass. Ports are shaped as if they crossed a network: coarse calls, batched writes,
//! paged reads, serializable types. Every storage call takes a [`Scope`], and no call can
//! cross scopes. The computer port's in-process implementation runs real processes, so it
//! lives in `glave-computer` (this machine) rather than here.

mod blob;
mod computer;
#[cfg(any(test, feature = "conformance"))]
pub mod conformance;
mod failure;
mod log;
mod memory;
mod model;
mod scope;
pub mod secrets;
mod spaces;
mod tools;

pub use blob::{BlobError, BlobStore, digest};
pub use computer::{
    Bytes, Command, ComputerError, ComputerId, ComputerInfo, Computers, Ending, Entry, EntryKind,
    Environment, FileWrite, Finished, GitCredential, Glob, Globbed, Image, Listed, Listing, Match,
    Network, Output, PROTECTED, Sandbox, SandboxLevel, Save, Saved, Search, Searched, Size,
    Sources, Stat, State, WorkspaceId, WorkspaceInfo, is_protected, is_protected_refusal,
    looks_binary, refuse_protected,
};
pub use failure::Failure;
pub use log::{EventLog, LogError, check_append};
pub use memory::{MemoryBlobs, MemoryLog, MemoryTools};
pub use model::{
    Completion, Delta, Message, ModelProvider, ModelRequest, ProviderError, Purpose, RequestedCall,
    Served, ToolCall, ToolChoice, ToolSpec,
};
pub use scope::{OrgId, Scope, SpaceId};
pub use spaces::{FolderKey, MAX_NAME, MemorySpaces, Space, SpaceError, SpaceKind, Spaces};
pub use tools::{
    Assessment, BoxFuture, Cancellation, Effect, ToolError, ToolInfo, ToolOutput, ToolSource,
    describe_call, unless_cancelled,
};
