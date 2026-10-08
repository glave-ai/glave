//! glave's ports: the interfaces the harness depends on, so where data lives and which model
//! answers are configuration.
//!
//! Each port ships an in-memory implementation and a conformance suite every implementation
//! must pass. Ports are shaped as if they crossed a network: coarse calls, batched writes,
//! paged reads, serializable types. Every storage call takes a [`Scope`], and no call can
//! cross scopes.

mod blob;
#[cfg(any(test, feature = "conformance"))]
pub mod conformance;
mod failure;
mod log;
mod memory;
mod model;
mod scope;

pub use blob::{BlobError, BlobStore, digest};
pub use failure::Failure;
pub use log::{EventLog, LogError, check_append};
pub use memory::{MemoryBlobs, MemoryLog};
pub use model::{
    Completion, Delta, Message, ModelProvider, ModelRequest, ProviderError, RequestedCall,
    ToolCall, ToolSpec,
};
pub use scope::{OrgId, Scope, SpaceId};
