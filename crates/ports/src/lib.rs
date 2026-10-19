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
pub mod computer;
#[cfg(any(test, feature = "conformance"))]
pub mod conformance;
mod env;
mod failure;
pub mod file;
mod in_memory;
mod log;
pub mod memories;
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
pub use env::{Clock, Entropy, EntropyError, OsEntropy, SystemClock};
pub use failure::Failure;
pub use in_memory::{InMemoryBlobs, InMemoryLog, InMemorySpaces, InMemoryTools};
pub use log::{EventLog, LogError, check_append, reserved_ledger};
pub use model::{
    Completion, Delta, Message, ModelProvider, ModelRequest, ProviderError, Purpose, RequestedCall,
    Served, ToolCall, ToolChoice, ToolSpec,
};
pub use scope::{OrgId, Scope, SpaceId};
pub use spaces::{FolderKey, MAX_NAME, Space, SpaceError, SpaceKind, Spaces, origin_remote};
pub use sse::{SseDecoder, SseError};
pub use tools::{
    Assessment, BoxFuture, Cancellation, Effect, NeverCancelled, ToolError, ToolInfo, ToolOutput,
    ToolSource, describe_call, unless_cancelled,
};
pub use view::{
    Standing, THREAD_VIEW_VERSION, TITLE_CHARS, ThreadCursor, ThreadQuery, ThreadView,
    fold_from_log, title_from, title_of,
};
