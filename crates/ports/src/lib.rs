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
mod erase;
mod failure;
pub mod file;
pub mod host;
mod in_memory;
mod leases;
mod log;
pub mod memories;
mod model;
mod objects;
mod projects;
mod remove;
mod scope;
pub mod search;
pub mod secrets;
pub mod settings;
mod sse;
pub mod stream;
#[cfg(any(test, feature = "conformance"))]
pub mod testing;
mod tools;
mod view;

pub use blob::{
    Blob, BlobError, BlobStore, Chunks, Pieces, Swept, UNREFERENCED_FOR_MS, digest, read_all,
};
pub use caller::Caller;
pub use env::{Clock, Entropy, EntropyError, OsEntropy, SystemClock};
pub use erase::{Erasable, EraseError, Eraser};
pub use failure::Failure;
pub use in_memory::{
    InMemoryBlobs, InMemoryLeases, InMemoryLog, InMemoryObjects, InMemoryProjects, InMemoryTools,
};
pub use leases::{Holder, Lease, LeaseError, Leased, Leases, ttl_millis};
pub use log::{EventLog, LogError, check_append};
pub use model::{
    Completion, Delta, InlineFile, Message, ModelProvider, ModelRequest, ProviderError, Purpose,
    RequestedCall, Served, ToolCall, ToolChoice, ToolSpec,
};
pub use objects::{Listed, ObjectWriter, Objects, object_name_ok};
pub use projects::{
    FolderKey, MAX_NAME, Project, ProjectError, ProjectKind, Projects, origin_remote,
};
pub use remove::remove_tree;
pub use scope::{Located, OrgId, ProjectId, Scope};
pub use sse::{SseDecoder, SseError};
pub use tools::{
    Assessment, BoxFuture, Cancellation, Effect, NeverCancelled, ToolError, ToolInfo, ToolOutput,
    ToolSource, describe_call, unless_cancelled,
};
pub use view::{
    Standing, THREAD_VIEW_VERSION, TITLE_CHARS, ThreadCursor, ThreadQuery, ThreadView,
    fold_from_log, title_from, title_of,
};
